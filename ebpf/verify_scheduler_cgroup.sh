#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
verify_dir=$(mktemp -d /tmp/kutrace-scheduler-cgroup-verify.XXXXXX)
cleanup() {
  if [[ -n "${fixture_pid:-}" ]] && kill -0 "$fixture_pid" 2>/dev/null; then
    kill "$fixture_pid" 2>/dev/null || true
    wait "$fixture_pid" 2>/dev/null || true
  fi
  find "$verify_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$verify_dir"
}
trap cleanup EXIT

collector="$root_dir/target/release/kutrace-collector"
transform="$root_dir/target/release/kutrace-transform"
ebpf_object="$root_dir/kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
cgroup_relative=$(awk -F: '$1 == "0" {print $3}' /proc/self/cgroup)
cgroup_path="/sys/fs/cgroup$cgroup_relative"
cgroup_id=$(stat -c %i "$cgroup_path")

KUTRACE_THREAD_BEFORE_ATTACH=1 \
  python3 "$root_dir/scheduler_thread_fixture.py" >"$verify_dir/fixture.jsonl" &
fixture_pid=$!
for _ in $(seq 1 100); do
  if [[ $(wc -l <"$verify_dir/fixture.jsonl") -ge 2 ]]; then
    break
  fi
  sleep 0.01
done
[[ $(wc -l <"$verify_dir/fixture.jsonl") -ge 2 ]]
target_pid=$(sed -n '1p' "$verify_dir/fixture.jsonl" | jq -r .pid)
thread_tid=$(sed -n '2p' "$verify_dir/fixture.jsonl" | jq -r .tid)
[[ "$target_pid" == "$fixture_pid" ]]

sudo "$collector" \
  --ebpf "$ebpf_object" \
  --output "$verify_dir/capture.kuevents" \
  --agent-shm "$verify_dir/agent.shm" \
  --agent-socket "$verify_dir/agent.sock" \
  --cgroup-id "$cgroup_id" \
  --sample-hz 0 \
  --duration-secs 1.5 >"$verify_dir/collector.out" 2>"$verify_dir/collector.log" &
collector_pid=$!

for _ in $(seq 1 100); do
  if [[ -e "$verify_dir/agent.shm" ]]; then
    break
  fi
  sleep 0.01
done
[[ -e "$verify_dir/agent.shm" ]]
kill -USR1 "$fixture_pid"
wait "$fixture_pid"
fixture_pid=
wait "$collector_pid"

"$transform" events "$verify_dir/capture.kuevents" --output "$verify_dir/events.txt"
events_path="$verify_dir/events.txt" thread_tid="$thread_tid" python3 - <<'PY'
import os

tid = int(os.environ["thread_tid"])
records = []
with open(os.environ["events_path"], encoding="utf-8") as stream:
    for line in stream:
        fields = line.split()
        if len(fields) < 10 or not fields[0].isdigit():
            continue
        event = int(fields[2])
        pid = int(fields[4])
        if pid == tid and (event == 0x200 or 0x800 <= event <= 0xbfe):
            records.append((int(fields[0]), event, fields[9]))
assert records, "pre-existing cgroup thread produced no scheduler/syscall records"
assert records[0][1] == 0x200, records[:10]
assert any(0x800 <= event <= 0xbfe for _, event, _ in records), records[:10]
print(f"thread_tid={tid} first_event=0x{records[0][1]:x} records={len(records)}")
PY

g++ -O2 "$root_dir/../postproc/eventtospan3.cc" -o "$verify_dir/eventtospan3"
"$verify_dir/eventtospan3" "Scheduler cgroup preseed integration" \
  <"$verify_dir/events.txt" >"$verify_dir/capture.json"
jq -e '.version == 3' "$verify_dir/capture.json" >/dev/null
grep -Eq 'seeded [1-9][0-9]* existing target cgroup TIDs .* before scheduler attachment' \
  "$verify_dir/collector.log"
grep -q 'bpf_dropped_events=0' "$verify_dir/collector.log"
echo "cgroup_id=$cgroup_id strict_legacy_json=true first_existing_thread_switch_retained=true bpf_dropped_events=0"
