#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/usdt-$(date -u +%Y%m%dT%H%M%SZ)"}
iterations=${KUTRACE_USDT_BENCH_ITERATIONS:-1000}
samples=${KUTRACE_USDT_BENCH_SAMPLES:-20}
duration=${KUTRACE_USDT_BENCH_DURATION_SECS:-4}
benchmark_cpu=${KUTRACE_USDT_BENCH_CPU:-$(python3 - <<'PY'
import os
print(min(os.sched_getaffinity(0)))
PY
)}
collector_cpu=${KUTRACE_USDT_COLLECTOR_CPU:-$(python3 - <<'PY'
import os
cpus = sorted(os.sched_getaffinity(0))
print(cpus[1] if len(cpus) > 1 else cpus[0])
PY
)}

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-usdt-fixture"
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
for artifact in "$collector" "$transform" "$fixture" "$ebpf_object"; do
  [[ -e "$artifact" ]] || {
    echo "missing build artifact: $artifact" >&2
    exit 1
  }
done

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

mkdir -p "$result_dir"
if find "$result_dir" -mindepth 1 -maxdepth 1 -print -quit | awk 'NF {found=1} END {exit !found}'; then
  echo "result directory must be empty: $result_dir" >&2
  exit 1
fi

# The fixture contains the real compiled semaphore branches in both runs. With
# no collector attached, both semaphores remain zero and establish the disabled
# cost of an instrumented application.
taskset -c "$benchmark_cpu" "$fixture" \
  --iterations "$iterations" --samples "$samples" \
  --delay-ms 0 --settle-ms 0 \
  >"$result_dir/baseline.json" 2>"$result_dir/baseline.log"

# Keep the fixture alive after its timed region so the collector has detached
# before the fixture reads the semaphore values for the lifecycle assertion.
taskset -c "$benchmark_cpu" "$fixture" \
  --iterations "$iterations" --samples "$samples" \
  --delay-ms 1500 --settle-ms 3000 \
  >"$result_dir/active.json" 2>"$result_dir/active.log" &
fixture_pid=$!
sudo taskset -c "$collector_cpu" "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/capture.kuevents" \
  --agent-shm "$result_dir/agent.shm" \
  --agent-socket "$result_dir/agent.sock" \
  --pid "$fixture_pid" \
  --usdt "$fixture:kutrace:agent_begin:agent_end=agent.usdt" \
  --sample-hz 0 \
  --duration-secs "$duration" \
  >"$result_dir/collector.out" 2>"$result_dir/collector.log"
wait "$fixture_pid"
fixture_pid=

"$transform" events "$result_dir/capture.kuevents" --output "$result_dir/events.txt"
expected_spans=$((iterations * samples * 4))
events_path="$result_dir/events.txt" \
expected_spans="$expected_spans" \
iterations="$iterations" \
samples="$samples" \
python3 - <<'PY'
import os

spans = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if (
            len(fields) >= 10
            and not line.startswith("#")
            and fields[2] == "645"
            and fields[9] == "agent.usdt"
        ):
            spans.append(fields)
expected = int(os.environ["expected_spans"])
roots = int(os.environ["iterations"]) * int(os.environ["samples"])
assert len(spans) == expected, (len(spans), expected)
assert sum(int(fields[7]) == 0 for fields in spans) == roots
assert sum(int(fields[7]) != 0 for fields in spans) == expected - roots
assert all(int(fields[1]) > 0 for fields in spans)
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$result_dir/eventtospan3"
"$result_dir/eventtospan3" "USDT overhead" \
  <"$result_dir/events.txt" >"$result_dir/capture.json"
jq -e --argjson expected "$expected_spans" \
  '.version == 3
   and ([.events[] | select(.[5] == 645 and .[9] == "agent.usdt")] | length) == $expected' \
  "$result_dir/capture.json" >/dev/null
jq -e '.semaphores_before == 0 and .semaphores_after == 0' \
  "$result_dir/baseline.json" >/dev/null
jq -e '.semaphores_before == 0 and .semaphores_after == 0' \
  "$result_dir/active.json" >/dev/null
grep -q 'paired USDT spans attached: 1' "$result_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$result_dir/collector.log"
grep -q 'probe_dropped_events=0' "$result_dir/collector.log"

capture_records=$(awk '!/^#/ && NF {count++} END {print count + 0}' "$result_dir/events.txt")
jq -n \
  --arg date "$(date -u +%FT%TZ)" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --argjson logical_cpus "$(getconf _NPROCESSORS_ONLN)" \
  --argjson benchmark_cpu "$benchmark_cpu" \
  --argjson collector_cpu "$collector_cpu" \
  --argjson expected_spans "$expected_spans" \
  --argjson capture_records "$capture_records" \
  --slurpfile baseline "$result_dir/baseline.json" \
  --slurpfile active "$result_dir/active.json" \
  '{
    date_utc:$date,
    host:{
      cpu:$cpu,
      logical_cpus:$logical_cpus,
      kernel:$kernel,
      architecture:$architecture
    },
    configuration:{
      target:"release kutrace-usdt-fixture",
      provider:"kutrace",
      begin_probe:"agent_begin",
      end_probe:"agent_end",
      benchmark_cpu:$benchmark_cpu,
      collector_cpu:$collector_cpu,
      recursion_depth:3,
      iterations_per_sample:$active[0].iterations_per_sample,
      samples:$active[0].samples,
      semantic_spans:$expected_spans,
      sample_hz:0,
      ipc:false
    },
    results:{
      disabled_semaphore:{
        minimum_ns_per_potential_span:$baseline[0].min_ns_per_span,
        median_ns_per_potential_span:$baseline[0].median_ns_per_span,
        p95_ns_per_potential_span:$baseline[0].p95_ns_per_span,
        mean_ns_per_potential_span:$baseline[0].mean_ns_per_span
      },
      active_capture:{
        minimum_ns_per_span:$active[0].min_ns_per_span,
        median_ns_per_span:$active[0].median_ns_per_span,
        p95_ns_per_span:$active[0].p95_ns_per_span,
        mean_ns_per_span:$active[0].mean_ns_per_span,
        median_added_ns_per_span:
          ($active[0].median_ns_per_span-$baseline[0].median_ns_per_span),
        p95_added_ns_per_span:
          ($active[0].p95_ns_per_span-$baseline[0].p95_ns_per_span),
        paired_agent_spans:$expected_spans,
        capture_records:$capture_records,
        strict_v3_json:true,
        bpf_dropped_events:0,
        probe_dropped_events:0
      },
      semaphore_values:{
        before_attach:$active[0].semaphores_before,
        after_detach:$active[0].semaphores_after
      }
    },
    status:"pass"
  }' | tee "$result_dir/summary.json"

echo "results: $result_dir"
