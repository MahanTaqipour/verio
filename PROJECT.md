# Verio — Project Master Documentation

> **Verio** is an ultra-low latency, high-fidelity peer-to-peer (P2P) voice chat application designed for Windows, tailored specifically for gamers who need clear, lag-free voice communication without third-party server bloat, telemetry, or account requirements.

---

## 1. Project Overview & Vision

* **Core Purpose:** Instant, lag-free group voice communication (2–10 participants) with crystal-clear audio quality during intensive gaming sessions.
* **Key Differentiator:** Direct P2P audio where possible, with a self-hosted, lightweight fallback relay for restrictive symmetric NATs and CGNAT connections. Zero audio decoding or re-encoding on any server.
* **Audio Standards:** 48 kHz mono, 10 ms Opus frames, AI neural noise suppression (RNNoise), VAD gate, and synthesized in-ear audio cues (earcons).
* **Privacy & Control:** No accounts, no centralized user databases, no corporate servers. Rooms are ephemeral, identified by random 4-digit codes or direct IP:port connections.

---

## 2. Architecture & Tech Stack

### Repository Structure

```text
C:\dev\verio\
├── Cargo.toml                    # Workspace configuration
├── crates\
│   ├── verio-audio\              # Hardware audio capture & playback via CPAL
│   ├── verio-dsp\                # RNNoise, Opus codec, VAD, resampling, earcons, WAV
│   ├── verio-transport\          # WebRTC (str0m), UDP relay, peer-host, telemetry
│   ├── verio-discovery\          # VPS signaling client, direct-connect, mDNS, SDP codec
│   ├── verio-app\                # Audio pipeline, room orchestrator, multi-peer mixer, settings
│   ├── verio-server\             # Standalone lightweight Linux/Windows signaling & UDP relay server
│   └── soak-harness\             # Automated long-running loopback & latency verification tool
└── ui\
    ├── src\                      # React 19 + TypeScript + Vite 6 frontend
    └── src-tauri\                # Tauri v2 native bridge (Rust)
```

### Core Technologies

| Layer | Technology | Details |
|---|---|---|
| **GUI Framework** | Tauri v2 | Native Windows windowing with WebView2, zero-overhead memory footprint |
| **Frontend** | React 19 + TypeScript | Dark modern gaming interface, real-time level meters without React re-render overhead |
| **Audio Capture/Playback** | CPAL (0.15) | Low-latency WASAPI backend on dedicated stream-owner thread |
| **Noise Suppression** | `nnnoiseless` | Pure-Rust RNNoise neural network port (zero C/C++ dependencies) |
| **Voice Codec** | Opus (`audiopus`) | 10 ms aligned frames (480 samples @ 48 kHz), 32 kbps VOIP mode, in-band FEC & DTX |
| **Transport Engine** | `str0m` (0.23) | Pure-Rust Sans-IO WebRTC with Windows CNG (`wincrypto`), unreliable DataChannel audio |
| **Relay & Host Engine** | Custom Tokio UDP | Multiplexed VoIP relay on port 3478 & configurable UDP port with STUN RFC 5389 server |
| **Signaling Server** | `axum` + `tokio` | Ephemeral 4-digit room management and SDP offer/answer/candidate routing |

---

## 3. Detailed Chronological Evolution & Development History

### Phase 0: Workspace Scaffold & Tauri Bridge (2026-08-29)
* **Goal:** Establish a modular Rust workspace and Tauri 2 application with bi-directional IPC.
* **Work Completed:**
  - Setup 5 workspace crates with clean dependency separation.
  - Built Tauri bridge commands (`ping`, `emit_test_event`) and validated round-trip IPC.
  - Fixed Windows ICO header specification compliance and toolchain pinning issues.
  - Verified clean compilation with `cargo clippy --workspace -- -D warnings`.

