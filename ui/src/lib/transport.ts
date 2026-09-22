/**
 * Transport descriptors — how Verio names the path its voice traffic takes.
 *
 * Only the **live** mode is described here: the value reported by the transport
 * itself in `TransportDiagnostics.mode`, which says what is *actually* carrying
 * voice right now. It is only meaningful once a call exists.
 *
 * There used to be a second, *configured* mode as well (`Settings.transport_mode`
 * — what the user asked for). It was removed in Session 20 together with the
 * "change voice transport mode in Settings" button on the Host Call card, because
 * that surface told the user three untrue things at once: no control in Settings
 * could change the value, the frontend never called the existing
 * `set_transport_mode` command, and `crates/verio-app/src/room.rs` pinned the
 * effective mode to cloud relay regardless. Transport selection is a legacy
 * concept; Verio is cloud-relay only. `Settings.transport_mode` still exists on
 * the wire and is still in the TS `Settings` interface — it is simply no longer
 * surfaced as something the user can act on.
 *
 * Every string below is derived from the live value. Nothing is invented, and
 * there is no fallback to a made-up mode.
 *
 * Values verified against the backend — `unknown` (initial), `cloud_relay`,
 * `peer_host`, `direct_p2p`, `forced_relay`:
 *   crates/verio-transport/src/lib.rs, crates/verio-app/src/room.rs
 */

/** `TransportDiagnostics.mode` — what is actually carrying voice. */
export type LiveTransportMode =
  | "unknown"
  | "cloud_relay"
  | "peer_host"
  | "direct_p2p"
  | "forced_relay";

export type TransportTone = "neutral" | "accent" | "idle";

export interface TransportDescriptor {
  /** Compact label for chips. */
  short: string;
  /** Full label for the diagnostics row. */
  label: string;
  /** One sentence, shown as the diagnostics sub-line. */
  hint: string;
  tone: TransportTone;
  /**
   * False when the app genuinely does not know the answer yet. Callers must
   * render this as an absence ("—"), never as a mode.
   */
  known: boolean;
}

const UNKNOWN: TransportDescriptor = {
  short: "—",
  label: "Not established",
  hint: "No voice path has been established yet.",
  tone: "neutral",
  known: false,
};

const LIVE: Record<Exclude<LiveTransportMode, "unknown">, Omit<TransportDescriptor, "known">> = {
  cloud_relay: {
    short: "Relay",
    label: "Encrypted relay",
    hint: "Every call runs through the Verio relay server.",
    tone: "accent",
  },
  peer_host: {
    short: "Peer host",
    label: "Peer host",
    hint: "The room creator hosts the call; others connect to them.",
    tone: "idle",
  },
  direct_p2p: {
    short: "Direct",
    label: "Direct P2P",
    hint: "Voice flows straight between peers with no relay in the path.",
    tone: "idle",
  },
  forced_relay: {
    short: "Forced relay",
    label: "Forced relay",
    hint: "Relay forced by the transport mode setting.",
    tone: "accent",
  },
};

/** Narrow an arbitrary string from the wire to a known live mode. */
function asLiveMode(mode: string | null | undefined): LiveTransportMode {
  if (
    mode === "cloud_relay" ||
    mode === "peer_host" ||
    mode === "direct_p2p" ||
    mode === "forced_relay"
  ) {
    return mode;
  }
  return "unknown";
}

/**
 * Describe the transport that is actually carrying voice. Pass
 * `diagnostics.mode`. Returns an explicitly-unknown descriptor when there is no
 * call yet or the transport has not reported a path.
 */
export function describeLiveTransport(
  mode: string | null | undefined,
): TransportDescriptor {
  const key = asLiveMode(mode);
  if (key === "unknown") return UNKNOWN;
  return { ...LIVE[key], known: true };
}
