/**
 * Verio icon module.
 *
 * design.md constrains icons tightly:
 *   - single-stroke SVG only, stroke width 1.3–1.5, round line caps
 *   - fills only when the icon represents a filled state (e.g. mute when muted)
 *   - no icon fonts, no emoji, no external image assets
 *   - 14px inline, 16px in buttons, 20px for header brand marks
 *
 * Every icon therefore goes through `makeIcon`, which is the single place those
 * rules are applied. Nothing in the app should render a bare <svg> icon or
 * import an icon library directly — if a rule changes, it changes here.
 *
 * `absoluteStrokeWidth` is what makes "1.4" mean 1.4 *rendered* pixels: without
 * it, stroke width is expressed in the 24-unit viewBox and a 14px icon would
 * draw a ~0.8px stroke, below the specified range.
 */

import type { ComponentType, SVGProps } from "react";
import {
  Activity,
  AlertCircle,
  AlertTriangle,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  CircleDot,
  Clock,
  Copy,
  Ear,
  EarOff,
  ExternalLink,
  FileAudio,
  FolderOpen,
  Gauge,
  Hash,
  Headphones,
  Info,
  Keyboard,
  LoaderCircle,
  LogIn,
  LogOut,
  Maximize2,
  Mic,
  MicOff,
  Minimize2,
  Music,
  Pause,
  Pencil,
  PhoneOff,
  Play,
  Plus,
  Power,
  Radio,
  RefreshCw,
  Repeat,
  Search,
  Server,
  Settings,
  Shield,
  Signal,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  Square,
  Terminal,
  Trash2,
  Users,
  Volume2,
  VolumeX,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";

/** design.md: 14px inline, 16px in buttons, 20px for header brand marks. */
export type IconSize = 14 | 16 | 20;

export interface IconProps {
  /** Defaults to 16 — the button size. */
  size?: IconSize;
  className?: string;
  /** Only for icons that represent a filled state. */
  filled?: boolean;
}

/** The shape every lucide icon satisfies; kept local so the wrapper is typed. */
type LucideComponent = ComponentType<
  SVGProps<SVGSVGElement> & {
    size?: number | string;
    absoluteStrokeWidth?: boolean;
  }
>;

function makeIcon(Lucide: LucideComponent, name: string) {
  function Icon({ size = 16, className, filled = false }: IconProps) {
    return (
      <Lucide
        size={size}
        strokeWidth={1.4}
        absoluteStrokeWidth
        strokeLinecap="round"
        strokeLinejoin="round"
        fill={filled ? "currentColor" : "none"}
        className={className}
        // Icons are decorative. The accessible name lives on the control that
        // contains them, via aria-label.
        aria-hidden="true"
        focusable="false"
      />
    );
  }
  Icon.displayName = `Icon(${name})`;
  return Icon;
}

// --- Shell / navigation ----------------------------------------------------
export const IconSettings = makeIcon(Settings, "settings");
export const IconSliders = makeIcon(SlidersHorizontal, "sliders");
export const IconKeyboard = makeIcon(Keyboard, "keyboard");
export const IconServer = makeIcon(Server, "server");
export const IconMusic = makeIcon(Music, "music");
export const IconActivity = makeIcon(Activity, "activity");
export const IconInfo = makeIcon(Info, "info");
export const IconTerminal = makeIcon(Terminal, "terminal");
export const IconGauge = makeIcon(Gauge, "gauge");
export const IconShield = makeIcon(Shield, "shield");
export const IconRadio = makeIcon(Radio, "radio");

// --- Voice / room ----------------------------------------------------------
export const IconMic = makeIcon(Mic, "mic");
export const IconMicOff = makeIcon(MicOff, "micOff");
export const IconHeadphones = makeIcon(Headphones, "headphones");
/* Deafen is a hearing on/off pair, so it follows the module's `X` / `XOff`
   convention (`IconMic`/`IconMicOff`, `IconVolume`/`IconVolumeOff`). The two
   glyphs are from the same lucide family so the pair reads as one control
   changing state rather than two unrelated buttons. */
export const IconEar = makeIcon(Ear, "ear");
export const IconEarOff = makeIcon(EarOff, "earOff");
export const IconLeave = makeIcon(PhoneOff, "leave");
export const IconUsers = makeIcon(Users, "users");
export const IconVolume = makeIcon(Volume2, "volume");
export const IconVolumeOff = makeIcon(VolumeX, "volumeOff");
export const IconSignal = makeIcon(Signal, "signal");
export const IconWifi = makeIcon(Wifi, "wifi");
export const IconWifiOff = makeIcon(WifiOff, "wifiOff");
export const IconHash = makeIcon(Hash, "hash");
export const IconPower = makeIcon(Power, "power");

// --- Music -----------------------------------------------------------------
export const IconPlay = makeIcon(Play, "play");
export const IconPause = makeIcon(Pause, "pause");
export const IconStop = makeIcon(Square, "stop");
export const IconRepeat = makeIcon(Repeat, "repeat");
export const IconSkipBack = makeIcon(SkipBack, "skipBack");
export const IconSkipForward = makeIcon(SkipForward, "skipForward");
export const IconFolderOpen = makeIcon(FolderOpen, "folderOpen");
export const IconFileAudio = makeIcon(FileAudio, "fileAudio");
export const IconMonitor = makeIcon(Headphones, "monitor");

// --- Generic controls ------------------------------------------------------
export const IconCheck = makeIcon(Check, "check");
export const IconX = makeIcon(X, "close");
export const IconPlus = makeIcon(Plus, "plus");
export const IconCopy = makeIcon(Copy, "copy");
export const IconPencil = makeIcon(Pencil, "pencil");
export const IconSearch = makeIcon(Search, "search");
export const IconRefresh = makeIcon(RefreshCw, "refresh");
export const IconChevronDown = makeIcon(ChevronDown, "chevronDown");
export const IconChevronUp = makeIcon(ChevronUp, "chevronUp");
export const IconChevronLeft = makeIcon(ChevronLeft, "chevronLeft");
export const IconChevronRight = makeIcon(ChevronRight, "chevronRight");
export const IconArrowRight = makeIcon(ArrowRight, "arrowRight");
export const IconExpand = makeIcon(Maximize2, "expand");
export const IconCollapse = makeIcon(Minimize2, "collapse");
export const IconExternalLink = makeIcon(ExternalLink, "externalLink");
export const IconTrash = makeIcon(Trash2, "trash");
export const IconLogIn = makeIcon(LogIn, "logIn");
export const IconLogOut = makeIcon(LogOut, "logOut");
export const IconClock = makeIcon(Clock, "clock");
export const IconDot = makeIcon(CircleDot, "dot");

// --- Status ----------------------------------------------------------------
export const IconAlert = makeIcon(AlertTriangle, "alert");
export const IconError = makeIcon(AlertCircle, "error");

/**
 * Loading indicator.
 *
 * design.md says nothing else animates continuously beyond the speaking
 * indicator and the live mic meter. A spinner is functional feedback for a
 * state design.md explicitly requires ("loading where relevant"), so it is
 * permitted — but it is the only addition, and it is a single 0.8s rotation
 * with no bounce or overshoot.
 */
export function IconSpinner({ size = 16, className }: IconProps) {
  return (
    <LoaderCircle
      size={size}
      strokeWidth={1.4}
      absoluteStrokeWidth
      strokeLinecap="round"
      strokeLinejoin="round"
      fill="none"
      className={className}
      aria-hidden="true"
      focusable="false"
    />
  );
}
IconSpinner.displayName = "Icon(spinner)";
