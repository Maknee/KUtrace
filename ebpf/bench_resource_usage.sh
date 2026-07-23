#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/resources-$(date -u +%Y%m%dT%H%M%SZ)"}
benchmark_cpu=${KUTRACE_RESOURCE_BENCH_CPU:-$(python3 - <<'PY'
import os
print(max(os.sched_getaffinity(0)))
PY
)}
collector_cpu=${KUTRACE_RESOURCE_COLLECTOR_CPU:-$(python3 - <<'PY'
import os
cpus = sorted(os.sched_getaffinity(0))
print(cpus[-2] if len(cpus) > 1 else cpus[-1])
PY
)}

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
bench="$root_dir/target/release/kutrace-bench"
uprobe_fixture="$root_dir/target/release/kutrace-uprobe-fixture"
usdt_fixture="$root_dir/target/release/kutrace-usdt-fixture"
default_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
stack_object="$root_dir/kutrace-ebpf/target/stack-traces/bpfel-unknown-none/release/kutrace-ebpf"

for command in awk jq pgrep python3 sudo taskset; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done
[[ "$benchmark_cpu" != "$collector_cpu" ]] || {
  echo 'benchmark and collector CPUs must differ' >&2
  exit 2
}
for artifact in \
  "$collector" "$transform" "$bench" "$uprobe_fixture" "$usdt_fixture" \
  "$default_object" "$stack_object"; do
  [[ -e "$artifact" ]] || {
    echo "missing build artifact: $artifact" >&2
    exit 1
  }
done

mkdir -p "$result_dir"
if find "$result_dir" -mindepth 1 -maxdepth 1 -print -quit |
  awk 'NF {found=1} END {exit !found}'; then
  echo "result directory must be empty: $result_dir" >&2
  exit 1
fi

mem_total_kib=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
logical_cpus=$(getconf _NPROCESSORS_ONLN)
metrics_file="$result_dir/metrics.jsonl"
: >"$metrics_file"
if pgrep -x kutrace-collect >/dev/null; then
  echo 'another kutrace-collector is already running; resource attribution would be ambiguous' >&2
  exit 1
fi