### Phase 1: Local Audio Pipeline & DSP Engine (2026-08-30)
* **Goal:** High-fidelity local microphone capture, AI noise reduction, VAD gating, Opus encoding, and low-latency loopback.
* **Work Completed:**
  - **CPAL Integration:** Lock-free ringbuffers (`ringbuf`) between audio device callbacks and processing threads.
  - **Dedicated Stream-Owner Thread:** Solved CPAL `!Send` stream limitation by keeping streams isolated on a parked owner thread.
  - **Neural Noise Suppression:** Integrated `nnnoiseless` (RNNoise) processing 480-sample chunks with voice probability output.
  - **Voice Activity Detection (VAD):** Adaptive gate (enter threshold > 0.6, 250 ms hangover time).
  - **Opus Codec Wrapper:** Encodes 48 kHz audio into Opus packets; decoder handles FEC/PLC packet loss concealment.
  - **Global Hotkeys:** System-wide Mute (Ctrl+Shift+M), Deafen (Ctrl+Shift+D), and Push-to-Talk (PTT) with state restoration.
  - **Measurement:** Developed `latency_check` binary measuring round-trip hardware pipeline latency: **6.8 ms** (audible ~47 ms including 40 ms pre-buffer), CPU usage < 1% of one core.

### Phase 2: WebRTC Transport & Invite System (2026-09-01)
* **Goal:** Full-duplex direct P2P audio streaming between machines.
* **Decision Tree & Str0m Adoption:**
  - Standard `libdatachannel` bindings failed due to missing system OpenSSL, Perl, and libclang.
  - Selected `str0m` with `wincrypto` (Windows SChannel/CNG), enabling pure-Rust Sans-IO WebRTC with zero external C build requirements.
* **Audio Packet Specification (12-byte header):**
  - Byte 0: Magic (`0x56` = 'V')
  - Byte 1: Version (`1`)
  - Bytes 2..6: Sequence number (`u32` LE) for jitter and loss tracking
  - Bytes 6..12: Capture timestamp (`u48` LE unix ms) for one-way latency calculation
  - Bytes 12..: Raw Opus frame
* **10-Minute Soak Harness:** Built `room_soak` validating 30,000+ frames with 0.06% loss, <18 ms one-way transport latency, and <1 ms RTT.
* **Manual Invite Codes:** Base64-encoded SDP exchange (`VR1-...`) for manual copy-paste connection.

### Phase 3: VPS Signaling Server, Multi-Peer Mixer & UI Overhaul (2026-09-03)
* **Goal:** Eliminate manual SDP copy-paste via 4-digit room codes, fix remote audio playback, and redesign UI.
* **Work Completed:**
  - **Multi-Source Mixer (`verio-app`):** Replaced single-queue playback with a continuous multi-peer mixer supporting individual 40 ms jitter pre-buffers, soft-clipping limiter, and individual volume sliders (0–200%).
  - **10 ms Opus Alignment:** Halved Opus frame size from 20 ms to 10 ms (480 samples @ 48 kHz). RNNoise directly feeds Opus without rechunking, eliminating 10 ms of pipeline buffering delay.
  - **Audio Earcons:** Synthesized pure sine waves with 2 ms cosine ramps for Mute ON (400 Hz), Mute OFF (800 Hz), and Deafen (dual 300 Hz blips).
  - **VPS Signaling Server (`verio-server`):** Axum WebSocket service managing 4-digit rooms (`{"type": "create"}` / `{"type": "join"}`) with fallback UDP relay on port 8444.
  - **Total UI Overhaul:** Sleek dark gaming theme with persistent dock, initials avatars, animated green speaking glows, copyable room badges, and settings modal.

