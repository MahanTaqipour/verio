/**
 * Room — clean floating-island in-call stage + optional full-height Chat sidebar.
 *
 * All duplicate room headers, duplicate Copy/Timer/Chat buttons, duplicate Leave
 * Call buttons, and stacked Split Cockpits are removed:
 *   - Room code, Copy, timer, and Chat toggle live once in AppHeader.
 *   - Leave Call lives once in Dock.
 *   - When music is playing or a Music Bot is active, a sleek 1-line "Now Playing"
 *     glass bar renders above the participant grid; otherwise nothing extra appears.
 *   - Hardware/wire invariants preserved:
 *       IconHeadphoneOff (Deafen), IconEarOff (local mute),
 *       IconVolumeOff (room-wide mute), IconMicOff (mic muted),
 *       step={0.01} (1% steps, 0-200%) peer sliders, imperative MicLevelMeter.
 */

import { useEffect, useState, type ReactNode, type RefObject } from "react";
import { Button, IconButton } from "@/components/ui/button";
import { SectionLabel } from "@/components/ui/field";
import { Slider } from "@/components/ui/slider";
import { Panel } from "@/components/ui/surface";
import { StateChip, type ChipTone } from "@/components/verio/state-chip";
import { ScreenStage } from "@/components/verio/screen-stage";
import {
  MicLevelMeter,
  type LevelMeterHandle,
} from "@/components/verio/level-meter";
import {
  IconEarOff,
  IconEye,
  IconHeadphoneOff,
  IconMicOff,
  IconMusic,
  IconPause,
  IconPlay,
  IconScreen,
  IconSkipForward,
  IconStop,
  IconUsers,
  IconVolumeOff,
} from "@/components/verio/icons";
import { ChatPanel, type ChatMessage } from "@/components/verio/chat-panel";
import type { MusicController } from "@/lib/use-music";
import type { MusicBotController } from "@/lib/use-music-bot";
import type {
  ScreenSharerInfo,
  ScreenStreamStats,
} from "@/lib/use-screen-share";
import { cn } from "@/lib/utils";

export interface RoomPeer {
  uuid: string;
  user_id: string;
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  music: boolean;
  music_title: string | null;
  is_bot: boolean;
}

export interface RoomProps {
  roomCode: string | null;
  nickname: string;
  self: { speaking: boolean; muted: boolean; deafened: boolean };
  micMeterRef: RefObject<LevelMeterHandle | null>;
  peers: RoomPeer[];
  peerVolumes: Record<string, number>;
  onPeerVolume: (uuid: string, volume: number) => void;
  /** Toggles local mute for one peer: 0 <-> remembered level. */
  onTogglePeerMute: (uuid: string) => void;
  roomMutes: Record<string, string>;
  onToggleRoomMute: (userId: string, muted: boolean) => void;
  selfUserId: string;
  botHostUserId: string;
  /** Full-height chat sidebar */
  chatOpen: boolean;
  onCloseChat: () => void;
  chatMessages?: ChatMessage[];
  onSendMessage?: (text: string) => void;
  /** Now-Playing 1-line bar props (shown only while music/bot is active) */
  music: MusicController;
  bot: MusicBotController;
  botPresent: boolean;
  botAcceptsRemoteStop: boolean;
  roomMusicTitle: string;
  roomMusicHost: string;
  onRequestMusicStop: () => void;
  onOpenMusicTab: () => void;
  /** Session 37B: Watch-on-Demand screen sharing */
  isSharingScreen?: boolean;
  screenWatcherCount?: number;
  localScreenStream?: MediaStream | null;
  screenSharers?: Record<string, ScreenSharerInfo>;
  watchingPeerId?: string | null;
  remoteScreenStream?: MediaStream | null;
  screenStats?: ScreenStreamStats | null;
  onWatchPeer?: (peerId: string) => void;
  onStopWatching?: () => void;
  className?: string;
}

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

