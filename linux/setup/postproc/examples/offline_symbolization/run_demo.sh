#!/bin/bash
set -euo pipefail

example_dir=$(cd "$(dirname "$0")" && pwd)
postproc_dir=$(cd "$example_dir/../.." && pwd)
output_dir=${1:-"$example_dir/build"}
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)

cc -g -O0 -fno-inline -fno-pie -no-pie \
  "$example_dir/demo_target.c" -o "$output_dir/demo_target"

capture="$output_dir/demo.capture"
"$output_dir/demo_target" "$capture"
captured_pid=$(awk '{print $1}' "$capture")

# The target has returned and been reaped before any resolver runs.
if kill -0 "$captured_pid" 2>/dev/null; then
  echo "demo target is unexpectedly still alive: $captured_pid" >&2
  exit 1
fi

python3 "$example_dir/make_demo_trace.py" \
  "$capture" "$output_dir/demo_target" "$output_dir"
python3 "$postproc_dir/resolve_trace_offline.py" "$output_dir/demo.json" \
  --procmaps "$output_dir/demo.procmaps" \
  --pid-names "$output_dir/demo.pidnames" \
  --output "$output_dir/demo_resolved.json"

if ! grep -q '"PC=demo_hot_function"' "$output_dir/demo_resolved.json"; then
  echo "symbolization did not resolve demo_hot_function" >&2
  exit 1
fi

c++ -O2 "$postproc_dir/makeself.cc" -o "$output_dir/makeself"
(
  cd "$postproc_dir"
  gzip -9 -c "$output_dir/demo_resolved.json" | xxd -p -c 1 | \
    sed 's/.*/0x&,/' | tr -d '\n' | \
    "$output_dir/makeself" show_cpu_fast.html \
    > "$output_dir/demo_resolved.html"
)

echo "PASS: target PID $captured_pid was dead before symbolization"
echo "PASS: PC resolved to demo_hot_function"
echo "JSON: $output_dir/demo_resolved.json"
echo "HTML: $output_dir/demo_resolved.html"
