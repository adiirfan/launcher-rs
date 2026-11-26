#!/usr/bin/env bash
# Build the Rust library into the Android project, package it, and optionally install.
# Usage: scripts/build.sh [install|release]
#   install  build the debug APK and install it
#   release  build the shrunk release APK and install it
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/28.2.13676358}"
# GALLERY_EMULATOR=1 also builds for the PC-chip Android emulator. Its own
# copy is removed otherwise, so a phone build never carries it.
ABIS=(-t arm64-v8a)
if [[ "${GALLERY_EMULATOR:-}" == 1 ]]; then
    [[ "${1:-}" == "release" ]] && { echo "no emulator build in a release" >&2; exit 1; }
    ABIS+=(-t x86_64)
else
    rm -rf android-gallery/app/src/main/jniLibs/x86_64
fi
cargo ndk "${ABIS[@]}" -P 26 -o android-gallery/app/src/main/jniLibs build --release -p gallery-app
if [[ "${1:-}" == "release" ]]; then
    (cd android-gallery && ./gradlew --console=plain -q assembleRelease)
    APK=android-gallery/app/build/outputs/apk/release/app-release.apk
else
    (cd android-gallery && ./gradlew --console=plain -q assembleDebug)
    APK=android-gallery/app/build/outputs/apk/debug/app-debug.apk
fi
echo "built $APK ($(du -h "$APK" | cut -f1))"
if [[ "${1:-}" == "install" || "${1:-}" == "release" ]]; then
    timeout 120 adb install -r "$APK"
fi
