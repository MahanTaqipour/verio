import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  SCREEN_PRESET_ORDER,
  SCREEN_QUALITY_PRESETS,
  extractScreenStatsFromReport,
  type GenericStatsRecord,
  type ScreenPresetId,
  type ScreenQualityPreset,
  type ScreenStreamStats,
} from "./screen-share-utils";

export {
  SCREEN_PRESET_ORDER,
  SCREEN_QUALITY_PRESETS,
  extractScreenStatsFromReport,
  type ScreenPresetId,
  type ScreenQualityPreset,
  type ScreenStreamStats,
};

export interface IceServerConfig {
  urls: string[];
  username?: string;
  credential?: string;
}

export interface ScreenSharerInfo {
  label: string;
}

type ScreenSignalPayload =
  | { type: "watch_request" }
  | { type: "watch_stop" }
  | { type: "offer"; sdp: string }
  | { type: "answer"; sdp: string }
  | { type: "ice"; candidate: RTCIceCandidateInit };

export interface UseScreenShareOptions {
  inCall: boolean;
  onStatus?: (text: string, isError?: boolean) => void;
}

export interface ScreenShareController {
  isSharing: boolean;
  watcherCount: number;
  localStream: MediaStream | null;
  screenSharers: Record<string, ScreenSharerInfo>;
  watchingPeerId: string | null;
  remoteStream: MediaStream | null;
  streamStats: ScreenStreamStats | null;
  startSharing: (presetId: ScreenPresetId, includeAudio: boolean) => Promise<void>;
  stopSharing: () => Promise<void>;
  watchPeer: (peerId: string) => Promise<void>;
  stopWatching: () => Promise<void>;
  handlePeerDisconnected: (peerId: string) => void;
}

