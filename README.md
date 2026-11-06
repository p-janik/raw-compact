# RAW Compact

A small desktop app for shrinking Nikon **R3D NE (.R3D)** and **N-RAW (.NEV)** footage while keeping the original resolution and RAW controls. Built with Tauri 2 and the open-source [Blue Compressor](https://github.com/redn3/blue-compressor) engine.

## Downloads

[Get the 0.1.0 preview](https://github.com/p-janik/raw-compact/releases/tag/preview-0.1.0): macOS Apple silicon DMG/ZIP, Windows x64 installers, and Linux x64 AppImage, Debian and RPM packages. macOS, Windows and Linux all passed the [build and test workflow](https://github.com/p-janik/raw-compact/actions/runs/42304913326).

On macOS, [download the DMG](https://github.com/p-janik/raw-compact/releases/download/preview-0.1.0/RAW-Compact_0.1.0_macOS_arm64.dmg), open it and drag RAW Compact to Applications or a folder on an external SSD. No developer tools or separate engine installation is required.

Only macOS Apple silicon has been exercised with real footage. Mac builds are ad hoc signed and not notarized; Windows installers are unsigned.

## First working version

- Add multiple clips with a native file picker or drag and drop.
- Light, Medium and Strong compression presets; Medium is the default.
- Choose any output folder, including an external SSD.
- Sequential batch queue with live progress, speed and actual savings.
- Cancel the batch; closing the app waits for cancellation and cleanup.
- Each run creates a new output subfolder and preserves the original clip names.
- Checks format, coded resolution, frame count, frame rate and audio track count after writing.

These are **lossy RAW** presets, approximately 25%, 45% and 65% smaller. The actual target is derived from each clip's source bitrate. Header checks are not a full decode or a visual quality guarantee. Review copies in your editor before deleting any originals.

This first version intentionally has no trimming, card offload, proxy export, grading or built-in image comparison. It does not send footage anywhere and has no update check or telemetry.

## Build

Install [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), Rust 1.90+ and Node.js 22+.

```sh
npm ci
npm run dev
npm run build
```

The UI is plain HTML, CSS and JavaScript. There is no frontend build step. `npm run build` produces platform bundles in `src-tauri/target/release/bundle` (or `$CARGO_TARGET_DIR/release/bundle`).

To keep build files and dependency caches off an internal drive, set `CARGO_HOME`, `CARGO_TARGET_DIR`, `npm_config_cache` and `TMPDIR` to folders on your external drive before building. Do not unplug that drive while running or building the app.

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml --check
```

The GitHub workflow builds macOS, Windows and Linux packages. Only macOS Apple silicon has been exercised with real footage so far; other platforms require runtime qualification. Preview macOS builds are ad hoc signed, not notarized; Windows builds are unsigned.

## Validation so far

On an Apple M4 Mac, a 561-frame Nikon ZR 6K R3D NE clip at 59.94 fps went from 4.35 GB to 2.47 GB at Medium (43.3% smaller) in a 28.3-second GUI run. Native file selection, external-drive output, live progress and cancellation were exercised. Cancellation left no partial video behind; the source SHA-256 stayed unchanged.

The app output was byte-for-byte identical to the earlier Medium output independently reviewed in DaVinci Resolve Studio 21.1. That output passed full-frame decoding, metadata, timing and byte-identical audio checks. With matching RED IPP2 grading, sampled 6K frames looked close, with differences mainly in grain and fine texture. This is one short indoor sample, not broad camera or quality qualification. No user footage is included in this repository.

## Engine and attribution

The codec is **Blue Compressor 0.3.6**, pinned to commit `186fdf785fe0b735b27eb16b62fbf5a6c8cfab9d`. RAW Compact is a new UI around its Rust core, not a new compression codec. The codec and RAW transformations are unchanged. Automatic backend selection uses the engine's GPU implementation when available and falls back to CPU.

Licensed under Apache-2.0. Engine notices and the rawmakase MIT license are in `licenses/` and bundled with the app. No FFmpeg executable is distributed. Nikon, RED, R3D and N-RAW belong to their respective owners; this project is independent.
