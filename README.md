# Frisp

Frisp is a noise-cancelling virtual microphone for Windows and macOS. Pick the microphone you want cleaned. Other apps, including OBS and browsers, use **Frisp Mic** as their input.

Frisp has to stay open while you want the cleaned microphone. Your original microphone is left unchanged.

## Install

### macOS

1. Unzip the macOS release.
2. Run `install-macos.sh` and enter an administrator password. This copies the virtual microphone into `/Library/Audio/Plug-Ins/HAL` and restarts Core Audio.
3. Open `Frisp.app` and allow microphone access.
4. Choose your microphone in Frisp.
5. In OBS, Streamyard, or another app, select **Frisp Mic**.

### Windows

1. Unzip the Windows release.
2. Run `install-windows.ps1` from an administrator PowerShell. This installs the virtual microphone driver.
3. Start `frisp.exe` and choose your microphone.
4. In OBS, Streamyard, or another app, select **Frisp Mic**.

Windows only loads a virtual microphone driver that the system trusts. A test-signed driver requires test signing, which also requires Secure Boot to be off. Production installs need a Microsoft-attested driver signature.

## Build

The app is a Rust program. The macOS virtual microphone is a Core Audio server plugin in `driver/macos`.

```bash
cargo build --release
```

On a Mac:

```bash
scripts/package-macos.sh
```

That produces `dist/Frisp-macOS.zip` containing `Frisp.app`, `FrispMic.driver`, and the installer.

## How it works

Frisp reads the microphone you pick, mixes it to mono, and runs the open-source RNNoise model at 48 kHz through the `nnnoiseless` library. The cleaned audio is written to a virtual audio device. Recording apps read that device like any other microphone.

Noise cancellation can be turned off in the window. Frisp still publishes the selected microphone on Frisp Mic.

## License

MIT. See [LICENSE](LICENSE).

RNNoise and nnnoiseless are BSD licensed. Their license notices ship with those projects and with the crates Frisp builds against.
