/**
 * Room — the in-call view: a stage grid of participant tiles.
 *
 * PRESENTATION ONLY. Every value and every command lives in App.tsx and arrives
 * as props, exactly as with the Lobby and the Settings surface. The mic meter is
 * the one exception to "no imperative code here", and it is deliberate: the fill
 * is written to a ref at ~20 Hz so the grid does not re-render twenty times a
 * second. See components/verio/level-meter.tsx.
 *
 * WHAT THE DECK SHOWS AND THIS DOES NOT
 * -------------------------------------
 * The room deck (`verio_desktop_in_call_room_view_phase_4`) is far richer than
 * the data. `PeerInfo` is `uuid, name, speaking, mute, deafen, music,
 * music_title` — seven fields, and that is the entire per-peer vocabulary the
 * app has. So everything else the deck draws is absent here by decision, not by
 * omission (`REDESIGN-PLAN.md` §3 C/D):
 *
 *   - per-peer audio level meters, per-peer RTT, per-peer codec bitrate
 *   - per-peer jitter / packet-loss percentages
 *   - a waveform, `-6.2 dBFS`, `-inf dBFS`, `128 KBPS`, `8ms (LOCAL LOOP)`
 *   - `EXCHANGING ICE CANDIDATES…`, `Audio Sink Closed`, `UNSTABLE`,
 *     `PACKET DROP`, `LOW SIGNAL`, `HANDSHAKE`, `SYNCING`
 *   - `HOST` / `BUS GAIN` / `BUS 04` / `AUX 01` and the `AUDIO BUS NODES` rail
 *   - the `DSP LOAD`, `RING BUFFER`, `NODE SYNC`, `LOCAL CLOCK` readouts
 *
 * `ice_state` does exist, but on `TransportDiagnostics` — it describes *this*
 * client's transport, not any peer's, so it belongs in the diagnostics drawer
 * and not on a peer tile. Putting it on a tile would attribute a local
 * measurement to someone else.
 *
 * WHAT REPLACED THE LEGACY MARKUP
 * -------------------------------
 * Two silent defects in the old room view are worth recording, because both
 * looked fine:
 *
 *   1. It used `var(--text-muted)` (twice) and `var(--text-primary)` (once).
 *      **Neither token exists.** The declarations were therefore dropped and the
 *      elements inherited instead, so "Waiting for others" and the room code
 *      rendered at full text brightness rather than dimmed. Same shape as the
 *      `duration-fast` no-op found in Session 21: it renders, it looks
 *      plausible, it does nothing.
 *   2. The empty-state tile drew a **music-note glyph** next to "Waiting for
 *      others". It is now IconUsers.
 *
 * The legacy peer volume slider also had no accessible name — a bare
 * `<input type="range">`. It has one now.
 *
 * GRID AND SCROLL
 * ---------------
 * `REDESIGN-PLAN.md` chose "auto-sizing 1→3 columns handles 1–10 participants
 * without a scrolling panel". The first half is implemented; **the second half
 * is false, and measured so.** At 860×640 the grid has 318px of height. Ten
 * peers plus yourself is eleven tiles, which at three columns is four rows — and
 * four rows of a card containing a 32px avatar cannot fit in 318px at any
 * padding that leaves the card usable. The honest options were to scroll or to
 * shrink the card to ~70px tall, which is not a card any more.
 *
 * So the grid scrolls at high participant counts, and the scroll is confined to
 * the grid: the room code and Leave are outside the scrolling region and stay
 * put, which the legacy layout did not do (it scrolled the header away, so at
 * ten participants the Leave button left the viewport). The plan's claim is
 * corrected with the measurement in PROGRESS.md.
 */

