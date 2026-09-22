/**
 * Settings — a full-screen, two-pane surface: a nav rail on the left, one
 * section's content on the right.
 *
 * PRESENTATION ONLY. Every value, every command, and the hotkey-capture keyboard
 * handling live in App.tsx and arrive here as props. That is deliberate: the
 * capture logic is the single highest-risk behaviour in the app (see below), and
 * leaving it exactly where it is means this rebuild cannot change it.
 *
 * NOT A ROUTE. It still opens over both the lobby and the room, and still closes
 * back to wherever you were. The legacy version was a centred 560px modal on a
 * translucent scrim; `REDESIGN-PLAN.md` R6 decided the design's full-screen
 * surface is what to build, so the scrim and the 560px cap are gone and the
 * surface is opaque. Escape closes it (the legacy modal had no keyboard exit at
 * all — with no scrim left to click, a close button alone would strand
 * keyboard-only users).
 *
 * Escape while CAPTURING A HOTKEY does not close this screen. The capture
 * handler in App.tsx listens on `window` in the *capture* phase and calls
 * `stopPropagation()`, so it consumes the key before this component's
 * bubble-phase listener ever sees it. That ordering is load-bearing, not luck.
 *
 * SECTIONS ARE ONLY BUILT WHERE THERE IS REAL CONTENT
 * --------------------------------------------------
 * Six of them: transmission, shortcuts, audio, quality, server, diagnostics.
 *
 * The legacy modal also had a "Voice Transport" section whose entire body was a
 * static row reading *"Encrypted relay — Every call runs through the Verio relay
 * server. Nothing to configure."* plus an "Active" badge. A settings screen row
 * that says there is nothing to configure is not content, so it is not carried
 * over — and it would have contradicted the header decision in the same session,
 * where the same constant value was removed from the status chip for the same
 * reason. The live transport is still described in the diagnostics drawer.
 *
 * `debug_wavs` is the opposite case: the command (`set_debug_wavs`) and the
 * persisted field both exist and it has been unreachable from the UI. It gets a
 * real row under Diagnostics (`REDESIGN-PLAN.md` B4).
 *
 * DELIBERATELY NOT HERE: signalling port, host port, transport mode. Session 11
 * removed the transport plumbing from the UI on purpose; the plan's §6 lists
 * those commands as live-but-intentionally-unexposed, so re-surfacing them would
 * undo a decision rather than fix an omission. The VPS address stays because it
 * is already user-facing today and works.
 */

