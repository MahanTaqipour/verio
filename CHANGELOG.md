# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Warm redesign of the entire UI (Session 15)
- Local music player with per-peer mixing
- Multi-peer rooms (3+ users)
- STUN-enveloped relay transport (bypasses ISP UDP DPI)

### Changed
- Relay is now the primary transport; P2P auto-upgrade path removed from UI
- Room model is now multi-peer with per-peer attribution

### Fixed
- Deafen now silences local playback as well as microphone
- Leaving a room notifies the server immediately
- Fluctuating RTT in multi-peer rooms

## [0.1.0] - 2026-09-01

Initial tracked release.
