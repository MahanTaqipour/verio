export type ScreenPresetId = "720p30" | "720p60" | "1080p30" | "1080p60";

export interface ScreenQualityPreset {
  id: ScreenPresetId;
  width: number;
  height: number;
  frameRate: number;
  maxBitrate: number;
  label: string;
  badge?: string;
}

export const SCREEN_QUALITY_PRESETS: Record<ScreenPresetId, ScreenQualityPreset> = {
  "720p30": {
    id: "720p30",
    width: 1280,
    height: 720,
    frameRate: 30,
    maxBitrate: 1_500_000,
    label: "720p · 30fps",
    badge: "Low Traffic",
  },
  "720p60": {
    id: "720p60",
    width: 1280,
    height: 720,
    frameRate: 60,
    maxBitrate: 2_500_000,
    label: "720p · 60fps",
  },
  "1080p30": {
    id: "1080p30",
    width: 1920,
    height: 1080,
    frameRate: 30,
    maxBitrate: 2_800_000,
    label: "1080p · 30fps",
  },
  "1080p60": {
    id: "1080p60",
    width: 1920,
    height: 1080,
    frameRate: 60,
    maxBitrate: 4_500_000,
    label: "1080p · 60fps",
  },
};

export const SCREEN_PRESET_ORDER: ScreenPresetId[] = [
  "720p30",
  "720p60",
  "1080p30",
  "1080p60",
];

export interface ScreenStreamStats {
  width: number | null;
  height: number | null;
  fps: number | null;
  transport: "P2P" | "Relay" | null;
}

export interface GenericStatsRecord {
  id?: string;
  type?: string;
  kind?: string;
  mediaType?: string;
  state?: string;
  nominated?: boolean;
  localCandidateId?: string;
  remoteCandidateId?: string;
  selectedCandidatePairId?: string;
  candidateType?: string;
  frameWidth?: number;
  frameHeight?: number;
  framesPerSecond?: number;
}

/**
 * Parse live RTCPeerConnection.getStats() entries into measured resolution,
 * frame rate, and active ICE candidate transport path (P2P vs Relay).
 * Returns null fields when measurements have not yet been reported.
 */
export function extractScreenStatsFromReport(
  report: Iterable<GenericStatsRecord> | { values: () => Iterable<GenericStatsRecord> },
): ScreenStreamStats {
  const entries = new Map<string, GenericStatsRecord>();
  const list: GenericStatsRecord[] = [];
  const iterable =
    typeof (report as { values?: unknown }).values === "function"
      ? (report as { values: () => Iterable<GenericStatsRecord> }).values()
      : (report as Iterable<GenericStatsRecord>);

  for (const item of iterable) {
    if (!item || typeof item !== "object") continue;
    list.push(item);
    if (typeof item.id === "string" && item.id) {
      entries.set(item.id, item);
    }
  }

  let selectedPairId: string | undefined;
  for (const item of list) {
    if (item.type === "transport" && typeof item.selectedCandidatePairId === "string") {
      selectedPairId = item.selectedCandidatePairId;
      break;
    }
  }

  let activePair: GenericStatsRecord | undefined = selectedPairId
    ? entries.get(selectedPairId)
    : undefined;

  if (!activePair || activePair.type !== "candidate-pair") {
    activePair = list.find(
      (item) =>
        item.type === "candidate-pair" &&
        item.state === "succeeded" &&
        item.nominated === true,
    );
  }
  if (!activePair) {
    activePair = list.find(
      (item) => item.type === "candidate-pair" && item.state === "succeeded",
    );
  }

  let transport: "P2P" | "Relay" | null = null;
  if (activePair && activePair.state === "succeeded") {
    const local =
      typeof activePair.localCandidateId === "string"
        ? entries.get(activePair.localCandidateId)
        : undefined;
    const remote =
      typeof activePair.remoteCandidateId === "string"
        ? entries.get(activePair.remoteCandidateId)
        : undefined;
    if (local?.candidateType === "relay" || remote?.candidateType === "relay") {
      transport = "Relay";
    } else if (local?.candidateType || remote?.candidateType) {
      transport = "P2P";
    }
  }

  let width: number | null = null;
  let height: number | null = null;
  let fps: number | null = null;

  for (const item of list) {
    if (
      item.type === "inbound-rtp" &&
      (item.kind === "video" || item.mediaType === "video")
    ) {
      if (typeof item.frameWidth === "number" && Number.isFinite(item.frameWidth) && item.frameWidth > 0) {
        width = Math.round(item.frameWidth);
      }
      if (typeof item.frameHeight === "number" && Number.isFinite(item.frameHeight) && item.frameHeight > 0) {
        height = Math.round(item.frameHeight);
      }
      if (
        typeof item.framesPerSecond === "number" &&
        Number.isFinite(item.framesPerSecond) &&
        item.framesPerSecond >= 0
      ) {
        fps = Math.round(item.framesPerSecond);
      }
      break;
    }
  }

  return { width, height, fps, transport };
}