import { useEffect, useState, type ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SectionLabel, SettingsRow } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { SegmentedControl } from "@/components/ui/segmented";
import { Select } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import {
  IconActivity,
  IconGauge,
  IconKeyboard,
  IconMic,
  IconServer,
  IconSliders,
  IconTerminal,
  IconTrash,
  IconX,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";

/** The three bindable actions. Mirrors App.tsx's union structurally. */
export type HotkeyAction = "ptt" | "mute" | "deafen";

export interface DeviceOption {
  name: string;
  is_default: boolean;
}

/** Only the fields this screen reads. The caller's own `Settings` is wider and
 *  therefore assignable without a shared import. */
export interface SettingsValues {
  input_device: string | null;
  output_device: string | null;
  input_gain_db: number;
  noise_suppression: boolean;
  loopback: boolean;
  debug_wavs: boolean;
  voice_bitrate_kbps: number;
  music_bitrate_kbps: number;
  hotkeys: { ptt: string; mute: string; deafen: string };
}

export interface SettingsProps {
  settings: SettingsValues | null;
  devices: { inputs: DeviceOption[]; outputs: DeviceOption[] };
  /** Which action is currently awaiting a keypress, or null. */
  capturingAction: HotkeyAction | null;
  /** Controlled value for the VPS address field. */
  vpsAddress: string;
  onVpsAddressChange: (next: string) => void;
  onSaveVpsAddress: () => void;
  onClose: () => void;
  onSelectVoiceActivity: () => void;
  onSelectPushToTalk: () => void;
  onBeginCapture: (action: HotkeyAction) => void;
  onClearHotkey: (action: HotkeyAction) => void;
  onSetInputDevice: (name: string | null) => void;
  onSetOutputDevice: (name: string | null) => void;
  onSetInputGain: (db: number) => void;
  onSetNoiseSuppression: (on: boolean) => void;
  onSetLoopback: (on: boolean) => void;
  onSetDebugWavs: (on: boolean) => void;
  onSetVoiceQuality: (kbps: number) => void;
  onSetMusicQuality: (kbps: number) => void;
}

const SECTIONS = [
  { id: "transmission", label: "Transmission", icon: IconActivity },
  { id: "shortcuts", label: "Shortcuts", icon: IconKeyboard },
  { id: "audio", label: "Audio", icon: IconSliders },
  { id: "quality", label: "Quality", icon: IconGauge },
  { id: "server", label: "Server", icon: IconServer },
  { id: "diagnostics", label: "Diagnostics", icon: IconTerminal },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const VOICE_RATES = [24, 32, 48, 64, 96] as const;
const MUSIC_RATES = [64, 96, 128, 160, 192] as const;

/**
 * The keybind control.
 *
 * A button, not a `<kbd>`: `KbdChip` is a read-only display (the dock uses it to
 * advertise a binding), whereas this is the thing you press to *change* one. The
 * legacy markup used a bare `<button class="keybind-chip">` with no accessible
 * name, so a screen reader announced only the current combo and never that it
 * was editable.
 */
function KeybindButton({
  combo,
  placeholder,
  capturing,
  label,
  onBegin,
}: {
  combo: string;
  placeholder: string;
  capturing: boolean;
  label: string;
  onBegin: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onBegin}
      aria-label={`${label} hotkey: ${combo || "not set"}. Activate to record a new key.`}
      className={cn(
        "inline-flex h-8 min-w-28 items-center justify-center rounded-sm border px-3",
        "font-mono text-11 tracking-normal transition-colors ease-verio",
        capturing
          ? "border-accent bg-accent/15 text-accent"
          : "border-border bg-bg text-text hover:border-border-2 hover:bg-surface-2",
      )}
    >
      {capturing ? "Press a key…" : combo || placeholder}
    </button>
  );
}

/** A row's trailing control cluster, so every row's right edge lines up. */
function Control({ children }: { children: ReactNode }) {
  return <div className="flex shrink-0 items-center gap-2">{children}</div>;
}

export function Settings({
  settings,
  devices,
  capturingAction,
  vpsAddress,
  onVpsAddressChange,
  onSaveVpsAddress,
  onClose,
  onSelectVoiceActivity,
  onSelectPushToTalk,
  onBeginCapture,
  onClearHotkey,
  onSetInputDevice,
  onSetOutputDevice,
  onSetInputGain,
  onSetNoiseSuppression,
  onSetLoopback,
  onSetDebugWavs,
  onSetVoiceQuality,
  onSetMusicQuality,
}: SettingsProps) {
  const [section, setSection] = useState<SectionId>("transmission");

  // Escape closes — but never while a key is being captured. The capture
  // handler is registered on `window` with `capture: true` and calls
  // `stopPropagation()`, so it runs first and this listener never fires.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const pttCombo = settings?.hotkeys.ptt ?? "";
  const isPttMode = pttCombo.trim().length > 0;
  const active = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0];

  const body = () => {
    switch (section) {
      case "transmission":
        return (
          <>
            <SettingsRow label="Mode" hint="How your microphone is keyed">
              <SegmentedControl
                legend="Transmission mode"
                value={isPttMode ? "ptt" : "voice"}
                onChange={(v) => (v === "ptt" ? onSelectPushToTalk() : onSelectVoiceActivity())}
                options={[
                  { value: "voice", label: "Voice activity", icon: <IconMic size={14} /> },
                  { value: "ptt", label: "Push to talk", icon: <IconKeyboard size={14} /> },
                ]}
              />
            </SettingsRow>

            {isPttMode && (
              <SettingsRow label="Push-to-talk key" hint="Hold this key to transmit">
                <Control>
                  <KeybindButton
                    combo={pttCombo}
                    placeholder="Not set"
                    capturing={capturingAction === "ptt"}
                    label="Push to talk"
                    onBegin={() => onBeginCapture("ptt")}
                  />
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={onSelectVoiceActivity}
                    title="Clear the key and switch to voice activity"
                    aria-label="Clear push-to-talk key and switch to voice activity"
                  >
                    <IconTrash size={14} />
                  </Button>
                </Control>
              </SettingsRow>
            )}
          </>
        );

      case "shortcuts":
        return (
          <>
            <SettingsRow label="Toggle mute" hint="Mutes and unmutes your microphone">
              <Control>
                <KeybindButton
                  combo={settings?.hotkeys.mute ?? ""}
                  placeholder="None"
                  capturing={capturingAction === "mute"}
                  label="Toggle mute"
                  onBegin={() => onBeginCapture("mute")}
                />
                {settings?.hotkeys.mute && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onClearHotkey("mute")}
                    aria-label="Clear the toggle-mute hotkey"
                  >
                    <IconTrash size={14} />
                  </Button>
                )}
              </Control>
            </SettingsRow>

            <SettingsRow label="Toggle deafen" hint="Silences incoming audio and your microphone">
              <Control>
                <KeybindButton
                  combo={settings?.hotkeys.deafen ?? ""}
                  placeholder="None"
                  capturing={capturingAction === "deafen"}
                  label="Toggle deafen"
                  onBegin={() => onBeginCapture("deafen")}
                />
                {settings?.hotkeys.deafen && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => onClearHotkey("deafen")}
                    aria-label="Clear the toggle-deafen hotkey"
                  >
                    <IconTrash size={14} />
                  </Button>
                )}
              </Control>
            </SettingsRow>
          </>
        );

      case "audio":
        return (
          <>
            <SettingsRow label="Input device" hint="Where your voice is captured">
              {/* Width lives on a wrapper, not on the Select. `Select`'s own
                  wrapper is `w-full` unless `auto` is set, and `auto` sizes it to
                  the widest *option* — which would leave the chevron (positioned
                  against the wrapper) detached from the control's right edge
                  whenever a device name is longer than the width you asked for.
                  `flex-1` + `max-w-80` uses the row's spare space and stops the
                  native control clipping real device names, which a fixed 240px
                  did: "Shure SM7B (Elgato Wave XLR)" rendered as
                  "Shure SM7B (Elgato Wave XL". */}
              <div className="min-w-0 max-w-80 flex-1">
                <Select
                  aria-label="Input device"
                  value={settings?.input_device ?? ""}
                  onChange={(e) => onSetInputDevice(e.target.value || null)}
                >
                  <option value="">System default</option>
                  {devices.inputs.map((d) => (
                    <option key={d.name} value={d.name}>
                      {d.name}
                      {d.is_default ? " (default)" : ""}
                    </option>
                  ))}
                </Select>
              </div>
            </SettingsRow>

            <SettingsRow label="Output device" hint="Where incoming audio is played">
              <div className="min-w-0 max-w-80 flex-1">
                <Select
                  aria-label="Output device"
                  value={settings?.output_device ?? ""}
                  onChange={(e) => onSetOutputDevice(e.target.value || null)}
                >
                  <option value="">System default</option>
                  {devices.outputs.map((d) => (
                    <option key={d.name} value={d.name}>
                      {d.name}
                      {d.is_default ? " (default)" : ""}
                    </option>
                  ))}
                </Select>
              </div>
            </SettingsRow>

            <SettingsRow
              label="Input gain"
              hint="Pre-amp applied before noise suppression"
            >
              <Control>
                <Slider
                  min={0}
                  max={30}
                  step={1}
                  value={settings?.input_gain_db ?? 0}
                  fillPercent={((settings?.input_gain_db ?? 0) / 30) * 100}
                  valueText={`${Math.round(settings?.input_gain_db ?? 0)} decibels`}
                  onChange={(e) => onSetInputGain(parseFloat(e.target.value))}
                  aria-label="Input gain in decibels"
                  className="w-32"
                />
                <span className="mono-num w-12 shrink-0 text-right text-13 text-text-dim">
                  +{Math.round(settings?.input_gain_db ?? 0)} dB
                </span>
              </Control>
            </SettingsRow>

            <SettingsRow label="Noise suppression" hint="RNNoise background-noise filter">
              <Switch
                checked={settings?.noise_suppression ?? false}
                onChange={(e) => onSetNoiseSuppression(e.target.checked)}
                label="Noise suppression"
              />
            </SettingsRow>

            <SettingsRow label="Loopback test" hint="Hear your own microphone locally">
              <Switch
                checked={settings?.loopback ?? false}
                onChange={(e) => onSetLoopback(e.target.checked)}
                label="Loopback test"
              />
            </SettingsRow>
          </>
        );

      case "quality":
        return (
          <>
            <SettingsRow label="Voice bitrate" hint="Encoder rate used for speech">
              <Select
                auto
                aria-label="Voice encoder bitrate"
                value={String(settings?.voice_bitrate_kbps ?? 32)}
                onChange={(e) => onSetVoiceQuality(parseInt(e.target.value, 10))}
              >
                {VOICE_RATES.map((k) => (
                  <option key={k} value={k}>
                    {k} kbps{k === 32 ? " (default)" : ""}
                  </option>
                ))}
              </Select>
            </SettingsRow>

            <SettingsRow
              label="Music bitrate"
              hint="Used instead of the voice rate while music plays"
            >
              <Select
                auto
                aria-label="Music encoder bitrate"
                value={String(settings?.music_bitrate_kbps ?? 96)}
                onChange={(e) => onSetMusicQuality(parseInt(e.target.value, 10))}
              >
                {MUSIC_RATES.map((k) => (
                  <option key={k} value={k}>
                    {k} kbps{k === 96 ? " (default)" : ""}
                  </option>
                ))}
              </Select>
            </SettingsRow>
          </>
        );

      case "server":
        return (
          <SettingsRow
            label="Signalling server"
            hint="WebSocket host every call is negotiated through"
          >
            {/* Not <Control>: this cluster grows into the row's spare space
                rather than sitting at its natural width, so it cannot carry the
                `shrink-0` that Control hardcodes — `shrink-0` and `flex-1` set
                `flex-shrink` to opposite values and which one wins would depend
                on Tailwind's emission order rather than on anything here. */}
            <div className="flex min-w-0 max-w-80 flex-1 items-center gap-2">
              <Input
                value={vpsAddress}
                onChange={(e) => onVpsAddressChange(e.target.value)}
                aria-label="Signalling server address"
                spellCheck={false}
                /* The width goes on the wrapper: `className` styles the inner
                   <input>, which is `flex-1` inside a wrapper that is itself
                   `w-full` — a width there would be ignored. */
                wrapperClassName="min-w-0 flex-1"
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={onSaveVpsAddress}
                disabled={!vpsAddress.trim()}
                className="shrink-0"
              >
                Save
              </Button>
            </div>
          </SettingsRow>
        );

      case "diagnostics":
        return (
          <SettingsRow
            label="Capture debug WAVs"
            hint="Writes each pipeline stage to disk for offline inspection"
          >
            <Switch
              checked={settings?.debug_wavs ?? false}
              onChange={(e) => onSetDebugWavs(e.target.checked)}
              label="Capture debug WAVs"
            />
          </SettingsRow>
        );
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      className="fixed inset-0 z-100 flex bg-bg"
    >
      {/* Nav rail */}
      <nav
        aria-label="Settings sections"
        className="flex w-44 shrink-0 flex-col gap-1 border-r border-border p-3"
      >
        <SectionLabel className="px-2 pb-1">System config</SectionLabel>
        {SECTIONS.map((s) => {
          const Icon = s.icon;
          const selected = s.id === section;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => setSection(s.id)}
              aria-current={selected ? "true" : undefined}
              className={cn(
                "flex items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-13",
                "transition-colors ease-verio",
                selected
                  ? "bg-surface-3 text-text"
                  : "text-text-dim hover:bg-surface-2 hover:text-text",
              )}
            >
              <Icon size={14} className="shrink-0" />
              <span className="truncate">{s.label}</span>
            </button>
          );
        })}

        {/* The current keying mode, stated in the rail so it is visible from
            every section — it changes what the mic button in the dock does. */}
        <div className="mt-auto px-2 pt-3">
          <Badge tone={isPttMode ? "accent" : "idle"}>
            {isPttMode ? "Push to talk" : "Open mic"}
          </Badge>
        </div>
      </nav>

      {/* Content */}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border px-5">
          <h2 className="text-15 font-semibold text-text">{active.label}</h2>
          <Button
            variant="ghost"
            size="sm"
            onClick={onClose}
            aria-label="Close settings"
            title="Close settings (Esc)"
          >
            <IconX size={14} />
          </Button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          <div className="mx-auto flex w-full max-w-160 flex-col gap-5">{body()}</div>
        </div>
      </div>
    </div>
  );
}