const DEFAULT_ICE_SERVERS: RTCIceServer[] = [
  { urls: ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"] },
];

function toRtcIceServers(servers: IceServerConfig[] | null | undefined): RTCIceServer[] {
  if (!servers || servers.length === 0) {
    return DEFAULT_ICE_SERVERS;
  }
  return servers.map((s) => ({
    urls: s.urls,
    ...(s.username ? { username: s.username } : {}),
    ...(s.credential ? { credential: s.credential } : {}),
  }));
}

export function useScreenShare({
  inCall,
  onStatus,
}: UseScreenShareOptions): ScreenShareController {
  const [isSharing, setIsSharing] = useState(false);
  const [watcherCount, setWatcherCount] = useState(0);
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [screenSharers, setScreenSharers] = useState<Record<string, ScreenSharerInfo>>({});
  const [watchingPeerId, setWatchingPeerId] = useState<string | null>(null);
  const [remoteStream, setRemoteStream] = useState<MediaStream | null>(null);
  const [streamStats, setStreamStats] = useState<ScreenStreamStats | null>(null);

  const onStatusRef = useRef(onStatus);
  onStatusRef.current = onStatus;
  const say = useCallback((text: string, isError = false) => {
    onStatusRef.current?.(text, isError);
  }, []);

  const iceServersRef = useRef<RTCIceServer[]>(DEFAULT_ICE_SERVERS);
  const isSharingRef = useRef(false);
  const activePresetRef = useRef<ScreenQualityPreset>(SCREEN_QUALITY_PRESETS["720p30"]);
  const localStreamRef = useRef<MediaStream | null>(null);
  const watcherPcsRef = useRef<Map<string, RTCPeerConnection>>(new Map());
  const watcherDisconnectTimersRef = useRef<Map<string, number>>(new Map());

  const watchingPeerIdRef = useRef<string | null>(null);
  const watchPcRef = useRef<RTCPeerConnection | null>(null);
  const watchStatsTimerRef = useRef<number | null>(null);
  const pendingIceRef = useRef<Map<string, RTCIceCandidateInit[]>>(new Map());

  const clearWatcherTimer = useCallback((peerId: string) => {
    const timer = watcherDisconnectTimersRef.current.get(peerId);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      watcherDisconnectTimersRef.current.delete(peerId);
    }
  }, []);

  const closeWatcherPc = useCallback(
    (peerId: string) => {
      clearWatcherTimer(peerId);
      pendingIceRef.current.delete(`share:${peerId}`);
      const pc = watcherPcsRef.current.get(peerId);
      if (pc) {
        pc.onicecandidate = null;
        pc.onconnectionstatechange = null;
        try {
          pc.close();
        } catch {
          // ignore
        }
        watcherPcsRef.current.delete(peerId);
      }
      setWatcherCount(watcherPcsRef.current.size);
    },
    [clearWatcherTimer],
  );

  const flushPendingIce = useCallback(async (key: string, pc: RTCPeerConnection) => {
    const queued = pendingIceRef.current.get(key);
    if (!queued || queued.length === 0) return;
    pendingIceRef.current.delete(key);
    for (const candidate of queued) {
      try {
        await pc.addIceCandidate(new RTCIceCandidate(candidate));
      } catch {
        // Ignore stale candidates
      }
    }
  }, []);

  const cleanupWatchPc = useCallback(() => {
    if (watchStatsTimerRef.current !== null) {
      window.clearInterval(watchStatsTimerRef.current);
      watchStatsTimerRef.current = null;
    }
    const peerId = watchingPeerIdRef.current;
    if (peerId) {
      pendingIceRef.current.delete(`watch:${peerId}`);
    }
    const pc = watchPcRef.current;
    if (pc) {
      pc.ontrack = null;
      pc.onicecandidate = null;
      pc.onconnectionstatechange = null;
      try {
        pc.close();
      } catch {
        // ignore
      }
      watchPcRef.current = null;
    }
    watchingPeerIdRef.current = null;
    setWatchingPeerId(null);
    setRemoteStream(null);
    setStreamStats(null);
  }, []);

  const stopWatching = useCallback(async () => {
    const target = watchingPeerIdRef.current;
    cleanupWatchPc();
    if (target) {
      await invoke("send_screen_signal", {
        target,
        payload: { type: "watch_stop" },
      }).catch(() => {});
    }
  }, [cleanupWatchPc]);

  const stopSharing = useCallback(async () => {
    const wasSharing = isSharingRef.current;
    isSharingRef.current = false;
    setIsSharing(false);

    for (const peerId of Array.from(watcherPcsRef.current.keys())) {
      closeWatcherPc(peerId);
    }

    const stream = localStreamRef.current;
    localStreamRef.current = null;
    setLocalStream(null);
    if (stream) {
      for (const track of stream.getTracks()) {
        track.onended = null;
        try {
          track.stop();
        } catch {
          // ignore
        }
      }
    }
    setWatcherCount(0);

    if (wasSharing) {
      await invoke("send_screen_state", { sharing: false, label: "" }).catch(() => {});
    }
  }, [closeWatcherPc]);

  const stopSharingRef = useRef(stopSharing);
  stopSharingRef.current = stopSharing;

  const startSharing = useCallback(
    async (presetId: ScreenPresetId, includeAudio: boolean) => {
      if (!inCall) {
        say("Join a room before sharing your screen.", true);
        return;
      }
      if (!navigator.mediaDevices?.getDisplayMedia) {
        say("Screen capture is not supported in this runtime.", true);
        return;
      }

      const preset = SCREEN_QUALITY_PRESETS[presetId];
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getDisplayMedia({
          video: {
            width: { ideal: preset.width },
            height: { ideal: preset.height },
            frameRate: { ideal: preset.frameRate, max: preset.frameRate },
          },
          audio: includeAudio,
        });
      } catch (err: unknown) {
        const errName = err instanceof Error ? err.name : "";
        if (includeAudio && (errName === "NotSupportedError" || errName === "TypeError")) {
          try {
            stream = await navigator.mediaDevices.getDisplayMedia({
              video: {
                width: { ideal: preset.width },
                height: { ideal: preset.height },
                frameRate: { ideal: preset.frameRate, max: preset.frameRate },
              },
              audio: false,
            });
            say("Sharing video only (system audio unavailable for this source).");
          } catch (retryErr: unknown) {
            const retryName = retryErr instanceof Error ? retryErr.name : "";
            if (retryName !== "AbortError" && retryName !== "NotAllowedError") {
              say(`Could not capture screen: ${String(retryErr)}`, true);
            }
            return;
          }
        } else {
          if (errName !== "AbortError" && errName !== "NotAllowedError") {
            say(`Could not capture screen: ${String(err)}`, true);
          }
          return;
        }
      }

      // Tear down any previous share cleanly before replacing
      if (isSharingRef.current) {
        await stopSharing();
      }

      const videoTrack = stream.getVideoTracks()[0];
      if (!videoTrack) {
        for (const t of stream.getTracks()) t.stop();
        say("Selected screen source provided no video track.", true);
        return;
      }

      if ("contentHint" in videoTrack) {
        videoTrack.contentHint = presetId.endsWith("60") ? "motion" : "detail";
      }
      videoTrack.onended = () => {
        void stopSharingRef.current();
      };

      activePresetRef.current = preset;
      localStreamRef.current = stream;
      isSharingRef.current = true;
      setLocalStream(stream);
      setIsSharing(true);
      setWatcherCount(0);

      try {
        await invoke("send_screen_state", { sharing: true, label: preset.label });
      } catch (err) {
        await stopSharing();
        say(`Could not publish screen share: ${String(err)}`, true);
      }
    },
    [inCall, say, stopSharing],
  );

  const watchPeer = useCallback(
    async (peerId: string) => {
      if (!inCall) return;
      if (watchingPeerIdRef.current && watchingPeerIdRef.current !== peerId) {
        await stopWatching();
      }
      cleanupWatchPc();
      watchingPeerIdRef.current = peerId;
      setWatchingPeerId(peerId);
      setStreamStats(null);

      try {
        await invoke("send_screen_signal", {
          target: peerId,
          payload: { type: "watch_request" },
        });
      } catch (err) {
        cleanupWatchPc();
        say(`Could not request screen stream: ${String(err)}`, true);
      }
    },
    [cleanupWatchPc, inCall, say, stopWatching],
  );

  const handleWatchRequest = useCallback(
    async (fromPeerId: string) => {
      const stream = localStreamRef.current;
      if (!isSharingRef.current || !stream) return;

      closeWatcherPc(fromPeerId);
      const pc = new RTCPeerConnection({ iceServers: iceServersRef.current });
      watcherPcsRef.current.set(fromPeerId, pc);
      setWatcherCount(watcherPcsRef.current.size);

      let videoSender: RTCRtpSender | null = null;
      for (const track of stream.getTracks()) {
        const sender = pc.addTrack(track, stream);
        if (track.kind === "video") {
          videoSender = sender;
        }
      }

      if (videoSender) {
        try {
          const params = videoSender.getParameters();
          if (!params.encodings || params.encodings.length === 0) {
            params.encodings = [{}];
          }
          params.encodings[0].maxBitrate = activePresetRef.current.maxBitrate;
          params.degradationPreference = "balanced";
          await videoSender.setParameters(params);
        } catch {
          // Some WebView implementations restrict runtime encoding overrides.
        }
      }

      pc.onicecandidate = (ev) => {
        if (!ev.candidate) return;
        void invoke("send_screen_signal", {
          target: fromPeerId,
          payload: { type: "ice", candidate: ev.candidate.toJSON() },
        }).catch(() => {});
      };

      pc.onconnectionstatechange = () => {
        const state = pc.connectionState;
        if (state === "connected") {
          clearWatcherTimer(fromPeerId);
          return;
        }
        if (state === "failed" || state === "closed") {
          if (watcherPcsRef.current.get(fromPeerId) === pc) {
            closeWatcherPc(fromPeerId);
          }
          return;
        }
        if (state === "disconnected" && !watcherDisconnectTimersRef.current.has(fromPeerId)) {
          const timer = window.setTimeout(() => {
            watcherDisconnectTimersRef.current.delete(fromPeerId);
            const current = watcherPcsRef.current.get(fromPeerId);
            if (current === pc && (pc.connectionState === "disconnected" || pc.connectionState === "failed")) {
              closeWatcherPc(fromPeerId);
            }
          }, 5000);
          watcherDisconnectTimersRef.current.set(fromPeerId, timer);
        }
      };

      try {
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        if (watcherPcsRef.current.get(fromPeerId) !== pc || !offer.sdp) return;
        await invoke("send_screen_signal", {
          target: fromPeerId,
          payload: { type: "offer", sdp: offer.sdp },
        });
      } catch {
        if (watcherPcsRef.current.get(fromPeerId) === pc) {
          closeWatcherPc(fromPeerId);
        }
      }
    },
    [clearWatcherTimer, closeWatcherPc],
  );

  const handleOfferFromSharer = useCallback(
    async (fromPeerId: string, sdp: string) => {
      if (watchingPeerIdRef.current !== fromPeerId) return;

      // Preserve any ICE candidates queued before offer arrived for this watch session.
      const queuedBeforeOffer = pendingIceRef.current.get(`watch:${fromPeerId}`);
      if (watchStatsTimerRef.current !== null) {
        window.clearInterval(watchStatsTimerRef.current);
        watchStatsTimerRef.current = null;
      }
      if (watchPcRef.current) {
        watchPcRef.current.ontrack = null;
        watchPcRef.current.onicecandidate = null;
        watchPcRef.current.onconnectionstatechange = null;
        try {
          watchPcRef.current.close();
        } catch {
          // ignore
        }
      }

      const pc = new RTCPeerConnection({ iceServers: iceServersRef.current });
      watchPcRef.current = pc;
      if (queuedBeforeOffer && queuedBeforeOffer.length > 0) {
        pendingIceRef.current.set(`watch:${fromPeerId}`, queuedBeforeOffer);
      }

      const inboundStream = new MediaStream();
      pc.ontrack = (ev) => {
        if (watchPcRef.current !== pc) return;
        if (ev.streams && ev.streams[0]) {
          setRemoteStream(ev.streams[0]);
        } else {
          inboundStream.addTrack(ev.track);
          setRemoteStream(new MediaStream(inboundStream.getTracks()));
        }
      };

      pc.onicecandidate = (ev) => {
        if (!ev.candidate || watchPcRef.current !== pc) return;
        void invoke("send_screen_signal", {
          target: fromPeerId,
          payload: { type: "ice", candidate: ev.candidate.toJSON() },
        }).catch(() => {});
      };

      pc.onconnectionstatechange = () => {
        if (watchPcRef.current !== pc) return;
        if (pc.connectionState === "failed" || pc.connectionState === "closed") {
          cleanupWatchPc();
          say("Screen share connection lost.", true);
        }
      };

      const pollStats = async () => {
        if (watchPcRef.current !== pc) return;
        try {
          const report = await pc.getStats();
          if (watchPcRef.current !== pc) return;
          const values = Array.from(report.values()) as unknown as GenericStatsRecord[];
          setStreamStats(extractScreenStatsFromReport(values));
        } catch {
          // ignore transient stats failures
        }
      };
      watchStatsTimerRef.current = window.setInterval(() => {
        void pollStats();
      }, 2000);

      try {
        await pc.setRemoteDescription({ type: "offer", sdp });
        if (watchPcRef.current !== pc) return;
        await flushPendingIce(`watch:${fromPeerId}`, pc);
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        if (watchPcRef.current !== pc || !answer.sdp) return;
        await invoke("send_screen_signal", {
          target: fromPeerId,
          payload: { type: "answer", sdp: answer.sdp },
        });
        void pollStats();
      } catch (err) {
        if (watchPcRef.current === pc) {
          cleanupWatchPc();
          say(`Failed to connect to screen share: ${String(err)}`, true);
        }
      }
    },
    [cleanupWatchPc, flushPendingIce, say],
  );

  const handleAnswerFromWatcher = useCallback(
    async (fromPeerId: string, sdp: string) => {
      const pc = watcherPcsRef.current.get(fromPeerId);
      if (!pc) return;
      try {
        await pc.setRemoteDescription({ type: "answer", sdp });
        await flushPendingIce(`share:${fromPeerId}`, pc);
      } catch {
        closeWatcherPc(fromPeerId);
      }
    },
    [closeWatcherPc, flushPendingIce],
  );

  const handleRemoteIce = useCallback(
    async (fromPeerId: string, candidate: RTCIceCandidateInit) => {
      const sharePc = watcherPcsRef.current.get(fromPeerId);
      if (sharePc) {
        if (sharePc.remoteDescription) {
          try {
            await sharePc.addIceCandidate(new RTCIceCandidate(candidate));
          } catch {
            // ignore
          }
        } else {
          const key = `share:${fromPeerId}`;
          const arr = pendingIceRef.current.get(key) ?? [];
          arr.push(candidate);
          pendingIceRef.current.set(key, arr);
        }
        return;
      }

      if (watchingPeerIdRef.current === fromPeerId) {
        const watchPc = watchPcRef.current;
        if (watchPc && watchPc.remoteDescription) {
          try {
            await watchPc.addIceCandidate(new RTCIceCandidate(candidate));
          } catch {
            // ignore
          }
        } else {
          const key = `watch:${fromPeerId}`;
          const arr = pendingIceRef.current.get(key) ?? [];
          arr.push(candidate);
          pendingIceRef.current.set(key, arr);
        }
      }
    },
    [],
  );

  const handlePeerDisconnected = useCallback(
    (peerId: string) => {
      setScreenSharers((prev) => {
        if (!(peerId in prev)) return prev;
        const next = { ...prev };
        delete next[peerId];
        return next;
      });
      if (watcherPcsRef.current.has(peerId)) {
        closeWatcherPc(peerId);
      }
      if (watchingPeerIdRef.current === peerId) {
        cleanupWatchPc();
      }
    },
    [cleanupWatchPc, closeWatcherPc],
  );

  // Subscribe once to Tauri backend signaling & ICE config events (StrictMode-safe).
  useEffect(() => {
    let disposed = false;
    const unlistens: Array<() => void> = [];
    const subscribe = <T,>(
      event: string,
      handler: Parameters<typeof listen<T>>[1],
    ) => {
      void listen<T>(event, handler).then((u) => {
        if (disposed) u();
        else unlistens.push(u);
      });
    };

    subscribe<{ servers?: IceServerConfig[]; ice_servers?: IceServerConfig[] }>(
      "ice_config",
      (ev) => {
        const raw = ev.payload.servers ?? ev.payload.ice_servers;
        if (raw && raw.length > 0) {
          iceServersRef.current = toRtcIceServers(raw);
        }
      },
    );

    subscribe<{ peer_id: string; sharing: boolean; label: string }>(
      "screen_state",
      (ev) => {
        const { peer_id, sharing, label } = ev.payload;
        if (!peer_id) return;
        setScreenSharers((prev) => {
          if (sharing) {
            return { ...prev, [peer_id]: { label: label || "Screen" } };
          }
          if (!(peer_id in prev)) return prev;
          const next = { ...prev };
          delete next[peer_id];
          return next;
        });
        if (!sharing && watchingPeerIdRef.current === peer_id) {
          cleanupWatchPc();
        }
      },
    );

    subscribe<{ from?: string; from_peer_id?: string; payload: ScreenSignalPayload }>(
      "screen_signal",
      (ev) => {
        const fromId = ev.payload.from ?? ev.payload.from_peer_id;
        const sig = ev.payload.payload;
        if (!fromId || !sig || typeof sig !== "object") return;

        switch (sig.type) {
          case "watch_request":
            void handleWatchRequest(fromId);
            break;
          case "watch_stop":
            closeWatcherPc(fromId);
            break;
          case "offer":
            if (typeof sig.sdp === "string") {
              void handleOfferFromSharer(fromId, sig.sdp);
            }
            break;
          case "answer":
            if (typeof sig.sdp === "string") {
              void handleAnswerFromWatcher(fromId, sig.sdp);
            }
            break;
          case "ice":
            if (sig.candidate) {
              void handleRemoteIce(fromId, sig.candidate);
            }
            break;
        }
      },
    );

    return () => {
      disposed = true;
      for (const u of unlistens) u();
    };
  }, [
    cleanupWatchPc,
    closeWatcherPc,
    handleAnswerFromWatcher,
    handleOfferFromSharer,
    handleRemoteIce,
    handleWatchRequest,
  ]);

  // Fetch initial ICE servers on entering a room, and tear down when leaving a room.
  const prevInCallRef = useRef(inCall);
  useEffect(() => {
    if (inCall) {
      let cancelled = false;
      invoke<IceServerConfig[]>("get_ice_servers")
        .then((servers) => {
          if (!cancelled && servers && servers.length > 0) {
            iceServersRef.current = toRtcIceServers(servers);
          }
        })
        .catch(() => {});
      prevInCallRef.current = true;
      return () => {
        cancelled = true;
      };
    }

    if (prevInCallRef.current) {
      prevInCallRef.current = false;
      setScreenSharers({});
      iceServersRef.current = DEFAULT_ICE_SERVERS;
      void stopSharing();
      void stopWatching();
    }
  }, [inCall, stopSharing, stopWatching]);

  // Unmount safety cleanup for media tracks & RTCPeerConnections.
  useEffect(() => {
    return () => {
      cleanupWatchPc();
      for (const peerId of Array.from(watcherPcsRef.current.keys())) {
        closeWatcherPc(peerId);
      }
      if (localStreamRef.current) {
        for (const t of localStreamRef.current.getTracks()) {
          t.onended = null;
          try {
            t.stop();
          } catch {
            // ignore
          }
        }
        localStreamRef.current = null;
      }
    };
  }, [cleanupWatchPc, closeWatcherPc]);

  return {
    isSharing,
    watcherCount,
    localStream,
    screenSharers,
    watchingPeerId,
    remoteStream,
    streamStats,
    startSharing,
    stopSharing,
    watchPeer,
    stopWatching,
    handlePeerDisconnected,
  };
}
