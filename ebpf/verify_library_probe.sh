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

for command in cc g++ jq python3 sudo; do
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

cc -O2 -Wall -Wextra -Werror -fPIC -shared \
  "$root_dir/kutrace-dlopen-fixture/late_probe.c" \
  -o "$verify_dir/libkutrace_late.so"
cc -O2 -Wall -Wextra -Werror \
  "$root_dir/kutrace-dlopen-fixture/main.c" \
  -ldl -o "$verify_dir/kutrace-dlopen-fixture"

"$verify_dir/kutrace-dlopen-fixture" \
  "$verify_dir/libkutrace_late.so" 900 400 "$iterations" \
  >"$verify_dir/dlopen-workload.json" 2>"$verify_dir/dlopen-workload.log" &
fixture_pid=$!
sudo -n "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/dlopen-capture.kuevents" \
  --pid "$fixture_pid" \
  --agent-shm "$verify_dir/dlopen-agent.shm" \
  --agent-socket "$verify_dir/dlopen-agent.sock" \
  --uprobe-module "libkutrace_late.so:kutrace_late_probe=late.module.call" \
  --wait-for-modules \
  --sample-hz 0 \
  --duration-secs 3 2>"$verify_dir/dlopen-collector.log"
wait "$fixture_pid"
fixture_pid=

"$transform" events "$verify_dir/dlopen-capture.kuevents" \
  --output "$verify_dir/dlopen-events.txt"
events_path="$verify_dir/dlopen-events.txt" expected="$iterations" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and fields[2] == "645" and fields[9] == "late.module.call":
            spans.append(fields)
expected = int(os.environ["expected"])
assert len(spans) == expected, (len(spans), expected)
assert all(int(fields[1]) > 0 and int(fields[7]) == 0 for fields in spans)
PY

"$verify_dir/eventtospan3" "Late dlopen module probe" \
  <"$verify_dir/dlopen-events.txt" >"$verify_dir/dlopen-capture.json"
jq -e --argjson expected "$iterations" \
  '.version == 3
   and ([.events[] | select(.[5] == 645 and .[9] == "late.module.call")] | length) == $expected' \
  "$verify_dir/dlopen-capture.json" >/dev/null
grep -q 'waiting for mapped module probe: libkutrace_late.so' \
  "$verify_dir/dlopen-collector.log"
grep -q 'late mapped module probe attached: libkutrace_late.so' \
  "$verify_dir/dlopen-collector.log"
grep -q 'module_probes_attached=1' "$verify_dir/dlopen-collector.log"
grep -q 'module_probes_unresolved=0' "$verify_dir/dlopen-collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/dlopen-collector.log"
grep -q 'probe_dropped_events=0' "$verify_dir/dlopen-collector.log"

echo "late_module=libkutrace_late.so symbol=kutrace_late_probe spans=$iterations strict_legacy_json=true module_probes_attached=1 module_probes_unresolved=0 bpf_dropped_events=0 probe_dropped_events=0"
