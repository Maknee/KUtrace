#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-usdt-verify.XXXXXX)
cleanup() {
  find "$verify_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$verify_dir"
}
trap cleanup EXIT

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-usdt-fixture"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

"$fixture" >"$verify_dir/fixture.json" 2>"$verify_dir/fixture.log" &
fixture_pid=$!
sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --pid "$fixture_pid" \
  --uprobe "$fixture:kutrace_usdt_fixture=agent.external" \
  --usdt "$fixture:kutrace:agent_begin:agent_end=agent.usdt" \
  --sample-hz 0 \
  --duration-secs 2.5 2>"$verify_dir/collector.log" &
collector_one=$!
sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture-two.kuevents" \
  --agent-shm "$verify_dir/agent-two.shm" \
  --agent-socket "$verify_dir/agent-two.sock" \
  --pid "$fixture_pid" \
  --usdt "$fixture:kutrace:agent_begin:agent_end=agent.usdt" \
  --sample-hz 0 \
  --duration-secs 2.5 2>"$verify_dir/collector-two.log" &
collector_two=$!
wait "$collector_one"
wait "$collector_two"
wait "$fixture_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
"$transform" events "$verify_dir/capture-two.kuevents" --output "$verify_dir/events-two.txt"
events_path="$verify_dir/events.txt" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and not line.startswith("#") and fields[2] == "645":
            spans.append(fields)
roots = sum(int(fields[7]) == 0 for fields in spans)
children = sum(int(fields[7]) != 0 for fields in spans)
labels = {}
for fields in spans:
    labels[fields[9]] = labels.get(fields[9], 0) + 1
assert (len(spans), roots, children) == (800, 100, 700)
assert all(int(fields[1]) > 0 for fields in spans)
assert labels == {"agent.external": 400, "agent.usdt": 400}
print(
    f"external_spans={labels['agent.external']} usdt_spans={labels['agent.usdt']} "
    f"roots={roots} children={children}"
)
PY
test "$(awk '$3 == 645 && $10 == "agent.usdt" {count++} END {print count+0}' "$verify_dir/events-two.txt")" -eq 400

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "USDT integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e '.version == 3 and ([.events[] | select(.[5] == 645)] | length) == 800' \
  "$verify_dir/capture.json" >/dev/null
jq -e '.semaphores_before == 0 and .semaphores_after == 0' \
  "$verify_dir/fixture.json" >/dev/null
grep -q 'paired USDT spans attached: 1' "$verify_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
grep -q 'probe_dropped_events=0' "$verify_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/collector-two.log"
grep -q 'probe_dropped_events=0' "$verify_dir/collector-two.log"
echo 'strict_legacy_json=true concurrent_collectors=2 semaphores_restored=true bpf_dropped_events=0 probe_dropped_events=0'
