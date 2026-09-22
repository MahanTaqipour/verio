# Verio — Frontend Redesign: Phase 0 Analysis

**Status:** awaiting approval of the feature triage in §3. No application code has been changed.
**Scope of this session:** frontend presentation + frontend architecture only. Rust, protocol, transport, and server are untouched.

---

## 0. What was inspected

| Source | Notes |
|---|---|
| `PROJECT.md`, `PROGRESS.md` (1,882 lines, Sessions 0–16), `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md` | Read. Session 11 deliberately removed transport-mode plumbing from the UI; Session 15 introduced the warm palette. |
| `design.md` | Read in full. Treated as the token authority. |
| `ui/src/App.tsx` (1,821 lines) | Read in full. |
| `ui/src/App.css` (2,062 lines, 363 top-level selectors) | Structural inventory + token/base sections read. |
| `ui/src/main.tsx`, `index.html`, `vite.config.ts`, `tsconfig.json`, `ui/package.json`, `pnpm-lock.yaml` | Read. |
| `ui/src-tauri/src/lib.rs` (commands + emitted events), `capabilities/default.json`, `tauri.conf.json`, `Cargo.toml` | Read. |
| `crates/verio-app/src/settings.rs`, `crates/verio-app/src/pipeline.rs` (`MusicStatus`) | Read to establish what data actually exists. |
| `stitch_verio_desktop_interface_suite.zip` | Extracted to `design/stitch/` (that path is gitignored). All 4 `code.html` files read in full; all 4 `screen.png` renders inspected; `broadcast_acoustic/DESIGN.md` read in full. |
| Baseline build | `pnpm build` in `ui/` → **PASS** (tsc clean, vite 33 modules, 4.28 s). Recorded before any change. |

### Verified runtime inventory (the contract we must not break)

**41 Tauri commands:** `ping`, `emit_test_event`, `debug_log`, `get_settings`, `set_nickname`, `list_devices`, `get_audio_state`, `set_input_device`, `set_output_device`, `set_input_gain`, `set_noise_suppression`, `set_loopback`, `set_debug_wavs`, `set_hotkey`, `prepare_hotkey_capture`, `toggle_mute`, `toggle_deafen`, `get_room_state`, `create_room`, `join_room`, `leave_room`, `set_peer_volume`, `set_vps_address`, `connect_direct`, `create_invite`, `accept_invite`, `complete_invite`, `disconnect_peer`, `set_signaling_port`, `get_transport_diagnostics`, `set_transport_mode`, `set_host_port`, `music_status`, `music_open`, `music_play`, `music_pause`, `music_stop`, `music_seek`, `music_set_volume`, `music_set_loop`, `music_set_monitor`, `music_pick_file`, `set_voice_quality`, `set_music_quality`.

**10 emitted events:** `audio_state_changed`, `speaking_changed`, `input_level`, `music_status_changed`, `room_state`, `room_created`, `room_joined`, `room_error`, `audio_error`, `test_event`.
(`peer_connected`, `peer_disconnected`, `peer_state`, `rtt_updated` are also listened to in `App.tsx`.)

**Data that genuinely exists** — `Settings` (nickname, in/out device, input_gain_db 0–30, noise_suppression, loopback, debug_wavs, hotkeys{mute,deafen,ptt}, signaling_port, vps_address, transport_mode, host_port, voice_bitrate_kbps, music_bitrate_kbps, music_volume 0–2, music_loop, music_monitor) · `AudioState` (muted, deafened, ptt_mode, ptt_held, speaking, loopback) · `PeerInfo` (uuid, name, speaking, mute, deafen, music, music_title) · `TransportDiagnostics` (mode, remote_endpoint, local_addr, packets_sent/recv, bytes_sent/recv, last_recv_ms_ago, rtt_ms, ice_state, status, last_error, relay_packets_sent/recv, relay_bytes_sent/recv, relay_mode) · `MusicStatus` (active, playing, path, title, duration_ms, position_ms, volume, loop, monitor).

**Data that does NOT exist anywhere:** per-peer audio level · per-peer latency · jitter · packet-loss % · music playlist/queue · track buffered % · room history · encryption-cipher name · DSP load / ring-buffer occupancy · sample-rate of the open file · a persistent identity keypair.

---

## 1. Design inventory (Stitch package)

The package is a **spec deck**, not four finished screens. Each `code.html` contains several alternative directions for one screen plus a "required states" matrix. The `screen.png` renders are the state-matrix portion of each deck (the live window panes render blank in the static captures because the decks are tab-switched).

### 1.1 `broadcast_acoustic/DESIGN.md` — the deck's own design system
- Aesthetic: *rack-mounted audio gear / analog console / field recorder*. Dense, hairline-bordered, calibrated. No shadows except one crisp overlay shadow `0 4px 12px rgba(0,0,0,.45)`.
- 5 surface tiers (base `#171412`, panel `#1d1917`, elevated `#241f1c`, overlay `#2d2723`, interactive `#352e2a`), 3 border tiers, 3 text tiers.
- Signals: Amber `#c9873f` = primary/connecting/mic-standby; Mossy green `#5b7a72` = transmitting/online/low-latency; Vermilion `#c75242` = mute override / packet loss / disconnect.
- Named type ramp (`display`, `headline-lg/md/sm`, `body-lg/md/sm`, `mono-lg/md/sm`, `label-caps`), `label-caps` = uppercase JetBrains Mono 10px/0.08em, `tnum` forced on numeric readouts.
- Breakpoints: **compact 700–900** (single panel, rail collapses to 56 px, trays slide over, 12 px margin) · **standard 901–1150** (fixed 220 px nav + flexible stage + collapsible telemetry, 16 px margin) · **wide 1151–1400** (56 px rail + 240 px explorer + flex stage + 280 px participant matrix, 20 px margin).
- Component specs: primary/secondary/destructive buttons, tactile 32 px push buttons with inset well + amber indicator line, stage tiles with square 4 px avatars, speaking = **flat mossy-green outline (explicitly no pulsating rings/halos)**, muted = 16 px circular badge with struck-through mic, telemetry chips 20 px tall in JetBrains Mono, recessed inputs, channel rows 32 px with a 2 px amber left marker when selected, and a very specific slider spec (4 px track, 12×16 px knurled thumb).

