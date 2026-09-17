## Session 7 (2026-09-15) — Forced Relay Instrumentation

Goal: make a **forced-relay** test possible with full per-packet visibility, so a retry with the
friend shows exactly where packets stop. No rewrite, no new dependencies, no DSP/RNNoise/VAD/Opus
changes, no P2P behaviour changes (the P2P code path is untouched; it is only *bypassed* when the
force switch is on).

### CRITICAL-FINDING CHECK (asked for explicitly)

**The relay send path IS wired into the audio pump — verified, no defect found.**

- `crates/verio-app/src/room.rs:212-235` — `spawn_audio_pump()` loops on the shielded `net_out`
  channel and calls `handle.send_audio(p.capture_ts_ms, p.opus)` at `room.rs:234`.
- `handle` is the active session's `PeerHandle`, which for Cloud Tunnel is the relay handle
  (`room.rs:339`, set by `activate_relay_session`).
- `crates/verio-transport/src/lib.rs:256` — `PeerHandle::send_audio` pushes `PeerCommand::Audio`
  onto the driver channel; the relay driver's `PeerCommand::Audio` arm serialises it and sends it
  to `active_relay_addr`.
- Live confirmation this session: the local relay probe (two synthetic peers, real
  `verio-server`) carried audio bidirectionally (5 audio + 2 control frames each way) and the
  server logged the matching `relay_in` / `relay_out` pairs.

So the Session 6 diagnosis stands: the recorded failures were identity collapse, not a missing
pump wiring.

### 1. Hard "force relay" mode

- New setting `force_relay: bool` (default `false`) — `crates/verio-app/src/settings.rs:72`, default
  at `:94`. Persisted in `settings.json` under the snake_case key `force_relay`.
- Mode selection — `crates/verio-app/src/room.rs:562-574`: when `force_relay` is true every
  transport mode resolves to `CloudRelay`, so the ICE/P2P branch is never entered: no host or srflx
  candidates are gathered, no SDP offer/answer is produced, and no later P2P upgrade happens (the
  relay session is created directly by `activate_relay_session`). The session's diagnostics mode
  label becomes `forced_relay` (drawer shows it as `forced`).
- Startup log (exact text, em dash emitted):
  `force_relay=ON — all voice traffic will route through the relay.`
  Emitted at launch when the setting is on (`ui/src-tauri/src/lib.rs:844-847`) and on every toggle
  (`:628-636`).
- UI: checkbox **"Force relay (skip P2P)"** in the Settings modal, under *Voice Transport
  Architecture* — `ui/src/App.tsx:1130-1145`, handler at `:483-493`, Tauri command
  `set_force_relay` at `ui/src-tauri/src/lib.rs:628`, registered at `:787`.

### 2. Client packet-level logging (`crates/verio-transport/src/lib.rs`)

One `tracing::debug!` line per relay datagram, fields: `direction`, `room`, `peer_uuid`
(first 8 chars), `dst` (send) / `src` (recv), `len`, `kind` (`audio|control|stun|unknown`).
Instrumented at every relay socket send site (registration prefix, hello, heartbeat probe, control,
ping, pong, **audio**) and at the single receive entry point.

- Throttle: `PacketLogThrottle` (`:271`), `RELAY_LOG_BURST = 20` first packets per direction always
  logged, then at most `RELAY_LOG_PER_SEC = 10` lines/second. Logging helper `log_relay_packet`
  (`:323`); classifier `relay_packet_kind` (`:307`).
- New diagnostics counters (`:227-234`): `relay_packets_sent`, `relay_packets_recv`,
  `relay_bytes_sent`, `relay_bytes_recv`, `relay_mode` (`forced | auto-upgraded | direct`),
  maintained by `bump_relay_sent` (`:358`) / `bump_relay_recv` (`:367`).

**IMPORTANT — how to see these lines.** Packet lines are DEBUG level, and the app's subscriber
previously hard-coded INFO, so they could not reach the log at all. The level is now
`RUST_LOG`-controllable (`ui/src-tauri/src/lib.rs:826-838`): launch with

```powershell
$env:RUST_LOG='debug'; .\target\release\verio.exe
```

otherwise the log stays at INFO (unchanged default) and the startup line
`relay packet log is OFF (launch with RUST_LOG=debug to capture per-packet relay lines)` tells you so.

### 3. Server packet-level logging (`crates/verio-server/src/main.rs`)

Throttled the same way (`PacketLogThrottle` `:491`, `pkt_log` `:562`), plus `short_uuid` (`:527`):

