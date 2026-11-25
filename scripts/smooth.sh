#!/usr/bin/env bash
# Smoothness test: runs each movement on the phone and reads, from the
# screen compositor, when every frame of ours reached the screen.
# A steady 60 per second means a frame every 16.7 ms. Two counts per movement:
#   gaps   - refreshes the screen showed no new frame of ours (over 20 ms).
#            While a finger is held still there is often nothing new to show,
#            so gaps there are not drops.
#   missed - frames the launcher wanted and drew more than a refresh late
#            (its own "slow beat" log). This is the real dropped-frame count.
# Held and dragged moves use scripts/touchgen.c, a steady 125-a-second finger
# on the XZ3's touchscreen; Android's `input swipe` feeds touches unevenly.
# Usage: scripts/smooth.sh   (XZ3, launcher is Home, showing page 1)
set -euo pipefail

if ! adb shell test -x /data/local/tmp/touchgen; then
    ndk="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/28.2.13676358}"
    tmp=$(mktemp -d)
    "$ndk"/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang -O2 \
        -o "$tmp/touchgen" "$(dirname "$0")/touchgen.c"
    adb push "$tmp/touchgen" /data/local/tmp/ >/dev/null
    adb shell chmod +x /data/local/tmp/touchgen
    rm -r "$tmp"
fi
finger() { adb shell /data/local/tmp/touchgen "$@"; }

# The launcher's own timing lines need the log switched on.
adb shell setprop debug.launcherrs.log info
adb shell am force-stop dev.launcherrs.app
adb shell am start -W -a android.intent.action.MAIN -c android.intent.category.HOME >/dev/null
sleep 2

layer() {
    adb shell dumpsys SurfaceFlinger --list | tr -d '\r' | grep -E '^dev.launcherrs.app/dev.launcherrs.app.MainActivity#' | tail -1
}

# report NAME: read the frames since the last clear and print one line.
report() {
    adb shell dumpsys SurfaceFlinger --latency "$(layer)" | tr -d '\r' | python3 -c '
import sys
name = sys.argv[1]
lines = sys.stdin.read().split("\n")
period = int(lines[0]) / 1e6
t = sorted(int(c[1]) for c in (l.split() for l in lines[1:]) if len(c) == 3 and 0 < int(c[1]) < 2**62)
gaps = [(b - a) / 1e6 for a, b in zip(t, t[1:])]
inside = [g for g in gaps if g < 150]
late = [g for g in inside if g > period * 1.2]
worst = max(inside) if inside else 0
import re
slow = re.findall(r"slow (?:beat|frame|step).*", sys.argv[2])
missed = sum(1 for x in slow if x.startswith("slow beat"))
notes = "; ".join(x.strip() for x in slow)[:160]
print(f"{name:<24} frames {len(t):>3}  gaps {len(late):>2}  longest {worst:5.1f} ms  missed {missed:>2}  {notes}")
' "$1" "$(adb logcat -d -s launcher)"
}

clear() { adb shell dumpsys SurfaceFlinger --latency-clear "$(layer)" >/dev/null; adb logcat -c; }

run() { # run NAME COMMAND...
    local name=$1; shift
    clear
    "$@"
    sleep 1.2
    report "$name"
}

home() { adb shell input keyevent HOME; sleep 1; }

home
run "page swipe left"        finger 900 1150 150 1150 200
run "page swipe right"       finger 150 1150 900 1150 200
run "list open (swipe up)"   finger 540 1250 540 500 150
run "list fling down"        finger 540 1600 540 500 80
run "list fling back"        finger 540 500 540 1600 80
sleep 1.5
run "list close (pull down)" finger 540 600 540 1500 200
run "list open (button)"     adb shell input tap 540 1717
run "list close (Back)"      adb shell input keyevent BACK
run "long-press menu"        finger 153 640 153 640 900
adb shell input keyevent BACK; sleep 0.5
run "drag an icon"           finger 927 640 927 640 600 670 885 600 670 885 200
run "drag it back"           finger 670 885 670 885 600 927 640 600 927 640 200
run "settings open"          sh -c 'adb shell /data/local/tmp/touchgen 400 1130 400 1130 900; sleep 0.4; adb shell input tap 590 1026'
run "settings fling"         finger 540 1800 540 500 80
sleep 1.5
run "settings fling back"    finger 540 500 540 1800 80
sleep 1.5
run "settings close (Back)"  adb shell input keyevent BACK

# Idle: after everything stops, no frames at all (the clock redraws once
# when the minute changes, so the window avoids a minute boundary).
sleep 2
while [ "$(date +%S | sed 's/^0//')" -gt 45 ]; do sleep 1; done
clear
sleep 10
echo "idle 10 s: $(adb shell dumpsys SurfaceFlinger --latency "$(layer)" | tr -d '\r' | awk 'NR>1 && $2>0 && $2<4611686018427387904' | wc -l) frames"
