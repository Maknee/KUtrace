#!/bin/bash
# arg 1 filename stem (no .trace), arg 2 "title", arg 3/4 spantrim args

set -x

# Must sort by pure byte values, not local collating sequence
export LC_ALL=C

# Strip trailing .trace if it is there
var1=${1%.trace}

cat $var1.trace  |./rawtoevent |sort -n |./eventtospan3 "$2" |sort >$var1.json
echo "  $var1.json written"

# sudo cat /proc/kallsyms | sort > kallsyms.txt
# cat $var1.json | ./samptoname_k ${var1}.kallsyms > $var1.json
# sudo ls /proc/self/maps |xargs -I % sh -c 'echo \"\\n====\" %; sudo cat %' > symbols.txt
# cat $var1.json | ./samptoname_u ${var1}.procmaps > $var1.json
cat $var1.json | ./samptoname_k ${var1}.kallsyms | ./samptoname_u ${var1}.procmaps > ${var1}_sym.json
mv ${var1}_sym.json $var1.json

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

