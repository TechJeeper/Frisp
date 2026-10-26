#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi
cargo build --release --bin frisp
mkdir -p "$ROOT/dist"
clang -mmacosx-version-min=12.0 -arch x86_64 -arch arm64 \
  -framework CoreAudio -framework CoreFoundation \
  -o "$ROOT/dist/frisp-cable" "$ROOT/scripts/macos/frisp-cable.c"
codesign --force --sign - "$ROOT/dist/frisp-cable"
APP="$ROOT/dist/Frisp.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp target/release/frisp "$APP/Contents/MacOS/frisp"
chmod +x "$APP/Contents/MacOS/frisp"
codesign --force --sign - "$APP"
STAGE="$ROOT/dist/Frisp-macOS"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
cp "$ROOT/dist/frisp-cable" "$STAGE/frisp-cable"
cp scripts/install-macos.sh "$STAGE/install-macos.sh"
chmod +x "$STAGE/install-macos.sh" "$STAGE/frisp-cable"
rm -f "$ROOT/dist/Frisp-macOS.zip"
(cd "$ROOT/dist" && zip -ry Frisp-macOS.zip Frisp-macOS)
echo "Wrote $ROOT/dist/Frisp-macOS.zip"
