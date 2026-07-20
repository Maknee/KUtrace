#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-packet-gso-verify.XXXXXX)
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
python3 "$root_dir/packet_gso_fixture.py" >"$verify_dir/fixture.json"
wait "$collector_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
capture_path="$verify_dir/capture.kuevents" fixture_path="$verify_dir/fixture.json" python3 - <<'PY'
import json
import os
import struct

fixture = json.load(open(os.environ["fixture_path"], encoding="utf-8"))
expected = set(fixture["hashes"])
record = struct.Struct("<QQQ6QqHHiI16s4x")
tx = []
rx = []
with open(os.environ["capture_path"], "rb") as stream:
    assert stream.read(8) == b"KUEBPF01"
    stream.seek(64)
    while data := stream.read(record.size):
        assert len(data) == record.size
        fields = record.unpack(data)
        args = fields[3:9]
        kind = fields[10]
        if args[0] not in expected:
            continue
        if kind == 16:
            tx.append(args)
        elif kind == 15:
            rx.append(args)

assert {args[0] for args in tx} == expected, tx
assert {args[0] for args in rx} == expected, rx
assert {(args[3], args[4], args[5]) for args in tx} == {
    (index, fixture["segments"], fixture["segment_size"])
    for index in range(fixture["segments"])
}, tx
assert {(args[3], args[4], args[5]) for args in rx} == {
    (index, fixture["segments"], fixture["gro_size"])
    for index in range(fixture["segments"])
}, rx
print(
    f"udp_gso_tx_segments={len(tx)} udp_gro_rx_segments={len(rx)} "
    f"hashes={len(expected)} gso_gro_size={fixture['segment_size']}"
)
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "UDP GSO integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e '.version == 3' "$verify_dir/capture.json" >/dev/null
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
echo 'strict_legacy_json=true bpf_dropped_events=0'
