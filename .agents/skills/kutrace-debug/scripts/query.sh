#!/usr/bin/env bash
set -euo pipefail

if (($# < 2 || $# > 3)); then
  echo "usage: $0 URL SQL [LIMIT]" >&2
  exit 2
fi

url=${1%/}
sql=$2
limit=${3:-1000}
[[ "$limit" =~ ^[0-9]+$ ]] && ((limit > 0 && limit <= 50000)) || {
  echo "LIMIT must be an integer from 1 through 50000" >&2
  exit 2
}

for command in curl jq; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done

payload=$(jq -n --arg sql "$sql" --argjson limit "$limit" '{sql:$sql,limit:$limit}')
curl --fail-with-body --silent --show-error \
  -H 'content-type: application/json' \
  --data "$payload" \
  "$url/api/query" |
  jq .
