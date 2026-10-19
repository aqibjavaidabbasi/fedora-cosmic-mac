#!/bin/sh
# Run once as root (the Control Center asks via pkexec): lets members of the "wheel"
# group turn the touchscreen on and off by writing its sysfs "inhibited" switch.
set -e
cat > /etc/udev/rules.d/70-cosmic-mac-touch.rules <<'RULES'
# COSMIC Mac: let wheel users turn the touchscreen on/off (Control Center "Touch" tile)
ACTION=="add|change", SUBSYSTEM=="input", KERNEL=="input[0-9]*", ENV{ID_INPUT_TOUCHSCREEN}=="1", RUN+="/bin/sh -c 'chgrp wheel /sys%p/inhibited; chmod g+w /sys%p/inhibited'"
ACTION=="add|change", SUBSYSTEM=="input", KERNEL=="input[0-9]*", ATTR{name}=="Intel Touch Host Controller", RUN+="/bin/sh -c 'chgrp wheel /sys%p/inhibited; chmod g+w /sys%p/inhibited'"
RULES
udevadm control --reload
# apply to the devices that are already there
for n in /sys/class/input/input*/name; do
    d=$(dirname "$n")
    case "$(cat "$n")" in "Intel Touch Host Controller") ;; *)
        grep -qs "ID_INPUT_TOUCHSCREEN=1" /run/udev/data/c13:$(cat "$d"/event*/dev 2>/dev/null | cut -d: -f2 | head -1) || continue ;;
    esac
    chgrp wheel "$d/inhibited" && chmod g+w "$d/inhibited"
done
