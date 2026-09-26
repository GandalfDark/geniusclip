# GeniusClip

Instant replay for Windows: GeniusClip keeps the last minutes of your game or
screen in memory and saves them as a clip when you press a hotkey — like NVIDIA
Instant Replay, but independent of the GPU vendor and without touching the game.

*Запись последних минут игры по горячей клавише. Сайт и загрузка:*
**https://gandalfdark.github.io**

## Features

- Replay buffer (up to 60 minutes, in memory or on disk), clip on a hotkey;
  short clips, screenshots, manual recording.
- Hardware encoding: NVENC (NVIDIA), AMF (AMD), Media Foundation (Intel and others);
  H.264, HEVC, AV1.
- Game and microphone audio in separate tracks, microphone noise suppression
  (DeepFilterNet 3).
- Clip gallery sorted by game, trim editor with per-track volume.
- In-game menu (Alt+X) as a separate always-on-top window: no injection into
  games, safe with anti-cheat systems.
- 14 interface languages.

## Download

Get the installer from the website or from
[releases](https://github.com/GandalfDark/geniusclip-releases/releases/latest).
Windows 10 or 11 (64-bit).

## Code signing policy

Windows releases are built by GitHub Actions from this repository
([`release.yml`](.github/workflows/release.yml)). Free code signing is
provided by [SignPath.io](https://about.signpath.io), certificate by
[SignPath Foundation](https://signpath.org) *(application in progress; until it
is approved, releases are unsigned)*.

- Committers and reviewers: [GandalfDark](https://github.com/GandalfDark)
- Approvers: [GandalfDark](https://github.com/GandalfDark)

Only binaries built from this repository are signed.

## Privacy policy

GeniusClip does not collect or send any personal data, usage statistics or
recordings. Clips and screenshots stay on your computer and leave it only when
you share them yourself.

The program connects to the internet only to:

- check for updates: it downloads
  `https://github.com/GandalfDark/geniusclip-releases/releases/latest/download/latest.json`
  every few hours and, when you choose to install an update, the new installer
  (turn the check off in Settings → Update automatically);
- the installer may download Microsoft's WebView2 runtime if Windows lacks it.

The interface itself can't load anything from the internet (it is locked to
local content by its Content Security Policy).

## How it works

- `crates/engine` — the Rust engine: screen capture (DXGI Desktop Duplication)
  → BGRA→NV12 on the GPU (D3D11 video processor) → hardware encoding
  → ring buffer → MP4 without re-encoding. Audio: WASAPI (system + microphone)
  → AAC, up to 3 tracks.
- `src-tauri` — the Tauri 2 app: tray, hotkeys, settings, autostart, updates,
  overlay notifications, in-game menu.
- `src` — the interface (SvelteKit).
- `crates/setup` — the branded installer window around the NSIS setup.
- `site` — the website.

## Building

Needs Rust (MSVC), Node.js, Visual Studio Build Tools and LLVM (libclang for
bindgen).

```powershell
# once: the FFmpeg build (made by the "Build FFmpeg" workflow)
./scripts/fetch-ffmpeg.ps1
npm install
npm run tauri dev
```

Engine test without the interface:

```powershell
cargo run -p geniusclip-engine --example replay -- 8 target/replay-test
```

## License

GeniusClip is free software under the [GNU General Public License v3.0 or
later](LICENSE).

Third-party components: [FFmpeg](https://ffmpeg.org) (LGPL 2.1+, a minimal
build made from source by [`ffmpeg.yml`](.github/workflows/ffmpeg.yml), used as
DLLs), [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet) (MIT /
Apache-2.0), [Tauri](https://tauri.app), [Svelte](https://svelte.dev),
[tract](https://github.com/sonos/tract) and the crates listed in `Cargo.lock`.
