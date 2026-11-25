#!/usr/bin/env bash
# Smoothness test for the Galaxy Z Fold cover screen (1080x2520, up to 120 Hz).
# Android 17 blocks writing to its touchscreen, so moves use `input swipe`;
# judge by "missed" (frames the launcher drew late mid-movement). Android 17
# also gives no per-frame timings for this window, so the frame columns are
# empty there. Usage: ANDROID_SERIAL=<fold> scripts/smooth_fold.sh
: "${ANDROID_SERIAL:?set ANDROID_SERIAL to the Fold}"
layer() { adb shell dumpsys SurfaceFlinger --list | tr -d '\r' | grep -E '^dev.launcherrs.app/dev.launcherrs.app.MainActivity#' | tail -1; }
report() {
  adb shell dumpsys SurfaceFlinger --latency "$(layer)" | tr -d '\r' | python3 -c '
import sys, re
name = sys.argv[1]
lines = sys.stdin.read().split("\n")
t = sorted(int(c[1]) for c in (l.split() for l in lines[1:]) if len(c) == 3 and 0 < int(c[1]) < 2**62)
gaps = [(b - a) / 1e6 for a, b in zip(t, t[1:])]
inside = [g for g in gaps if g < 150]
fast = sorted(inside)[len(inside)//4] if inside else 0
slow = re.findall(r"slow (?:beat|frame|step).*", sys.argv[2])
missed = sum(1 for x in slow if x.startswith("slow beat"))
fps = 1000*len(inside)/sum(inside) if inside else 0
print(f"{name:<24} frames {len(t):>3}  avg {fps:5.1f}/s  typical gap {fast:4.1f} ms  longest {max(inside) if inside else 0:5.1f}  missed {missed:>2}  {"; ".join(slow)[:300]}")
' "$1" "$(adb logcat -d -s launcher)"
}
clear() { adb shell dumpsys SurfaceFlinger --latency-clear "$(layer)" >/dev/null; adb logcat -c; }
run() { local n=$1; shift; clear; "$@"; sleep 1.2; report "$n"; }
adb shell setprop debug.launcherrs.log info
adb shell am force-stop dev.launcherrs.app
adb shell am start -W -a android.intent.action.MAIN -c android.intent.category.HOME >/dev/null
sleep 3
run "list open (swipe up)"   adb shell input swipe 540 1500 540 600 150
run "list fling down"        adb shell input swipe 540 1900 540 600 80
run "list fling down 2"      adb shell input swipe 540 1900 540 600 80
run "list fling back"        adb shell input swipe 540 600 540 1900 80
sleep 1.5
run "list close (pull down)" adb shell input swipe 540 700 540 1700 200
run "list open (button)"     adb shell input tap 540 2175
run "list close (Back)"      adb shell input keyevent BACK
run "long-press dock icon"   adb shell input swipe 145 2175 145 2175 900
adb shell input keyevent BACK; sleep 0.5
run "settings open"          sh -c 'adb shell input swipe 400 1300 400 1300 900; sleep 0.5; adb shell input tap 590 1196'
run "settings fling"         adb shell input swipe 540 2000 540 600 80
sleep 1.5
run "settings close (Back)"  adb shell input keyevent BACK
sleep 2; while [ "$(date +%S | sed 's/^0//')" -gt 45 ]; do sleep 1; done
clear; sleep 10
echo "idle 10 s: $(adb shell dumpsys SurfaceFlinger --latency "$(layer)" | tr -d '\r' | awk 'NR>1 && $2>0 && $2<4611686018427387904' | wc -l) frames"
