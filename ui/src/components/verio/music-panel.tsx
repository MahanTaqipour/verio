/**
 * MusicPanel — the music player surface.
 *
 * Presentation only. All state and command calls live in `useMusic`, so this file
 * can be replaced without touching behaviour.
 *
 * Two behaviour details carried over deliberately from the pre-redesign version:
 *
 *   - **The seek bar drafts, then commits.** `onChange` only moves the handle;
 *     the seek is sent on pointer-up or on a keyboard commit. Seeking on every
 *     `change` during a drag would issue a decoder seek per pixel.
 *   - **The filename is shown under the title.** The old markup put this in a
 *     class called `track-artist`, which was a misnomer — the backend reports no
 *     artist, only a path. It is labelled as a file here rather than dressed up
 *     as metadata the app does not have.
 *
 * The queue shows the queue's real shape: `queue_index` from the backend marks
 * the loaded entry. There is no artwork, no artist and no "shuffle" — the backend
 * has none of those, and inventing them would be a fake feature.
 */

import { Button, IconButton } from "@/components/ui/button";
import { SectionLabel } from "@/components/ui/field";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { EmptyState } from "@/components/verio/empty-state";
import { StateChip, TONE_DOT, type ChipTone } from "@/components/verio/state-chip";
import { Panel } from "@/components/ui/surface";
import {
  IconChevronDown,
  IconChevronUp,
  IconFileAudio,
  IconFolderOpen,
  IconHeadphones,
  IconMusic,
  IconPause,
  IconPlay,
  IconRepeat,
  IconSkipBack,
  IconSkipForward,
  IconStop,
  IconTrash,
  IconVolume,
  IconX,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";
import {
  fileNameOf,
  formatTime,
  type MusicController,
} from "@/lib/use-music";
import type { MusicBotController } from "@/lib/use-music-bot";

export interface MusicPanelProps {
  music: MusicController;
  /**
   * Session 29b: the bot's state, and the Play/Stop policy that goes with it.
   *
   * The transport buttons below call *this* rather than `music` directly, because
   * in a room pressing Play is not one action but two in sequence — start the
   * player, then open the bot session that streams it. Outside a room it delegates
   * straight back to `music`, so the panel needs no room awareness of its own.
   */
  bot: MusicBotController;
  /** True when this client is deafened — the local monitor is silenced. */
  deafened: boolean;
  /**
   * Session 29b Force Stop: this user's stored preference.
   *
   * Drives the toggle, and nothing else. It is applied when this user *becomes*
   * host, so it says nothing about the music playing right now — which is why the
   * listener's control below reads a different value.
   */
  forceStop: boolean;
  /** True when a music bot is in the room, whoever owns it. */
  botPresent: boolean;
  /**
   * Session 29b Force Stop: what the bot in this room announced it will accept.
   *
   * The only thing that may decide whether a Stop control is *enabled*. Taken from
   * the wire rather than from `forceStop`, because the host may be somebody else
   * entirely and their decision is the one that applies.
   */
  botAcceptsRemoteStop: boolean;
  /** True while this client is in a room. */
  inCall: boolean;
  /**
   * Session 29c: the track the room's music peer is playing.
   *
   * Read from the bot peer's `music_title`, which the bot announces over the
   * control channel on every track change and every 3 s. Empty until that first
   * announcement lands, and empty when the bot has nothing loaded.
   */
  roomMusicTitle: string;
  /**
   * Session 29c: the person hosting the room's music.
   *
   * Deliberately not the bot's own name — that is always "Music", which says
   * nothing about whose music it is. This is the *voice* peer that shares the
   * bot's persistent id, the same link the Host badge is placed by. Empty while
   * that peer is unknown, which the panel renders as "somebody else's music"
   * rather than guessing a name.
   */
  roomMusicHost: string;
  onSetForceStop: (on: boolean) => void;
  /** Ask the room's host to stop. Whether it is honoured is theirs to decide. */
  onRequestMusicStop: () => void;
  onClose: () => void;
  className?: string;
}

/** Playback state as one chip. `active` already means "loaded and playing". */
function statusChip(music: MusicController): { tone: ChipTone; label: string } {
  if (music.status?.active) return { tone: "accent", label: "Sending to room" };
  if (music.hasTrack) return { tone: "idle", label: "Paused" };
  return { tone: "neutral", label: "Idle" };
}

export function MusicPanel({
  music,
  bot,
  deafened,
  forceStop,
  botPresent,
  botAcceptsRemoteStop,
  inCall,
  roomMusicTitle,
  roomMusicHost,
  onSetForceStop,
  onRequestMusicStop,
  onClose,
  className,
}: MusicPanelProps) {
  const { status, queue, hasTrack } = music;
  const chip = statusChip(music);
  const hosting = bot.state.hosting;
  const durationMs = music.durationMs;
  const seekMax = Math.max(1, Math.round(durationMs));

  /**
   * Session 29c: somebody else is hosting the music in this room.
   *
   * All three conditions are load-bearing. `inCall` because a bot only exists in
   * a room; `!hosting` because when *we* host, this panel is describing our own
   * player and the Hosting chip already says so; `botPresent` because a listener
   * with no bot in the room has nothing to report but their own player.
   */
  const remoteMusic = inCall && !hosting && botPresent;

  /**
   * The line a listener reads instead of "No track loaded".
   *
   * Built only from what the room actually told us, rather than padded with
   * placeholders: with no title yet it reads "Room music", and while the host's
   * voice peer is unknown the clause is dropped rather than filled with a guess.
   * Both are transient — the bot re-announces every 3 s.
   */
  const roomLine =
    "Room music" +
    (roomMusicTitle ? `: ${roomMusicTitle}` : "") +
    (roomMusicHost ? ` — hosted by ${roomMusicHost}` : "");

  /** Send the seek for a raw slider value. No-op when nothing is loaded. */
  const commitSeek = (raw: number) => {
    void music.seekTo(raw / seekMax);
  };

  return (
    // Panel, not a hand-rolled `<section class="rounded-md border border-border
    // bg-surface p-4">`. That string WAS the Panel default, spelled out — the
    // Phase 4 rebuild re-implemented a Phase 3 atom. `role="region"` keeps the
    // labelled landmark that `<section aria-label>` gave implicitly.
    <Panel
      role="region"
      aria-label="Music player"
      // Session 29d: the panel lifts into place instead of snapping open.
      className={cn("animate-lift flex flex-col gap-4", className)}
    >
      <header className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <IconMusic size={16} className="shrink-0 text-text-dim" />
          <h2 className="truncate text-14 font-medium text-text">
            {status?.active ? "Now playing" : "Music"}
          </h2>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {/* Session 29b: hosting is a separate fact from playback state — the bot
              session outlives a pause — so it gets its own chip rather than
              overloading the one that reports what the player is doing. The two
              disagree on purpose when the music is paused. */}
          {hosting && (
            <StateChip
              tone="idle"
              title="You are hosting the music for this room. Stop ends it for everyone."
            >
              Hosting
            </StateChip>
          )}
          {/* Session 29c: omitted while somebody else hosts, and that is the point
              rather than an oversight. This chip reports *this client's* player —
              "Sending to room" / "Paused" / "Idle" — and none of those three is
              true of somebody else's music. Leaving it in place is precisely how
              the panel came to read "Idle" while the room was plainly playing. */}
          {!remoteMusic && (
            <StateChip
              tone={chip.tone}
              leading={
                <span className={cn("size-1.5 rounded-full", TONE_DOT[chip.tone])} />
              }
            >
              {chip.label}
            </StateChip>
          )}
          <IconButton label="Collapse music player panel" size="sm" onClick={onClose}>
            <IconX size={14} />
          </IconButton>
        </div>
      </header>

      {/* Track identity */}
      {remoteMusic ? (
        // Session 29c: the room's music, not ours. The track is the bot's own
        // announcement; the host is the voice peer sharing its persistent id.
        // The second line is not decoration — the volume slider below stays
        // enabled because it is *this client's* player volume, and without this
        // line a reader would take it for the room's music volume, which is the
        // per-peer slider on the Music card.
        <div className="flex min-w-0 flex-col gap-1">
          <span className="truncate text-13 text-text" title={roomLine}>
            {roomLine}
          </span>
          <span className="text-11 text-text-faint">
            Volume and mute for this music are on the Music card in the room.
          </span>
        </div>
      ) : (
        <div className="flex min-w-0 flex-col gap-1">
          {status?.title ? (
            <>
              <span className="truncate text-13 text-text">{status.title}</span>
              <span className="flex min-w-0 items-center gap-1.5 text-11 text-text-faint">
                <IconFileAudio size={14} className="shrink-0" />
                <span className="truncate">{fileNameOf(status.path)}</span>
              </span>
            </>
          ) : (
            <span className="text-13 text-text-faint">No track loaded</span>
          )}
        </div>
      )}

      {/* Seek */}
      <div className="flex items-center gap-3">
        <Slider
          className="min-w-0 flex-1"
          min={0}
          max={seekMax}
          value={Math.round(music.positionMs)}
          fillPercent={music.percent}
          disabled={durationMs <= 0}
          aria-label="Track playback position"
          valueText={`${formatTime(music.positionMs)} of ${formatTime(durationMs)}`}
          onChange={(e) => music.setSeekDraft(Number(e.currentTarget.value))}
          onPointerUp={(e) => commitSeek(Number(e.currentTarget.value))}
          onKeyUp={(e) => {
            // Only commit on keys that actually move the handle; otherwise Tab
            // and modifiers would fire a seek.
            if (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End") {
              commitSeek(Number(e.currentTarget.value));
            }
          }}
        />
        <span className="mono-num shrink-0 text-11 text-text-dim">
          {formatTime(music.positionMs)} / {formatTime(durationMs)}
        </span>
      </div>

      {/* Transport */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        {/* Session 29c: every control in this cluster drives *this client's*
            player, so while somebody else hosts the room's music they are all
            disabled rather than left looking effective. The volume slider on the
            right is deliberately NOT disabled: it is this client's own player
            volume — a stored preference of theirs, like Force Stop — whereas the
            room's loudness is the per-peer slider on the Music card, which the
            line above the transport says. */}
        <div className="flex items-center gap-1">
          <IconButton
            label="Previous track"
            onClick={() => void music.prev()}
            disabled={remoteMusic || queue.length === 0}
          >
            <IconSkipBack size={16} />
          </IconButton>
          <IconButton
            label={status?.playing ? "Pause track" : "Play track"}
            variant="secondary"
            size="lg"
            // Session 29b: not `music.playPause()`. In a room, starting playback is
            // also what opens the bot session — and the room gets to refuse first,
            // so a listener who presses Play is told the music is already playing
            // instead of quietly broadcasting through the pre-29b voice path.
            onClick={() => void bot.playPause()}
            disabled={remoteMusic || !hasTrack}
          >
            {status?.playing ? <IconPause size={16} /> : <IconPlay size={16} />}
          </IconButton>
          <IconButton
            // One stable name, deliberately. The first version of this made the
            // label depend on hosting ("Stop the music for the room"), which is
            // true but broke every harness that drives the control by label —
            // `verify-music.mjs` waited 30s for "Stop playback" and timed out.
            // The scope belongs on the *state*, not on the action: the Hosting
            // chip beside the status already says it, and says who it applies to.
            label="Stop playback"
            // Session 29b: stops the bot session too when this client hosts it. A
            // stopped player with a live bot would be a peer sitting in the room
            // sending nothing.
            onClick={() => void bot.stop()}
            disabled={remoteMusic || !hasTrack}
          >
            <IconStop size={16} />
          </IconButton>
          <IconButton
            label="Next track"
            onClick={() => void music.next()}
            disabled={remoteMusic || queue.length === 0}
          >
            <IconSkipForward size={16} />
          </IconButton>
          <IconButton
            label="Repeat current track"
            aria-pressed={Boolean(status?.loop)}
            className={cn(status?.loop && "text-accent")}
            disabled={remoteMusic}
            onClick={() => void music.setTrackLoop(!status?.loop)}
          >
            <IconRepeat size={16} />
          </IconButton>
        </div>

        <div className="flex min-w-0 items-center gap-2">
          <IconVolume size={14} className="shrink-0 text-text-dim" />
          <Slider
            className="w-24"
            min={0}
            max={200}
            value={Math.round(music.volumeDraft * 100)}
            fillPercent={(music.volumeDraft / 2) * 100}
            aria-label="Music volume"
            valueText={`${Math.round(music.volumeDraft * 100)}%`}
            onChange={(e) => music.setVolume(Number(e.currentTarget.value) / 200)}
          />
          <span className="mono-num w-8 shrink-0 text-right text-11 text-text-dim">
            {Math.round(music.volumeDraft * 100)}%
          </span>
        </div>
      </div>

      {/* Monitor + Force Stop + add */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
          <span className="flex min-w-0 items-center gap-2">
            <Switch
              label="Local monitor playback"
              checked={Boolean(status?.monitor)}
              onChange={(e) => void music.setMonitor(e.currentTarget.checked)}
            />
            <span className="text-11 text-text-dim">Local monitor</span>
            {deafened && status?.active ? (
              <span
                className="flex items-center gap-1.5 text-11 text-warn"
                title="Your client is deafened; the music monitor is silenced locally. Music still reaches the room."
              >
                <IconHeadphones size={14} className="shrink-0" />
                Silenced locally
              </span>
            ) : null}
          </span>

          {/* Session 29b Force Stop. A *setting*, not a live control, so it sits
              with the other music preference rather than with the transport — and
              its title says when it takes effect, because a switch that visibly
              changes nothing while music is already playing reads as broken. */}
          <span className="flex min-w-0 items-center gap-2">
            <Switch
              label="Force Stop"
              checked={forceStop}
              onChange={(e) => onSetForceStop(e.currentTarget.checked)}
            />
            <span
              className="text-11 text-text-dim"
              title="Let anyone in this room stop the music you are hosting. Applied the next time you start hosting, not to music already playing."
            >
              Force Stop
            </span>
          </span>
        </div>

        <div className="flex items-center gap-2">
          {/* Session 29b Force Stop: ending somebody else's music. Shown whenever a
              bot is in the room and it is not ours, and *disabled* rather than
              hidden when the host forbade it — a control that disappears leaves a
              listener unsure the feature exists, and the tooltip is where the host's
              decision gets explained. The tooltip lives on the wrapper, because a
              disabled Button sets `pointer-events-none` and would never show one. */}
          {inCall && !hosting && botPresent ? (
            <span
              title={
                botAcceptsRemoteStop
                  ? "Stop the music for everyone in this room."
                  : "Host has disabled force-stop."
              }
            >
              <Button
                variant="secondary"
                size="sm"
                icon={<IconStop size={14} />}
                disabled={!botAcceptsRemoteStop}
                onClick={onRequestMusicStop}
              >
                Stop for everyone
              </Button>
            </span>
          ) : null}
          <Button
            variant="secondary"
            size="sm"
            icon={<IconFolderOpen size={14} />}
            onClick={() => void music.addFiles()}
          >
            Add files
          </Button>
        </div>
      </div>

      <div className="h-px w-full bg-border" />

      {/* Queue */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <SectionLabel level={4} className="text-text-dim">
            Queue{queue.length > 0 ? ` · ${queue.length}` : ""}
          </SectionLabel>
          {queue.length > 0 ? (
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                aria-pressed={Boolean(status?.queue_looping)}
                className={cn(status?.queue_looping && "text-accent")}
                onClick={() => void music.setQueueLooping(!status?.queue_looping)}
              >
                Repeat queue
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void music.clearQueue()}>
                Clear
              </Button>
            </div>
          ) : null}
        </div>

        {queue.length === 0 ? (
          <EmptyState
            title="Queue is empty"
            description="Add audio files to build a playlist. Tracks play in order and advance on their own."
            className="px-2 py-6"
          />
        ) : (
          <ul className="flex max-h-40 flex-col overflow-y-auto">
            {queue.map((item, i) => {
              const isCurrent = status?.queue_index === i;
              return (
                <li
                  key={`${item.path}:${i}`}
                  className={cn(
                    "group flex items-center gap-2 rounded-sm px-2 py-1.5",
                    isCurrent && "bg-surface-2",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => void music.jumpTo(i)}
                    title={`Play ${item.title}`}
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                  >
                    <span
                      className={cn(
                        "mono-num w-5 shrink-0 text-right text-10",
                        isCurrent ? "text-accent" : "text-text-faint",
                      )}
                    >
                      {isCurrent ? "▶" : i + 1}
                    </span>
                    <span
                      className={cn(
                        "truncate text-13",
                        isCurrent ? "text-accent" : "text-text",
                      )}
                    >
                      {item.title}
                    </span>
                    <span className="mono-num ml-auto shrink-0 text-10 text-text-faint">
                      {formatTime(item.duration_ms)}
                    </span>
                  </button>
                  {/* Session 29c: the queue can be reordered. `music_queue_move`
                      has existed since Session 20 and `moveItem` has been on the
                      controller the whole time, but nothing ever called it — so
                      the order was fixed the moment files were added. Up/down
                      rather than drag-and-drop: the rows live in a 160px
                      scroller, and a button is reachable by keyboard, which a
                      drag handle is not. */}
                  <IconButton
                    label={`Move ${item.title} up`}
                    size="sm"
                    disabled={i === 0}
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={() => void music.moveItem(i, i - 1)}
                  >
                    <IconChevronUp size={14} />
                  </IconButton>
                  <IconButton
                    label={`Move ${item.title} down`}
                    size="sm"
                    disabled={i === queue.length - 1}
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={() => void music.moveItem(i, i + 1)}
                  >
                    <IconChevronDown size={14} />
                  </IconButton>
                  <IconButton
                    label={`Remove ${item.title} from queue`}
                    size="sm"
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={() => void music.removeAt(i)}
                  >
                    <IconTrash size={14} />
                  </IconButton>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </Panel>
  );
}
