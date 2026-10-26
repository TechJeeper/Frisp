# Frisp

Frisp is a noise-cancelling virtual microphone for Windows and macOS. Pick the microphone you want cleaned. Other apps, including OBS and browsers, use **Frisp Mic** as their input.

Frisp has to stay open while you want the cleaned microphone. Your original microphone is left unchanged.

## Install

Both systems use a signed virtual audio cable. Frisp writes the cleaned voice into that cable, and the installer names the microphone **Frisp Mic**.

### macOS

macOS uses [BlackHole 2ch](https://existential.audio/blackhole/), a signed virtual audio cable. Install that cable yourself.

1. Download BlackHole 2ch and install it.
2. Unzip the macOS release and run `install-macos.sh`. This names the cable **Frisp Mic**.
3. Open `Frisp.app` and allow microphone access.
4. Choose your microphone in Frisp.
5. In OBS, Streamyard, or another app, select **Frisp Mic**.

### Windows

Windows uses [VB-Audio Virtual Cable](https://vb-audio.com/Cable/), a signed virtual audio cable. Secure Boot stays on. Install that cable yourself, then reboot.

1. Download VB-Audio Virtual Cable, install it, and reboot.
2. Unzip the Windows release and run `install-windows.ps1` from an administrator PowerShell. This names the cable **Frisp Mic**.
3. Start `frisp.exe`.
4. Choose your microphone in Frisp.
5. In OBS, Streamyard, or another app, select **Frisp Mic**.

Until that installer has been run, other apps still see the cable's original name: BlackHole 2ch on macOS, CABLE Output on Windows.

## Build

The app is a Rust program.

```bash
cargo build --release
```

On a Mac:

```bash
scripts/package-macos.sh
```

That produces `dist/Frisp-macOS.zip` containing `Frisp.app` and the installer that names the virtual cable.

## How it works

Frisp reads the microphone you pick, mixes it to mono, and runs the open-source RNNoise model at 48 kHz through the `nnnoiseless` library. The cleaned audio is written to a virtual audio device. Recording apps read that device like any other microphone.

Noise cancellation can be turned off in the window. Frisp still publishes the selected microphone on Frisp Mic.

## License

MIT. See [LICENSE](LICENSE).

RNNoise and nnnoiseless are BSD licensed. Their license notices ship with those projects and with the crates Frisp builds against.