function Avatar({
  name,
  speaking,
  isBot,
}: {
  name: string;
  speaking: boolean;
  isBot?: boolean;
}) {
  if (isBot) {
    return (
      <span
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-full border",
          "border-accent/40 bg-accent/14 text-accent",
        )}
      >
        <IconMusic size={16} />
      </span>
    );
  }
  return (
    <span
      className={cn(
        "grid size-10 shrink-0 place-items-center rounded-full border font-mono text-11 font-bold",
        speaking
          ? "border-accent bg-accent text-on-accent"
          : "border-border-2 bg-surface-3/90 text-text",
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
  screenShareLabel,
  isWatching,
  onWatch,
  onStopWatch,
  selfSharing,
  selfWatcherCount,
  previewingLocal,
  onToggleLocalPreview,
  compact,
  children,
}: {
  name: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  isSelf?: boolean;
  isBot?: boolean;
  host?: boolean;
  music?: boolean;
  musicTitle?: string | null;
  volume?: number;
  onVolume?: (next: number) => void;
  onToggleMute?: () => void;
  roomMuted?: boolean;
  onToggleRoomMute?: () => void;
  screenShareLabel?: string;
  isWatching?: boolean;
  onWatch?: () => void;
  onStopWatch?: () => void;
  selfSharing?: boolean;
  selfWatcherCount?: number;
  previewingLocal?: boolean;
  onToggleLocalPreview?: () => void;
  compact?: boolean;
  children?: ReactNode;
}) {
  const locallyMuted = volume === 0;
  const canRoomMute = onToggleRoomMute !== undefined && !isBot;

  return (
    <Panel
      glass
      padding="none"
      className={cn(
        "relative flex h-full flex-col gap-3 rounded-2xl transition-[border-color,box-shadow] ease-verio",
        compact ? "p-3" : "p-4",
        speaking &&
          "border-accent shadow-[0_0_24px_rgba(232,255,58,0.14)]",
      )}
    >
      {/* Active speaker left accent indicator pill */}
      {speaking && (
        <span
          aria-hidden="true"
          className="absolute inset-y-3 left-0.5 w-[3px] rounded-full bg-accent"
        />
      )}

      <div className="flex min-w-0 items-start gap-3">
        <Avatar name={name} speaking={speaking} isBot={isBot} />

        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <span className="truncate text-14 font-semibold text-text" title={name}>
              {name}
            </span>

            {!isSelf && !isBot && screenShareLabel !== undefined && (
              <button
                type="button"
                onClick={isWatching ? onStopWatch : onWatch}
                aria-pressed={isWatching}
                title={
                  isWatching
                    ? `Stop watching ${name}'s screen`
                    : `Watch ${name}'s screen (${screenShareLabel})`
                }
                className={cn(
                  "inline-flex h-7 shrink-0 items-center gap-1 rounded-full px-3 text-11 font-bold transition-colors ease-verio press",
                  isWatching
                    ? "border border-accent bg-accent/18 text-accent hover:bg-accent/25"
                    : "bg-accent text-[#120D1D] hover:bg-accent-hi active:bg-accent-lo",
                )}
              >
                <IconScreen size={14} />
                <span>{isWatching ? "Watching" : "Watch"}</span>
              </button>
            )}

            {isSelf && selfSharing && onToggleLocalPreview && (
              <button
                type="button"
                onClick={onToggleLocalPreview}
                aria-pressed={previewingLocal}
                title={previewingLocal ? "Hide local preview" : "Preview your shared screen"}
                className={cn(
                  "inline-flex h-7 shrink-0 items-center gap-1 rounded-full border px-2.5 text-11 font-semibold transition-colors ease-verio press",
                  previewingLocal
                    ? "border-accent bg-accent/18 text-accent"
                    : "border-border bg-surface-2/85 text-text-dim hover:border-border-2 hover:text-text",
                )}
              >
                <IconEye size={14} />
                <span>{previewingLocal ? "Hide" : "Preview"}</span>
              </button>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-1">
            {isSelf && <Chip tone="neutral">You</Chip>}
            {isSelf && selfSharing && (
              <Chip
                tone="accent"
                title="You are currently advertising a Watch-on-Demand screen share"
              >
                <span className="mono-num">
                  LIVE · {selfWatcherCount ?? 0} watching
                </span>
              </Chip>
            )}
            {host && (
              <Chip tone="idle" title="Hosting the music for this room">
                Host
              </Chip>
            )}
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

      {/* Bottom hardware well: Input meter on self, or local/room controls on remote */}
      {isSelf ? (
        <div className="mt-auto flex items-center gap-2.5 pt-1">
          {children}
          <span className="font-mono text-10 uppercase tracking-wider text-text-faint">
            Input
          </span>
        </div>
      ) : (
        <div className="mt-auto flex items-center gap-2 pt-1">
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
              "mono-num w-9 shrink-0 text-right font-mono text-11",
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
              "grid size-7 shrink-0 place-items-center rounded-xl border border-transparent",
              "transition-colors ease-verio hover:border-border hover:bg-surface-3",
              locallyMuted
                ? "bg-warn/12 text-warn"
                : "text-text-dim hover:text-text",
            )}
          >
            <IconEarOff size={14} />
          </button>
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
                "grid size-7 shrink-0 place-items-center rounded-xl border border-transparent",
                "transition-colors ease-verio hover:border-border hover:bg-surface-3",
                roomMuted
                  ? "bg-danger/15 text-[#fca5a5]"
                  : "text-text-dim hover:text-text",
              )}
            >
              <IconVolumeOff size={14} />
            </button>
          )}
        </div>
      )}
    </Panel>
  );
}

