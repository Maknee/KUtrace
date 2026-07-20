#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-packet-ipv6-verify.XXXXXX)
cleanup() {
  find "$verify_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$verify_dir"
}
trap cleanup EXIT

cgroup_relative=$(cut -d: -f3 /proc/self/cgroup)
cgroup_path="/sys/fs/cgroup$cgroup_relative"
collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --pid 1 \
  --packet-cgroup "$cgroup_path" \
  --sample-hz 0 \
  --duration-secs 2 >"$verify_dir/collector.out" 2>"$verify_dir/collector.log" &
collector_pid=$!
sleep 0.5
sudo /usr/bin/python3 "$root_dir/packet_ipv6_fixture.py"
wait "$collector_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
events_path="$verify_dir/events.txt" python3 - <<'PY'
import os

matches = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and fields[2] == "533" and fields[9] == "tx.5448":
            matches.append(fields)
assert len(matches) == 4, matches
assert {int(fields[6]) for fields in matches} == {1242308164}
print("ipv6_hop_five_and_eight_header_chains_atomic_fragment_tx=4 nine_header_and_noninitial_tx=0 hash=1242308164")
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "IPv6 extension integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e \
  '.version == 3 and ([.events[] | select(.[5] == 533 and .[9] == "tx.5448")] | length) == 4' \
  "$verify_dir/capture.json" >/dev/null
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
echo 'strict_legacy_json=true bpf_dropped_events=0'
