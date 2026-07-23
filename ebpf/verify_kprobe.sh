#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-kprobe-verify.XXXXXX)
collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-bench"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
kernel_symbol=${KUTRACE_KPROBE_VERIFY_SYMBOL:-__do_sys_getpid}

fixture_pid=
cleanup() {
  local status=$?
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  find "$verify_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$verify_dir"
  exit "$status"
}
trap cleanup EXIT INT TERM

for command in awk g++ jq sudo; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done
sudo -n awk -v symbol="$kernel_symbol" '$1 == symbol {found=1} END {exit !found}' \
  /sys/kernel/tracing/available_filter_functions || {
  echo "kernel function is not traceable: $kernel_symbol" >&2
  exit 1
}

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"

"$fixture" --mode getpid --iterations 100 --samples 4 --delay-ms 1200 \
  >"$verify_dir/workload.json" 2>"$verify_dir/workload.log" &
fixture_pid=$!
sudo -n "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --pid "$fixture_pid" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --kprobe "$kernel_symbol=kernel.getpid" \
  --sample-hz 0 \
  --duration-secs 3 2>"$verify_dir/collector.log"
wait "$fixture_pid"
fixture_pid=

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
events_path="$verify_dir/events.txt" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and fields[2] == "645" and fields[9] == "kernel.getpid":
            spans.append(fields)
assert len(spans) == 412, len(spans)
assert all(int(fields[1]) > 0 for fields in spans)
assert all(int(fields[7]) == 0 for fields in spans)
PY

"$verify_dir/eventtospan3" "Generic kernel probe integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e \
  '.version == 3
   and ([.events[] | select(.[5] == 645 and .[9] == "kernel.getpid")] | length) == 412' \
  "$verify_dir/capture.json" >/dev/null
grep -q "paired generic kernel probe attached: $kernel_symbol as kernel.getpid" \
  "$verify_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
grep -q 'probe_dropped_events=0' "$verify_dir/collector.log"

echo "kernel_symbol=$kernel_symbol spans=412 roots=412 strict_legacy_json=true bpf_dropped_events=0 probe_dropped_events=0"