export function Room({
  roomCode,
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
  chatOpen,
  onCloseChat,
  chatMessages = [],
  onSendMessage,
  music,
  bot,
  botPresent,
  botAcceptsRemoteStop,
  roomMusicTitle,
  roomMusicHost,
  onRequestMusicStop,
  onOpenMusicTab,
  isSharingScreen = false,
  screenWatcherCount = 0,
  localScreenStream = null,
  screenSharers = {},
  watchingPeerId = null,
  remoteScreenStream = null,
  screenStats = null,
  onWatchPeer,
  onStopWatching,
  className,
}: RoomProps) {
  const [previewingLocal, setPreviewingLocal] = useState(false);

  // Close local preview when sharing ends or user switches to watching a remote peer.
  useEffect(() => {
    if (!isSharingScreen || watchingPeerId !== null) {
      setPreviewingLocal(false);
    }
  }, [isSharingScreen, watchingPeerId]);

  const isShowingLocalPreview = Boolean(
    isSharingScreen && previewingLocal && watchingPeerId === null,
  );
  const theaterOpen = watchingPeerId !== null || isShowingLocalPreview;
  const watchedPeer = watchingPeerId
    ? peers.find((p) => p.uuid === watchingPeerId)
    : undefined;
  const theaterSharerName = isShowingLocalPreview
    ? nickname || "You"
    : watchedPeer?.name ?? "Peer";

  const humans = peers.filter((p) => !p.is_bot);
  const count = humans.length + 1;
  const selfHosts = botHostUserId !== "" && botHostUserId === selfUserId;

  const isRoomMutedId = (userId: string) =>
    userId !== "" && roomMutes[userId] !== undefined;

  const selfRoomMuted = isRoomMutedId(selfUserId);
  const selfRoomMutedBy = selfUserId ? roomMutes[selfUserId] ?? "" : "";

  const hosting = bot.state.hosting;
  const localActive = Boolean(music.status?.active);
  const showNowPlayingBar = localActive || botPresent;
  const nowPlayingTitle = hosting
    ? music.status?.title || "Playing track"
    : botPresent
      ? roomMusicTitle || music.status?.title || "Room music active"
      : music.status?.title || "Playing track";
  const nowPlayingHostLabel = hosting
    ? `Hosted by ${nickname || "You"}`
    : roomMusicHost
      ? `Hosted by ${roomMusicHost}`
      : "Playing in room";

  const peerCards = (
    <>
      <li
        className={cn(
          "animate-fade min-w-0",
          theaterOpen && "min-w-[230px] max-w-[270px] shrink-0",
        )}
      >
        <PeerTile
          name={nickname}
          speaking={self.speaking}
          muted={self.muted}
          deafened={self.deafened}
          isSelf
          host={selfHosts}
          selfSharing={isSharingScreen}
          selfWatcherCount={screenWatcherCount}
          previewingLocal={isShowingLocalPreview}
          onToggleLocalPreview={() => {
            if (watchingPeerId !== null) {
              onStopWatching?.();
            }
            setPreviewingLocal((v) => !v);
          }}
          compact={theaterOpen}
        >
          <MicLevelMeter ref={micMeterRef} className="min-w-0 flex-1" />
        </PeerTile>
      </li>

      {peers.map((p) => {
        const shareInfo = !p.is_bot ? screenSharers[p.uuid] : undefined;
        const isWatchingThisPeer = watchingPeerId === p.uuid;
        return (
          <li
            key={p.uuid}
            className={cn(
              "animate-fade min-w-0",
              theaterOpen && "min-w-[230px] max-w-[270px] shrink-0",
            )}
          >
            <PeerTile
              name={p.name}
              speaking={p.speaking}
              muted={p.mute}
              deafened={p.deafen}
              isBot={p.is_bot}
              host={!p.is_bot && p.user_id !== "" && p.user_id === botHostUserId}
              music={p.music}
              musicTitle={p.music_title}
              volume={peerVolumes[p.uuid] ?? 1}
              onVolume={(v) => onPeerVolume(p.uuid, v)}
              onToggleMute={() => onTogglePeerMute(p.uuid)}
              roomMuted={isRoomMutedId(p.user_id)}
              onToggleRoomMute={
                p.user_id
                  ? () => onToggleRoomMute(p.user_id, !isRoomMutedId(p.user_id))
                  : undefined
              }
              screenShareLabel={shareInfo?.label}
              isWatching={isWatchingThisPeer}
              onWatch={() => onWatchPeer?.(p.uuid)}
              onStopWatch={() => onStopWatching?.()}
              compact={theaterOpen}
            />
          </li>
        );
      })}
    </>
  );

  return (
    <div
      className={cn(
        "flex min-h-0 flex-1 flex-row gap-2.5 overflow-hidden",
        className,
      )}
    >
      {/* Left / Center Stage Floating Glass Island */}
      <div className="glass-card flex min-w-0 flex-1 flex-col gap-3 overflow-hidden rounded-[22px] p-5">
        {/* Sleek 1-Line Now Playing Bar (only when music is active/bot present) */}
        {showNowPlayingBar && (
          <div className="flex h-12 shrink-0 items-center justify-between gap-4 rounded-2xl border border-border-2 bg-[rgba(185,167,230,0.12)] px-4 backdrop-blur-xl">
            <div className="flex min-w-0 items-center gap-3">
              <span className="grid size-7 shrink-0 place-items-center rounded-xl bg-accent/18 text-accent shadow-[0_0_12px_rgba(232,255,58,0.22)]">
                <IconMusic size={14} />
              </span>
              <span className="truncate text-13 font-semibold text-text">
                {nowPlayingTitle}
              </span>
              <span className="hidden truncate text-11 text-lavender sm:inline">
                {nowPlayingHostLabel}
              </span>
            </div>

            <div className="flex shrink-0 items-center gap-1.5">
              {hosting || !botPresent ? (
                <>
                  <IconButton
                    label={music.status?.playing ? "Pause track" : "Play track"}
                    size="sm"
                    onClick={() => void bot.playPause()}
                    className="text-accent hover:bg-accent/15"
                  >
                    {music.status?.playing ? <IconPause size={14} /> : <IconPlay size={14} />}
                  </IconButton>
                  <IconButton
                    label="Next track"
                    size="sm"
                    disabled={music.queue.length === 0}
                    onClick={() => void music.next()}
                  >
                    <IconSkipForward size={14} />
                  </IconButton>
                </>
              ) : (
                botAcceptsRemoteStop && (
                  <Button
                    variant="secondary"
                    size="sm"
                    icon={<IconStop size={14} />}
                    onClick={onRequestMusicStop}
                  >
                    Stop for everyone
                  </Button>
                )
              )}
              <Button variant="ghost" size="sm" onClick={onOpenMusicTab}>
                Open Music
              </Button>
            </div>
          </div>
        )}

        {/* Room-mute warning banner */}
        {selfRoomMuted && (
          <Panel
            glass
            padding="sm"
            className="flex shrink-0 items-center justify-between gap-3 rounded-2xl border-danger/45 bg-danger/12"
          >
            <span className="flex min-w-0 items-center gap-2 text-13 text-text">
              <IconVolumeOff size={14} className="shrink-0 text-[#fca5a5]" />
              <span className="truncate">
                {selfRoomMutedBy ? `Muted by ${selfRoomMutedBy}` : "Muted for everyone"}
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

        {theaterOpen ? (
          <>
            <ScreenStage
              stream={isShowingLocalPreview ? localScreenStream : remoteScreenStream}
              sharerName={theaterSharerName}
              isLocalPreview={isShowingLocalPreview}
              deafened={self.deafened}
              stats={isShowingLocalPreview ? null : screenStats}
              onStopWatching={() => {
                if (isShowingLocalPreview) {
                  setPreviewingLocal(false);
                } else {
                  onStopWatching?.();
                }
              }}
            />

            {/* Compact Horizontal Participant Strip */}
            <div className="shrink-0 overflow-x-auto pb-1">
              <ul className="flex w-full list-none items-stretch gap-3">
                {peerCards}
              </ul>
            </div>
          </>
        ) : (
          <>
            <div className="flex shrink-0 items-center justify-between">
              <SectionLabel level={2}>
                In call — {count} {count === 1 ? "participant" : "participants"}
              </SectionLabel>
            </div>

            {/* 3-Column Bento Grid */}
            <div className="min-h-0 flex-1 overflow-y-auto pr-1">
              <ul
                className={cn(
                  "grid w-full list-none gap-4 auto-rows-fr",
                  chatOpen
                    ? "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"
                    : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4",
                )}
              >
                {peerCards}

                {humans.length === 0 && (
                  <li className="min-w-0">
                    <Panel
                      padding="none"
                      className="flex h-full flex-col items-start justify-center gap-2 rounded-2xl border-dashed border-border bg-[rgba(36,27,58,0.34)] p-4"
                    >
                      <div className="flex items-center gap-3">
                        <span className="grid size-10 shrink-0 place-items-center rounded-full border border-border bg-[var(--bg-input)] text-lavender">
                          <IconUsers size={14} />
                        </span>
                        <span className="text-13 font-medium text-text-dim">
                          Waiting for others
                        </span>
                      </div>
                      <span className="text-12 text-text-faint">
                        Share room code{" "}
                        <span className="mono-num font-mono font-semibold text-accent">
                          #{roomCode || "----"}
                        </span>{" "}
                        to invite squadmates.
                      </span>
                    </Panel>
                  </li>
                )}
              </ul>
            </div>
          </>
        )}
      </div>

      {/* Right Full-Height Room Chat Island (only when open) */}
      {chatOpen && onSendMessage && (
        <ChatPanel
          messages={chatMessages}
          onSendMessage={onSendMessage}
          onClose={onCloseChat}
          className="w-[340px] shrink-0"
        />
      )}
    </div>
  );
}

export { IconMicOff };