### 1.2 Lobby / Main App Area
- **Direction 1 — Split Workbench (primary):** simulated window chrome → 320 px identity sidebar (local identity card, edit nickname, Settings + audio-check buttons at the bottom) + centred "gateway" stage: `VOICE MESH READY` eyebrow, "Connect or Initialize Room" title, two side-by-side action panels — *Join Voice Room* (4-digit mono input + Connect) and *Create New Room* (featured) — plus a "Previous: #4821 · Rejoin" quick-access bar.
- **Direction 2 — Centered Console:** top bar (node name, Online badge, relay name, Audio Test / Settings) + a single 540 px console card with a Dispatch header, code input + Connect, an "or initiate new session" divider, and a footer with `Opus 48kHz / ChaCha20` + "Audio ready".
- **Direction 3 — Compact Rail & Dock:** 56 px icon rail + 360 px config pane (identity + KeyID, join, host, relay node) + a stage of "Recent Voice Channels" rows with Rejoin buttons.
- **States matrix:** STATE-01 fresh/empty lobby (identity unset, actions disabled) · STATE-02 invalid code (error border + inline explanation + Clear/Retry) · STATE-03 room allocation in progress (spinner, "Allocating mesh keys…", cancel) · STATE-04 joining handshake (step 2/3, DTLS progress bar, Direct UDP 7100, RTT) · STATE-05 relay unreachable (offline badge, error copy, Reconnect Relay Node) · STATE-06 control token matrix (default/hover/focus/active/disabled for primary button + input).

### 1.3 In-Call Room View
- **Master telemetry bar:** `SESSION: #4821` + COPY button, a `CHACHA20-POLY1305 / P2P MESH` badge, `16ms RTT / 0.0% LOSS / JITTER 0.4ms`, a `PRESET: 1P | 6P | 10P` switcher, and a `LEAVE ROOM` eject button.
- **Direction 1 — Acoustic Stage Grid (primary):** auto-sizing 1→3 column grid of 170 px-min participant tiles. Each tile: 40 px square initials avatar, name, role tag (`(YOU)`, `HOST`, `PEER`), a mono sub-line (codec/device/vol), a state chip (`ACTIVE` / `TRANSMITTING` / `MIC OFF` / `DEAFENED` / `LOW SIGNAL` / `PACKET DROP` / `HANDSHAKE`), a centre well (input level + dB, waveform + dBFS + kbps, or a recessed idle well), and a footer with a per-peer volume fader, a local-mute icon button, and a per-peer ms figure. Tiles 8–10 are hidden extras revealed by the preset switcher.
- **Direction 2 — Broadcast Channel Strip:** horizontally scrolling 144 px vertical mixing strips per participant (labelled `CH 01 [YOU]`, dual-column VU bed, MUTE/SOLO), with an `AUX 01 [AUDIO SINK]` music strip.
- **Direction 3 — Compact Rail & Stage:** 320 px peer list (dot + name + mono status) beside a "Mesh Routing Matrix" with a scrolling timestamped telemetry log and frame-loss counters.
- **Integrated music tray:** `VOICE CHANNEL AUDIO INJECTOR` header with a `MONITOR: ON` toggle, 48 px art tile, track title + `M83 • Hurry Up, We're Dreaming`, transport row (prev / amber play-pause / next / loop-active), a volume fader with %, and a 02:14 → 04:03 seek bar. A fixed 48 px footer bar holds `SHURE SM7B [BUS 01]` + meter, `MONITOR OUT 82%` + meter, `14 MS`, `BITRATE: 128 KBPS`, MUTE / DEAFEN / DISCONNECT.
- Includes real interactive JS for copy-code, direction switching, roster presets, play/pause, and monitor toggle.

