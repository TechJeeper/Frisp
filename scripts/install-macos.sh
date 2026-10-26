#!/bin/bash
# Names the BlackHole virtual cable Frisp Mic, the same way install-windows.ps1
# names VB-Audio Virtual Cable. BlackHole itself is installed separately.
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"

if [[ -x "$DIR/frisp-cable" ]]; then
  HELPER="$DIR/frisp-cable"
elif [[ -x "$DIR/macos/frisp-cable" ]]; then
  HELPER="$DIR/macos/frisp-cable"
else
  SRC="$DIR/macos/frisp-cable.c"
  if [[ ! -f "$SRC" ]]; then
    echo "frisp-cable is missing. Use the copy shipped next to this script." >&2
    exit 1
  fi
  HELPER="$DIR/macos/frisp-cable"
  clang -mmacosx-version-min=12.0 -arch x86_64 -arch arm64 \
    -framework CoreAudio -framework CoreFoundation \
    -o "$HELPER" "$SRC"
  codesign --force --sign - "$HELPER"
fi

exec "$HELPER"
