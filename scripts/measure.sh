#!/usr/bin/env bash
# One-line memory report for a package, from `dumpsys meminfo` (kB).
# Usage: scripts/measure.sh <package>
set -euo pipefail
pkg="$1"
out="$(timeout 30 adb shell dumpsys meminfo "$pkg")"
if grep -q "No process found" <<<"$out"; then
    echo "$pkg not running" >&2
    exit 1
fi
summary() { awk -v k="$1" '/App Summary/{s=1} s && index($0, k) {print $(NF-1) == "" ? $NF : $0; exit}' <<<"$out" | grep -oE '[0-9]+' | head -1; }
pss=$(awk '/^ *TOTAL /{print $2; exit}' <<<"$out")
rss=$(grep -oE 'TOTAL RSS: *[0-9]+' <<<"$out" | grep -oE '[0-9]+' || true)
echo "$pkg PSS=$pss RSS=${rss:-n/a} java=$(summary 'Java Heap:') native=$(summary 'Native Heap:') graphics=$(summary 'Graphics:') code=$(summary 'Code:')"
