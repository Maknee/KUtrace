#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")" && pwd)
bench_dir=$(mktemp -d /tmp/kutrace-ui-import-bench.XXXXXX)
event_count=${KUTRACE_UI_BENCH_EVENTS:-1000000}
name_cardinality=${KUTRACE_UI_BENCH_NAME_CARDINALITY:-1}
index_workers=${KUTRACE_UI_BENCH_INDEX_WORKERS:-0}
max_seconds=${KUTRACE_UI_BENCH_MAX_SECONDS:-30}
max_rss_kib=${KUTRACE_UI_BENCH_MAX_RSS_KIB:-32768}
port=${KUTRACE_UI_BENCH_PORT:-38127}
[[ "$event_count" =~ ^[0-9]+$ ]] && ((event_count >= 125)) || {
  echo 'KUTRACE_UI_BENCH_EVENTS must be an integer of at least 125' >&2
  exit 2
}
[[ "$name_cardinality" =~ ^[0-9]+$ ]] && ((name_cardinality >= 1 && name_cardinality <= event_count)) || {
  echo 'KUTRACE_UI_BENCH_NAME_CARDINALITY must be between 1 and KUTRACE_UI_BENCH_EVENTS' >&2
  exit 2
}
query_end=$(awk -v events="$event_count" 'BEGIN {printf "%.9f", events / 1000000.0}')
query_bucket=$(awk -v end="$query_end" 'BEGIN {printf "%.12f", end / 125.0}')
if awk -v bucket="$query_bucket" 'BEGIN {exit !(bucket >= 0.064)}'; then
  mipmap_table=timeline_mipmap_coarse
  mipmap_level=coarse
else
  mipmap_table=timeline_mipmap
  mipmap_level=fine
fi

cleanup() {
  if [[ -n "${server_pid:-}" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  find "$bench_dir" -mindepth 1 -maxdepth 1 -delete
  rmdir "$bench_dir"
}
trap cleanup EXIT

fixture="$root_dir/target/release/kutrace-ui-fixture"
workspace="$root_dir/target/release/kutrace-ui"
[[ -x "$fixture" && -x "$workspace" ]] || {
  echo "build first with: cargo build --release -p kutrace-ui" >&2
  exit 2
}

"$fixture" "$bench_dir/trace.json" "$event_count" "$name_cardinality"
/usr/bin/time \
  -f '{"wall_seconds":%e,"user_seconds":%U,"system_seconds":%S,"maximum_resident_kib":%M}' \
  -o "$bench_dir/resources.json" \
  "$workspace" "$bench_dir/trace.json" \
    --database "$bench_dir/trace.sqlite" \
    --index-workers "$index_workers" \
    --rebuild --import-only >"$bench_dir/import.json"

"$workspace" "$bench_dir/trace.json" \
  --database "$bench_dir/trace.sqlite" \
  --listen "127.0.0.1:$port" >"$bench_dir/server.out" 2>"$bench_dir/server.log" &
server_pid=$!
for _ in $(seq 1 100); do
  if curl --silent --fail "http://127.0.0.1:$port/" >/dev/null; then
    break
  fi
  sleep 0.02
done

bucket_sql="WITH RECURSIVE mapped(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
  SELECT MAX(0,CAST(bucket_start/$query_bucket AS INTEGER)),
         MIN(124,CAST(bucket_end/$query_bucket AS INTEGER)),
         cpu,event,name,category,ipc,bucket_start,bucket_end,weight,count
    FROM $mipmap_table WHERE bucket_start < $query_end AND bucket_end > 0.0
), map_expanded(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
  SELECT * FROM mapped
  UNION ALL
  SELECT bucket+1,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count
    FROM map_expanded WHERE bucket<last_bucket
), long_events(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
  SELECT cpu,event,name,category,ipc,ts,ts_end,
         MAX(0,CAST(ts/$query_bucket AS INTEGER)),
         MIN(124,CAST(ts_end/$query_bucket AS INTEGER))
    FROM events
   WHERE (dur=0 OR dur>0.256)
     AND ((dur=0 AND ts>=0.0 AND ts<$query_end) OR (dur>0 AND ts<$query_end AND ts_end>0.0))
), long_expanded(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
  SELECT * FROM long_events
  UNION ALL
  SELECT cpu,event,name,category,ipc,ts,ts_end,bucket+1,last_bucket
    FROM long_expanded WHERE bucket<last_bucket
), combined AS (
  SELECT bucket,cpu,event,name,category,ipc,
         weight*MAX(0,MIN(source_end,$query_end,(bucket+1)*$query_bucket)-MAX(source_start,0.0,bucket*$query_bucket))/(source_end-source_start) weight,
         count
    FROM map_expanded
  UNION ALL
  SELECT bucket,cpu,event,name,category,ipc,
         MAX(0,MIN(ts_end,$query_end,(bucket+1)*$query_bucket)-MAX(ts,0.0,bucket*$query_bucket)) weight,
         1 count
    FROM long_expanded
), scored AS (
  SELECT bucket,cpu,event,name,category,ipc,SUM(weight) weight,SUM(count) count
    FROM combined GROUP BY bucket,cpu,event,name,category,ipc
), ranked AS (
  SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket,cpu ORDER BY weight DESC,count DESC,event,name,category,ipc) rank
    FROM scored
)
SELECT bucket,cpu,event,name,category,ipc,weight,count
  FROM ranked WHERE rank=1 ORDER BY cpu,bucket"
