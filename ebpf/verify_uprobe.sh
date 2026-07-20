#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
probe_dir=$(mktemp -d /tmp/kutrace-uprobe-verify.XXXXXX)
cleanup() {
  find "$probe_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$probe_dir"
}
trap cleanup EXIT

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-uprobe-fixture"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

for command in g++ jq nm python3 readelf rg strip sudo; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$probe_dir/eventtospan3"

run_probe() {
  local binary=$1 selector=$2 label=$3 stem=$4
  "$binary" >"$probe_dir/$stem-fixture.out" 2>"$probe_dir/$stem-fixture.log" &
  local fixture_pid=$!
  sudo "$collector" \
    --ebpf "$ebpf_object" \
    --output "$probe_dir/$stem.kuevents" \
    --agent-shm "$probe_dir/$stem-agent.shm" \
    --agent-socket "$probe_dir/$stem-agent.sock" \
    --pid "$fixture_pid" \
    --uprobe "$binary:$selector=$label" \
    --sample-hz 0 \
    --duration-secs 3 2>"$probe_dir/$stem-collector.log"
  wait "$fixture_pid"

  "$transform" events "$probe_dir/$stem.kuevents" --output "$probe_dir/$stem-events.txt"
  events_path="$probe_dir/$stem-events.txt" expected_label="$label" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and not line.startswith("#") and fields[2] == "645":
            spans.append(fields)
roots = sum(int(fields[7]) == 0 for fields in spans)
children = sum(int(fields[7]) != 0 for fields in spans)
assert (len(spans), roots, children) == (400, 100, 300)
assert all(int(fields[1]) > 0 for fields in spans)
assert {fields[9] for fields in spans} == {os.environ["expected_label"]}
print(f"external_uprobe_spans={len(spans)} roots={roots} children={children}")
PY

  "$probe_dir/eventtospan3" "External uprobe integration" \
    <"$probe_dir/$stem-events.txt" >"$probe_dir/$stem.json"
  jq -e --arg expected_label "$label" \
    '.version == 3 and ([.events[] | select(.[5] == 645 and .[9] == $expected_label)] | length) == 400' \
    "$probe_dir/$stem.json" >/dev/null
  grep -q 'bpf_dropped_events=0' "$probe_dir/$stem-collector.log"
  grep -q 'probe_dropped_events=0' "$probe_dir/$stem-collector.log"
}

run_probe "$fixture" kutrace_probe_fixture agent.fixture.recursive symbol

file_offset=$(python3 - "$fixture" <<'PY'
import subprocess
import sys

binary = sys.argv[1]
symbols = subprocess.check_output(["nm", "-n", "--defined-only", binary], text=True)
values = [int(line.split()[0], 16) for line in symbols.splitlines()
          if len(line.split()) >= 3 and line.split()[2] == "kutrace_probe_fixture"]
assert len(values) == 1, values
address = values[0]
headers = subprocess.check_output(["readelf", "-Wl", binary], text=True)
for line in headers.splitlines():
    fields = line.split()
    if len(fields) >= 6 and fields[0] == "LOAD":
        offset, virtual, file_size = map(lambda value: int(value, 16),
                                         (fields[1], fields[2], fields[4]))
        if virtual <= address < virtual + file_size:
            print(f"{offset + address - virtual:x}")
            break
else:
    raise SystemExit(f"symbol address {address:#x} is not in a file-backed LOAD segment")
PY
)
stripped_fixture="$probe_dir/stripped-fixture"
cp "$fixture" "$stripped_fixture"
strip --strip-all "$stripped_fixture"
if nm --defined-only "$stripped_fixture" 2>/dev/null | rg -q 'kutrace_probe_fixture'; then
  echo 'strip retained kutrace_probe_fixture; offset-only fixture is invalid' >&2
  exit 1
fi
run_probe "$stripped_fixture" "@0x$file_offset" agent.fixture.stripped stripped

echo "symbol_spans=400 stripped_offset_spans=400 file_offset=0x$file_offset strict_legacy_json=true bpf_dropped_events=0 probe_dropped_events=0"
