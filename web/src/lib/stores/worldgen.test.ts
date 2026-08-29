// WorldgenStore: keyframe hydration, pending-on-retry, optimistic clear.

import { describe, expect, it } from "vitest";

import { WorldgenStore } from "./worldgen.svelte";

const draftPayload = {
  room: "#42",
  room_name: "The Pawn Shop",
  builder: "#7",
  model: "claude-opus-5",
  current: { desc: "old desc" },
  proposed: { desc: "new desc", short_desc: "", details: [], exits: [] },
};

describe("WorldgenStore", () => {
  it("starts empty, not pending", () => {
    const s = new WorldgenStore();
    expect(s.draft).toBeNull();
    expect(s.pending).toBe(false);
  });

  it("applyKeyframe hydrates the draft and clears pending", () => {
    const s = new WorldgenStore();
    s.markPending(); // no-op: no draft yet
    const ok = s.applyKeyframe(draftPayload);
    expect(ok).toBe(true);
    expect(s.draft?.room).toBe("#42");
    expect(s.draft?.proposed.desc).toBe("new desc");
    expect(s.pending).toBe(false);
  });

  it("applyKeyframe on a malformed payload leaves state untouched and reports false", () => {
    const s = new WorldgenStore();
    s.applyKeyframe(draftPayload);
    const ok = s.applyKeyframe({ nonsense: true });
    expect(ok).toBe(false);
    expect(s.draft?.room).toBe("#42"); // unchanged
  });

  it("markPending only flags a stale draft when one is loaded", () => {
    const s = new WorldgenStore();
    s.markPending();
    expect(s.pending).toBe(false); // nothing to mark stale

    s.applyKeyframe(draftPayload);
    s.markPending();
    expect(s.pending).toBe(true);
    expect(s.draft).not.toBeNull(); // stays visible while regenerating
  });

  it("a fresh keyframe after markPending clears pending again", () => {
    const s = new WorldgenStore();
    s.applyKeyframe(draftPayload);
    s.markPending();
    s.applyKeyframe({ ...draftPayload, proposed: { desc: "regenerated", short_desc: "", details: [], exits: [] } });
    expect(s.pending).toBe(false);
    expect(s.draft?.proposed.desc).toBe("regenerated");
  });

  it("clear resets everything (the optimistic accept/discard path)", () => {
    const s = new WorldgenStore();
    s.applyKeyframe(draftPayload);
    s.markPending();
    s.clear();
    expect(s.draft).toBeNull();
    expect(s.pending).toBe(false);
  });
});
