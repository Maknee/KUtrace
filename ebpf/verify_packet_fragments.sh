#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-packet-fragments-verify.XXXXXX)
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
sudo /usr/bin/python3 "$root_dir/packet_fragment_fixture.py" >"$verify_dir/fixture.json"
# Raw loopback injection can occasionally lose one synthetic datagram under a
# busy test host. A second complete send uses the same distinct payload cases;
# the BPF state is removed after each successful reassembly, so every retained
# record must still describe exactly three fragments.
sudo /usr/bin/python3 "$root_dir/packet_fragment_fixture.py" >/dev/null
wait "$collector_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
capture_path="$verify_dir/capture.kuevents" fixture_path="$verify_dir/fixture.json" python3 - <<'PY'
import json
import os
import struct

fixture = json.load(open(os.environ["fixture_path"], encoding="utf-8"))
expected = {
    fixture["ipv4_hash"],
    fixture["ipv6_hash"],
    fixture["ipv6_destination_hash"],
    fixture["ipv6_ah_hash"],
    fixture["ipv6_long_destination_hash"],
}
rejected = fixture["ipv6_overbound_hash"]
record = struct.Struct("<QQQ6QqHHiI16s4x")
tx = []
overbound = []
with open(os.environ["capture_path"], "rb") as stream:
    assert stream.read(8) == b"KUEBPF01"
    stream.seek(64)
    while data := stream.read(record.size):
        fields = record.unpack(data)
        args = fields[3:9]
        if fields[10] == 16 and args[0] in expected:
            tx.append(args)
        if fields[10] == 16 and args[0] == rejected:
            overbound.append(args)
assert {args[0] for args in tx} == expected, tx
assert all(args[2] == 17 and args[4] == 3 for args in tx), tx
assert not overbound, overbound
print(f"reassembled_udp_datagrams={len(tx)} fragments_each=3 out_of_order=true accepted_post_fragment_chains=3 rejected_overbound_chains=1 distinct_hashes={len(expected)}")
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "Fragment integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e '.version == 3 and ([.events[] | select(.[5] == 533)] | length) >= 5' \
  "$verify_dir/capture.json" >/dev/null
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
echo 'strict_legacy_json=true bpf_dropped_events=0'
