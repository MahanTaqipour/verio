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
  IconHeadphoneOff,
  IconLeave,
  IconMicOff,
  IconMusic,
  IconUsers,
  IconVolumeOff,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";

/** The fields `PeerInfo` actually carries. Nothing is added here. */
export interface RoomPeer {
  uuid: string;
  /**
   * Session 29: the peer's PERSISTENT id. A room-wide mute is keyed on this, and
   * it is empty until their `Hello` reaches us — which is why the room-mute
   * control is withheld rather than guessed at.
   */
  user_id: string;
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  music: boolean;
  music_title: string | null;
  /**
   * Session 29b: this peer is a music bot session, not a person.
   *
   * It is an ordinary peer in every other respect — same relay, same control
   * channel, same per-peer volume — which is exactly why it gets an ordinary tile
   * with an ordinary volume slider rather than a bespoke control. What the flag
   * changes is only how the tile is drawn, and that a bot is not counted as a
   * participant.
   */
  is_bot: boolean;
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
  /**
   * Session 29: room-wide mutes, keyed by the silenced peer's persistent id.
   * The value is whoever set it — empty when unknown, which happens when the
   * mute was restored from disk and the actor was never on the wire for us.
   */
  roomMutes: Record<string, string>;
  onToggleRoomMute: (userId: string, muted: boolean) => void;
  /** This user's own persistent id, to tell whether a mute names them. */
  selfUserId: string;
  /**
   * Session 29b: the persistent id of whoever is hosting the music, read from the
   * bot peer's `user_id`. Empty when nobody is hosting.
   *
   * The badge goes on the host's *voice* card rather than on the Music card,
   * because the card that needs explaining is the person's: two peers appear for
   * one person, and this is what says which of them is in charge of the music.
   */
  botHostUserId: string;
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
function Avatar({
  name,
  speaking,
  isBot,
}: {
  name: string;
  speaking: boolean;
  isBot?: boolean;
}) {
  // Session 29b: a bot gets a glyph and a tinted disc, not initials on a solid one.
  //
  // Initials would render "M" for "Music" — indistinguishable from a person whose
  // name starts with M — and telling the two apart is the entire job of this
  // treatment, because one person now appears as two peers.
  //
  // The colours are the `accent` chip tone's three classes, verbatim: this is not a
  // new colour combination, it is the one `state-chip.tsx` already ships for
  // "actively putting audio into the room", on a round shape. Inventing a tint here
  // would mean a pair nobody has measured. `accent` cannot collide with the speaking
  // fill below because a bot never speaks and a person never plays through the bot's
  // session, so a tile is never both.
  if (isBot) {
    return (
      <span
        className={cn(
          "grid size-8 shrink-0 place-items-center rounded-full border",
          "border-accent/25 bg-accent/8 text-accent",
        )}
      >
        <IconMusic size={14} />
      </span>
    );
  }
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

function Chip({
  tone,
  children,
  title,
  ariaLabel,
  className,
}: {
  tone: ChipTone;
  children: ReactNode;
  title?: string;
  ariaLabel?: string;
  /**
   * Session 29d: forwarded so a chip can opt into truncation — `max-w-full
   * min-w-0` on the chip plus `truncate` on its label. The music chip is the one
   * caller: a track title is a filename, so it is the only value a tile renders
   * that has no upper bound.
   */
  className?: string;
}) {
  return (
    <StateChip
      tone={tone}
      title={title}
      ariaLabel={ariaLabel}
      className={className}
    >
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
  isBot,
  host,
  music,
  musicTitle,
  volume,
  onVolume,
  onToggleMute,
  roomMuted,
  onToggleRoomMute,
  children,
}: {
  name: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  isSelf?: boolean;
  /** Session 29b: draw the bot treatment. */
  isBot?: boolean;
  /** Session 29b: this peer is the one hosting the music. */
  host?: boolean;
  music?: boolean;
  musicTitle?: string | null;
  volume?: number;
  onVolume?: (next: number) => void;
  onToggleMute?: () => void;
  /**
   * Session 29: this peer is silenced for the whole room. Absent (not `false`)
   * for a peer whose persistent id we do not know yet — there is nothing to
   * toggle in that case, so the control is withheld rather than shown disabled.
   */
  roomMuted?: boolean;
  onToggleRoomMute?: () => void;
  children?: ReactNode;
}) {
  const locallyMuted = volume === 0;
  /**
   * Session 29b: never offered on a bot's tile.
   *
   * A room mute is keyed on the persistent id, and a bot shares its host's — so
   * the button here would silence the *person's* voice too while labelling itself
   * "Mute Music for everyone". Two buttons for one key, and the second one lies
   * about what it does. The chip is a different matter and stays: when the host is
   * room-muted the bot really is silenced for the room, and saying so is honest.
   */
  const canRoomMute = onToggleRoomMute !== undefined && !isBot;

  return (
    <Panel
      padding="sm"
      className={cn(
        // `h-full` so every tile in a row is the same size. The grid cell already
        // stretches, but the panel inside it stayed content-sized — so the moment
        // one tile's chips wrapped, that tile alone stood taller than its
        // neighbours and the row jumped.
        "flex h-full flex-col gap-2",
        // The only per-tile emphasis. A speaking tile lifts by border colour and
        // a 4% accent wash — no glow, no shadow, no scale.
        speaking && "border-accent/35 bg-linear-to-b from-accent/4 to-transparent",
      )}
    >
      <div className="flex min-w-0 items-start gap-2.5">
        <Avatar name={name} speaking={speaking} isBot={isBot} />

        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="truncate text-14 text-text" title={name}>
            {name}
          </span>
          {/* `flex-wrap` so a long name plus three chips wraps instead of
              squeezing the name to nothing. */}
          <div className="flex flex-wrap items-center gap-1">
            {isSelf && <Chip tone="neutral">You</Chip>}
            {/* Session 29b: the badge the plan asked to keep "subtle, not loud".
                `idle` is the tone for "quiet, connected, at rest" — a role rather
                than an action or a warning — and it is the pair the music panel's
                "Paused" chip already ships, so it adds no unmeasured colour. */}
            {host && (
              <Chip tone="idle" title="Hosting the music for this room">
                Host
              </Chip>
            )}
            {/* Session 29d: there is no "Speaking" chip. The border colour and the
                4% accent wash above already say it, and the chip was a *second*
                signal for one state — one that added a row, so the tile grew the
                moment somebody spoke. The tile now keeps its size and only its
                border changes. */}
            {/* Icon-only chips. The glyph carries the state, so the accessible
                name has to come from `ariaLabel` — every icon in the module is
                `aria-hidden`, so without it these would be silent to a screen
                reader. Muted is `--warn`; deafened is `--danger`, which is
                already how the dock styles its deafen button, so the same state
                reads the same in both places. */}
            {muted && (
              <Chip tone="warn" title="Muted" ariaLabel="Muted">
                <IconMicOff size={14} />
              </Chip>
            )}
            {deafened && (
              <Chip tone="danger" title="Deafened" ariaLabel="Deafened">
                <IconHeadphoneOff size={14} />
              </Chip>
            )}
            {/* Session 29: silenced for everyone — a different claim from the
                mic-off chip above, which is the peer's own doing. The glyph is a
                muted speaker rather than an ear, because this says nothing about
                anyone's listening habits: the room has stopped their audio. */}
            {roomMuted && (
              <Chip
                tone="danger"
                title="Muted for everyone"
                ariaLabel="Muted for everyone"
              >
                <IconVolumeOff size={14} />
              </Chip>
            )}
            {music && (
              // `accent` because a peer injecting music is actively putting
              // audio into the room — the same semantic as speaking, and the
              // same tone the music panel uses for "Sending to room".
              //
              // Session 29d: the title truncates. A track is a filename and a
              // filename is arbitrarily long; a chip never wraps, so without a
              // shrinkable label the text ran straight out of the card. The full
              // value is still in `title`.
              <Chip
                tone="accent"
                title={musicTitle || "Playing music"}
                className="min-w-0 max-w-full"
              >
                <span className="flex min-w-0 items-center gap-1">
                  <IconMusic size={14} className="shrink-0" />
                  <span className="truncate">{musicTitle || "Music"}</span>
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
            step={0.01}
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
            {/* Session 29d: a speaker, not an ear. The ear glyph named the organ
                rather than the act and sat confusingly close to the deafen pair;
                this control silences one peer for *this* listener, which is what a
                crossed-out speaker already means everywhere else in the app. */}
            <IconVolumeOff size={14} />
          </button>
          {/* Session 29: the room-wide mute, beside — not merged with — the local
              one. They are two owners of one verb: the speaker button changes what
              *this* user hears, this one silences the peer for everyone. They
              are separate buttons because someone who wants to stop hearing a
              person must not be able to silence them for the room by accident.
              Session 29d: this one is a muted microphone, because that is what it
              does — it takes that person's voice out of everybody's mix — and
              because two identical speakers side by side said nothing about which
              scope each had. */}
          {canRoomMute && (
            <button
              type="button"
              onClick={onToggleRoomMute}
              aria-pressed={roomMuted}
              aria-label={
                roomMuted
                  ? `Unmute ${name} for everyone`
                  : `Mute ${name} for everyone`
              }
              title={roomMuted ? "Unmute for everyone" : "Mute for everyone"}
              className={cn(
                "grid size-5 shrink-0 place-items-center rounded-sm",
                "transition-colors ease-verio",
                "hover:bg-surface-3",
                roomMuted ? "text-danger" : "text-text-faint hover:text-text-dim",
              )}
            >
              <IconMicOff size={14} />
            </button>
          )}
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
  roomMutes,
  onToggleRoomMute,
  selfUserId,
  botHostUserId,
  className,
}: RoomProps) {
  // `peers` is remote only, so the participant count is peers + you. The legacy
  // label read "In call - N of 10"; the "of 10" is dropped because no backend
  // enforces a capacity, so the app cannot know it and an eleventh joiner would
  // have produced "11 of 10". PROJECT.md documents 2–10 as the design target,
  // which is not the same as a limit the client can report.
  //
  // Session 29b: a bot is not a participant. Counting it would make a two-person
  // call with music read "3 participants", and — worse — it would stop the room
  // saying "Waiting for others" to a host who really is on their own, because their
  // own bot kept the roster non-empty.
  const humans = peers.filter((p) => !p.is_bot);
  const count = humans.length + 1;

  /**
   * Session 29b: is the music being hosted by *this* user? The bot shares its
   * host's persistent id, so the self tile needs the same comparison the peer
   * tiles get — the host's own card carries the badge too.
   */
  const selfHosts = botHostUserId !== "" && botHostUserId === selfUserId;

  /**
   * Session 29: presence, not truthiness. `by` is legitimately the empty string
   * for a mute restored from disk, and `roomMutes[id] !== undefined` is the test
   * that survives that — `id in roomMutes` would also be true for `"constructor"`.
   */
  const isRoomMutedId = (userId: string) =>
    userId !== "" && roomMutes[userId] !== undefined;

  /** True when the room has silenced *this* user. */
  const selfRoomMuted = isRoomMutedId(selfUserId);
  const selfRoomMutedBy = selfUserId ? roomMutes[selfUserId] ?? "" : "";

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

      {/* Session 29: if the room has silenced us, say so — and say who did it.
          Without this the user hears their own mic working and concludes the app
          is broken. The way out is here too, because the policy is that the
          target can always lift their own mute.

          The colours are measured, not chosen by eye. On this 8 % danger wash
          (`oklab(0.4549 … / 0.08)` over `--bg`, i.e. rgb(32, 23, 20)) the theme's
          tokens measure: `--danger` 2.29:1, `--warn` 4.35:1, `--text` 13.32:1.
          So the *message* — the one thing that must be read — is `--text`, and
          the icon is `--warn` (above the 3:1 a graphic needs). `--danger` is left
          to the border and wash, which are decoration: the state is carried by
          the words. The first version of this banner used `text-danger` for the
          message and measured 2.29:1, below even the 3:1 threshold. */}
      {selfRoomMuted && (
        <Panel
          padding="sm"
          className="flex shrink-0 items-center justify-between gap-3 border-danger/35 bg-danger/8"
        >
          <span className="flex min-w-0 items-center gap-2 text-13 text-text">
            <IconVolumeOff size={14} className="text-warn" />
            <span className="truncate">
              {selfRoomMutedBy ? `Muted by ${selfRoomMutedBy}` : "Muted"}
            </span>
          </span>
          <Button
            variant="secondary"
            size="sm"
            className="shrink-0"
            onClick={() => onToggleRoomMute(selfUserId, false)}
          >
            Unmute me
          </Button>
        </Panel>
      )}

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
            {/* Session 29d: a tile fades in as it arrives. Opacity only — the
                grid measures these, so the entrance must not touch geometry. */}
            <li className="animate-fade">
              <PeerTile
                name={nickname}
                speaking={self.speaking}
                muted={self.muted}
                deafened={self.deafened}
                isSelf
                host={selfHosts}
              >
                <MicLevelMeter ref={micMeterRef} className="min-w-0 flex-1" />
              </PeerTile>
            </li>

            {peers.map((p) => (
              <li key={p.uuid} className="animate-fade">
                <PeerTile
                  name={p.name}
                  speaking={p.speaking}
                  muted={p.mute}
                  deafened={p.deafen}
                  isBot={p.is_bot}
                  // The badge belongs to the *voice* card, so the bot's own tile
                  // never carries it — even though it shares the id that selects it.
                  host={
                    !p.is_bot && p.user_id !== "" && p.user_id === botHostUserId
                  }
                  music={p.music}
                  musicTitle={p.music_title}
                  volume={peerVolumes[p.uuid] ?? 1}
                  onVolume={(v) => onPeerVolume(p.uuid, v)}
                  onToggleMute={() => onTogglePeerMute(p.uuid)}
                  roomMuted={isRoomMutedId(p.user_id)}
                  // Withheld, not disabled, when the persistent id has not
                  // arrived: there is nothing a click could send yet.
                  onToggleRoomMute={
                    p.user_id
                      ? () =>
                          onToggleRoomMute(p.user_id, !isRoomMutedId(p.user_id))
                      : undefined
                  }
                />
              </li>
            ))}

            {humans.length === 0 && (
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
