# Verio

Low-latency encrypted relay voice chat for Windows, built for gaming.

## What it is

Verio is a small-group voice chat for gaming sessions. Voice rides a self-hosted
relay server (a tiny Rust binary you can run on any VPS) with a peer-to-peer
path where the network allows it. Rooms are created on the fly with 4-digit
codes; nobody creates an account, and no audio is decoded or re-encoded by the
relay.

It ships a desktop client (Tauri 2 + React) and the companion relay/signalling
server in one Rust workspace. A local music player is built in: you can play
files from your machine into the room, mixed alongside your microphone with its
own quality tier.

Status: active development, pre-1.0, testing with a small group. Expect rough
edges and wire-format changes.

## Features

- Relay-first voice transport (datagrams over UDP; STUN-shaped envelope survives ISPs that filter unknown UDP)
- RNNoise neural noise suppression and voice-activity gating
- Opus 48 kHz mono, 10 ms frames, in-band FEC and DTX
- Multi-peer rooms (3+ users) with per-peer volume
- Local music player with per-peer mixing, loop, seek, and local monitor
- Global hotkeys for mute, deafen, and push-to-talk
- 4-digit room codes over a lightweight WebSocket signaling server
- Self-hostable relay + signaling server (Windows and Linux/musl builds)
- Synthesized earcon cues for mute, deafen, join, and leave

## Screenshots

<!-- TODO: add screenshots -->

## Build from source

Prerequisites: Rust (stable), Node.js 22+, and the Tauri CLI via npx (no
global install needed). Windows with WebView2 is the supported desktop target.

Clone and build the client:

```powershell
git clone <repository-url>
cd verio/ui
npm install
npx tauri build --no-bundle
```

The client binary lands in `target/release/verio.exe` at the repository root.

Build the relay/signaling server:

```powershell
cargo build --release -p verio-server
```

Output: `target/release/verio-server.exe` (Windows) or build with
`--target x86_64-unknown-linux-musl` for a static Linux binary.

Run the test suite:

```powershell
cargo test --workspace
```

## Running the relay server

The server binary speaks WebSocket signaling (default TCP 8443, `-p` to
change) plus the UDP relay and STUN responder (default UDP 8444/3478, `-u` to
change). See PROJECT.md for the full deployment guide, firewall ports, and a
sample systemd unit.

## License

MIT. See [LICENSE](LICENSE).

## Contributing

Issues and bug reports are welcome.
PRs should keep tests passing.
Use conventional commits.

