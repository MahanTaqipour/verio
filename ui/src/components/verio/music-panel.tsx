/**
 * MusicPanel — spacious 2-column horizontal widescreen Music Studio:
 *   - Left column (lg:col-span-5): Now Playing rounded glass island
 *   - Right column (lg:col-span-7): Playlist Queue rounded glass island
 *
 * Strictly preserves Session 29b/29c playback/bot/force-stop behavior and
 * commit-on-pointer-up seek semantics.
 */

import { Button, IconButton } from "@/components/ui/button";
import { SectionLabel } from "@/components/ui/field";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { EmptyState } from "@/components/verio/empty-state";
import {
  StateChip,
  TONE_DOT,
  type ChipTone,
} from "@/components/verio/state-chip";
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
  bot: MusicBotController;
  deafened: boolean;
  forceStop: boolean;
  botPresent: boolean;
  botAcceptsRemoteStop: boolean;
  inCall: boolean;
  roomMusicTitle: string;
  roomMusicHost: string;
  onSetForceStop: (on: boolean) => void;
  onRequestMusicStop: () => void;
  className?: string;
}

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
  className,
}: MusicPanelProps) {
  const { status, queue, hasTrack } = music;
  const chip = statusChip(music);
  const hosting = bot.state.hosting;
  const durationMs = music.durationMs;
  const seekMax = Math.max(1, Math.round(durationMs));
  const remoteMusic = inCall && !hosting && botPresent;

  const roomLine =
    "Room music" +
    (roomMusicTitle ? `: ${roomMusicTitle}` : "") +
    (roomMusicHost ? ` — hosted by ${roomMusicHost}` : "");

  const commitSeek = (raw: number) => {
    void music.seekTo(raw / seekMax);
  };

  return (
    <section
      aria-label="Music studio"
      className={cn(
        "animate-lift mx-auto my-auto grid w-full max-w-[1080px] grid-cols-1 gap-5 lg:grid-cols-12",
        className,
      )}
    >
      {/* Left Column: Now Playing Glass Card */}
      <Panel
        glass
        padding="none"
        className="flex min-w-0 flex-col justify-between gap-5 rounded-[22px] p-6 lg:col-span-5"
      >
        <div className="flex items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2.5">
            <IconMusic
              size={16}
              className={cn(
                "shrink-0",
                status?.playing || remoteMusic ? "text-accent" : "text-lavender",
              )}
            />
            <h2 className="truncate text-15 font-semibold text-text">
              {status?.active || remoteMusic ? "Now Playing" : "Music Player"}
            </h2>
          </div>

          <div className="flex shrink-0 items-center gap-1.5">
            {hosting && (
              <StateChip
                tone="idle"
                title="You are hosting the music for this room. Stop ends it for everyone."
              >
                Hosting
              </StateChip>
            )}
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
          </div>
        </div>

        {/* Rounded Visualizer / Cover Well */}
        <div className="flex items-center gap-4 rounded-2xl border border-border bg-[var(--bg-input)] p-4">
          <div
            className={cn(
              "grid size-14 shrink-0 place-items-center rounded-2xl border transition-colors",
              status?.playing || remoteMusic
                ? "border-accent/45 bg-accent/14 text-accent shadow-[0_0_20px_rgba(232,255,58,0.16)]"
                : "border-border-2 bg-surface-2/80 text-lavender",
            )}
          >
            <IconMusic size={20} />
          </div>

          {remoteMusic ? (
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span
                className="truncate text-14 font-semibold text-text"
                title={roomLine}
              >
                {roomMusicTitle || "Room music active"}
              </span>
              <span className="truncate text-13 text-lavender">
                {roomMusicHost ? `Hosted by ${roomMusicHost}` : "Hosted by a squadmate"}
              </span>
              <span className="text-11 text-text-faint">
                Adjust loudness using the Music card in the room.
              </span>
            </div>
          ) : (
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              {status?.title ? (
                <>
                  <span
                    className="truncate text-14 font-semibold text-text"
                    title={status.title}
                  >
                    {status.title}
                  </span>
                  <span className="flex min-w-0 items-center gap-1.5 text-11 text-text-dim">
                    <IconFileAudio size={14} className="shrink-0 text-lavender" />
                    <span className="truncate font-mono">
                      {fileNameOf(status.path)}
                    </span>
                  </span>
                </>
              ) : (
                <>
                  <span className="text-14 font-medium text-text">No track loaded</span>
                  <span className="text-11 text-text-dim">
                    Add audio files on the right to start playing.
                  </span>
                </>
              )}
            </div>
          )}
        </div>

        {/* Seek Bar */}
        <div className="flex flex-col gap-1.5">
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
                if (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End") {
                  commitSeek(Number(e.currentTarget.value));
                }
              }}
            />
          </div>
          <div className="flex items-center justify-between font-mono text-11 mono-num text-text-dim">
            <span>{formatTime(music.positionMs)}</span>
            <span>{formatTime(durationMs)}</span>
          </div>
        </div>

        {/* Rounded Transport Row */}
        <div className="flex items-center justify-center gap-2">
          <IconButton
            label="Previous track"
            size="md"
            onClick={() => void music.prev()}
            disabled={remoteMusic || queue.length === 0}
          >
            <IconSkipBack size={16} />
          </IconButton>
          <IconButton
            label={status?.playing ? "Pause track" : "Play track"}
            variant={hasTrack && !remoteMusic ? "primary" : "secondary"}
            size="lg"
            className="size-12 rounded-2xl"
            onClick={() => void bot.playPause()}
            disabled={remoteMusic || !hasTrack}
          >
            {status?.playing ? <IconPause size={20} /> : <IconPlay size={20} />}
          </IconButton>
          <IconButton
            label="Stop playback"
            size="md"
            onClick={() => void bot.stop()}
            disabled={remoteMusic || !hasTrack}
          >
            <IconStop size={16} />
          </IconButton>
          <IconButton
            label="Next track"
            size="md"
            onClick={() => void music.next()}
            disabled={remoteMusic || queue.length === 0}
          >
            <IconSkipForward size={16} />
          </IconButton>
          <IconButton
            label="Repeat current track"
            size="md"
            aria-pressed={Boolean(status?.loop)}
            className={cn(status?.loop && "bg-accent/15 text-accent")}
            disabled={remoteMusic}
            onClick={() => void music.setTrackLoop(!status?.loop)}
          >
            <IconRepeat size={16} />
          </IconButton>
        </div>

        {/* Volume */}
        <div className="flex items-center gap-3 rounded-xl border border-border bg-[var(--bg-input)] px-3.5 py-2.5">
          <IconVolume size={14} className="shrink-0 text-lavender" />
          <span className="text-11 font-medium text-text-dim">Volume</span>
          <Slider
            className="min-w-0 flex-1"
            min={0}
            max={200}
            value={Math.round(music.volumeDraft * 100)}
            fillPercent={(music.volumeDraft / 2) * 100}
            aria-label="Music volume"
            valueText={`${Math.round(music.volumeDraft * 100)}%`}
            onChange={(e) => music.setVolume(Number(e.currentTarget.value) / 200)}
          />
          <span className="mono-num w-10 shrink-0 text-right font-mono text-11 text-text-dim">
            {Math.round(music.volumeDraft * 100)}%
          </span>
        </div>

        {/* Switches + Remote Stop */}
        <div className="flex flex-wrap items-center justify-between gap-3 pt-1">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <span className="flex items-center gap-2">
              <Switch
                label="Local monitor playback"
                checked={Boolean(status?.monitor)}
                onChange={(e) => void music.setMonitor(e.currentTarget.checked)}
              />
              <span className="text-13 text-text-dim">Local monitor</span>
            </span>

            <span className="flex items-center gap-2">
              <Switch
                label="Force Stop"
                checked={forceStop}
                onChange={(e) => onSetForceStop(e.currentTarget.checked)}
              />
              <span
                className="text-13 text-text-dim"
                title="Let anyone in this room stop the music you are hosting. Applied the next time you start hosting."
              >
                Force Stop
              </span>
            </span>
          </div>

          {deafened && status?.active && (
            <span
              className="flex items-center gap-1.5 text-11 text-warn"
              title="Your client is deafened; local monitor is silenced while the room still hears music."
            >
              <IconHeadphones size={14} className="shrink-0" />
              Silenced locally
            </span>
          )}

          {inCall && !hosting && botPresent && (
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
          )}
        </div>
      </Panel>

      {/* Right Column: Playlist Queue Glass Card */}
      <Panel
        glass
        padding="none"
        className="flex min-h-[380px] max-h-[540px] min-w-0 flex-col gap-4 rounded-[22px] p-6 lg:col-span-7"
      >
        <div className="flex shrink-0 flex-wrap items-center justify-between gap-3">
          <SectionLabel level={3} className="text-lavender">
            Playlist Queue{queue.length > 0 ? ` · ${queue.length}` : ""}
          </SectionLabel>

          <div className="flex items-center gap-2">
            {queue.length > 0 && (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-pressed={Boolean(status?.queue_looping)}
                  className={cn(status?.queue_looping && "bg-accent/14 text-accent")}
                  onClick={() => void music.setQueueLooping(!status?.queue_looping)}
                >
                  Repeat queue
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void music.clearQueue()}
                >
                  Clear
                </Button>
              </>
            )}
            <Button
              variant="secondary"
              size="sm"
              icon={<IconFolderOpen size={14} />}
              onClick={() => void music.addFiles()}
            >
              Add tracks
            </Button>
          </div>
        </div>

        {queue.length === 0 ? (
          <EmptyState
            title="Queue is empty"
            description="Add MP3, FLAC, WAV, or OGG files to build a playlist for the room."
            icon={<IconMusic size={20} />}
            className="my-auto py-10"
          />
        ) : (
          <ul className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto pr-1">
            {queue.map((item, i) => {
              const isCurrent = status?.queue_index === i;
              return (
                <li
                  key={`${item.path}:${i}`}
                  className={cn(
                    "group flex items-center gap-2 rounded-xl border px-3 py-2 transition-colors",
                    isCurrent
                      ? "border-accent/45 bg-accent/14"
                      : "border-transparent hover:border-border hover:bg-surface-2/60",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => void music.jumpTo(i)}
                    title={`Play ${item.title}`}
                    className="flex min-w-0 flex-1 items-center gap-2.5 text-left"
                  >
                    <span
                      className={cn(
                        "mono-num w-5 shrink-0 text-right font-mono text-11",
                        isCurrent ? "text-accent" : "text-text-faint",
                      )}
                    >
                      {isCurrent ? "▶" : i + 1}
                    </span>
                    <span
                      className={cn(
                        "truncate text-13",
                        isCurrent ? "font-semibold text-accent" : "text-text",
                      )}
                    >
                      {item.title}
                    </span>
                    <span className="mono-num ml-auto shrink-0 font-mono text-11 text-text-faint">
                      {formatTime(item.duration_ms)}
                    </span>
                  </button>
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
      </Panel>
    </section>
  );
}