### Phase 4: Winsock WSAECONNRESET (10054) & STUN Hardening (2026-09-05)
* **Bugs Solved:**
  - **STUN RFC 5389 Typo:** Corrected magic cookie typo (`0x2112A444` → `0x2112A442`) which caused public STUN queries to fail silently, resulting in calls lacking public server-reflexive candidates.
  - **Windows Winsock 10054 Crash:** Local routers returning ICMP Port Unreachable caused Winsock to throw `WSAECONNRESET` (10054) on UDP `recv_from`. Handled this by disabling `SIO_UDP_CONNRESET` and treating error codes `10054`, `10051`, `10065`, and `10060` as non-fatal transient events.
  - **Built-in STUN Server:** Added RFC 5389 STUN Binding Request responder into `verio-server`.

### Phase 5: Hybrid Voice Transport & Diagnostics (2026-09-10 to 2026-09-11)
* **Problem:** Direct P2P over fiber/restrictive consumer NATs without UPnP dropped after 20–30s (`ice connection lost`) because symmetric NAT dropped unsolicited hole-punching packets.
* **Solution — Hybrid Architecture:**
  1. **Mode 1: Auto Hybrid (Default):** Instantly connects via Encrypted Cloud Tunnel while probing direct P2P in the background, upgrading seamlessly if reachable.
  2. **Mode 2: Encrypted Cloud Tunnel:** Voice datagrams route through the VPS relay switchboard on standard VoIP port `3478` (and fallback UDP port). Zero audio decoding or re-encoding.
  3. **Mode 3: Peer-Host ("Host on My PC"):** Room creator runs an embedded UDP relay switchboard (`EmbeddedVoiceHost`) on their machine; all peers connect directly to the creator's IP.
  4. **Mode 4: Direct P2P:** Pure WebRTC mesh.
* **Real-Time Network Diagnostics Drawer:** UI drawer displaying active transport mode, round-trip latency (RTT), voice packets in/out, inbound stream health, remote IP:port, and local socket interface.
* **Same-PC Multi-Instance Fix:** Replaced static UUID loading with dynamic instance UUID generation (`Uuid::new_v4()`), preventing UUID collisions and false self-echo packet suppression when testing multiple windows on the same PC.
* **Dual-Port Probing:** Client sends relay packets to both port `3478` and port `9092`, automatically locking onto whichever port the VPS firewall allows.

---

## 4. Current State of the Project

```text
Build Status:           PASSING (59 / 59 unit tests green)
Client Executable:      target\release\verio.exe (16.7 MB, optimized release)
Windows Server Exec:    target\release\verio-server.exe (1.8 MB)
Linux Server Binary:    target\x86_64-unknown-linux-musl\release\verio-server (2.8 MB, static musl ELF)
Disk Space:             9.43 GB free on C: (stable)
Signaling Server:       Tested & operational on ws://141.11.1.110:9091/ws
```

### What Works Fully Today:
1. **Audio Capture & Rendering:** Crystal-clear WASAPI capture/playback, RNNoise background noise reduction, VAD gating, 10 ms Opus encoding/decoding.
2. **Controls & Audio Cues:** Global hotkeys for mute, deafen, and PTT with state restoration and audible earcon tones.
3. **Room Signaling:** Instant 4-digit room creation and joining over WebSockets.
4. **Cloud Tunnel Relay:** Datagram routing on standard VoIP port 3478 and UDP 9092.
5. **Local Multi-Window Testing:** Seamlessly run multiple instances of `verio.exe` on one machine without UUID collisions.
6. **Live Telemetry:** In-call badge and slide-out Diagnostics Drawer showing real-time packets, latency, and socket states.

### Pending Configuration Item:
* **VPS Firewall UDP Ports:** The VPS at `141.11.1.110` has TCP port 9091 open (WebSocket signaling works), but inbound UDP ports `3478` and `9092` must be permitted in `ufw` and cloud provider security groups for cloud relay audio packets to flow.

---

## 5. How to Run & Deploy

### A. Testing 100% Offline on Local PC

1. **Start the local server:**
   ```powershell
   cd C:\dev\verio
   .\target\release\verio-server.exe
   ```
