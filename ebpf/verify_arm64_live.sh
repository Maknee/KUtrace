#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
result_dir=${1:-"$root_dir/benchmark-results/arm64-$(date -u +%Y%m%dT%H%M%SZ)"}
fixture_pid=
collector_pid=
cleanup() {
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  if [[ -n "$collector_pid" ]] && kill -0 "$collector_pid" 2>/dev/null; then
    kill "$collector_pid" 2>/dev/null || true
    wait "$collector_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

[[ $(uname -m) == aarch64 ]] || {
  echo 'verify_arm64_live.sh must run on an aarch64 kernel' >&2
  exit 2
}
for command in cargo g++ jq python3 rg sudo; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done
mkdir -p "$result_dir"

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-diff"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
for executable in "$collector" "$transform" "$fixture"; do
  [[ -x "$executable" ]] || { echo "missing executable: $executable" >&2; exit 2; }
done
[[ -f "$ebpf_object" ]] || { echo "missing eBPF object: $ebpf_object" >&2; exit 2; }

wait_for_line() {
  local path=$1 pattern=$2
  for _ in $(seq 1 200); do
    if [[ -f "$path" ]] && rg -q "$pattern" "$path"; then
      return 0
    fi
    sleep 0.02
  done
  echo "timed out waiting for $pattern in $path" >&2
  return 1
}

"$fixture" >"$result_dir/fixture.jsonl" 2>"$result_dir/fixture.log" &
fixture_pid=$!
wait_for_line "$result_dir/fixture.jsonl" '"pid"'
reported_pid=$(sed -n '1p' "$result_dir/fixture.jsonl" | jq -r .pid)
[[ "$reported_pid" == "$fixture_pid" ]]

sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/capture.kuevents" \
  --agent-shm "$result_dir/agent.shm" \
  --agent-socket "$result_dir/agent.sock" \
  --pid "$fixture_pid" \
  --sample-hz 0 \
  --duration-secs 2 >"$result_dir/collector.out" 2>"$result_dir/collector.log" &
collector_pid=$!
wait_for_line "$result_dir/collector.log" 'capturing to'
kill -USR1 "$fixture_pid"
wait "$fixture_pid"
fixture_pid=
wait "$collector_pid"
collector_pid=

rg -q 'arm64 software page-fault capture attached to [1-9][0-9]* CPUs' "$result_dir/collector.log"
rg -q 'bpf_dropped_events=0' "$result_dir/collector.log"
rg -q 'probe_dropped_events=0' "$result_dir/collector.log"
"$transform" events "$result_dir/capture.kuevents" --output "$result_dir/events.txt"
events_path="$result_dir/events.txt" python3 - <<'PY' >"$result_dir/semantics.json"
import collections
import json
import os

iterations = 128
expected = {
    "getppid": 1,
    "getuid": 1,
    "getgid": 1,
    "getpgid": 2,
    "getsid": 2,
    "sched_yield": 1,
    "kill": 1,
}
calls = collections.Counter()
returns = collections.Counter()
failed = collections.Counter()
faults = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) < 10 or line.startswith("#"):
            continue
        event = int(fields[2])
        name = fields[9].removeprefix("/")
        if name in expected and 0x800 <= event <= 0xbff:
            calls[name] += 1
        elif name in expected and 0xc00 <= event <= 0xfff:
            returns[name] += 1
            if int(fields[7]) < 0:
                failed[name] += 1
        if event == 0x40e and fields[9] == "page_fault_user":
            faults.append(fields)
expected_counts = {name: count * iterations for name, count in expected.items()}
assert calls == expected_counts, (calls, expected_counts)
assert returns == expected_counts, (returns, expected_counts)
assert failed == {"getpgid": iterations, "getsid": iterations, "kill": iterations}, failed
assert len(faults) >= 128, len(faults)
assert all(int(row[1]) > 0 and int(row[6]) > 0 for row in faults)
print(json.dumps({
    "syscall_pairs": sum(expected_counts.values()),
    "calls": calls,
    "returns": returns,
    "failed_returns": failed,
    "user_page_faults": len(faults),
    "page_fault_addresses_nonzero": True,
    "page_fault_durations_positive": True,
}, sort_keys=True))
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$result_dir/eventtospan3"
"$result_dir/eventtospan3" "Aya arm64 live validation" \
  <"$result_dir/events.txt" >"$result_dir/capture.json" 2>"$result_dir/eventtospan3.log"
jq -e --slurpfile semantics "$result_dir/semantics.json" \
  '.version == 3 and ([.events[] | select(.[5] == 1038 and .[9] == "page_fault_user")] | length) >= $semantics[0].user_page_faults' \
  "$result_dir/capture.json" >/dev/null

KUTRACE_BENCH_MODES='getpid client-span' \
KUTRACE_SAMPLE_HZ=0 \
KUTRACE_COLLECTOR_DURATION_SECS=3 \
"$root_dir/bench_overhead.sh" "$result_dir/overhead" >"$result_dir/overhead.log"

jq -n \
  --arg date "$(date -u +%FT%TZ)" \
  --arg kernel "$(uname -r)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --argjson logical_cpus "$(getconf _NPROCESSORS_ONLN)" \
  --slurpfile semantics "$result_dir/semantics.json" \
  --slurpfile overhead "$result_dir/overhead/summary.json" \
  '{date_utc:$date,
    host:{cpu:$cpu,logical_cpus:$logical_cpus,kernel:$kernel,architecture:"aarch64"},
    semantics:$semantics[0],
    overhead:$overhead[0],
    strict_v3_json:true,
    bpf_dropped_events:0,
    probe_dropped_events:0,
    status:"pass"}' | tee "$result_dir/summary.json"

echo "results: $result_dir"