```
registered room=<id> uuid=<short> addr=<ip:port>
relay_in  room=<id> from_uuid=<short> from_addr=<ip:port> len=<bytes> peers_in_room=<n>
relay_out room=<id> to_uuid=<short> to_addr=<ip:port> len=<bytes>
drop      room=<id> reason=<no_peers|unknown_room> from_uuid=<short>
```

`relay_in` and `relay_out` pair up per forwarded datagram. Verified live this session against the
rebuilt server (room `1234`, two synthetic peers):

```
DEBUG verio_server: registered room=1234 uuid=6483cd3a addr=127.0.0.1:49839
DEBUG verio_server: relay_in   room=1234 from_uuid=6483cd3a from_addr=127.0.0.1:49839 len=48 peers_in_room=2
DEBUG verio_server: relay_out  room=1234 to_uuid=1bdc7e64 to_addr=127.0.0.1:49840 len=48
```

Reading the triage: `relay_in` with no matching `relay_out` ⇒ server-side bug;
`relay_out` with no matching client `recv` line ⇒ path between server and the friend's machine
(firewall / NAT / wrong address).

### 4. Diagnostics drawer counters

New cards in the drawer (`ui/src/App.tsx:678-716`): **Relay Packets S / R**,
**Last Relay Recv** (turns red past 2000 ms with "No relay packets - firewall / NAT / wrong
address"), and **Relay Mode** plus relay bytes out/in. TS mirrors added at `:31-37` and
`:52`. Counters refresh with the existing 1 s diagnostics poll.

Not implemented (and why): pushing the **server's** counters to the client would need a new
signaling message and server-side per-room aggregation; the WS protocol has no such message today
and adding one is beyond "instrumentation only". The client-side `last_relay_recv_ms_ago` already
answers the "are the friend's packets reaching me" question. Say the word and it can be added next.

### 5. Builds (real output)

- `npx tauri build --no-bundle` (from `ui/`) → built twice: `Finished release profile [optimized]
  target(s) in 2m 55s` (instrumentation) then `2m 01s` after the RUST_LOG log-level fix — the second
  is the shipped binary (vite `built in 2.33s`)
  → `C:\dev\verio\target\release\verio.exe` (15.95 MB, 2026-09-15 13:28:45)
- `cargo build --release -p verio-server` → `Finished release profile [optimized] target(s) in 26.10s`
  → `C:\dev\verio\target\release\verio-server.exe`
- `cargo rustc --target x86_64-unknown-linux-musl --release -p verio-server -- -C linker=rust-lld`
  → `Finished release profile [optimized] target(s) in 25.51s`
  → `C:\dev\verio\target\x86_64-unknown-linux-musl\release\verio-server` (2.70 MB, 2026-09-15
  13:20:33 — deploy this to the VPS)
- Windows server binary: `C:\dev\verio\target\release\verio-server.exe` (1.74 MB, 2026-09-15 13:20:07)
- Boot smoke test of the shipped client: process alive after 8 s, killed cleanly.
- `cargo test --workspace` → all green: verio_app 11, verio_discovery 8, verio_dsp 24,
  verio_server 6, verio_transport 8 (0 failed).
- `npx tsc --noEmit` → exit 0.

### 6. Clippy status (honest report)

`cargo clippy --workspace --all-targets -- -D warnings` **fails, but on two PRE-EXISTING lints that
are not part of this session's changes**:

- `crates/verio-discovery/src/vps.rs:195-196` — collapsible `if let`
- `crates/verio-server/src/main.rs:533` — `type_complexity` on the pre-existing
  `SharedRelayHub.rooms` type

My one new lint (`room.rs:532 too_many_arguments`, caused by adding the `force_relay` parameter)
was silenced with a targeted `#[allow(clippy::too_many_arguments)]`; every other file changed this
session is lint-clean. I deliberately did not "fix" the two pre-existing lints (no unrelated
cleanups) — flagging them for a decision.

### PENDING USER ACTION — friend retry (NOT yet run, NOT claimed to pass)

1. Deploy the new musl server binary to the VPS and restart the service.
2. Launch the client with `$env:RUST_LOG='debug'`, tick **Force relay (skip P2P)**, and confirm the
   startup line `force_relay=ON — all voice traffic will route through the relay.`
3. Create a room; friend joins with the same setting on; both hold PTT.
4. Watch server `journalctl` for `relay_in` ↔ `relay_out` pairs, and each client log for
   `relay packet` lines with `direction=recv`.
5. Report back: the first stage where the pairing/recv stops.