2. **Launch two client instances:**
   ```powershell
   Start-Process .\target\release\verio.exe
   Start-Process .\target\release\verio.exe
   ```
3. In both windows, click the **Settings** gear icon and set **VPS Server Address** to:
   ```text
   ws://127.0.0.1:8443
   ```
4. Click **Create Room** on Window 1, copy the 4-digit code, enter it in Window 2, and click **Join Room**.
5. Both windows will connect with live bi-directional audio and diagnostic telemetry.

---

### B. Deploying the Server to a Linux VPS (e.g. Ubuntu / Debian)

1. **Upload the static binary from your PC:**
   ```powershell
   scp C:\dev\verio\target\x86_64-unknown-linux-musl\release\verio-server root@<YOUR-VPS-IP>:/usr/local/bin/verio-server
   ```
2. **SSH into the VPS and set executable permissions:**
   ```bash
   chmod +x /usr/local/bin/verio-server
   ```
3. **Open firewall ports on Ubuntu / Debian:**
   ```bash
   sudo ufw allow 9091/tcp comment "Verio WebSocket Signaling"
   sudo ufw allow 9092/udp comment "Verio Voice Relay Port"
   sudo ufw allow 3478/udp comment "Verio VoIP/STUN Port"
   sudo ufw reload
   ```
   *(Ensure ports 9091 TCP, 9092 UDP, and 3478 UDP are also allowed in your cloud provider's web console firewall, e.g. Hetzner, Oracle Cloud, DigitalOcean, or AWS).*
4. **Create a background systemd service:**
   ```bash
   sudo tee /etc/systemd/system/verio.service > /dev/null << 'EOF'
   [Unit]
   Description=Verio Signaling and Voice Relay Server
   After=network.target

   [Service]
   Type=simple
   ExecStart=/usr/local/bin/verio-server -p 9091 -u 9092
   Restart=always
   RestartSec=3
   LimitNOFILE=65535

   [Install]
   WantedBy=multi-user.target
   EOF

   sudo systemctl daemon-reload
   sudo systemctl enable --now verio
   sudo systemctl status verio
   ```

---

### C. Rebuilding Binaries from Source

* **Rebuild Windows Client Release:**
  ```powershell
  cd C:\dev\verio\ui
  npx tauri build --no-bundle
  ```
  *(Output: `C:\dev\verio\target\release\verio.exe`)*

* **Rebuild Windows Server Release:**
  ```powershell
  cd C:\dev\verio
  cargo build --release -p verio-server
  ```
  *(Output: `C:\dev\verio\target\release\verio-server.exe`)*

* **Rebuild Linux Server Musl Binary on Windows:**
  ```powershell
  cd C:\dev\verio
  cargo rustc --target x86_64-unknown-linux-musl --release -p verio-server -- -C linker=rust-lld
  ```
  *(Output: `C:\dev\verio\target\x86_64-unknown-linux-musl\release\verio-server`)*

* **Run Workspace Unit Tests:**
  ```powershell
  cd C:\dev\verio
  cargo test --workspace
  ```

---

## 6. Project Roadmap & Future Enhancements

1. **Automatic Port Forwarding (UPnP / NAT-PMP):**
   - Automatically request temporary router port mappings when creating a room in Peer-Host mode, enabling seamless inbound direct connections without manual router configuration.
2. **Adaptive Jitter Buffer:**
   - Expand the current fixed 40–60 ms buffer with dynamic delay estimation to gracefully handle erratic packet jitter on variable wireless networks.
3. **Full Mesh Expansion (3–10 Participants):**
   - Extend the multi-peer mixer and signaling hub to support mesh and star topologies for multi-person gaming squads.
4. **Acoustic Echo Cancellation (AEC):**
   - Implement speaker-to-mic feedback cancellation for users playing without headphones.
5. **Production Installer Packaging:**
   - Package `verio.exe` into a signed NSIS Windows installer with auto-updater support.
