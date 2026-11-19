/**
 * Lobby — clean centered floating glass launchpad with #1111 instant join,
 * private room creation, and 4-digit code entry.
 */

import { Button } from "@/components/ui/button";
import { Panel } from "@/components/ui/surface";
import { CoreSpinLoader } from "@/components/verio/core-spin-loader";
import { IconZap } from "@/components/verio/icons";
import { RoomCodeInput } from "@/components/verio/room-code-input";
import { cn } from "@/lib/utils";

export interface LobbyProps {
  code: string;
  onCodeChange: (next: string) => void;
  onCreateRoom: () => void;
  onJoinRoom: () => void;
  onJoinMainRoom: () => void;
  busy?: boolean;
  busyLabel?: string;
  className?: string;
}

const CODE_LENGTH = 4;

export function Lobby({
  code,
  onCodeChange,
  onCreateRoom,
  onJoinRoom,
  onJoinMainRoom,
  busy = false,
  busyLabel,
  className,
}: LobbyProps) {
  const canJoin = code.trim().length === CODE_LENGTH && !busy;

  return (
    <div
      className={cn(
        "flex min-h-full w-full flex-col justify-center px-6 py-6",
        className,
      )}
    >
      <div className="mx-auto my-auto flex w-full max-w-[980px] flex-col gap-6">
        {/* Clean Minimal Welcome Header */}
        <header className="flex flex-col items-center gap-2 text-center">
          <h1 className="text-32 font-semibold tracking-tight text-text">
            Ready to talk
          </h1>
          <p className="text-14 text-text-dim">
            Jump into Main Room #1111 or start a private room.
          </p>
        </header>

        {/* Orbital spinner banner only while dialing a room */}
        {busy && (
          <Panel glass className="rounded-[22px] py-5">
            <CoreSpinLoader statusText={busyLabel} />
          </Panel>
        )}

        {/* 3-Column Soft Rounded Glass Gateway Grid */}
        <div className="grid grid-cols-1 gap-5 md:grid-cols-3">
          {/* Card 1: Main Room #1111 */}
          <Panel
            glass
            padding="none"
            className="relative flex min-w-0 flex-col gap-4 rounded-[22px] border-border-2 p-6 shadow-[0_0_28px_rgba(232,255,58,0.08)]"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="label-caps text-accent">Main Squad</span>
              <span className="rounded-full border border-accent/45 bg-accent/14 px-2.5 py-0.5 font-mono text-11 font-semibold tabular-nums text-accent">
                #1111
              </span>
            </div>
            <div className="flex flex-col gap-1.5">
              <h2 className="text-18 font-semibold text-text">
                Main Room #1111
              </h2>
              <p className="text-13 leading-relaxed text-text-dim">
                Drop straight into the persistent squad channel with a single click.
              </p>
            </div>
            <Button
              variant="primary"
              size="lg"
              className="mt-auto h-11 w-full rounded-xl font-bold"
              onClick={onJoinMainRoom}
              disabled={busy}
            >
              <IconZap size={16} />
              Join #1111
            </Button>
          </Panel>

          {/* Card 2: New Private Room */}
          <Panel
            glass
            padding="none"
            className="flex min-w-0 flex-col gap-4 rounded-[22px] p-6"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="label-caps text-lavender">Private Call</span>
              <span className="rounded-full border border-border bg-[var(--bg-input)] px-2.5 py-0.5 font-mono text-10 uppercase tracking-wider text-text-dim">
                Host
              </span>
            </div>
            <div className="flex flex-col gap-1.5">
              <h2 className="text-18 font-semibold text-text">
                New Private Room
              </h2>
              <p className="text-13 leading-relaxed text-text-dim">
                Generate a fresh 4-digit code to invite friends for a private session.
              </p>
            </div>
            <Button
              variant="secondary"
              size="lg"
              className="mt-auto h-11 w-full rounded-xl"
              onClick={onCreateRoom}
              disabled={busy}
            >
              Create Room
            </Button>
          </Panel>

          {/* Card 3: Join with Code */}
          <Panel
            glass
            padding="none"
            className="flex min-w-0 flex-col gap-4 rounded-[22px] p-6"
            onKeyDown={(e) => {
              if (e.key === "Enter" && canJoin) {
                e.preventDefault();
                onJoinRoom();
              }
            }}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="label-caps text-lavender">Direct Entry</span>
              <span className="rounded-full border border-border bg-[var(--bg-input)] px-2.5 py-0.5 font-mono text-10 uppercase tracking-wider text-text-dim">
                4-Digit
              </span>
            </div>
            <div className="flex flex-col gap-1.5">
              <h2 className="text-18 font-semibold text-text">
                Join with Code
              </h2>
              <p className="text-13 leading-relaxed text-text-dim">
                Type or paste the 4-digit room code shared by your host.
              </p>
            </div>
            <div className="flex justify-center py-1">
              <RoomCodeInput
                value={code}
                onChange={onCodeChange}
                length={CODE_LENGTH}
              />
            </div>
            <Button
              variant={canJoin ? "primary" : "secondary"}
              size="lg"
              className="mt-auto h-11 w-full rounded-xl"
              onClick={onJoinRoom}
              disabled={!canJoin}
            >
              Join Room
            </Button>
          </Panel>
        </div>
      </div>
    </div>
  );
}
