#!/usr/bin/env bash
# Build the Rust library into the Android project, package it, and optionally install.
# Usage: scripts/build.sh [install]
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/28.2.13676358}"
cargo ndk -t arm64-v8a -P 26 -o android/app/src/main/jniLibs build --release -p launcher-app
(cd android && ./gradlew --console=plain -q assembleDebug)
APK=android/app/build/outputs/apk/debug/app-debug.apk
echo "built $APK ($(du -h "$APK" | cut -f1))"
if [[ "${1:-}" == "install" ]]; then
    timeout 120 adb install -r "$APK"
fi
