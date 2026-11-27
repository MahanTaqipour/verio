# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-11-27

### Added
- **VPS Auto-Update Mirror Generation (`scripts/build-release.ps1`)**: Generates `latest-vps.json` pointing to `https://update.verio.drfake.ir:9443/` alongside GitHub `latest.json` (both UTF-8 without BOM).

### Changed
- **Dual-Endpoint Auto-Updater with Download Fallback**: Primary check and download now query `https://update.verio.drfake.ir:9443/latest.json` with a 12-second request timeout and fall back automatically to GitHub Releases on connection, TLS, HTTP, or download failures.

## [0.2.0] - 2026-11-27

### Added
- **Watch-on-Demand WebRTC Screen Sharing**: Direct P2P with automatic ephemeral `coturn` TURN relay fallback (`verio-server` HMAC-SHA1 credentials on port `3479`), `720p · 30fps`–`1080p · 60fps` presets, live `RTCPeerConnection.getStats()` badges (`width×height · fps` and `P2P` / `Relay`), system audio sharing toggle, stream volume & mute controls integrated with Deafen, fullscreen Theater stage, and zero idle bandwidth until watched.
- **Branded Windows NSIS Installer & Auto-Updater**: Signed `tauri-plugin-updater` releases (`Verio_0.2.0_x64-setup.exe` + `latest.json`) with custom Midnight Violet / Electric Lavender / Acid Cyber Lime NSIS wizard artwork.
- **Ephemeral In-Call Chat & Permanent Lobby Room `#1111`**: Live in-call sidebar chat over encrypted room transport and instant single-click entry to permanent squad room `#1111`.

### Changed
- **Soft-Rounded Acrylic Glass UI**: Floating translucent islands (`#241B3A` / `#B9A7E6` / `#E8FF3A`), 3-tab icon rail (`Room`, `Music`, `Settings`), unified top command bar & hardware bottom dock.
- **Default `verio.drfake.ir` Domain Signaling**: Bare hostname defaults transparently to `:9091` TCP WebSocket and `:3478` / `:9092` UDP relay with automatic migration of legacy hardcoded VPS IPs.
- **Non-Blocking Game-Proof Hotkeys & Audio/Music Bot Overhaul**: Low-level Windows keyboard hook (`WH_KEYBOARD_LL`) delivering responsive push-to-talk/mute/deafen inside full-screen games, dedicated room Music Bot session with queue management & optional Force Stop, and 20 Hz imperative mic meter.

## [0.1.0] - 2026-09-01

Initial tracked release.