import type { ReactNode, RefObject } from "react";
import { Button } from "@/components/ui/button";
import { SectionLabel } from "@/components/ui/field";
import { Slider } from "@/components/ui/slider";
import { Panel } from "@/components/ui/surface";
import { StateChip, type ChipTone } from "@/components/verio/state-chip";
import { MicLevelMeter, type LevelMeterHandle } from "@/components/verio/level-meter";
import {
  IconCheck,
  IconCopy,
  IconEarOff,
  IconLeave,
  IconMicOff,
  IconMusic,
  IconUsers,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";

/** The seven fields `PeerInfo` actually carries. Nothing is added here. */
export interface RoomPeer {
  uuid: string;
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  music: boolean;
  music_title: string | null;
}

export interface RoomProps {
  roomCode: string | null;
  /** True briefly after a successful copy, so the button can confirm it. */
  copied: boolean;
  onCopyCode: () => void;
  onLeave: () => void;
  nickname: string;
  self: { speaking: boolean; muted: boolean; deafened: boolean };
  /** The self tile's meter. Driven by App.tsx's single `input_level` fan-out. */
  micMeterRef: RefObject<LevelMeterHandle | null>;
  peers: RoomPeer[];
  peerVolumes: Record<string, number>;
  onPeerVolume: (uuid: string, volume: number) => void;
  /** Toggles local mute for one peer: 0 ↔ the remembered level. */
  onTogglePeerMute: (uuid: string) => void;
  className?: string;
}

/** First two initials, uppercase. Moved here from App.tsx — it is presentation. */
function getInitials(name?: string | null): string {
  if (!name) return "U";
  const initials = name
    .trim()
    .split(/\s+/)
    .map((part) => part[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
  return initials || "U";
}

/**
 * The avatar. `rounded-full` is one of the two places design.md permits a fully
 * round shape (the other is a status dot).
 *
 * A speaking participant gets the accent fill — no ring, no pulse. That is a
 * decision, not an omission, and it is worth recording because the first version
 * of this component got it wrong in two ways at once:
 *
 *   1. It wrapped the avatar in `animate-speak-ring`, an animation utility that
 *      **does not exist**. There is no `--animate-speak-ring` in `@theme`, so the
 *      class resolved to no rule at all — a phantom class, caught by
 *      check-phantom-classes.mjs and by nothing else. It looked right in the
 *      source and did nothing at runtime.
 *   2. Even had it resolved, it would have been wrong. `design.md` says the
 *      speaking indicator *may* use a single 1.8s pulse — permissive, not
 *      prescriptive — while the deck states "flat mossy-green outline
 *      (explicitly no pulsating rings/halos)" and `REDESIGN-PLAN.md` §4.0
 *      resolves that row to the same. Choosing not to use a permission is not a
 *      conflict, so the plan's resolution stands: flat.
 *
 * The legacy app *did* pulse here (`@keyframes ring` on
 * `.peer.is-speaking .peer-avatar::after`), so this removes an animation. That
 * is the one place the Room screen is not a pure restyle, and it is deliberate.
 *
 * Colour note: `--accent` (amber), not the deck's mossy green. `state-chip.tsx`
 * — shipped in Phase 2 — defines the tone vocabulary as "`accent` … reserved for
 * primary actions and the speaking indicator", and the legacy `.peer.is-speaking`
 * was amber too. Changing it here would make one screen disagree with three
 * primitives, so the shipped convention wins and the discrepancy is reported.
 *
 * A muted or deafened peer's avatar is NOT dimmed, and that was measured rather
 * than judged. The first version used `bg-surface-3 text-text-faint`, which
 * probe-room-contrast.mjs reports at **1.75:1** — the initials are effectively
 * invisible, so the dimming was not a scan aid, it was a smudge. The state is
 * already carried explicitly by the MUTED / DEAFENED chips, which is also what
 * `state-chip.tsx` says the `warn` tone is for ("intentional states that should
 * still be visible"), and neither `design.md` nor the deck asks for a dimmed
 * avatar. So the fill stays `--idle` and the chips do the talking.
 */
function Avatar({ name, speaking }: { name: string; speaking: boolean }) {
  return (
    <span
      className={cn(
        "grid size-8 shrink-0 place-items-center rounded-full font-mono text-11 font-semibold",
        speaking ? "bg-accent text-bg" : "bg-idle text-bg",
      )}
    >
      {getInitials(name)}
    </span>
  );
}

function Chip({ tone, children, title }: { tone: ChipTone; children: ReactNode; title?: string }) {
  return (
    <StateChip tone={tone} title={title}>
      {children}
    </StateChip>
  );
}

/** One participant tile. Used for the local user and for every remote peer. */
function PeerTile({
  name,
  speaking,
  muted,
  deafened,
  isSelf,
  music,
  musicTitle,
  volume,
  onVolume,
  onToggleMute,
  children,
}: {
  name: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  isSelf?: boolean;
  music?: boolean;
  musicTitle?: string | null;
  volume?: number;
  onVolume?: (next: number) => void;
  onToggleMute?: () => void;
  children?: ReactNode;
}) {
  const locallyMuted = volume === 0;

  return (
    <Panel
      padding="sm"
      className={cn(
        "flex flex-col gap-2",
        // The only per-tile emphasis. A speaking tile lifts by border colour and
        // a 4% accent wash — no glow, no shadow, no scale.
        speaking && "border-accent/35 bg-linear-to-b from-accent/4 to-transparent",
      )}
    >
      <div className="flex min-w-0 items-start gap-2.5">
        <Avatar name={name} speaking={speaking} />

        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="truncate text-14 text-text" title={name}>
            {name}
          </span>
          {/* `flex-wrap` so a long name plus three chips wraps instead of
              squeezing the name to nothing. */}
          <div className="flex flex-wrap items-center gap-1">
            {isSelf && <Chip tone="neutral">You</Chip>}
            {speaking && <Chip tone="accent">Speaking</Chip>}
            {muted && <Chip tone="warn">Muted</Chip>}
            {deafened && <Chip tone="warn">Deafened</Chip>}
            {music && (
              // `accent` because a peer injecting music is actively putting
              // audio into the room — the same semantic as speaking, and the
              // same tone the music panel uses for "Sending to room".
              <Chip tone="accent" title={musicTitle || "Playing music"}>
                <span className="inline-flex items-center gap-1">
                  <IconMusic size={14} />
                  {musicTitle || "Music"}
                </span>
              </Chip>
            )}
          </div>
        </div>
      </div>

      {/* The self tile shows the live input level instead of a volume control:
          you cannot set your own listening volume, and the meter is real.
          The meter arrives as `children` rather than being reached for from
          here, so this component needs no knowledge of the ref. */}
      {isSelf ? (
        <div className="flex items-center gap-2">
          {children}
          <span className="text-10 text-text-faint">Input</span>
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <Slider
            min={0}
            max={2}
            step={0.05}
            value={volume ?? 1}
            fillPercent={((volume ?? 1) / 2) * 100}
            valueText={`${Math.round((volume ?? 1) * 100)} percent`}
            onChange={(e) => onVolume?.(parseFloat(e.target.value))}
            aria-label={`Volume for ${name}`}
            className="min-w-0 flex-1"
          />
          <span
            className={cn(
              "mono-num w-9 shrink-0 text-right text-11",
              locallyMuted ? "text-warn" : "text-text-dim",
            )}
          >
            {Math.round((volume ?? 1) * 100)}%
          </span>
          <button
            type="button"
            onClick={onToggleMute}
            aria-pressed={locallyMuted}
            aria-label={
              locallyMuted
                ? `Unmute ${name} locally`
                : `Mute ${name} locally`
            }
            title={locallyMuted ? "Unmute locally" : "Mute locally"}
            className={cn(
              "grid size-5 shrink-0 place-items-center rounded-sm",
              "transition-colors ease-verio",
              "hover:bg-surface-3",
              locallyMuted ? "text-warn" : "text-text-faint hover:text-text-dim",
            )}
          >
            <IconEarOff size={14} />
          </button>
        </div>
      )}
    </Panel>
  );
}

export function Room({
  roomCode,
  copied,
  onCopyCode,
  onLeave,
  nickname,
  self,
  micMeterRef,
  peers,
  peerVolumes,
  onPeerVolume,
  onTogglePeerMute,
  className,
}: RoomProps) {
  // `peers` is remote only, so the participant count is peers + you. The legacy
  // label read "In call - N of 10"; the "of 10" is dropped because no backend
  // enforces a capacity, so the app cannot know it and an eleventh joiner would
  // have produced "11 of 10". PROJECT.md documents 2–10 as the design target,
  // which is not the same as a limit the client can report.
  const count = peers.length + 1;

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col gap-4", className)}>
      {/* Fixed. The code and Leave must stay reachable while the grid scrolls —
          the legacy layout put them inside the scroll container, so at ten
          participants they scrolled out of the viewport. */}
      <header className="flex shrink-0 items-center justify-between gap-4">
        <div className="flex min-w-0 items-center gap-3">
          <span className="label-caps text-text-dim">Room</span>
          <span className="mono-num text-18 tracking-chip text-text">
            {roomCode || "----"}
          </span>
          {roomCode && (
            <Button
              variant="secondary"
              size="sm"
              onClick={onCopyCode}
              // The label change is the confirmation; the icon carries the same
              // state so it is not conveyed by text alone.
              icon={copied ? <IconCheck size={14} /> : <IconCopy size={14} />}
              aria-live="polite"
            >
              {copied ? "Copied" : "Copy code"}
            </Button>
          )}
        </div>

        <Button
          variant="danger-outline"
          size="sm"
          onClick={onLeave}
          icon={<IconLeave size={14} />}
          className="shrink-0"
        >
          Leave call
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <SectionLabel level={2} className="shrink-0">
          In call — {count} {count === 1 ? "participant" : "participants"}
        </SectionLabel>

        {/* The scrolling region. `max-w-256` matches the Lobby's column cap so a
            wide window does not stretch a tile to 450px. */}
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ul
            className={cn(
              "mx-auto grid w-full max-w-256 list-none gap-3",
              "grid-cols-1 sm:grid-cols-3 xl:grid-cols-4",
            )}
          >
            <li>
              <PeerTile
                name={nickname}
                speaking={self.speaking}
                muted={self.muted}
                deafened={self.deafened}
                isSelf
              >
                <MicLevelMeter ref={micMeterRef} className="min-w-0 flex-1" />
              </PeerTile>
            </li>

            {peers.map((p) => (
              <li key={p.uuid}>
                <PeerTile
                  name={p.name}
                  speaking={p.speaking}
                  muted={p.mute}
                  deafened={p.deafen}
                  music={p.music}
                  musicTitle={p.music_title}
                  volume={peerVolumes[p.uuid] ?? 1}
                  onVolume={(v) => onPeerVolume(p.uuid, v)}
                  onToggleMute={() => onTogglePeerMute(p.uuid)}
                />
              </li>
            ))}

            {peers.length === 0 && (
              // A dashed slot, not a peer tile: it reads as "space waiting to be
              // filled" rather than as a participant with no name.
              <li>
                <Panel
                  padding="sm"
                  className="flex h-full flex-col items-start gap-2 border-dashed"
                >
                  <div className="flex items-center gap-2.5">
                    <span className="grid size-8 shrink-0 place-items-center rounded-full text-text-faint">
                      <IconUsers size={14} />
                    </span>
                    <span className="text-13 text-text-dim">Waiting for others</span>
                  </div>
                  <span className="text-13 text-text-faint">
                    Share room code{" "}
                    <span className="mono-num text-text">{roomCode || "----"}</span> to
                    invite someone.
                  </span>
                </Panel>
              </li>
            )}
          </ul>
        </div>
      </div>
    </div>
  );
}

/** Re-exported so App.tsx does not need a second import for the mute icon. */
export { IconMicOff };
