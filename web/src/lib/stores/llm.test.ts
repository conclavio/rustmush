// LlmDashboardStore: keyframe hydration, the staff-refusal vs unavailable
// distinction, and clear().

import { describe, expect, it } from "vitest";

import { LlmDashboardStore } from "./llm.svelte";

const payload = {
  kill_switch_engaged: false,
  budgets: [
    {
      scope: "global",
      input_tokens: 10,
      output_tokens: 5,
      cost_usd: 0.01,
      tokens_cap: null,
      cost_cap: null,
      tokens_remaining: null,
    },
  ],
  top_actors: [],
  top_npcs: [],
  daily_series: [],
  recent_calls: [],
};

describe("LlmDashboardStore", () => {
  it("starts empty, not loaded", () => {
    const s = new LlmDashboardStore();
    expect(s.data).toBeNull();
    expect(s.loaded).toBe(false);
    expect(s.refused).toBe(false);
    expect(s.unavailable).toBe(false);
  });

  it("markRequested flags refreshing; applyKeyframe clears it and hydrates", () => {
    const s = new LlmDashboardStore();
    s.markRequested();
    expect(s.refreshing).toBe(true);
    const ok = s.applyKeyframe(payload);
    expect(ok).toBe(true);
    expect(s.refreshing).toBe(false);
    expect(s.loaded).toBe(true);
    expect(s.data?.budgets[0]?.inputTokens).toBe(10);
  });

  it("applyKeyframe with an unusable payload does not hydrate", () => {
    const s = new LlmDashboardStore();
    expect(s.applyKeyframe("nonsense")).toBe(false);
    expect(s.loaded).toBe(false);
    expect(s.data).toBeNull();
  });

  it("markRefused distinguishes a staff-only refusal from generic unavailability", () => {
    const staffOnly = new LlmDashboardStore();
    staffOnly.markRequested();
    staffOnly.markRefused("Staff only (the LLM dashboard needs Royalty or better).");
    expect(staffOnly.refreshing).toBe(false);
    expect(staffOnly.refused).toBe(true);
    expect(staffOnly.unavailable).toBe(false);

    const notWired = new LlmDashboardStore();
    notWired.markRefused("No such panel feed \"llm.dashboard\".");
    expect(notWired.refused).toBe(false);
    expect(notWired.unavailable).toBe(true);
  });

  it("a fresh keyframe after a refusal clears both refusal flags", () => {
    const s = new LlmDashboardStore();
    s.markRefused("Staff only.");
    expect(s.refused).toBe(true);
    s.applyKeyframe(payload);
    expect(s.refused).toBe(false);
    expect(s.loaded).toBe(true);
  });

  it("clear resets everything", () => {
    const s = new LlmDashboardStore();
    s.applyKeyframe(payload);
    s.clear();
    expect(s.data).toBeNull();
    expect(s.loaded).toBe(false);
    expect(s.refused).toBe(false);
    expect(s.unavailable).toBe(false);
    expect(s.refreshing).toBe(false);
  });
});
