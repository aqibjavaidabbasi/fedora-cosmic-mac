#!/bin/bash
# Samples battery draw (60 s), interrupt deltas (10 s) and CPU of the shell parts.
# Usage: measure.sh [label]     battery draw, interrupts, CPU
#        measure.sh --wakeups    wakeups/s of the top bar and the cosmic-mac applets (30 s)
# Results are also appended to power/measurements.txt.
D=$HOME/.local/share/cosmic-mac/power
OUT=$D/measurements.txt
if [ "${1:-}" != --wakeups ]; then {
echo "=== ${1:-sample} $(date '+%F %T') ==="
B=$(ls -d /sys/class/power_supply/BAT* | head -1)
if [ "$(cat "$B/status")" = Discharging ]; then
    s=0; n=0; min=; max=
    for _ in $(seq 30); do
        p=$(( $(cat "$B/power_now") / 1000 )); s=$((s+p)); n=$((n+1))
        [ -z "$min" ] || [ $p -lt $min ] && min=$p; [ -z "$max" ] || [ $p -gt $max ] && max=$p
        sleep 2
    done
    echo "battery: avg $((s/n)) mW  min $min  max $max  ($n samples, 2 s apart)"
else
    echo "battery: on AC ($(cat "$B/status")), power_now not meaningful"
fi
irq() { awk '/^ *LOC:/{s=0;for(i=2;i<=NF&&$i~/^[0-9]+$/;i++)s+=$i;print "LOC",s}
             /i915/{s=0;for(i=2;i<=NF&&$i~/^[0-9]+$/;i++)s+=$i;print "i915",s}' /proc/interrupts; }
irq > /tmp/.irq1.$$; sleep 10; irq > /tmp/.irq2.$$
join /tmp/.irq1.$$ /tmp/.irq2.$$ | awk '{printf "irq %s: %d/s\n",$1,($3-$2)/10}'
rm -f /tmp/.irq[12].$$
echo "cpu (top, 10 s average):"
top -b -d 10 -n 2 -w 200 | awk '/^top -/{n++} n==2' | grep -E 'cosmic|cosmic-mac|Xwayland|claude|electron' | head -25 \
    | awk '{printf "  %-6s %5s%%  %s\n",$1,$9,$12}'
echo
} | tee -a "$OUT"
fi
# Wakeups (context switches) per second of the top bar and the cosmic-mac applets, over 30 s.
if [ "${1:-}" = --wakeups ]; then
    echo "=== wakeups $(date '+%F %T') ===" | tee -a "$OUT"
    pids=$(pgrep -x cosmic-panel; pgrep -f '^/?[^ ]*(cosmic-mac-[a-z-]+|cosmic-applet-notifications)( |$)')
    snap() { for p in $pids; do cat /proc/$p/task/*/status 2>/dev/null | awk -v p=$p '/ctxt_switches/{s+=$2} END{print p, s}'; done; }
    snap > /tmp/.cs1.$$; sleep 30; snap > /tmp/.cs2.$$
    join /tmp/.cs1.$$ /tmp/.cs2.$$ | while read -r p a b; do printf '  %-30s %6.1f wakeups/s\n' "$(cat /proc/$p/comm 2>/dev/null)" "$(echo "($b-$a)/30" | bc -l)"; done | sort -k1,1 | tee -a "$OUT"
    rm -f /tmp/.cs[12].$$
fi
