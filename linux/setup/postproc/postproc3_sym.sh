#!/bin/bash
# Offline-only symbol/name postprocessing.
# arg 1 filename stem (no .trace), arg 2 "title", arg 3/4 spantrim args
# Optional saved sidecars (this script never reads /proc or /sys):
#   STEM.kallsyms, STEM.procmaps, STEM.pidnames
# Optional KUTRACE_BINARY_ROOT points to an offline filesystem snapshot.

set -x

# Must sort by pure byte values, not local collating sequence
export LC_ALL=C

# Strip trailing .trace if it is there
var1=${1%.trace}

cat "$var1.trace" | ./rawtoevent | sort -n | ./eventtospan3 "$2" | sort > "$var1.json"
echo "  $var1.json written"

resolver_args=("$var1.json" --output "$var1.json")
if [ -f "${var1}.kallsyms" ]; then resolver_args+=(--kallsyms "${var1}.kallsyms"); fi
if [ -f "${var1}.procmaps" ]; then resolver_args+=(--procmaps "${var1}.procmaps"); fi
if [ -f "${var1}.pidnames" ]; then resolver_args+=(--pid-names "${var1}.pidnames"); fi
if [ -n "$KUTRACE_BINARY_ROOT" ]; then resolver_args+=(--binary-root "$KUTRACE_BINARY_ROOT"); fi
./resolve_trace_offline.py "${resolver_args[@]}"

trim_arg='0'
if [ -n "$3" ]
then
delimit=' '
trim_arg=$3$delimit$4
fi

# cat $var1.json | jq -c '.' | gzip | xxd -p -c 1 | sed 's/.*/0x&,/' | tr -d '\n' | ./makeself show_cpu.html >$var1.html
cat $var1.json | ./spantotrim $trim_arg | jq -c '.' | gzip -9 | xxd -p -c 1 | sed 's/.*/0x&,/' | tr -d '\n' | ./makeself show_cpu_fast.html >$var1.html
# cat $var1.json |./spantotrim $trim_arg | ./makeself show_cpu.html >$var1.html
# cat $var1.json |./spantotrim $trim_arg | ./makeself show_cpu.html >$var1.html
echo "  $var1.html written"

# Open in a browser only when one is available and there is a display.
# On a headless node (e.g. a remote server), just report the path.
if [ -n "$DISPLAY" ] && command -v google-chrome >/dev/null 2>&1; then
  google-chrome $var1.html &
elif [ -n "$DISPLAY" ] && command -v xdg-open >/dev/null 2>&1; then
  xdg-open $var1.html &
else
  echo "  (headless: copy $var1.html to a desktop and open in Chrome to view)"
fi
