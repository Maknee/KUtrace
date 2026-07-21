#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
repo_dir=$(cd "$root_dir/.." && pwd)
result_dir=${1:-"$root_dir/benchmark-results/profiler-$(date -u +%Y%m%dT%H%M%SZ)"}
iterations=${KUTRACE_PROFILER_ITERATIONS:-200000000}
samples=${KUTRACE_PROFILER_SAMPLES:-40}
aya_duration_secs=${KUTRACE_PROFILER_AYA_DURATION_SECS:-20}
benchmark_cpu=${KUTRACE_BENCH_CPU:-$(python3 - <<'PY'
import os
print(min(os.sched_getaffinity(0)))
PY
)}
collector_cpu=${KUTRACE_COLLECTOR_CPU:-$(python3 - <<'PY'
import os
cpus = sorted(os.sched_getaffinity(0))
print(cpus[1] if len(cpus) > 1 else cpus[0])
PY
)}

bench="$root_dir/target/release/kutrace-bench"
collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
module="$repo_dir/linux/setup/module/kutrace_mod.ko"

for command in awk g++ jq modinfo python3 rg sudo taskset uname; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done
[[ $(uname -m) == x86_64 ]] || { echo 'profiler comparison currently requires x86_64' >&2; exit 1; }
rg -q '^kutrace_mod ' /proc/modules || {
  echo 'kutrace_mod is not loaded; refusing to insert it automatically' >&2
  exit 1
}
[[ -f "$module" ]] || { echo "missing reference module: $module" >&2; exit 1; }
module_vermagic=$(modinfo -F vermagic "$module" | awk '{print $1}')
[[ "$module_vermagic" == "$(uname -r)" ]] || {
  echo "module vermagic $module_vermagic does not match running kernel $(uname -r)" >&2
  exit 1
}
for artifact in "$bench" "$collector" "$transform" "$ebpf_object"; do
  [[ -e "$artifact" ]] || { echo "missing build artifact: $artifact" >&2; exit 1; }
done
[[ "$benchmark_cpu" != "$collector_cpu" ]] || {
  echo 'benchmark and collector CPUs must differ' >&2
  exit 1
}

legacy_state() {
  python3 - <<'PY'
import ctypes
import errno

libc = ctypes.CDLL(None, use_errno=True)
state = libc.syscall(1023, 10, 0)
if state < 0:
    raise SystemExit(f"kutrace control syscall unavailable: errno={ctypes.get_errno()}")
print(state)
PY
}

[[ $(legacy_state) == 0 ]] || {
  echo 'legacy KUtrace is already tracing; refusing to reset another capture' >&2
  exit 1
}

kernel_config="/boot/config-$(uname -r)"
[[ -r "$kernel_config" ]] || { echo "missing kernel config: $kernel_config" >&2; exit 1; }
sample_hz=$(sed -n 's/^CONFIG_HZ=//p' "$kernel_config")
[[ "$sample_hz" =~ ^[0-9]+$ ]] || { echo 'unable to determine legacy CONFIG_HZ' >&2; exit 1; }

mkdir -p "$result_dir"
if find "$result_dir" -mindepth 1 -maxdepth 1 -print -quit | rg -q .; then
  echo "result directory must be empty: $result_dir" >&2
  exit 1
fi

g++ -O2 "$repo_dir/postproc/kutrace_control.cc" "$repo_dir/postproc/kutrace_lib.cc" \
  -o "$result_dir/kutrace_control"
g++ -O2 "$repo_dir/postproc/rawtoevent.cc" "$repo_dir/postproc/from_base40.cc" \
  -o "$result_dir/rawtoevent"

