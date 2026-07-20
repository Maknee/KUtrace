#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/uprobe-$(date -u +%Y%m%dT%H%M%SZ)"}
iterations=${KUTRACE_UPROBE_BENCH_ITERATIONS:-5000}
samples=${KUTRACE_UPROBE_BENCH_SAMPLES:-9}
duration=${KUTRACE_UPROBE_BENCH_DURATION_SECS:-4}
benchmark_cpu=${KUTRACE_UPROBE_BENCH_CPU:-$(python3 - <<'PY'
import os
print(min(os.sched_getaffinity(0)))
PY
)}
mkdir -p "$result_dir"

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-uprobe-fixture"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

for command in g++ jq nm python3 readelf sudo taskset; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done
[[ "$iterations" =~ ^[0-9]+$ ]] && ((iterations > 0)) || { echo 'iterations must be positive' >&2; exit 2; }
[[ "$samples" =~ ^[0-9]+$ ]] && ((samples >= 3)) || { echo 'samples must be at least three' >&2; exit 2; }

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

taskset -c "$benchmark_cpu" "$fixture" \
  --iterations "$iterations" --samples "$samples" --delay-ms 0 \
  >"$result_dir/baseline.json" 2>"$result_dir/baseline.log"

taskset -c "$benchmark_cpu" "$fixture" \
  --iterations "$iterations" --samples "$samples" --delay-ms 1500 \
  >"$result_dir/active.json" 2>"$result_dir/active.log" &
fixture_pid=$!
sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/capture.kuevents" \
  --agent-shm "$result_dir/agent.shm" \
  --agent-socket "$result_dir/agent.sock" \
  --pid "$fixture_pid" \
  --uprobe "$fixture:@0x$file_offset=agent.fixture.offset" \
  --sample-hz 0 \
  --duration-secs "$duration" 2>"$result_dir/collector.log"
wait "$fixture_pid"

"$transform" events "$result_dir/capture.kuevents" --output "$result_dir/events.txt"
expected_spans=$((iterations * samples * 4))
events_path="$result_dir/events.txt" expected_spans="$expected_spans" iterations="$iterations" samples="$samples" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and not line.startswith("#") and fields[2] == "645":
            spans.append(fields)
expected = int(os.environ["expected_spans"])
roots = int(os.environ["iterations"]) * int(os.environ["samples"])
assert len(spans) == expected, (len(spans), expected)
assert sum(int(fields[7]) == 0 for fields in spans) == roots
assert sum(int(fields[7]) != 0 for fields in spans) == expected - roots
assert all(int(fields[1]) > 0 and fields[9] == "agent.fixture.offset" for fields in spans)
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$result_dir/eventtospan3"
"$result_dir/eventtospan3" "Absolute-offset uprobe overhead" \
  <"$result_dir/events.txt" >"$result_dir/capture.json"
jq -e --argjson expected "$expected_spans" \
  '.version == 3 and ([.events[] | select(.[5] == 645 and .[9] == "agent.fixture.offset")] | length) == $expected' \
  "$result_dir/capture.json" >/dev/null
grep -q 'bpf_dropped_events=0' "$result_dir/collector.log"
grep -q 'probe_dropped_events=0' "$result_dir/collector.log"

jq -n \
  --arg date "$(date -u +%FT%TZ)" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --argjson logical_cpus "$(getconf _NPROCESSORS_ONLN)" \
  --argjson benchmark_cpu "$benchmark_cpu" \
  --arg file_offset "0x$file_offset" \
  --argjson expected_spans "$expected_spans" \
  --slurpfile baseline "$result_dir/baseline.json" \
  --slurpfile active "$result_dir/active.json" \
  '{date_utc:$date,
    host:{cpu:$cpu,logical_cpus:$logical_cpus,kernel:$kernel,architecture:$architecture},
    configuration:{attachment:"absolute ELF file offset",file_offset:$file_offset,benchmark_cpu:$benchmark_cpu,recursion_depth:3,iterations_per_sample:$active[0].iterations_per_sample,samples:$active[0].samples,semantic_spans:$expected_spans,sample_hz:0,ipc:false},
    results:{baseline:{minimum_ns_per_span:$baseline[0].min_ns_per_span,median_ns_per_span:$baseline[0].median_ns_per_span,p95_ns_per_span:$baseline[0].p95_ns_per_span,mean_ns_per_span:$baseline[0].mean_ns_per_span},active_capture:{minimum_ns_per_span:$active[0].min_ns_per_span,median_ns_per_span:$active[0].median_ns_per_span,p95_ns_per_span:$active[0].p95_ns_per_span,mean_ns_per_span:$active[0].mean_ns_per_span,median_added_ns_per_span:($active[0].median_ns_per_span-$baseline[0].median_ns_per_span),p95_added_ns_per_span:($active[0].p95_ns_per_span-$baseline[0].p95_ns_per_span),paired_agent_spans:$expected_spans,strict_v3_json:true,bpf_dropped_events:0,probe_dropped_events:0}},
    status:"pass"}' | tee "$result_dir/summary.json"

echo "results: $result_dir"
