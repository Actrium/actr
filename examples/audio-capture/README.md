# Audio Capture Example

This example packages the Swift microphone sender and the Rust receiver in one place:

- `AudioCaptureApp/`: macOS SwiftUI sender that captures microphone audio, encodes it as Opus, and sends it over ACTR MediaTrack.
- `rust-receiver/`: Rust receiver that decodes Opus and writes `recorded_audio.wav`.

## Prerequisites

- Rust toolchain
- Swift 6 toolchain / Xcode command line tools
- Microphone permission on macOS

## Layout

```text
audio-capture/
├── AudioCaptureApp/
└── rust-receiver/
```

The shared actrix config stays at `../actrix-config.toml`.

## One-Time Setup

If you run the Rust receiver directly, create its local config first:

```bash
cp rust-receiver/Actr.example.toml rust-receiver/Actr.toml
```

Initialize the realm and ACL entries before the first run:

```bash
cargo run --manifest-path ../rust/Cargo.toml -p realm-setup -- \
  -c ../actrix-config.toml \
  -a rust-receiver/Actr.example.toml \
  -a AudioCaptureApp/AudioCaptureApp/Actr.toml
```

## Reproduce

Run these commands in separate terminals from `actr/examples/audio-capture`:

1. Start actrix:

```bash
cargo run --manifest-path ../../../actrix/Cargo.toml -- --config ../actrix-config.toml
```

2. Start the Rust receiver:

```bash
cargo run --manifest-path rust-receiver/Cargo.toml
```

3. Launch the Swift sender:

```bash
swift run --package-path AudioCaptureApp
```

4. In the app window:

- Click `Connect`
- Click `Start Recording`
- Wait for the built-in 5 second auto-stop, or stop manually earlier

5. Stop the Rust receiver with `Ctrl+C`.

The receiver writes the output file to:

```text
rust-receiver/recorded_audio.wav
```

## Verification

- `cargo check --manifest-path rust-receiver/Cargo.toml`
- `swift build --package-path AudioCaptureApp`

## Notes

- The Swift app currently auto-stops after 5 seconds to keep the demo bounded.
- If the Rust receiver stays alive across multiple recordings, the final WAV file will include all audio buffered in that process.