query_status=$(jq -nc --arg sql "$bucket_sql" '{sql:$sql,limit:10000}' \
  | curl --silent --show-error \
      -o "$bench_dir/query.json" -w '%{http_code}' \
      -H 'content-type: application/json' --data-binary @- \
      "http://127.0.0.1:$port/api/query")
if [[ "$query_status" != 200 ]]; then
  echo "timeline query failed with HTTP $query_status: $(cat "$bench_dir/query.json")" >&2
  exit 1
fi
if ((name_cardinality == 1)); then
  expected_distinct_names=3
  target_name=getpid
else
  expected_distinct_names=$name_cardinality
  target_name=trace.name.1
fi
name_sql="SELECT (SELECT COUNT(DISTINCT name) FROM events) AS distinct_names,
                 (SELECT COUNT(*) FROM events WHERE name = '$target_name') AS target_rows"
name_query_status=$(jq -nc --arg sql "$name_sql" '{sql:$sql,limit:1}' \
  | curl --silent --show-error \
      -o "$bench_dir/name-query.json" -w '%{http_code}' \
      -H 'content-type: application/json' --data-binary @- \
      "http://127.0.0.1:$port/api/query")
if [[ "$name_query_status" != 200 ]]; then
  echo "name-cardinality query failed with HTTP $name_query_status: $(cat "$bench_dir/name-query.json")" >&2
  exit 1
fi
kill "$server_pid"
wait "$server_pid" 2>/dev/null || true
server_pid=

json_bytes=$(stat -c %s "$bench_dir/trace.json")
database_bytes=$(stat -c %s "$bench_dir/trace.sqlite")
jq -e --argjson events "$event_count" '.events == $events' "$bench_dir/import.json" >/dev/null
jq -e --argjson maximum "$max_seconds" '.wall_seconds <= $maximum' \
  "$bench_dir/resources.json" >/dev/null
jq -e --argjson maximum "$max_rss_kib" '.maximum_resident_kib <= $maximum' \
  "$bench_dir/resources.json" >/dev/null
jq -e '.truncated == false and (.rows | length) == 8000 and .elapsed_ms < 2000' \
  "$bench_dir/query.json" >/dev/null
jq -e --argjson names "$expected_distinct_names" \
  '.truncated == false and .rows[0][0] == $names and .rows[0][1] > 0 and .elapsed_ms < 2000' \
  "$bench_dir/name-query.json" >/dev/null

jq -n \
  --arg date "$(date -u +%F)" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg cpu "$(awk -F: '/^model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' /proc/cpuinfo)" \
  --arg target_name "$target_name" \
  --arg mipmap_level "$mipmap_level" \
  --argjson logical_cpus "$(getconf _NPROCESSORS_ONLN)" \
  --argjson fixture_events "$event_count" \
  --argjson requested_name_cardinality "$name_cardinality" \
  --argjson distinct_names "$expected_distinct_names" \
  --argjson query_span_seconds "$query_end" \
  --argjson json_bytes "$json_bytes" \
  --argjson database_bytes "$database_bytes" \
  --argjson max_seconds "$max_seconds" \
  --argjson max_rss_kib "$max_rss_kib" \
  --slurpfile import_result "$bench_dir/import.json" \
  --slurpfile resources "$bench_dir/resources.json" \
  --slurpfile query "$bench_dir/query.json" \
  --slurpfile name_query "$bench_dir/name-query.json" \
  '{date_utc:$date,
    host:{cpu:$cpu,logical_cpus:$logical_cpus,kernel:$kernel,architecture:$architecture},
    fixture:{events:$fixture_events,json_bytes:$json_bytes,requested_name_cardinality:$requested_name_cardinality,distinct_names:$distinct_names},
    "import": (($import_result[0]+$resources[0]+{database_bytes:$database_bytes})|del(.database)),
    gates:{maximum_seconds:$max_seconds,maximum_resident_kib:$max_rss_kib},
    dominant_bucket_query:{input_events:$fixture_events,span_seconds:$query_span_seconds,mipmap_level:$mipmap_level,result_rows:($query[0].rows|length),elapsed_ms:$query[0].elapsed_ms,truncated:$query[0].truncated},
    name_cardinality_query:{target_name:$target_name,target_rows:$name_query[0].rows[0][1],elapsed_ms:$name_query[0].elapsed_ms,truncated:$name_query[0].truncated},
    status:"pass"}'