legacy_started=false
controller_pid=
collector_pid=
cleanup() {
  local status=$?
  if [[ "$legacy_started" == true ]]; then
    python3 - <<'PY' >/dev/null 2>&1 || true
import ctypes
ctypes.CDLL(None).syscall(1023, 0, 0)
PY
  fi
  if [[ -n "$controller_pid" ]] && kill -0 "$controller_pid" 2>/dev/null; then
    kill "$controller_pid" 2>/dev/null || true
    wait "$controller_pid" 2>/dev/null || true
  fi
  if [[ -n "$collector_pid" ]] && kill -0 "$collector_pid" 2>/dev/null; then
    sudo kill -INT "$collector_pid" 2>/dev/null || true
    wait "$collector_pid" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

run_cpu_bench() {
  local output=$1
  shift
  taskset -c "$benchmark_cpu" "$bench" --mode cpu \
    --iterations "$iterations" --samples "$samples" "$@" >"$output"
}

# Bracket both profilers with controls so slow host drift is visible and the
# summary can use the pooled control distribution.
run_cpu_bench "$result_dir/baseline-before.json"

mkfifo "$result_dir/legacy.commands"
"$result_dir/kutrace_control" <"$result_dir/legacy.commands" \
  >"$result_dir/legacy-control.out" 2>"$result_dir/legacy-control.log" &
controller_pid=$!
exec 3>"$result_dir/legacy.commands"
printf 'go\n' >&3
for _ in $(seq 1 100); do
  [[ $(legacy_state) == 1 ]] && break
  sleep 0.01
done
[[ $(legacy_state) == 1 ]] || { echo 'legacy tracing did not start' >&2; exit 1; }
legacy_started=true
run_cpu_bench "$result_dir/legacy-workload.json"
legacy_state_after_workload=$(legacy_state)
[[ "$legacy_state_after_workload" == 1 ]] || {
  echo 'legacy trace buffer filled and disabled tracing; result is invalid' >&2
  exit 1
}
printf 'stop %s\n' "$result_dir/legacy.trace" >&3
exec 3>&-
wait "$controller_pid"
controller_pid=
legacy_started=false
"$result_dir/rawtoevent" "$result_dir/legacy.trace" \
  >"$result_dir/legacy.events" 2>"$result_dir/legacy-rawtoevent.log"

run_cpu_bench "$result_dir/baseline-middle.json"

# Launch taskset directly here: backgrounding the run_cpu_bench shell function
# would make $! name a shell wrapper rather than the process the collector must
# put in its PID filter.
taskset -c "$benchmark_cpu" "$bench" --mode cpu \
  --iterations "$iterations" --samples "$samples" --delay-ms 1500 \
  >"$result_dir/aya-workload.json" &
aya_bench_pid=$!
sudo taskset -c "$collector_cpu" "$collector" \
  --ebpf "$ebpf_object" --output "$result_dir/aya.kuevents" \
  --pid "$aya_bench_pid" --agent-socket "$result_dir/aya.sock" \
  --agent-shm "$result_dir/aya.shm" --sample-hz "$sample_hz" \
  --duration-secs "$aya_duration_secs" \
  >"$result_dir/aya-collector.out" 2>"$result_dir/aya-collector.log" &
collector_pid=$!
for _ in $(seq 1 200); do
  if rg -q 'capturing to' "$result_dir/aya-collector.log" 2>/dev/null; then
    break
  fi
  sleep 0.025
done
rg -q 'capturing to' "$result_dir/aya-collector.log" || {
  echo 'Aya collector did not become ready' >&2
  exit 1
}
wait "$aya_bench_pid"
wait "$collector_pid"
collector_pid=
rg -q 'bpf_dropped_events=0' "$result_dir/aya-collector.log"
rg -q 'probe_dropped_events=0' "$result_dir/aya-collector.log"
"$transform" events "$result_dir/aya.kuevents" --output "$result_dir/aya.events"

run_cpu_bench "$result_dir/baseline-after.json"

aya_pid=$(jq -r .pid "$result_dir/aya-workload.json")
# Legacy stores only 16 PID bits in normal events, and rawtoevent can assign a
# reconstructed block-header PID after a block rollover. PC_U on the dedicated
# benchmark CPU is the unambiguous target attribution for this pinned workload.
legacy_target_samples=$(awk -v cpu="$benchmark_cpu" '$3 == 640 && $4 == cpu {n++} END {print n + 0}' "$result_dir/legacy.events")
legacy_all_samples=$(awk '$3 == 640 || $3 == 641 {n++} END {print n + 0}' "$result_dir/legacy.events")
aya_target_samples=$(awk -v p="$aya_pid" '$3 == 640 && $5 == p {n++} END {print n + 0}' "$result_dir/aya.events")
aya_all_samples=$(awk '$3 == 640 || $3 == 641 {n++} END {print n + 0}' "$result_dir/aya.events")
[[ "$legacy_target_samples" -gt 0 && "$aya_target_samples" -gt 0 ]]

export RESULT_DIR="$result_dir"
export ITERATIONS="$iterations" SAMPLES="$samples" SAMPLE_HZ="$sample_hz"
export BENCHMARK_CPU="$benchmark_cpu" COLLECTOR_CPU="$collector_cpu"
export LEGACY_TARGET_SAMPLES="$legacy_target_samples" LEGACY_ALL_SAMPLES="$legacy_all_samples"
export AYA_TARGET_SAMPLES="$aya_target_samples" AYA_ALL_SAMPLES="$aya_all_samples"
export LEGACY_STATE_AFTER_WORKLOAD="$legacy_state_after_workload"
export MODULE_VERMAGIC="$module_vermagic"
python3 - <<'PY'
import json
import math
import os
import platform
import random
import statistics
from datetime import datetime, timezone
from pathlib import Path

root = Path(os.environ["RESULT_DIR"])

def load(name):
    with (root / name).open(encoding="utf-8") as stream:
        return json.load(stream)

before = load("baseline-before.json")
middle = load("baseline-middle.json")
after = load("baseline-after.json")
legacy = load("legacy-workload.json")
aya = load("aya-workload.json")
baseline_samples = (
    before["sample_ns_per_op"] + middle["sample_ns_per_op"] + after["sample_ns_per_op"]
)
baseline_median = statistics.median(baseline_samples)
iterations = int(os.environ["ITERATIONS"])

def percentile(values, fraction):
    values = sorted(values)
    return values[math.ceil((len(values) - 1) * fraction)]

def bootstrap_added(active_samples, seed):
    rng = random.Random(seed)
    differences = []
    for _ in range(20000):
        b = [rng.choice(baseline_samples) for _ in baseline_samples]
        a = [rng.choice(active_samples) for _ in active_samples]
        differences.append(statistics.median(a) - statistics.median(b))
    differences.sort()
    return [differences[500], differences[19499]]

def result(report, target_samples, all_samples, seed):
    active = report["sample_ns_per_op"]
    active_median = statistics.median(active)
    added = active_median - baseline_median
    timed_operations = iterations * len(active)
    timed_seconds = sum(active) * iterations / 1e9
    return {
        "samples_per_distribution": len(active),
        "iterations_per_sample": iterations,
        "timed_operations": timed_operations,
        "median_ns_per_op": active_median,
        "p95_ns_per_op": percentile(active, 0.95),
        "added_ns_per_op": added,
        "relative_overhead_percent": added / baseline_median * 100.0,
        "bootstrap_95_ci_added_ns_per_op": bootstrap_added(active, seed),
        "target_user_pc_samples": target_samples,
        "all_pc_samples_in_capture": all_samples,
        "observed_target_samples_per_timed_second": target_samples / timed_seconds,
        "estimated_added_ns_per_target_sample": added * timed_operations / target_samples,
    }

legacy_result = result(
    legacy,
    int(os.environ["LEGACY_TARGET_SAMPLES"]),
    int(os.environ["LEGACY_ALL_SAMPLES"]),
    0x4B5554,
)
aya_result = result(
    aya,
    int(os.environ["AYA_TARGET_SAMPLES"]),
    int(os.environ["AYA_ALL_SAMPLES"]),
    0x415941,
)
with (root / "aya-collector.log").open(encoding="utf-8") as stream:
    collector_log = stream.read()

cpu_model = "unknown"
with open("/proc/cpuinfo", encoding="utf-8") as stream:
    for line in stream:
        if line.startswith("model name"):
            cpu_model = line.split(":", 1)[1].strip()
            break

summary = {
    "date_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    "host": {
        "cpu": cpu_model,
        "logical_cpus": os.cpu_count(),
        "kernel": platform.release(),
        "architecture": platform.machine(),
    },
    "methodology": {
        "workload": "dependency-chained integer recurrence with no syscalls in the timed loop",
        "benchmark_cpu": int(os.environ["BENCHMARK_CPU"]),
        "collector_cpu": int(os.environ["COLLECTOR_CPU"]),
        "matched_sample_hz": int(os.environ["SAMPLE_HZ"]),
        "baseline_distributions": 3,
        "baseline_samples_total": len(baseline_samples),
        "legacy_scope": "whole host (legacy KUtrace has no PID-scoped capture mode)",
        "legacy_target_sample_attribution": "PC_U records on the pinned benchmark CPU",
        "aya_scope": "PID-scoped output; perf callbacks remain attached on every online CPU",
        "module_vermagic": os.environ["MODULE_VERMAGIC"],
        "module_was_replaced": False,
    },
    "baseline": {
        "median_ns_per_op": baseline_median,
        "p95_ns_per_op": percentile(baseline_samples, 0.95),
        "before_median_ns_per_op": before["median_ns_per_op"],
        "middle_median_ns_per_op": middle["median_ns_per_op"],
        "after_median_ns_per_op": after["median_ns_per_op"],
    },
    "legacy_kutrace": legacy_result,
    "aya_ebpf": aya_result,
    "comparison": {
        "aya_to_legacy_added_overhead_ratio": (
            aya_result["added_ns_per_op"] / legacy_result["added_ns_per_op"]
            if legacy_result["added_ns_per_op"] > 0 else None
        ),
        "aya_minus_legacy_added_ns_per_op": (
            aya_result["added_ns_per_op"] - legacy_result["added_ns_per_op"]
        ),
    },
    "loss_status": {
        "legacy_capture_remained_active_until_stop": os.environ["LEGACY_STATE_AFTER_WORKLOAD"] == "1",
        "legacy_wrap_enabled": False,
        "legacy_explicit_drop_counter_available": False,
        "aya_bpf_dropped_events": 0 if "bpf_dropped_events=0" in collector_log else None,
        "aya_probe_dropped_events": 0 if "probe_dropped_events=0" in collector_log else None,
    },
    "limitations": [
        "Legacy KUtrace PC sampling is tied to the 250 Hz local APIC timer and cannot be isolated from its other trace hooks.",
        "Legacy records the whole host; Aya output is PID-scoped to avoid unrelated syscall traffic overwhelming the ring. Both sampling callbacks execute on every online CPU.",
        "Per-sample costs divide aggregate application slowdown by observed target samples; they are not probe-body timings.",
        "Legacy has no explicit drop counter. Remaining active with wrapping disabled proves the trace buffer did not fill, but not a formal zero-drop count.",
    ],
}
with (root / "summary.json").open("w", encoding="utf-8") as stream:
    json.dump(summary, stream, indent=2)
    stream.write("\n")
print(json.dumps(summary, indent=2))
PY

echo "results: $result_dir"
