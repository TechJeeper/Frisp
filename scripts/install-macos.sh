#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DRIVER="$ROOT/driver/macos/build/FrispMic.driver"
if [[ ! -d "$DRIVER" ]]; then
  make -C "$ROOT/driver/macos"
fi
sudo mkdir -p /Library/Audio/Plug-Ins/HAL
sudo rm -rf /Library/Audio/Plug-Ins/HAL/FrispMic.driver
sudo cp -R "$DRIVER" /Library/Audio/Plug-Ins/HAL/
sudo chmod -R 755 /Library/Audio/Plug-Ins/HAL/FrispMic.driver
sudo killall coreaudiod || true
echo "Frisp Mic is installed. It shows up as an input in a few seconds."
