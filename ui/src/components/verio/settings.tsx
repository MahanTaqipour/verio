/**
 * Settings — floating two-pane glass surface: sections nav on the left,
 * active configuration card on the right. Preserves Escape dismissal,
 * hotkey-capture cancellation, and `role="dialog"` semantics.
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
  capturingAction: HotkeyAction | null;
  onCancelCapture?: () => void;
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
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

const VOICE_RATES = [24, 32, 48, 64, 96] as const;
const MUSIC_RATES = [64, 96, 128, 160, 192] as const;

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
        "inline-flex h-9 min-w-28 items-center justify-center rounded-xl border px-3.5",
        "font-mono text-11 tracking-normal transition-colors ease-verio",
        capturing
          ? "border-accent bg-accent/15 text-accent shadow-[0_0_0_2px_rgba(232,255,58,0.18)]"
          : "border-border bg-[var(--bg-input)] text-text hover:border-border-2 hover:bg-surface-2",
      )}
    >
      {capturing ? "Press a key…" : combo || placeholder}
    </button>
  );
}

function Control({ children }: { children: ReactNode }) {
  return <div className="flex shrink-0 items-center gap-2">{children}</div>;
}

export function Settings({
  settings,
  devices,
  capturingAction,
  onCancelCapture,
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
                onChange={(v) =>
                  v === "ptt" ? onSelectPushToTalk() : onSelectVoiceActivity()
                }
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

            <SettingsRow
              label="Toggle deafen"
              hint="Silences incoming audio and your microphone"
            >
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
                  min={-12}
                  max={12}
                  step={0.5}
                  value={settings?.input_gain_db ?? 0}
                  fillPercent={(((settings?.input_gain_db ?? 0) + 12) / 24) * 100}
                  valueText={`${(settings?.input_gain_db ?? 0) > 0.001 ? "+" : ""}${Math.abs(settings?.input_gain_db ?? 0) < 0.001 ? "0.0" : (settings?.input_gain_db ?? 0).toFixed(1)} decibels`}
                  onChange={(e) => onSetInputGain(parseFloat(e.target.value))}
                  aria-label="Input gain in decibels"
                  className="w-32"
                />
                <span className="mono-num w-16 shrink-0 text-right text-13 text-text-dim">
                  {(settings?.input_gain_db ?? 0) > 0.001 ? "+" : ""}
                  {Math.abs(settings?.input_gain_db ?? 0) < 0.001
                    ? "0.0"
                    : (settings?.input_gain_db ?? 0).toFixed(1)}{" "}
                  dB
                </span>
              </Control>
            </SettingsRow>

            <SettingsRow
              label="Noise suppression"
              hint="RNNoise background-noise filter"
            >
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

            <SettingsRow
              label="Capture debug WAVs"
              hint="Writes audio pipeline stage recordings to disk for offline inspection"
            >
              <Switch
                checked={settings?.debug_wavs ?? false}
                onChange={(e) => onSetDebugWavs(e.target.checked)}
                label="Capture debug WAVs"
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
                value={String(settings?.voice_bitrate_kbps ?? 48)}
                onChange={(e) => onSetVoiceQuality(parseInt(e.target.value, 10))}
              >
                {VOICE_RATES.map((k) => (
                  <option key={k} value={k}>
                    {k} kbps{k === 48 ? " (default)" : ""}
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
                value={String(settings?.music_bitrate_kbps ?? 128)}
                onChange={(e) => onSetMusicQuality(parseInt(e.target.value, 10))}
              >
                {MUSIC_RATES.map((k) => (
                  <option key={k} value={k}>
                    {k} kbps{k === 128 ? " (default)" : ""}
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
            <div className="flex min-w-0 max-w-80 flex-1 items-center gap-2">
              <Input
                value={vpsAddress}
                onChange={(e) => onVpsAddressChange(e.target.value)}
                aria-label="Signalling server address"
                spellCheck={false}
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
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      className="animate-fade flex h-full min-h-0 flex-1 gap-2.5 overflow-hidden"
    >
      {/* Left Settings Nav Floating Island */}
      <nav
        aria-label="Settings sections"
        className="glass-card flex w-52 shrink-0 flex-col gap-1.5 rounded-[22px] p-3.5"
      >
        <SectionLabel className="px-2.5 pb-1 text-lavender">
          Settings
        </SectionLabel>
        {SECTIONS.map((s) => {
          const Icon = s.icon;
          const selected = s.id === section;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => {
                if (capturingAction !== null) {
                  onCancelCapture?.();
                }
                setSection(s.id);
              }}
              aria-current={selected ? "true" : undefined}
              className={cn(
                "relative flex items-center gap-2.5 rounded-xl px-3 py-2 text-left text-13 font-medium",
                "transition-colors ease-verio",
                selected
                  ? "bg-surface-3/80 text-text shadow-[inset_0_0_0_1px_rgba(185,167,230,0.22)]"
                  : "text-text-dim hover:bg-surface-2/60 hover:text-text",
              )}
            >
              {selected && (
                <span
                  aria-hidden="true"
                  className="absolute inset-y-2 left-0.5 w-[3px] rounded-full bg-accent"
                />
              )}
              <Icon
                size={14}
                className={cn("shrink-0", selected ? "text-accent" : "text-lavender")}
              />
              <span className="truncate">{s.label}</span>
            </button>
          );
        })}

        <div className="mt-auto px-2 pt-3">
          <Badge tone={isPttMode ? "accent" : "idle"}>
            {isPttMode ? "Push to talk" : "Open mic"}
          </Badge>
        </div>
      </nav>

      {/* Right Settings Content Floating Island */}
      <div className="glass-card flex min-w-0 flex-1 flex-col overflow-hidden rounded-[22px]">
        <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border px-6">
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

        <div className="min-h-0 flex-1 overflow-y-auto p-6">
          <div className="mx-auto flex w-full max-w-[680px] flex-col gap-5 rounded-2xl border border-border bg-[var(--bg-input)] p-6">
            {body()}
          </div>
        </div>
      </div>
    </div>
  );
}
