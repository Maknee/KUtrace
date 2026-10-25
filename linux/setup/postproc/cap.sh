#!/bin/bash
# cap.sh NAME WAIT_SEC TRIM "workload command..."
# Single-session KUtrace capture of an external (non-self-tracing) workload,
# then postprocess to /ssd/kutrace_results/NAME.html with the fast/WASD viewer.
#   NAME      output stem (-> /ssd/kutrace_results/NAME.html, ku_NAME.trace)
#   WAIT_SEC  seconds to trace (kutrace_control 'wait')
#   TRIM      "0" for whole trace, or "START STOP" seconds for spantotrim window
#   workload  command run in background while tracing (its stdout -> /tmp/NAME.out)
set -u
P=/users/makneee/tools/KUtrace/linux/setup/postproc
RES=/ssd/kutrace_results
cd "$P"
NAME="$1"; WAIT="$2"; TRIM="$3"; shift 3
export LC_ALL=C
rm -f "ku_${NAME}.trace"
( sleep 0.4; "$@" >"/tmp/${NAME}.out" 2>"/tmp/${NAME}.err" ) &
printf "goipc\nwait %s\nstop ku_%s\nquit\n" "$WAIT" "$NAME" | ./kutrace_control >/dev/null 2>&1
wait 2>/dev/null
[ -f "ku_${NAME}.trace" ] || { echo "NO TRACE for $NAME"; exit 1; }
echo "  trace: $(du -h ku_${NAME}.trace | cut -f1)"
cat "ku_${NAME}.trace" | ./rawtoevent 2>/dev/null | sort -n | ./eventtospan3 "$NAME" 2>/dev/null | sort > "/tmp/${NAME}.json"
cat "/tmp/${NAME}.json" | ./spantotrim $TRIM 2>/dev/null | jq -c '.' | gzip -9 | xxd -p -c 1 | sed 's/.*/0x&,/' | tr -d '\n' | ./makeself show_cpu_fast.html > "${RES}/${NAME}.html" 2>/dev/null
echo "  html:  ${RES}/${NAME}.html ($(du -h ${RES}/${NAME}.html | cut -f1))"
