#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-library-probe.XXXXXX)
collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-pthread-fixture"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
iterations=200

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

for command in g++ jq python3 sudo; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"

"$fixture" --iterations "$iterations" --delay-ms 1200 \
  >"$verify_dir/workload.json" 2>"$verify_dir/workload.log" &
fixture_pid=$!
sudo -n "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --pid "$fixture_pid" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --uprobe-module "libc.so.6:pthread_mutex_lock=pthread.mutex.lock" \
  --sample-hz 0 \
  --duration-secs 3 2>"$verify_dir/collector.log"
wait "$fixture_pid"
fixture_pid=

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
events_path="$verify_dir/events.txt" expected="$iterations" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and fields[2] == "645" and fields[9] == "pthread.mutex.lock":
            spans.append(fields)
expected = int(os.environ["expected"])
assert len(spans) == expected, (len(spans), expected)
assert all(int(fields[1]) > 0 and int(fields[7]) == 0 for fields in spans)
PY

"$verify_dir/eventtospan3" "Mapped pthread library probe" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e --argjson expected "$iterations" \
  '.version == 3
   and ([.events[] | select(.[5] == 645 and .[9] == "pthread.mutex.lock")] | length) == $expected' \
  "$verify_dir/capture.json" >/dev/null
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
grep -q 'probe_dropped_events=0' "$verify_dir/collector.log"

echo "module=libc.so.6 symbol=pthread_mutex_lock spans=$iterations strict_legacy_json=true bpf_dropped_events=0 probe_dropped_events=0"
