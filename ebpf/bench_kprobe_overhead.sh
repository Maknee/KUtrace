#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/kprobe-$(date -u +%Y%m%dT%H%M%SZ)"}
iterations=${KUTRACE_KPROBE_BENCH_ITERATIONS:-10000}
samples=${KUTRACE_KPROBE_BENCH_SAMPLES:-20}
duration=${KUTRACE_KPROBE_BENCH_DURATION_SECS:-3}
kernel_symbol=${KUTRACE_KPROBE_BENCH_SYMBOL:-__do_sys_getpid}
benchmark_cpu=${KUTRACE_KPROBE_BENCH_CPU:-$(python3 - <<'PY'
import os
print(min(os.sched_getaffinity(0)))
PY
)}
collector_cpu=${KUTRACE_KPROBE_COLLECTOR_CPU:-$(python3 - <<'PY'
import os
cpus = sorted(os.sched_getaffinity(0))
print(cpus[1] if len(cpus) > 1 else cpus[0])
PY
)}

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-bench"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

for command in awk g++ jq python3 sudo taskset; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done
[[ "$iterations" =~ ^[0-9]+$ ]] && ((iterations > 0)) || {
  echo 'iterations must be positive' >&2
  exit 2
}
[[ "$samples" =~ ^[0-9]+$ ]] && ((samples >= 3)) || {
  echo 'samples must be at least three' >&2
  exit 2
}
[[ "$benchmark_cpu" != "$collector_cpu" ]] || {
  echo 'benchmark and collector CPUs must differ' >&2
  exit 2
}
sudo -n awk -v symbol="$kernel_symbol" '$1 == symbol {found=1} END {exit !found}' \
  /sys/kernel/tracing/available_filter_functions || {
  echo "kernel function is not traceable: $kernel_symbol" >&2
  exit 1
}

mkdir -p "$result_dir"
if find "$result_dir" -mindepth 1 -maxdepth 1 -print -quit |
  awk 'NF {found=1} END {exit !found}'; then
  echo "result directory must be empty: $result_dir" >&2
  exit 1
fi

fixture_pid=
cleanup() {
  local status=$?
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

run_fixture() {
  local output=$1
  shift
  taskset -c "$benchmark_cpu" "$fixture" \
    --mode getpid --iterations "$iterations" --samples "$samples" "$@" \
    >"$output" 2>"${output%.json}.log"
}

run_fixture "$result_dir/baseline.json"

# Launch taskset directly for attached runs. Backgrounding the shell function
# would make $! identify a wrapper shell rather than the PID being traced.
taskset -c "$benchmark_cpu" "$fixture" \
  --mode getpid --iterations "$iterations" --samples "$samples" --delay-ms 1200 \
  >"$result_dir/standard.json" 2>"$result_dir/standard.log" &
fixture_pid=$!
sudo -n taskset -c "$collector_cpu" "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/standard.kuevents" \
  --pid "$fixture_pid" \
  --agent-shm "$result_dir/standard.shm" \
  --agent-socket "$result_dir/standard.sock" \
  --sample-hz 0 \
  --duration-secs "$duration" 2>"$result_dir/standard-collector.log"
wait "$fixture_pid"
fixture_pid=

taskset -c "$benchmark_cpu" "$fixture" \
  --mode getpid --iterations "$iterations" --samples "$samples" --delay-ms 1200 \
  >"$result_dir/kprobe.json" 2>"$result_dir/kprobe.log" &
fixture_pid=$!
sudo -n taskset -c "$collector_cpu" "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/kprobe.kuevents" \
  --pid "$fixture_pid" \
  --agent-shm "$result_dir/kprobe.shm" \
  --agent-socket "$result_dir/kprobe.sock" \
  --kprobe "$kernel_symbol=kernel.getpid" \
  --sample-hz 0 \
  --duration-secs "$duration" 2>"$result_dir/kprobe-collector.log"
wait "$fixture_pid"
fixture_pid=

"$transform" events "$result_dir/kprobe.kuevents" --output "$result_dir/events.txt"
expected_spans=$((iterations * samples + iterations / 10 + 2))
events_path="$result_dir/events.txt" expected_spans="$expected_spans" python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) >= 10 and fields[2] == "645" and fields[9] == "kernel.getpid":
            spans.append(fields)
expected = int(os.environ["expected_spans"])
assert len(spans) == expected, (len(spans), expected)
assert all(int(fields[1]) > 0 and int(fields[7]) == 0 for fields in spans)
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$result_dir/eventtospan3"
"$result_dir/eventtospan3" "Generic kernel probe overhead" \
  <"$result_dir/events.txt" >"$result_dir/capture.json"
jq -e --argjson expected "$expected_spans" \
  '.version == 3
   and ([.events[] | select(.[5] == 645 and .[9] == "kernel.getpid")] | length) == $expected' \
  "$result_dir/capture.json" >/dev/null
for log in "$result_dir/standard-collector.log" "$result_dir/kprobe-collector.log"; do
  grep -q 'bpf_dropped_events=0' "$log"
  grep -q 'probe_dropped_events=0' "$log"
done

jq -n \
  --arg date "$(date -u +%FT%TZ)" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --arg kernel_symbol "$kernel_symbol" \
  --argjson logical_cpus "$(getconf _NPROCESSORS_ONLN)" \
  --argjson benchmark_cpu "$benchmark_cpu" \
  --argjson collector_cpu "$collector_cpu" \
  --argjson expected_spans "$expected_spans" \
  --slurpfile baseline "$result_dir/baseline.json" \
  --slurpfile standard "$result_dir/standard.json" \
  --slurpfile kprobe "$result_dir/kprobe.json" \
  '{
    date_utc:$date,
    host:{
      cpu:$cpu,
      logical_cpus:$logical_cpus,
      kernel:$kernel,
      architecture:$architecture
    },
    configuration:{
      kernel_symbol:$kernel_symbol,
      attachment:"paired generic kprobe/kretprobe",
      benchmark_cpu:$benchmark_cpu,
      collector_cpu:$collector_cpu,
      iterations_per_sample:$kprobe[0].iterations_per_sample,
      samples:$kprobe[0].samples,
      semantic_spans:$expected_spans,
      sample_hz:0,
      ipc:false
    },
    results:{
      baseline_median_ns_per_call:$baseline[0].median_ns_per_op,
      standard_trace_median_ns_per_call:$standard[0].median_ns_per_op,
      kprobe_trace_median_ns_per_call:$kprobe[0].median_ns_per_op,
      standard_trace_added_ns_per_call:
        ($standard[0].median_ns_per_op-$baseline[0].median_ns_per_op),
      generic_kprobe_incremental_ns_per_call:
        ($kprobe[0].median_ns_per_op-$standard[0].median_ns_per_op),
      total_kprobe_trace_added_ns_per_call:
        ($kprobe[0].median_ns_per_op-$baseline[0].median_ns_per_op),
      kprobe_trace_p95_ns_per_call:$kprobe[0].p95_ns_per_op,
      paired_kernel_spans:$expected_spans,
      strict_v3_json:true,
      bpf_dropped_events:0,
      probe_dropped_events:0
    },
    status:"pass"
  }' | tee "$result_dir/summary.json"

echo "results: $result_dir"
