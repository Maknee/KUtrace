#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$result_dir"

iterations=${KUTRACE_BENCH_ITERATIONS:-10000}
samples=${KUTRACE_BENCH_SAMPLES:-20}
modes=${KUTRACE_BENCH_MODES:-"getpid client-span trap"}
capture_scope=${KUTRACE_BENCH_SCOPE:-pid}
sample_hz=${KUTRACE_SAMPLE_HZ:-99}
collector_duration_secs=${KUTRACE_COLLECTOR_DURATION_SECS:-3}
benchmark_cpu=${KUTRACE_BENCH_CPU:-$(python3 - <<'PY'
import os
print(min(os.sched_getaffinity(0)))
PY
)}
collector_cpu=${KUTRACE_COLLECTOR_CPU:-$(python3 - <<'PY'
import os
cpus=sorted(os.sched_getaffinity(0))
print(cpus[1] if len(cpus) > 1 else cpus[0])
PY
)}
collector="$root_dir/target/release/kutrace-collector"
bench="$root_dir/target/release/kutrace-bench"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"

case "$capture_scope" in
  pid|host) ;;
  *) echo "KUTRACE_BENCH_SCOPE must be pid or host" >&2; exit 2 ;;
esac

run_mode() {
  local mode=$1
  local socket="$result_dir/$mode.sock"
  local shm="$result_dir/$mode.shm"
  local capture="$result_dir/$mode.kuevents"
  local collector_log="$result_dir/$mode-collector.log"
  local filtered_file="$result_dir/$mode-baseline.json"

  taskset -c "$benchmark_cpu" "$bench" --mode "$mode" --iterations "$iterations" --samples "$samples" \
    > "$result_dir/$mode-baseline.json"

  if [[ "$mode" == "getpid" || "$mode" == "trap" || "$mode" == "cpu" || "$mode" == "scheduler" || "$mode" == "mixed" ]]; then
    # Keep a real, idle target alive for the filtered control. The collector
    # validates PID scope and seeds its existing TIDs before attachment, so a
    # fabricated PID is neither valid nor representative.
    sleep "$((collector_duration_secs + 2))" &
    local filter_target_pid=$!
    taskset -c "$benchmark_cpu" "$bench" --mode "$mode" --iterations "$iterations" --samples "$samples" \
      --delay-ms 1200 > "$result_dir/$mode-filtered.json" &
    local filtered_pid=$!
    sudo taskset -c "$collector_cpu" "$collector" --ebpf "$ebpf_object" \
      --output "$result_dir/$mode-filtered.kuevents" --pid "$filter_target_pid" \
      --agent-socket "$result_dir/$mode-filtered.sock" \
      --agent-shm "$result_dir/$mode-filtered.shm" --sample-hz "$sample_hz" \
      --duration-secs "$collector_duration_secs" \
      2> >(tee "$result_dir/$mode-filtered-collector.log" >&2)
    wait "$filtered_pid"
    kill "$filter_target_pid" 2>/dev/null || true
    wait "$filter_target_pid" 2>/dev/null || true
    filtered_file="$result_dir/$mode-filtered.json"
  fi

  KUTRACE_AGENT_SHM="$shm" KUTRACE_AGENT_SOCKET="$socket" \
    taskset -c "$benchmark_cpu" "$bench" --mode "$mode" \
      --iterations "$iterations" --samples "$samples" --delay-ms 1200 \
    > "$result_dir/$mode-traced.json" &
  local benchmark_pid=$!

  local -a scope_args=()
  if [[ "$capture_scope" == "pid" ]]; then
    scope_args=(--pid "$benchmark_pid")
  fi

  sudo taskset -c "$collector_cpu" "$collector" --ebpf "$ebpf_object" --output "$capture" \
    "${scope_args[@]}" --agent-socket "$socket" --agent-shm "$shm" \
    --sample-hz "$sample_hz" --duration-secs "$collector_duration_secs" \
    2> >(tee "$collector_log" >&2)
  wait "$benchmark_pid"

  grep -q 'bpf_dropped_events=0' "$collector_log"
  grep -q 'probe_dropped_events=0' "$collector_log"
  local paired_syscalls compact_syscalls
  paired_syscalls=$(sed -n 's/.*paired_syscall_records_received=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  compact_syscalls=$(sed -n 's/.*compact_syscall_records_received=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  [[ -n "$paired_syscalls" && -n "$compact_syscalls" ]]

  jq -n \
    --slurpfile baseline "$result_dir/$mode-baseline.json" \
    --slurpfile filtered "$filtered_file" \
    --slurpfile traced "$result_dir/$mode-traced.json" \
    --argjson benchmark_cpu "$benchmark_cpu" \
    --argjson collector_cpu "$collector_cpu" \
    --argjson paired_syscall_records_received "$paired_syscalls" \
    --argjson compact_syscall_records_received "$compact_syscalls" \
    --arg capture_scope "$capture_scope" \
    '{mode:$baseline[0].mode,
      capture_scope:$capture_scope,
      benchmark_cpu:$benchmark_cpu,
      collector_cpu:$collector_cpu,
      iterations_per_sample:$baseline[0].iterations_per_sample,
      samples:$baseline[0].samples,
      baseline_samples_ns:$baseline[0].sample_ns_per_op,
      baseline_median_ns:$baseline[0].median_ns_per_op,
      baseline_p95_ns:$baseline[0].p95_ns_per_op,
      loaded_filtered_median_ns:$filtered[0].median_ns_per_op,
      loaded_filtered_samples_ns:$filtered[0].sample_ns_per_op,
      loaded_filtered_p95_ns:$filtered[0].p95_ns_per_op,
      filter_added_ns:($filtered[0].median_ns_per_op-$baseline[0].median_ns_per_op),
      traced_median_ns:$traced[0].median_ns_per_op,
      traced_samples_ns:$traced[0].sample_ns_per_op,
      traced_p95_ns:$traced[0].p95_ns_per_op,
      added_ns:($traced[0].median_ns_per_op-$baseline[0].median_ns_per_op),
      ratio:($traced[0].median_ns_per_op/$baseline[0].median_ns_per_op),
      paired_syscall_records_received:$paired_syscall_records_received,
      compact_syscall_records_received:$compact_syscall_records_received,
      client_events_dropped:$traced[0].client_events_dropped}'
}

{
  for mode in $modes; do
    case "$mode" in
      getpid|client-span|trap|cpu|scheduler|mixed) run_mode "$mode" ;;
      *) echo "unsupported benchmark mode: $mode" >&2; exit 2 ;;
    esac
  done
} | jq -s '.' | tee "$result_dir/summary.json"

echo "results: $result_dir"