active_target_pid=
active_sudo_pid=
cleanup() {
  local status=$?
  if [[ -n "$active_sudo_pid" ]] && kill -0 "$active_sudo_pid" 2>/dev/null; then
    sudo kill -INT "$active_sudo_pid" 2>/dev/null || true
    wait "$active_sudo_pid" 2>/dev/null || true
  fi
  if [[ -n "$active_target_pid" ]] && kill -0 "$active_target_pid" 2>/dev/null; then
    kill "$active_target_pid" 2>/dev/null || true
    wait "$active_target_pid" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

measure_collector() {
  local label=$1
  local target_pid=$2
  local object=$3
  local duration=$4
  shift 4
  local capture="$result_dir/$label.kuevents"
  local collector_log="$result_dir/$label-collector.log"

  sudo -n taskset -c "$collector_cpu" "$collector" \
    --ebpf "$object" \
    --output "$capture" \
    --pid "$target_pid" \
    --agent-shm "$result_dir/$label.shm" \
    --agent-socket "$result_dir/$label.sock" \
    --duration-secs "$duration" \
    "$@" \
    >"$result_dir/$label-collector.out" 2>"$collector_log" &
  active_sudo_pid=$!

  local collector_pid=
  for _ in $(seq 1 200); do
    collector_pid=$(pgrep -n -x kutrace-collect || true)
    if [[ -n "$collector_pid" ]] &&
      grep -q 'capturing to' "$collector_log" 2>/dev/null; then
      break
    fi
    sleep 0.025
  done
  [[ -n "$collector_pid" && -r "/proc/$collector_pid/stat" ]] || {
    echo "collector did not become measurable for $label" >&2
    return 1
  }

  local map_memlock_bytes
  map_memlock_bytes=$(sudo -n sh -c \
    'awk "/^memlock:/ {total += \$2} END {print total + 0}" /proc/"$1"/fdinfo/*' \
    sh "$collector_pid")
  local start_runtime_ns last_runtime_ns
  start_runtime_ns=$(awk '{print $1}' \
    "/proc/$collector_pid/schedstat" 2>/dev/null || true)
  [[ -n "$start_runtime_ns" ]] || {
    echo "collector exited before CPU sampling began for $label" >&2
    return 1
  }
  last_runtime_ns=$start_runtime_ns
  local start_ns last_ns
  start_ns=$(date +%s%N)
  last_ns=$start_ns
  local max_rss_kib=0

  while [[ -r "/proc/$collector_pid/schedstat" ]]; do
    if [[ -r "/proc/$collector_pid/status" ]]; then
      local runtime_ns
      runtime_ns=$(awk '{print $1}' \
        "/proc/$collector_pid/schedstat" 2>/dev/null || true)
      if [[ -n "$runtime_ns" ]]; then
        last_runtime_ns=$runtime_ns
        last_ns=$(date +%s%N)
      fi
      local rss_kib
      rss_kib=$(sudo -n awk '/^VmRSS:/ {print $2}' \
        "/proc/$collector_pid/status" 2>/dev/null || true)
      if [[ -n "$rss_kib" ]] && ((rss_kib > max_rss_kib)); then
        max_rss_kib=$rss_kib
      fi
    fi
    sleep 0.05
  done
  wait "$active_sudo_pid"
  active_sudo_pid=

  grep -q 'bpf_dropped_events=0' "$collector_log"
  grep -q 'probe_dropped_events=0' "$collector_log"
  grep -q 'stack_dropped_samples=0' "$collector_log"

  "$transform" events "$capture" --output "$result_dir/$label-events.txt"
  local normalized_events
  normalized_events=$(awk '!/^#/ && NF {count++} END {print count + 0}' \
    "$result_dir/$label-events.txt")
  local bpf_dropped probe_dropped stack_dropped user_stacks kernel_stacks
  local client_events paired_syscalls compact_syscalls
  bpf_dropped=$(sed -n 's/.*bpf_dropped_events=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  probe_dropped=$(sed -n 's/.*probe_dropped_events=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  stack_dropped=$(sed -n 's/.*stack_dropped_samples=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  user_stacks=$(sed -n 's/.*user_stacks=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  kernel_stacks=$(sed -n 's/.*kernel_stacks=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  client_events=$(sed -n 's/.*client_events_received=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  paired_syscalls=$(sed -n 's/.*paired_syscall_records_received=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)
  compact_syscalls=$(sed -n 's/.*compact_syscall_records_received=\([0-9]*\).*/\1/p' "$collector_log" | tail -n 1)

  jq -n \
    --arg mode_name "$label" \
    --argjson elapsed_ns "$((last_ns - start_ns))" \
    --argjson cpu_runtime_ns "$((last_runtime_ns - start_runtime_ns))" \
    --argjson logical_cpus "$logical_cpus" \
    --argjson max_rss_kib "$max_rss_kib" \
    --argjson map_memlock_bytes "$map_memlock_bytes" \
    --argjson mem_total_kib "$mem_total_kib" \
    --argjson normalized_events "$normalized_events" \
    --argjson bpf_dropped "$bpf_dropped" \
    --argjson probe_dropped "$probe_dropped" \
    --argjson stack_dropped "$stack_dropped" \
    --argjson user_stacks "$user_stacks" \
    --argjson kernel_stacks "$kernel_stacks" \
    --argjson client_events "$client_events" \
    --argjson paired_syscalls "$paired_syscalls" \
    --argjson compact_syscalls "$compact_syscalls" \
    '{
      mode:$mode_name,
      measured_seconds:($elapsed_ns / 1000000000),
      collector_cpu_seconds:($cpu_runtime_ns / 1000000000),
      collector_cpu_percent_one_core:
        ($cpu_runtime_ns / $elapsed_ns * 100),
      collector_cpu_percent_host_capacity:
        ($cpu_runtime_ns / $elapsed_ns * 100 / $logical_cpus),
      collector_max_rss_kib:$max_rss_kib,
      bpf_map_memlock_kib:($map_memlock_bytes / 1024),
      combined_collector_and_map_kib:
        ($max_rss_kib + ($map_memlock_bytes / 1024)),
      combined_memory_percent_host:
        (($max_rss_kib + ($map_memlock_bytes / 1024)) / $mem_total_kib * 100),
      normalized_events:$normalized_events,
      bpf_dropped_events:$bpf_dropped,
      probe_dropped_events:$probe_dropped,
      stack_dropped_samples:$stack_dropped,
      user_stacks:$user_stacks,
      kernel_stacks:$kernel_stacks,
      client_events_received:$client_events,
      paired_syscall_records_received:$paired_syscalls,
      compact_syscall_records_received:$compact_syscalls
    }' | tee -a "$metrics_file"
}

run_core_mode() {
  local mode=$1
  local iterations=$2
  local samples=$3
  local duration=$4
  KUTRACE_AGENT_SHM="$result_dir/$mode.shm" \
  KUTRACE_AGENT_SOCKET="$result_dir/$mode.sock" \
    taskset -c "$benchmark_cpu" "$bench" \
      --mode "$mode" --iterations "$iterations" --samples "$samples" \
      --delay-ms 1200 \
      >"$result_dir/$mode-workload.json" 2>"$result_dir/$mode-workload.log" &
  active_target_pid=$!
  measure_collector "$mode" "$active_target_pid" "$default_object" "$duration" \
    --sample-hz 0
  wait "$active_target_pid"
  active_target_pid=
}

sleep 7 &
active_target_pid=$!
measure_collector idle "$active_target_pid" "$default_object" 5 --sample-hz 0
kill "$active_target_pid" 2>/dev/null || true
wait "$active_target_pid" 2>/dev/null || true
active_target_pid=

run_core_mode getpid 10000 20 3
run_core_mode client-span 10000 20 3
run_core_mode trap 10000 20 3
run_core_mode cpu 10000 20 3
run_core_mode scheduler 2000 20 3
run_core_mode mixed 2000 20 3

taskset -c "$benchmark_cpu" "$bench" \
  --mode getpid --iterations 10000 --samples 20 --delay-ms 1200 \
  >"$result_dir/ipc-workload.json" 2>"$result_dir/ipc-workload.log" &
active_target_pid=$!
measure_collector ipc "$active_target_pid" "$default_object" 3 --sample-hz 0 --ipc
wait "$active_target_pid"
active_target_pid=

taskset -c "$benchmark_cpu" "$bench" \
  --mode getpid --iterations 10000 --samples 20 --delay-ms 1200 \
  >"$result_dir/kernel-kprobe-workload.json" \
  2>"$result_dir/kernel-kprobe-workload.log" &
active_target_pid=$!
measure_collector kernel-kprobe "$active_target_pid" "$default_object" 3 \
  --sample-hz 0 --kprobe "__do_sys_getpid=kernel.getpid.resource"
wait "$active_target_pid"
active_target_pid=

taskset -c "$benchmark_cpu" "$bench" \
  --mode cpu --iterations 100000000 --samples 20 --delay-ms 1200 \
  >"$result_dir/sampling-workload.json" 2>"$result_dir/sampling-workload.log" &
active_target_pid=$!
measure_collector sampling-250hz "$active_target_pid" "$default_object" 5 \
  --sample-hz 250
wait "$active_target_pid"
active_target_pid=

taskset -c "$benchmark_cpu" "$bench" \
  --mode cpu --iterations 100000000 --samples 20 --delay-ms 1200 \
  >"$result_dir/stack-sampling-workload.json" \
  2>"$result_dir/stack-sampling-workload.log" &
active_target_pid=$!
measure_collector stack-sampling-250hz "$active_target_pid" "$stack_object" 5 \
  --sample-hz 250 --stacks "$result_dir/stacks.jsonl"
wait "$active_target_pid"
active_target_pid=

taskset -c "$benchmark_cpu" "$uprobe_fixture" \
  --iterations 1000 --samples 10 --delay-ms 1200 \
  >"$result_dir/uprobe-workload.json" 2>"$result_dir/uprobe-workload.log" &
active_target_pid=$!
measure_collector uprobe "$active_target_pid" "$default_object" 3 \
  --sample-hz 0 \
  --uprobe "$uprobe_fixture:kutrace_probe_fixture=agent.fixture.resource"
wait "$active_target_pid"
active_target_pid=

taskset -c "$benchmark_cpu" "$usdt_fixture" \
  --iterations 1000 --samples 10 --delay-ms 1200 --settle-ms 2500 \
  >"$result_dir/usdt-workload.json" 2>"$result_dir/usdt-workload.log" &
active_target_pid=$!
measure_collector usdt "$active_target_pid" "$default_object" 3 \
  --sample-hz 0 \
  --usdt "$usdt_fixture:kutrace:agent_begin:agent_end=agent.usdt.resource"
wait "$active_target_pid"
active_target_pid=

jq -s \
  --arg date "$(date -u +%FT%TZ)" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --argjson logical_cpus "$logical_cpus" \
  --argjson mem_total_kib "$mem_total_kib" \
  --argjson benchmark_cpu "$benchmark_cpu" \
  --argjson collector_cpu "$collector_cpu" \
  '{
    date_utc:$date,
    host:{
      cpu:$cpu,
      logical_cpus:$logical_cpus,
      memory_kib:$mem_total_kib,
      kernel:$kernel,
      architecture:$architecture
    },
    methodology:{
      benchmark_cpu:$benchmark_cpu,
      collector_cpu:$collector_cpu,
      cpu_denominator:"one logical core; host capacity divides by logical CPU count",
      memory_denominator:"MemTotal; numerator is collector peak RSS plus exact BPF map memlock",
      cpu_scope:"collector userspace after attachment; in-process BPF cost is reported separately as application overhead",
      zero_loss_required:true
    },
    results:.
  }' "$metrics_file" | tee "$result_dir/summary.json"

echo "results: $result_dir"