### 1.4 Music Player Panel
- **Direction 1 — Expanded Studio Rack (primary):** header telemetry bar (`BUS 04 // STEREO INJECTOR`, `ROUTED TO: STAGE ROOM #4821`, `OPUS CBR 192 KBPS`, `DSP CLOCK 48.0 kHz / 24-BIT LOCKED`); a source-file banner (`/storage/audio/.../m83_midnight_city_master_2448.flac` + Browse / Reload); a 12-column rack grid of album art (FLAC chip, title, artist, `48kHz • 24-bit • Stereo`), a 32-bar "Stereo Spectral Profile" with peak readout and 20 Hz–20 kHz axis, and an L/R calibrated output meter with a -inf…0 CLIP scale; a seek timeline with `BUFFERED: 78% / REMAINING: -01:49`; and a control deck (shuffle / prev / amber PAUSE / next / stop / `LOOP [ALL]`) plus two sub-panels — *Local Monitor* (dot + 80 % fader) and *Room Broadcast Gain* (`0.0 dB [100%]` fader + MUTE).
- **Direction 2 — Compact Collapsed Dock:** single-row dock with a 36 px art tile, title + green pip + `M83 • Stream: Stage #4821`, mini prev/play/next, a `MONITOR` badge, elapsed time, an `EXPAND RACK` button, and a 4 px scrubber. Plus an "INJECTION STANDBY — NO STREAM CONSUMING BUS BANDWIDTH" idle variant.
- **Direction 3 — Modular Channel Strip:** sidebar rack with a vertical VU bed, a vertical knurled studio fader, `INJECT ROOM / MONITOR AUD / SOLO BUS / KILL CH` routing buttons, and a 3-track deck queue.
- **States matrix:** A idle/no track (dashed drag-and-drop zone, "WAV, FLAC, MP3, OGG up to 96kHz", Browse Local Disk) · B playing/streaming (ACTIVE TRANSMISSION, 192 KBPS, PKT SENT 14,892, LOSS 0.0%) · C paused (STREAM SUSPENDED, FRAME #18,440 HELD, Resume) · D buffering/syncing (spinner, PRE-FETCHING OPUS FRAMES, ring buffer 42 %/80 %) · E decode error (`ERR_DECODER_UNDERFLOW`, Retry Codec / Dismiss) · F control tokens.

### 1.5 Settings
- 2-row fixed header: window chrome row + a nav row (`VERIO [RACK-04]` + Voice Stage / Settings tabs, right side `24MS • CONNECTED` + nickname chip).
- Context bar: `Verio // DeskEngine v0.9.4 · DSP Control Matrix`, a **density-mode selector** (Rack Layout / Utilitarian / Telemetry Split), and an **"Unsaved Audio Bus Edits · Discard · Write (Ctrl+S)"** block.
- Two-pane workspace: **left 3-col nav** — `ROUTING NODES / ONLINE` then six rows, each with an icon, a title, a mono sub-line and a trailing token: *Ephemeral Keypair* (`Mahan • 0x82f9`, `ed25519`) · *Audio Bus & Filters* (`48kHz • 24-bit PCM`, active) · *Tactile Hotkeys* (`4 Active • 1 Conflict`, `ERR`) · *Relay Mesh Cluster* (`14ms • relay.verio`) · *Aux Injection Bus* (`192kbps Opus St.`, `AUX`) · *Diagnostics & Specs* (`ChaCha20 • Poly1305`, `v0.9.4`) — plus a bottom DSP-load / ring-buffer telemetry well.
- **Right 9-col content** (Audio & DSP active): a critical hardware alert banner (`SECONDARY INPUT LOST`, PCIe aux line, Dismiss) → *Transducer Hardware Routing* (input capture device with 48000 Hz chip + preamp gain readout; master output with 24-bit stereo chip + buffer latency) → *Calibrated Signal Metering* (Test Loop Monitor button, PEAK readout, a ~30-segment LED meter with -∞…0 dBFS scale, noise floor + clip-detection footer) → *Neural & Analog Filter Chain* (RNNoise threshold / AEC filter decay / Opus CBR bitrate, three cards) → a 3-up subsystem grid: *Active Hardware Hotkeys* (bound rows as `kbd` chips, a live "RECORDING KEY COMBINATION… [Esc to Cancel]" row, and a conflict row with an explanation), *Relay Handshake* (target endpoint + Test Ping, RTT/jitter, crypto bus, mesh hops, a fallback-timeout note), *Ephemeral Mesh Identity* (session pseudonym with an `n / 16` counter, public mesh fingerprint + Copy, a zero-cloud notice box).
- Bottom status anchor: `AUDIO BUS: LOCKED · DRIVER: WASAPI 10.0.22631 · CRYPTO: ed25519+chacha20` / `CPU · MEM · READY`.

### 1.6 Recurring visual patterns worth reusing
Hairline 1 px borders on every surface · `label-caps` mono eyebrow labels above grouped content · mono `kbd` chips for hotkeys · recessed "wells" for meters and idle states · telemetry chips with a status pip · `status + value` mono rows · left-edge 2 px accent marker for the selected nav row · state chips that pair a dot/icon with an uppercase word (never colour alone) · a fixed bottom control bar · an eyebrow → title → mono-subtitle block for empty/idle states.

---

## 2. Existing-app mapping

| Stitch item | Status in Verio today | Note |
|---|---|---|
| App shell (brand + status + room chip + dock) | **Exists** | `header` / `main` / `footer.dock` in `App.tsx`. |
| Lobby: create room | **Exists** | `create_room`, `room_created`. |
| Lobby: join by 4-digit code | **Exists** | Already a 4-box OTP field; `join_room`. |
| Lobby: local identity + rename | **Exists** | `nickname`, `set_nickname`; rename lives in the dock, not the lobby. |
| Lobby: settings access | **Exists** | Gear in dock. |
| Lobby: "voice mesh ready" eyebrow / gateway panels | **Partially** | `lobby-hero` + two `minimal-card`s; no eyebrow, no icon wells, no descriptive mono sub-lines. |
| Lobby: recent rooms + Rejoin | **Doesn't exist** | Rooms are ephemeral 4-digit codes; nothing is persisted. |
| Lobby: KeyID / `0x82f9` / ed25519 fingerprint | **Doesn't exist** | `peer_id` is an internal UUID, never shown. |
| Room: participant tiles | **Exists** | `peer` cards with initials avatar, name, `Muted`/`Deafened`/music badges. |
| Room: speaking state | **Exists** | `speaking_changed` + `peer_state`; currently a green glow (design.md forbids glows). |
| Room: mute / deafen / PTT | **Exists** | `toggle_mute`, `toggle_deafen`, hotkeys, earcons. |
| Room: local mic meter | **Exists** | `input_level` → direct DOM width mutation (must stay imperative). |
| Room: per-peer volume | **Exists** | `set_peer_volume`, 0–200 %, slider per card. |
| Room: per-peer local mute | **Doesn't exist** | Only volume; achievable frontend-side via `set_peer_volume(id, 0)`. |
| Room: per-peer level / latency / jitter / loss | **Doesn't exist** | No per-peer telemetry is produced by the transport. |
| Room: room code + copy | **Exists** | `room-tag`, header `room-chip`. |
| Room: leave | **Exists** | `leave_room` + danger button. |
| Room: diagnostics | **Exists** | 1 Hz poll of `get_transport_diagnostics` + drawer. |
| Room: music access | **Exists** | Collapsed strip + expandable panel. |
| Room: capacity presets 1P/6P/10P | **Doesn't exist** | The grid auto-fills; a preset would only hide real peers. |
| Room: SOLO / channel strips / routing matrix / telemetry log | **Doesn't exist** | No solo bus, no event log stream. |
| Room: fixed footer with device names + MONITOR OUT | **Partially** | Dock shows nickname, PTT mode, mic meter, mute/deafen/settings — not device names. |
| Music: expand/collapse | **Exists** | `musicPanelOpen`. |
| Music: open file / play / pause / stop / seek / loop / volume / monitor | **Exists** | All 9 commands wired. |
| Music: title + file path | **Exists** | `music.title`, `music.path`. |
| Music: drag-and-drop a file | **Doesn't exist** | `music_pick_file` dialog only. |
| Music: art, spectral bars, L/R meters, peak dBFS, buffered %, queue, shuffle, prev/next | **Doesn't exist** | No decoder-side analysis and no playlist; the bus is single-file. |
| Music: room broadcast gain | **Partially** | `music_volume` (0–200 %) is the closest real control. |
| Settings: modal | **Exists** | Overlay panel, 6 sections, closes on backdrop click. |
| Settings: two-pane nav | **Doesn't exist** | One long scrolling column. |
| Settings: devices, gain, NS, loopback, hotkeys, quality, VPS address | **Exists** | All wired. |
| Settings: hotkey capture + clear | **Exists** | Global `keydown` capture with `capture: true`, Backspace/Delete clears, Esc cancels. |
| Settings: hotkey conflict detection | **Doesn't exist** | Rust would have to report registration failures; not exposed. |
| Settings: unsaved-edits / Write (Ctrl+S) | **Contradicts** | Settings persist immediately on change. Showing "unsaved" would be a lie. |
| Settings: density modes | **Doesn't exist** | — |
| Settings: hardware alert banner, DSP load, ring buffer, WASAPI driver string, CPU/MEM | **Doesn't exist** | Not exposed by any command. |
| Settings: `debug_wavs` | **Exists but unreachable** | `set_debug_wavs` is a live command; the field exists in `Settings`; no UI surfaces it. |
| Settings: version string | **Exists** | `ping` returns `{name, version}` from `CARGO_PKG_VERSION`. |
| Settings: WASAPI/48 kHz/Opus facts | **Real, displayable** | `PROJECT.md` confirms 48 kHz mono, 10 ms Opus frames. Sample rate of the *loaded file* is not exposed. |
| Window chrome (min/max/close buttons) | **Native, already provided** | `decorations` are on; the Stitch buttons are decorative. |
| Right-click app menu | **Exists** | Global `contextmenu` handler → custom menu. |
| Focus / hover / disabled states | **Partially** | Tokens exist in `design.md`; coverage is uneven (`design.md` requires hover + `:focus-visible` + pressed + disabled everywhere). |
| Custom warm scrollbars | **Partially** | Declared in `App.css`; several regions use nested scrolling. |

---

## 3. Feature classification

### A — existing feature, redesign only

| # | Feature | Reason |
|---|---|---|
| A1 | Lobby: create room, join by code, OTP entry | Commands exist; only presentation changes. |
| A2 | Lobby: local identity display + rename | `nickname` / `set_nickname` exist; move rename into the identity card (design) while keeping the dock affordance. |
| A3 | Room: participant tiles with speaking / muted / deafened / music states | All state exists in `PeerInfo` + `AudioState`. |
| A4 | Room: self tile with live mic meter | `input_level` exists; keep the imperative DOM path. |
| A5 | Room: per-peer volume 0–200 % | `set_peer_volume` exists. |
| A6 | Room: room code display + copy feedback | Exists. |
| A7 | Room: mute / deafen / PTT with hotkey chips | Exists; add the design's `kbd` chip treatment. |
| A8 | Room: leave | Exists. |
| A9 | Room: diagnostics surface | `get_transport_diagnostics` exists; restyle the drawer as a rack telemetry panel. |
| A10 | Music: collapsed dock + expanded rack, all 9 controls | Commands exist. |
| A11 | Settings: devices, gain, noise suppression, loopback, hotkeys + capture, voice/music quality, VPS address | All wired. |
| A12 | Settings: rebuild as a two-pane nav surface | Presentation only; see risk R6 for the modal→surface decision. |
| A13 | App shell: header, dock, section labels, toast/banner | Exists. |
| A14 | Right-click app menu | Exists; restyle only. |

### B — small, useful additions (implement during this redesign)

| # | Feature | Why it qualifies | Uses |
|---|---|---|---|
| B1 | **Per-peer local mute** (design's per-tile mute button) | Pure frontend: remember the previous volume, call `set_peer_volume(id, 0)`, restore on unmute. Immediately useful in a 10-person room. No Rust. | `set_peer_volume` |
| B2 | **Command palette / quick actions (Ctrl+K)** | Frontend-only over existing actions (mute, deafen, create, join, leave, open music, open settings, jump to a settings section). Justifies the shadcn `command` primitive and is a real power-user win in a desktop app. | existing commands |
| B3 | **About / version readout** | `ping` already returns the real version. Gives the design's "About" nav row honest content. | `ping` |
| B4 | **Surface the existing `debug_wavs` toggle** in a Diagnostics section | The command and the persisted field already exist; today it is unreachable. Zero new backend. | `set_debug_wavs` |
| B5 | **"Test loop monitor" quick action** in settings/room | `set_loopback` exists; the design's *Test Loop Monitor* button maps onto it exactly. | `set_loopback` |
| B6 | **Music drag-and-drop onto the player** | Tauri's built-in webview drag-drop event + existing `music_open`; matches STATE-A of the music deck. **Conditional:** verify the drag-drop event is reachable under the current capability set first; if not, drop B6 rather than add permissions. | `music_open` |
| B7 | **Honest transport readout** replacing the hardcoded "Relay" label | `diagnostics.mode` / `relay_mode` are real. Removes an existing lie (see §5). Frontend-only. | `get_transport_diagnostics` |
| B8 | **Empty / idle / error states** from the state matrices that map to real conditions: fresh lobby, invalid code, creating room, joining handshake, relay unreachable, music idle, music error | Each maps onto a real state (`roomState`, `room_error`, `statusMsg`, `music.active/playing`, `audio_error`). This is presentation of real conditions, not new features. | existing events |

### C — large or architectural (do NOT implement this session)

| # | Feature | Blocking reason |
|---|---|---|
| C1 | Per-peer audio level meters / per-peer RTT | Requires new transport instrumentation + new event payloads. |
| C2 | Jitter / packet-loss % readouts | Not measured; would require transport changes (and derived-from-counters values would mislead). |
| C3 | Music playlist, queue, prev/next, shuffle | The music bus is single-file; a playlist is a new subsystem plus persistence. |
| C4 | Track buffered %, spectral analysis, L/R output meters, peak dBFS | Requires decoder-side analysis in `verio-dsp` and new events. |
| C5 | Recent-rooms history + Rejoin | Needs persistence and, more importantly, is semantically hollow — codes are ephemeral and mostly dead on rejoin. Design reference only. |
| C6 | Hotkey conflict detection | Requires Rust to report registration failures. |
| C7 | Custom window chrome (frameless window + working min/max/close) | Needs `decorations: false` plus new window capability permissions and platform behaviour work. The design's buttons are decorative. |
| C8 | Music "reload" / re-open last file from disk | `music_open` needs a path; re-reading the last path is possible, but the UI state does not track it robustly and the value is low. Deferred, not skipped for architectural reasons. |

### D — unnecessary / out of scope (skip entirely; do not fake)

| # | Feature | Reason |
|---|---|---|
| D1 | Server rail, channel/node explorer, server discovery, "Mesh Routing Matrix" node graph | Discord-style server-ecosystem surface. Not Verio's product. |
| D2 | Friends / accounts / profiles / presence | Verio is accountless by design. |
| D3 | `CHACHA20-POLY1305` badge, `ed25519` fingerprint, `0x82f9` KeyID, "session pseudonym 5/16" | Verio does not implement or expose these. Displaying them would falsely claim a security property. |
| D4 | `PRESET: 1P / 6P / 10P` capacity switcher | Would hide real participants. The grid auto-sizes. |
| D5 | SOLO bus, `KILL CH`, per-channel `INJECT ROOM / MONITOR AUD` routing buttons, Direction 2 channel strips | No solo bus or routing matrix exists. |
| D6 | DSP engine load, ring-buffer occupancy, `WASAPI 10.0.22631`, CPU/MEM readouts | Not exposed by any command. |
| D7 | "Unsaved Audio Bus Edits · Discard · Write (Ctrl+S)" | Settings save immediately. Would be a fake dirty-state. |
| D8 | Density-mode selector (Rack / Utilitarian / Telemetry Split) | No such modes; would be a fake preference. |
| D9 | Hardware alert banner ("SECONDARY INPUT LOST · PCIe Aux Line #2") | Invents hardware the app cannot see. |
| D10 | Telemetry event log with timestamps, `FRAME LOSS: 0 / 14,290`, `BUFFER UNDERRUNS: 0` | No log stream and no such counters. |
| D11 | Decorative window min/max/close buttons | Dead controls; native chrome already exists. |
| D12 | Encryption-matrix badge as designed | Replaced by truthful transport info (B7) or omitted. |
| D13 | Lobby Directions 2 and 3 (Centered Console, Compact Rail & Dock) | Keep Direction 1 as the single lobby structure; the other two are alternatives, not additional screens. |
| D14 | Room Direction 2 (Broadcast Channel Strip) | Alternative, not additive. |
| D15 | Music Direction 3 (Modular Channel Strip) + deck queue | Requires C3/C4 and adds a second music UI. |
| D16 | `MONITOR OUT 82%` fader, "Local Monitor 80 %" fader | Monitor is a boolean in Verio (`music_monitor`). A fader would be fake. |
| D17 | Sample-rate / bit-depth chips for the loaded file, `FLAC` container chip, `48.0 kHz / 24-BIT LOCKED` DSP clock | The loaded file's format is not exposed. (The *engine's* fixed 48 kHz is real and may be shown as a static engine fact.) |
| D18 | Album art | No artwork extraction; the design's image is a stock asset. |
| D19 | "Mesh Routing Hops: Direct P2P Relay (0 Hops)", "Secondary node fallback timeout: 3000ms" | Not exposed. |
| D20 | BGM/aux bus naming, `BUS 04`, `AUX 01 [AUDIO SINK]`, `STAGE ROOM #4821` | Invented routing vocabulary. Room code is real and stays. |

**Net addition count: 8 (B1–B8).** Everything else is A (redesign) or skipped.

---

## 4. Redesign plan

### 4.0 Resolved conflicts (design.md wins — per the source-of-truth hierarchy)

These are the places where the Stitch deck and `design.md` disagree. In every case `design.md` is authoritative, so the deck's *structure* is adopted with `design.md`'s *values*.

| Item | Stitch | `design.md` (used) |
|---|---|---|
| Surface ramp | `#171412 / #1d1917 / #241f1c / #2d2723 / #352e2a` | `--bg #171412 / --surface #211c18 / --surface-2 #2a2420 / --surface-3 #332b25` (4 tiers). Floating overlays use `--surface-2` per the token's own "dropdown backgrounds" note. |
| Text tiers | `#e8dfd1 / #a89e90 / #6e6458` | `--text #e8dfd1 / --text-dim #7d7166 / --text-faint #57504a` |
| Destructive | Vermilion `#c75242` | `--danger #8a3a30` (destructive only); muted/deafened use `--warn #b86a4a` |
| Accent | `#ffb86f` primary, `#c9873f` primary-container | `--accent #c9873f`, hover `--accent-hi #d9994f`, pressed `--accent-lo #a86f2f` |
| Radii | 4 / 6 / 8 px | `--r-sm 5px / --r-md 8px / --r-lg 12px` |
| Type sizes | adds 12 px `body-sm` and 32 px `display` | scale is 10 / 11 / 13 / 14 / 15 / 18 / 24 — 12→13 px, 32→24 px |
| Icons | Material Symbols **icon font** | inline single-stroke SVG only, 1.3–1.5 stroke, **no icon fonts** → every icon is hand-written inline SVG |
| Elevation | `shadow-sm` / `shadow-md` on many cards | no shadows on cards/buttons/inputs; one shadow family reserved for floating overlays |
| Motion | `animate-pulse` / `animate-ping` / `animate-spin` used decoratively | 140 ms ease default, 200 ms for large surfaces; only the speaking indicator (one 1.8 s pulse ring) and the live mic meter may animate continuously |
| Speaking state | flat mossy-green outline, no pulse | same — **replace the current green glow** (`App.css` `.peer.is-speaking`) |
| Labels | `label-caps` in JetBrains Mono | `design.md`: section labels are Inter 11 px uppercase with 0.15em tracking. Mono is reserved for numbers/identifiers. → section labels use Inter; mono `label-caps` is kept only for genuine machine readouts. |

### 4.1 Direction selection (one structure per screen)

| Screen | Chosen | Why |
|---|---|---|
| Lobby | **Direction 1 Split Workbench**, content column capped at 1024 px; the two action panels sit side by side at every width the app supports and stack only below the `sm` breakpoint | Richest structure; scales down cleanly per the deck's own compact-desktop rule. |
| Room | **Direction 1 Acoustic Stage Grid** | Auto-sizing 1→3 columns handles 1–10 participants without a scrolling panel, satisfying the `design.md` scrollbar rule. |
| Music | **Direction 1 Expanded Studio Rack** (expanded) + **Direction 2 Compact Collapsed Dock** (collapsed) | Exactly the two states the app already has. |
| Settings | Two-pane nav rail + content, presented as a full-screen surface | Matches the deck; preserves the existing "open from anywhere, including in-call" behaviour. |

**Correction (Session 22) — the Lobby row previously read:** *"Direction 1 Split Workbench,
with the sidebar collapsing to a 56 px rail below 900 px and the two action panels stacking
below 900 px."* **Both halves were wrong**, and they were wrong in the same way — they
described deck presentation chrome as if it were app UI:

- **There is no sidebar in the lobby deck.** `verio_desktop_lobby_main_app_area/code.html`
  contains exactly two grids: `.action-grid` (the two action panels) and `.state-grid` (the
  six-card states matrix). The latter is the deck's own board layout, not a screen.
- **`.action-grid` is `grid-template-columns: 1fr 1fr` with no media query at all** — the deck
  never stacks the two panels. Only `.state-grid` carries breakpoints (1180px → 2 columns,
  768px → 1). The old sentence read those as the app's.

The deck is the visual source of truth, so the deck wins: the panels sit side by side at
700/860/900 and stack only below the `sm` breakpoint, which is what the shipped
`sm:grid-cols-2` does. The 1024px cap was added separately — uncapped, the two cards grew
302 / 382 / 402 / 502 / 652 px at 700 / 860 / 900 / 1100 / 1400, and `design.md` states no
maximum content width. 1024px is where the deck's own "standard desktop" band (901–1150px)
tops out; measured after the change, the cards hold at 484px for both 1100 and 1400 while
700/860/900 are unchanged.

### 4.2 Migration order and dependencies

```
Phase 1  Foundation ──────────────► blocks everything
         Tailwind v4 + @theme (design.md tokens) · preflight decision · fonts
         cn() · base layer (focus ring, scrollbars, selection) · icon module
              │
Phase 2  shadcn primitives ───────► blocks Phases 3–7
         button · icon-button · input · select · switch · slider · dialog
         popover · dropdown-menu · tooltip · tabs · separator · command · toast
         · scroll-area      (only those actually needed)
              │
Phase 3  App shell + shared structure ─► blocks 4–7
         AppShell (header / content / dock) · Dock · Panel · SectionLabel
         · TelemetryRow · StateChip · KbdChip · MeterWell · EmptyState
              │
   ┌──────────┼──────────┬───────────┐
   ▼          ▼          ▼           ▼
Phase 4    Phase 5    Phase 6     Phase 7
Music      Lobby      Settings    Room
   │          │          │           │
   └──────────┴──────────┴───────────┘
              │
Phase 8  Cleanup ───────────────────► delete obsolete CSS per screen, verify
```

Rationale for the order: Phase 3 first because every screen needs the shell and the shared atoms; Music before Lobby/Room because it is self-contained, has the most new visual surface, and proves the meter/well/fader primitives under load; Room last because it is the most important screen and depends on everything above it.

**Per-screen procedure (applies to Phases 4–7):**
1. Inventory the component's state, handlers, hooks, effects, keyboard behaviour, and command calls; write them down.
2. Extract the logic that is currently tangled into the JSX into a hook (only the minimum needed to rebuild the view safely).
3. Rebuild the presentation with Tailwind + shadcn + Verio tokens.
4. Delete that component's legacy CSS rules.
5. Verify build + runtime + the visual state list.

### 4.3 Phase 1 detail
- Tailwind v4 via `@tailwindcss/vite`, `@import "tailwindcss"` in `src/styles/index.css`.
- `@theme` maps **every** `design.md` token: colours (`--color-bg`, `--color-surface*`, `--color-border*`, `--color-text*`, `--color-accent*`, `--color-idle`, `--color-warn`, `--color-danger`), spacing (`--spacing-1: 4px … --spacing-15: 60px`, with the scale restricted to 4/8/12/16/20/24/32/40/60), radii (`--radius-sm 5px`, `--radius-md 8px`, `--radius-lg 12px`), fonts (`--font-ui`, `--font-mono`), type sizes (10/11/13/14/15/18/24), and motion (`--duration-fast 140ms`, `--duration-slow 200ms`, `--ease-verio ease`). Arbitrary off-scale values are prohibited by review.
- Preflight: keep it enabled (it supplies the border-box/border-colour/margin resets the app already assumes) but **verify** it against every legacy screen before migrating that screen, and re-declare the base reset, focus ring, custom scrollbars, and `::selection` in a `base` layer. If a specific preflight rule breaks legacy rendering, override that single rule in the base layer rather than disabling preflight globally.
- Load Inter + JetBrains Mono via the existing `<link>` in `index.html` (already present, correct per `design.md`).
- `cn()` = `clsx` + `tailwind-merge` (standard shadcn dependency pair).
- Icons: a local `components/verio/icon.tsx` with hand-written 1.4-stroke SVG paths — no icon font, no new dependency.

### 4.4 Verification protocol (every phase)
- `cd ui && pnpm build` (tsc + vite) — must pass.
- `cd ui && pnpm tauri build --no-bundle` — must pass at Phases 1, 3, and 8.
- `cargo test --workspace` once, to confirm Rust is untouched and still green.
- Runtime: launch and exercise lobby → create → join (two instances) → speaking → mute → deafen → music → settings → diagnostics → leave, plus keyboard-only navigation of menus/popovers/dialogs.
- Visual: render each redesigned screen at **700 / 900 / 1100 / 1400 px** and inspect default, hover, focus-visible, active, disabled, loading, error, empty, connected, disconnected, speaking, muted, deafened, long-name, long-address, and 10-participant cases.
- Because a Tauri webview cannot be driven headlessly here, I propose a **dev-only mock harness** (see D-1 below) so the screens can actually be rendered and screenshotted at the four widths; without it, "visual verification" would be an unverifiable claim.
- All command output recorded verbatim in `PROGRESS.md`. No fabricated results.

---

## 5. Architecture risks

**R1 — `App.tsx` is a 1,821-line monolith.** All state, all effects, and all JSX live in one component. Unsafe to rewrite wholesale. Mitigation: extract the logic into focused hooks (`useSettings`, `useRoom`, `usePeers`, `useAudioState`, `useHotkeyCapture`, `useMusic`, `useDiagnostics`, `useContextMenu`) *without* changing behaviour, then rebuild views on top. Each hook is verified against its current behaviour before its view is touched.

**R2 — The mic meter must stay imperative.** `input_level` fires at ~20 Hz and the current code writes `style.width` directly on a ref to avoid re-renders. The new meter component must accept a ref and never route this through React state. Highest-risk detail to lose.

**R3 — Hotkey capture depends on window-level capture-phase `keydown`.** `capture: true`, `preventDefault`, `stopPropagation`, modifier-only filtering, `Key*`/`Digit*`/`Space` normalisation, Backspace/Delete = clear, Esc = cancel. The design's "RECORDING KEY COMBINATION…" row is a faithful presentation of this — the behaviour must move verbatim. Also: global hotkeys are registered in Rust, so the UI must keep calling `set_hotkey` with the same combo string format.

**R4 — Music volume debounce.** `setMusicVolume` writes a draft immediately and calls `music_set_volume` after a 100 ms timeout. Preserve; a naive `onChange` → `invoke` would flood the backend.

**R5 — Peer identity across events.** Peers are keyed by `uuid` and the list is re-sorted on every `peer_connected`; `peer_state`/`peer_disconnected` match on `peer_id`. `peer_disconnected` with a null `peer_id` currently does **not** remove anyone. Preserve this exactly; do not "fix" it (Rust-side behaviour is out of scope).

**R6 — Settings is a modal overlay, not a route.** It opens over both lobby and room and closes back to the previous view. The design implies a full-screen *screen*. Decision: implement it as a full-screen overlay surface (still not a route, still openable in-call, still closes back to where you were). This preserves behaviour while matching the design. Needs approval (see Q3).

**R7 — Legacy CSS will override Tailwind utilities.** Tailwind v4 emits into `@layer utilities`; the existing `App.css` is unlayered plain CSS, and unlayered styles beat layered ones regardless of specificity. So any element carrying both a legacy class and Tailwind utilities will lose the Tailwind side. Mitigation: strict one-authority-per-element rule, migrate screen-by-screen, and delete each component's legacy rules in the same step as its rewrite. `App.css` must not survive as a permanent second system.

**R8 — No class-name collisions were found** between the 363 existing selectors and Tailwind utility names (checked). `.badge`, `.btn`, `.card`, `.mono`, `.switch`, `.kbd`, `.bar`, `.meter`, `.peer`, `.main`, `.header` are not utilities. Residual risk is only from future additions — review for it.

**R9 — Preflight changes base rendering.** It resets heading margins, list styles, `button`/`input` background and font inheritance, `img` max-width, and border colours. The existing components are class-driven so most of this is harmless or desirable, but it must be verified per screen rather than assumed.

**R10 — The default window is 860×640** (`tauri.conf.json`) with no minimum size. The 700–900 px band is therefore the *default* case, not the edge case, and the window can currently be shrunk below the supported range. The design's "standard desktop" (901–1150) is not the default. This inverts the usual responsive priority: design the compact band first, then let the wide band be a superset.

**R11 — Nested scrolling already exists** (`.main { overflow-y: auto }` plus the diagnostics drawer plus the settings modal body plus the music panel). `design.md` forbids nested scrolling without a strong reason. The redesign must give each screen exactly one scroll owner.

**R12 — Diagnostics shows incorrect/duplicated data today** (see §6). These are UI-only bugs inside the component being redesigned, so they are in scope; they must be fixed rather than faithfully re-rendered.

**R13 — Tauri capability surface.** `capabilities/default.json` grants `core:default` + global-shortcut only. Any custom-chrome work (C7) or drag-drop work (B6) must be validated against this; the plan is to add **no** permissions.

**R14 — Rust must stay byte-identical.** No new commands, events, protocol messages, or server APIs. Every B item above was chosen specifically because it needs none. If implementation reveals that a B item needs Rust, it drops to C and is skipped.

**R15 — Verification honesty.** A Tauri webview cannot be driven here. Either a dev-only mock harness is approved (D-1) so screens can genuinely be rendered and inspected, or visual verification is limited to what you exercise manually and report. I will not claim a visual state was verified that was not.

---

## 6. Bugs found during reconnaissance (in-scope UI bugs)

These sit inside components being redesigned, so they are UI-only and will be fixed as part of the rewrite:

1. **Dead control:** the header `status-pill` is a `<button>` with **no `onClick`** — it looks interactive and does nothing, while the comment claims it is the diagnostics toggle. Fix: wire it to the diagnostics panel (real functionality) or make it a non-interactive `span`.
2. **Hardcoded lie:** `getTransportBadgeInfo()` always returns `"Relay"` with the hint "Encrypted relay through the Verio server", regardless of the real transport. It is shown on the lobby card and in diagnostics. Fix: read `diagnostics.mode` / `relay_mode`.
3. **Fake fallback address:** diagnostics prints `141.11.1.110:3478` when `remote_endpoint` is null, presenting a hardcoded value as live data. Fix: show `—`.
4. **Duplicated metric:** "Last Packet Recv" and "Last Relay Recv" both render `last_recv_ms_ago`; the relay row is wrong. Fix: drop the duplicate and surface `relay_bytes_*` instead.
5. **Dead code:** the `{false && (…)}` Direct-LAN accordion plus `directAddrInput`, `isAdvancedOpen`, `handleConnectDirect` are unreachable; the `directCodes` state is only consumed inside that block, so removing the block also requires removing the state (otherwise `noUnusedLocals` fails the build).
6. **Stale comment:** `setInputDevice`/`setOutputDevice`/`setVoiceQuality` are invoked with parameter names (`gainDb`, `name`, `kbps`) that don't match the Rust snake_case argument names in every case — to be re-verified against the actual signatures during extraction so no silent no-op is carried forward.
7. **Toast has no timeout** and stacks only one message; `statusMsg` is overwritten by whichever event fires last. Presentation will follow the design's state blocks instead, but the underlying last-write-wins behaviour is preserved unless approved otherwise.

Documented under `found but not fixed` in `PROGRESS.md` (out of scope, Rust-side or non-UI):

- `peerVolumes` is never initialised from the backend — there is no getter for per-peer volume, so after a restart the UI shows 100 % even if the pipeline holds a different gain.
- `peer_disconnected` with a null `peer_id` leaves a stale card until the next roster event.
- `prepare_hotkey_capture` is registered but never invoked from the UI.
- `set_signaling_port`, `set_host_port`, `set_transport_mode`, `create_invite`, `accept_invite`, `complete_invite`, `disconnect_peer` are live commands with no UI (Session 11 removed the transport plumbing deliberately — not re-surfacing it).
- `PROJECT.md` notes the VPS UDP ports 3478/9092 still need opening; relay audio depends on it.
- `LICENSE` still contains the placeholder `Mahan [FULL NAME]`.

---

## 7. Decisions I need from you before Phase 1

**Q1 — Direction selection.** Approve the §4.1 choices (lobby Dir 1, room Dir 1, music Rack + Dock, settings two-pane)? Or do you prefer a different lobby direction?

**Q2 — Token precedence.** Confirm `design.md` wins on every conflict in §4.0 — in particular radii 5/8/12 (not 4/6/8), the 4-tier surface ramp (not 5 tiers), `--danger #8a3a30` (not vermilion), and no shadows on cards. This will look slightly flatter and slightly rounder than the Stitch renders.

**Q3 — Settings as a full-screen overlay surface** (not a modal, not a route), preserving in-call access and close-back-to-previous-view. Approve?

**Q4 — The B list.** Approve B1–B8, or trim it? My recommendation: keep B1, B3, B4, B5, B7, B8 (all near-zero risk); treat B2 (command palette) as optional scope; treat B6 (drag-and-drop) as conditional on the drag-drop event being reachable without new permissions.

**Q5 — Dev-only mock harness + browser-based visual verification.** May I add a `ui/src/dev/` harness that stubs `invoke`/`listen` with realistic sample data, gated behind a dev-only flag so it cannot ship, purely so the redesigned screens can be rendered and inspected at 700/900/1100/1400 px? Without it I cannot honestly verify visual states, only compilation and whatever you exercise by hand. It would be removed (or left dev-gated) at Phase 8.

**Q6 — Two micro-changes to `tauri.conf.json`** (config only, not Rust): `beforeDevCommand`/`beforeBuildCommand` → `pnpm`, and a `minWidth`/`minHeight` of ~720×560 so the supported range is enforced. Approve?

**Q7 — Dead-code cleanup.** Approve deleting the unreachable `{false && …}` Direct-LAN block and its three now-unused state values (§6.5)?

**Q8 — Bug fixes §6.1–6.4 and 6.6.** Confirm these are in scope as part of the rewrite.

**Q9 — Scope realism.** This is eight phases across four screens, ~30 primitives/components, and a full CSS teardown. It will not finish in one turn. I will work phase by phase, verify each, and report; expect several checkpoints. Confirm you want the full run rather than a reduced first pass (e.g. foundation + primitives + music player only).

---

## 8. What happens on approval

Phase 1 only: Tailwind v4 + `@theme` tokens + base layer + `cn()` + icon module, with `pnpm build` and `pnpm tauri build --no-bundle` re-run and the results recorded. No screen is redesigned until Phase 1 is verified green.

**STOP.** No application code will be modified until §7 is answered.
