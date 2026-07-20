#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-client-markers-verify.XXXXXX)
cleanup() {
  find "$verify_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$verify_dir"
}
trap cleanup EXIT

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-marker-fixture"
c_fixture="$verify_dir/kutrace-c-abi-fixture"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

cc -std=c11 -O2 -Wall -Wextra -Werror \
  -I"$root_dir/kutrace-client/include" \
  "$root_dir/kutrace-client/tests/c_abi_fixture.c" \
  -L"$root_dir/target/release" -Wl,-rpath,"$root_dir/target/release" \
  -lkutrace_client -o "$c_fixture"

sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --pid 1 \
  --ipc \
  --sample-hz 0 \
  --duration-secs 1.5 >"$verify_dir/collector.out" 2>"$verify_dir/collector.log" &
collector_pid=$!

for _ in $(seq 1 50); do
  if [[ -e "$verify_dir/agent.shm" ]]; then
    break
  fi
  sleep 0.02
done
[[ -e "$verify_dir/agent.shm" ]]
KUTRACE_AGENT_SHM="$verify_dir/agent.shm" \
  KUTRACE_AGENT_SOCKET="$verify_dir/agent.sock" \
  "$fixture" >"$verify_dir/fixture.json"
KUTRACE_AGENT_SHM="$verify_dir/agent.shm" \
  KUTRACE_AGENT_SOCKET="$verify_dir/agent.sock" \
  "$c_fixture" >"$verify_dir/c-fixture.json"
wait "$collector_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
grep -q '^# ## FLAGS: 128$' "$verify_dir/events.txt"
events_path="$verify_dir/events.txt" python3 - <<'PY'
import os

markers = []
annotations = []
spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and (fields[2] in {"513", "537", "538", "539"} or (fields[2] == "522" and int(fields[7]) <= 0)):
            markers.append(fields)
        if len(fields) >= 10 and fields[2] in {"522", "523", "524", "525"} and int(fields[7]) > 0:
            annotations.append(fields)
        if len(fields) >= 10 and fields[2] == "645":
            spans.append(fields)
assert len(markers) == 7, markers
assert any(row[2] == "513" and row[5:10] == ["77", "77", "0", "0", "agent.rpc.read"] for row in markers)
assert any(row[2] == "537" and row[5:10] == ["77", "9", "0", "0", "agent.resource"] for row in markers)
assert any(row[2] == "538" and row[5:10] == ["77", "3", "0", "0", "agent.queue"] for row in markers)
assert any(row[2] == "539" and row[5:10] == ["77", "3", "0", "0", "agent.queue"] for row in markers)
assert any(row[2] == "522" and row[5:10] == ["0", "123", "-5", "0", "agent.mark"] for row in markers)
assert any(row[2] == "537" and row[5:10] == ["88", "99", "0", "0", "agent.c.resource"] for row in markers)
assert [(row[2], row[6], row[9]) for row in annotations] == [
    ("522", "11", "agent.query.events"),
    ("523", "22", "agent.observation.latency"),
    ("524", "33", "agent.decision.retry"),
    ("525", "44", "agent.result.success"),
    ("522", "55", "agent.query.c.abi"),
], annotations
assert len({row[7] for row in annotations[:4]}) == 1
assert any(row[9] == "agent.c.tool" and int(row[1]) > 0 and int(row[7]) == 0 for row in spans), spans
print("client_markers=7 annotations=5 c_abi_span=1 rpc=77 resource=9,99 queue=3 retval=-5")
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "Client marker integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
json_predicate='
   .version == 3 and .flags == 128 and
   ([.events[] | select(.[5] == 513 and .[4] == 77)] | length) >= 1 and
   ([.events[] | select(.[5] == 537 and .[4] == 77 and .[6] == 9)] | length) >= 1 and
   ([.events[] | select(.[5] >= 522 and .[5] <= 525 and .[7] > 0)] | length) == 5 and
   ([.events[] | select(.[5] == 645 and .[9] == "agent.c.tool")] | length) == 1 and
   ([.events[] | select(.[5] == 537 and .[4] != 88 and .[6] == 99 and .[9] == "agent.c.resource")] | length) == 1
'
if ! jq -e "$json_predicate" "$verify_dir/capture.json" >/dev/null; then
  jq '{version, flags,
       rpc: ([.events[] | select(.[5] == 513 and .[4] == 77)] | length),
       rust_resource: ([.events[] | select(.[5] == 537 and .[4] == 77 and .[6] == 9)] | length),
       annotations: ([.events[] | select(.[5] >= 522 and .[5] <= 525 and .[7] > 0)] | length),
       c_span: [.events[] | select(.[5] == 645)],
       resources: [.events[] | select(.[5] == 537)]}' \
    "$verify_dir/capture.json" >&2
  exit 1
fi
jq -e '.markers == 6 and .annotations == 4 and .client_events_dropped == 0' "$verify_dir/fixture.json" >/dev/null
jq -e '.span_id > 0 and .client_events_dropped == 0' "$verify_dir/c-fixture.json" >/dev/null
grep -q 'client_events_received=16' "$verify_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
grep -q 'client_shm_dropped=0' "$verify_dir/collector.log"
echo 'strict_legacy_json=true rust_client_events=12 c_abi_events=4 client_events_received=16 bpf_dropped_events=0 client_shm_dropped=0'
