import test from "node:test";
import assert from "node:assert/strict";
import {
  SCREEN_PRESET_ORDER,
  SCREEN_QUALITY_PRESETS,
  extractScreenStatsFromReport,
  type GenericStatsRecord,
} from "../src/lib/screen-share-utils.ts";

test("screen quality presets default to 720p30 low-traffic and include all four tiers", () => {
  assert.deepEqual(SCREEN_PRESET_ORDER, ["720p30", "720p60", "1080p30", "1080p60"]);
  assert.equal(SCREEN_QUALITY_PRESETS["720p30"].width, 1280);
  assert.equal(SCREEN_QUALITY_PRESETS["720p30"].height, 720);
  assert.equal(SCREEN_QUALITY_PRESETS["720p30"].frameRate, 30);
  assert.equal(SCREEN_QUALITY_PRESETS["720p30"].maxBitrate, 1_500_000);
  assert.equal(SCREEN_QUALITY_PRESETS["1080p60"].maxBitrate, 4_500_000);
});

test("extractScreenStatsFromReport detects direct P2P path and video metrics", () => {
  const stats: GenericStatsRecord[] = [
    {
      id: "transport-0",
      type: "transport",
      selectedCandidatePairId: "pair-1",
    },
    {
      id: "cand-local",
      type: "local-candidate",
      candidateType: "srflx",
    },
    {
      id: "cand-remote",
      type: "remote-candidate",
      candidateType: "prflx",
    },
    {
      id: "pair-1",
      type: "candidate-pair",
      state: "succeeded",
      nominated: true,
      localCandidateId: "cand-local",
      remoteCandidateId: "cand-remote",
    },
    {
      id: "rtp-video",
      type: "inbound-rtp",
      kind: "video",
      frameWidth: 1920,
      frameHeight: 1080,
      framesPerSecond: 59.8,
    },
  ];

  const out = extractScreenStatsFromReport(stats);
  assert.equal(out.transport, "P2P");
  assert.equal(out.width, 1920);
  assert.equal(out.height, 1080);
  assert.equal(out.fps, 60);
});

test("extractScreenStatsFromReport detects Relay path when either candidate uses relay", () => {
  const stats: GenericStatsRecord[] = [
    {
      id: "cand-local",
      type: "local-candidate",
      candidateType: "host",
    },
    {
      id: "cand-remote",
      type: "remote-candidate",
      candidateType: "relay",
    },
    {
      id: "pair-relay",
      type: "candidate-pair",
      state: "succeeded",
      nominated: true,
      localCandidateId: "cand-local",
      remoteCandidateId: "cand-remote",
    },
  ];

  const out = extractScreenStatsFromReport(stats);
  assert.equal(out.transport, "Relay");
  assert.equal(out.width, null);
  assert.equal(out.height, null);
  assert.equal(out.fps, null);
});

test("extractScreenStatsFromReport never fabricates stats when pair is still connecting", () => {
  const stats: GenericStatsRecord[] = [
    {
      id: "cand-local",
      type: "local-candidate",
      candidateType: "host",
    },
    {
      id: "cand-remote",
      type: "remote-candidate",
      candidateType: "host",
    },
    {
      id: "pair-checking",
      type: "candidate-pair",
      state: "in-progress",
      localCandidateId: "cand-local",
      remoteCandidateId: "cand-remote",
    },
  ];

  const out = extractScreenStatsFromReport(stats);
  assert.equal(out.transport, null);
  assert.equal(out.width, null);
  assert.equal(out.height, null);
  assert.equal(out.fps, null);
});
