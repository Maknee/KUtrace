#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
repo_dir=$(cd "$root_dir/.." && pwd)
result_dir=${KUTRACE_DIFF_RESULT_DIR:-}
keep_results=true
if [[ -z "$result_dir" ]]; then
  result_dir=$(mktemp -d /tmp/kutrace-live-differential.XXXXXX)
  keep_results=false
else
  mkdir -p "$result_dir"
  if find "$result_dir" -mindepth 1 -maxdepth 1 -print -quit | rg -q .; then
    echo "KUTRACE_DIFF_RESULT_DIR must be empty: $result_dir" >&2
    exit 1
  fi
fi

legacy_started=false
controller_pid=
collector_pid=
fixture_pid=
cleanup() {
  local status=$?
  if [[ -n "$fixture_pid" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  if [[ "$legacy_started" == true ]]; then
    printf 'off\n' | "$result_dir/kutrace_control" >/dev/null 2>&1 || true
  fi
  if [[ -n "$controller_pid" ]] && kill -0 "$controller_pid" 2>/dev/null; then
    kill "$controller_pid" 2>/dev/null || true
    wait "$controller_pid" 2>/dev/null || true
  fi
  if [[ -n "$collector_pid" ]] && kill -0 "$collector_pid" 2>/dev/null; then
    kill -INT "$collector_pid" 2>/dev/null || true
    wait "$collector_pid" 2>/dev/null || true
  fi
  if [[ "$keep_results" == false && $status -eq 0 ]]; then
    rm -rf "$result_dir"
  elif [[ $status -ne 0 ]]; then
    printf 'differential artifacts retained at %s\n' "$result_dir" >&2
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
fixture="$root_dir/target/release/kutrace-diff"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
module="$repo_dir/linux/setup/module/kutrace_mod.ko"

for command in cargo g++ jq modinfo python3 rg sort sudo uname; do
  command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done
[[ $(uname -m) == x86_64 ]] || { echo 'live differential currently requires x86_64' >&2; exit 1; }
rg -q '^kutrace_mod ' /proc/modules || { echo 'kutrace_mod is not loaded; refusing to insert it automatically' >&2; exit 1; }
[[ -f "$module" ]] || { echo "missing reference module: $module" >&2; exit 1; }
module_vermagic=$(modinfo -F vermagic "$module" | awk '{print $1}')
[[ "$module_vermagic" == "$(uname -r)" ]] || {
  echo "module vermagic $module_vermagic does not match running kernel $(uname -r)" >&2
  exit 1
}

legacy_state=$(python3 - <<'PY'
import ctypes
import errno

libc = ctypes.CDLL(None, use_errno=True)
state = libc.syscall(1023, 10, 0)
if state < 0:
    raise SystemExit(f"kutrace control syscall unavailable: errno={ctypes.get_errno()}")
print(state)
PY
)
[[ "$legacy_state" == 0 ]] || {
  echo 'legacy KUtrace is already tracing; refusing to reset another capture' >&2
  exit 1
}

cargo build --release --manifest-path "$root_dir/Cargo.toml" \
  -p kutrace-collector -p kutrace-transform -p kutrace-differential-fixture >/dev/null
[[ -x "$collector" && -x "$transform" && -x "$fixture" ]] || exit 1
[[ -r "$ebpf_object" ]] || { echo "missing eBPF object: $ebpf_object" >&2; exit 1; }

g++ -O2 "$repo_dir/postproc/kutrace_control.cc" "$repo_dir/postproc/kutrace_lib.cc" \
  -o "$result_dir/kutrace_control"
g++ -O2 "$repo_dir/postproc/rawtoevent.cc" "$repo_dir/postproc/from_base40.cc" \
  -o "$result_dir/rawtoevent"
g++ -O2 "$repo_dir/postproc/eventtospan3.cc" -o "$result_dir/eventtospan3"

wait_for_line() {
  local file=$1 pattern=$2
  for _ in $(seq 1 200); do
    if [[ -f "$file" ]] && rg -q "$pattern" "$file"; then
      return 0
    fi
    sleep 0.025
  done
  echo "timed out waiting for '$pattern' in $file" >&2
  return 1
}

# Start and stop legacy capture in one controller process. Its calibration
# time-pair state is process-local, so two one-shot controller invocations
# produce a formally invalid trace even though the kernel buffer is shared.
mkfifo "$result_dir/legacy.commands"
"$result_dir/kutrace_control" <"$result_dir/legacy.commands" \
  >"$result_dir/legacy-control.out" 2>"$result_dir/legacy-control.log" &
controller_pid=$!
exec 3>"$result_dir/legacy.commands"
printf 'go\n' >&3
for _ in $(seq 1 100); do
  legacy_state=$(python3 - <<'PY'
import ctypes
print(ctypes.CDLL(None).syscall(1023, 10, 0))
PY
)
  [[ "$legacy_state" == 1 ]] && break
  sleep 0.01
done
[[ "$legacy_state" == 1 ]] || { echo 'legacy tracing did not start' >&2; exit 1; }
legacy_started=true

"$fixture" >"$result_dir/legacy-fixture.jsonl" 2>"$result_dir/legacy-fixture.log" &
fixture_pid=$!
wait_for_line "$result_dir/legacy-fixture.jsonl" '"pid"'
legacy_full_pid=$(sed -n '1p' "$result_dir/legacy-fixture.jsonl" | jq -r .pid)
[[ "$legacy_full_pid" == "$fixture_pid" ]]
kill -USR1 "$fixture_pid"
wait "$fixture_pid"
fixture_pid=
printf 'stop %s\n' "$result_dir/legacy.trace" >&3
exec 3>&-
wait "$controller_pid"
controller_pid=
legacy_started=false
"$result_dir/rawtoevent" "$result_dir/legacy.trace" \
  >"$result_dir/legacy.events" 2>"$result_dir/rawtoevent.log"
LC_ALL=C sort -n "$result_dir/legacy.events" \
  | "$result_dir/eventtospan3" 'Patched-kernel differential capture' \
  >"$result_dir/legacy.json" 2>"$result_dir/legacy-span.log"
jq -e '.version == 3' "$result_dir/legacy.json" >/dev/null

# Aya attaches to the already-waiting process. Polling its explicit ready log
# avoids comparing a workload that raced probe attachment.
"$fixture" >"$result_dir/aya-fixture.jsonl" 2>"$result_dir/aya-fixture.log" &
fixture_pid=$!
wait_for_line "$result_dir/aya-fixture.jsonl" '"pid"'
aya_pid=$(sed -n '1p' "$result_dir/aya-fixture.jsonl" | jq -r .pid)
[[ "$aya_pid" == "$fixture_pid" ]]
sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$result_dir/aya.kuevents" \
  --agent-shm "$result_dir/agent.shm" \
  --agent-socket "$result_dir/agent.sock" \
  --pid "$fixture_pid" \
  --sample-hz 0 \
  --duration-secs 1.25 \
  >"$result_dir/collector.out" 2>"$result_dir/collector.log" &
collector_pid=$!
wait_for_line "$result_dir/collector.log" 'capturing to'
kill -USR1 "$fixture_pid"
wait "$fixture_pid"
fixture_pid=
wait "$collector_pid"
collector_pid=
rg -q 'bpf_dropped_events=0' "$result_dir/collector.log"
rg -q 'probe_dropped_events=0' "$result_dir/collector.log"
"$transform" events "$result_dir/aya.kuevents" --output "$result_dir/aya.events"
"$result_dir/eventtospan3" 'Aya differential capture' \
  <"$result_dir/aya.events" >"$result_dir/aya.json" 2>"$result_dir/aya-span.log"
jq -e '.version == 3' "$result_dir/aya.json" >/dev/null

legacy_events="$result_dir/legacy.events" aya_events="$result_dir/aya.events" \
legacy_pid="$legacy_full_pid" aya_pid="$aya_pid" result_path="$result_dir/comparison.json" \
python3 - <<'PY'
import collections
import json
import os

iterations = 128
selected = {
    "getppid": 1,
    "getuid": 1,
    "getgid": 1,
    "getpgid": 2,
    "getsid": 2,
    "sched_yield": 1,
    "kill": 1,
}
arg0_syscalls = {"getpgid", "getsid", "kill"}

def return_class(value):
    value = int(value)
    if value < 0:
        return f"errno:{-value}"
    # Patched KUtrace stores syscall returns in 16 bits. Optimized negative
    # returns therefore appear as unsigned int16 values in rawtoevent output.
    if 0xff80 <= value <= 0xffff:
        return f"errno:{0x10000 - value}"
    return "success"

def signature(path, pid):
    result = collections.Counter()
    entries = collections.Counter()
    exits = collections.Counter()
    with open(path, encoding="utf-8") as stream:
        for line in stream:
            fields = line.split()
            if len(fields) < 11 or line.startswith("#") or int(fields[4]) != pid:
                continue
            event = int(fields[2])
            name = fields[9].removeprefix("/")
            if name not in selected:
                continue
            if 0x800 <= event < 0xa00:
                # The legacy record preserves only arg0's low 16 bits. Registers
                # for no-argument syscalls are unspecified and must not be
                # compared as arguments.
                arg0 = int(fields[6]) & 0xffff if name in arg0_syscalls else "unused"
                key = (event, name, arg0)
                entries[key] += 1
                result[("entry", *key)] += 1
            elif 0xa00 <= event < 0xc00:
                key = (event, name, return_class(fields[7]))
                exits[key] += 1
                result[("exit", *key)] += 1
    for name, calls_per_iteration in selected.items():
        expected = calls_per_iteration * iterations
        assert sum(v for k, v in entries.items() if k[1] == name) == expected, (path, name, entries)
        assert sum(v for k, v in exits.items() if k[1] == name) == expected, (path, name, exits)
    return result

legacy_full_pid = int(os.environ["legacy_pid"])
legacy_pid = legacy_full_pid & 0xffff
aya_pid = int(os.environ["aya_pid"])
legacy = signature(os.environ["legacy_events"], legacy_pid)
aya = signature(os.environ["aya_events"], aya_pid)
assert legacy == aya, {"legacy_only": legacy - aya, "aya_only": aya - legacy}

records = [
    {"kind": key[0], "event": key[1], "name": key[2], "value": key[3], "count": count}
    for key, count in sorted(legacy.items(), key=lambda item: tuple(map(str, item[0])))
]
report = {
    "schema": 1,
    "iterations": iterations,
    "syscalls_per_iteration": sum(selected.values()),
    "total_paired_syscalls": iterations * sum(selected.values()),
    "legacy_full_pid": legacy_full_pid,
    "legacy_pid16": legacy_pid,
    "aya_pid": aya_pid,
    "semantic_signatures_equal": True,
    "strict_legacy_json": {"patched_kernel": True, "aya": True},
    "bpf_dropped_events": 0,
    "probe_dropped_events": 0,
    "signature": records,
}
with open(os.environ["result_path"], "w", encoding="utf-8") as stream:
    json.dump(report, stream, indent=2)
    stream.write("\n")
print(json.dumps({key: report[key] for key in (
    "total_paired_syscalls", "semantic_signatures_equal", "strict_legacy_json",
    "bpf_dropped_events", "probe_dropped_events")}, sort_keys=True))
PY

if [[ "$keep_results" == true ]]; then
  echo "comparison=$result_dir/comparison.json"
fi
