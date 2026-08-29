// The LLM cost dashboard: defensive payload parsing (including the "no
// secret fields survive parsing" contract) and the pure bar/meter/table
// rendering helpers `LlmPanel.svelte` calls.

import { describe, expect, it } from "vitest";

import {
  budgetMeterPct,
  budgetOverCap,
  featureLabel,
  formatCost,
  formatTokens,
  isFeatureBudget,
  latestDay,
  parseDashboardPayload,
  providerTotalsForDay,
  relativeBarPct,
  scopeLabel,
  shortDayLabel,
  type DaySlice,
} from "./llm";

const fullPayload = {
  kill_switch_engaged: false,
  budgets: [
    {
      scope: "global",
      input_tokens: 1200,
      output_tokens: 400,
      cost_usd: 0.31,
      tokens_cap: 5_000_000,
      cost_cap: null,
      tokens_remaining: 4_998_400,
    },
    {
      scope: "feature:worldgen_room",
      input_tokens: 900,
      output_tokens: 300,
      cost_usd: 0.22,
      tokens_cap: 1_000_000,
      cost_cap: null,
      tokens_remaining: 998_800,
    },
    {
      scope: "actor_default",
      input_tokens: 0,
      output_tokens: 0,
      cost_usd: 0,
      tokens_cap: 50_000,
      cost_cap: null,
      tokens_remaining: 50_000,
    },
  ],
  top_actors: [
    { obj: { id: "#7", name: "Alice" }, input_tokens: 900, output_tokens: 300, cost_usd: 0.22 },
  ],
  top_npcs: [],
  daily_series: [
    {
      day: "2026-08-28",
      feature: "worldgen_room",
      provider: "openrouter",
      calls: 2,
      refusals: 0,
      budget_trips: 0,
      disabled: 0,
      errors: 0,
      input_tokens: 400,
      output_tokens: 150,
      cost_usd: 0.1,
    },
    {
      day: "2026-08-29",
      feature: "worldgen_room",
      provider: "openrouter",
      calls: 4,
      refusals: 1,
      budget_trips: 0,
      disabled: 0,
      errors: 0,
      input_tokens: 900,
      output_tokens: 300,
      cost_usd: 0.22,
    },
    {
      day: "2026-08-29",
      feature: "npc_dialogue_major",
      provider: "anthropic",
      calls: 3,
      refusals: 0,
      budget_trips: 1,
      disabled: 0,
      errors: 0,
      input_tokens: 200,
      output_tokens: 100,
      cost_usd: 0.05,
    },
  ],
  recent_calls: [
    {
      ts: 1_756_500_000,
      feature: "worldgen_room",
      provider: "openrouter",
      model: "z-ai/glm-5.3",
      actor: { id: "#7", name: "Alice" },
      npc: null,
      input_tokens: 120,
      output_tokens: 60,
      cost_usd: 0.01,
      latency_ms: 1400,
      outcome: "ok",
    },
  ],
};

describe("parseDashboardPayload", () => {
  it("parses a full payload end to end", () => {
    const d = parseDashboardPayload(fullPayload);
    expect(d).not.toBeNull();
    expect(d?.killSwitchEngaged).toBe(false);
    expect(d?.budgets).toHaveLength(3);
    expect(d?.budgets[0]).toEqual({
      scope: "global",
      inputTokens: 1200,
      outputTokens: 400,
      costUsd: 0.31,
      tokensCap: 5_000_000,
      costCap: null,
      tokensRemaining: 4_998_400,
      costRemaining: null,
    });
    expect(d?.topActors).toHaveLength(1);
    expect(d?.topActors[0]?.obj).toEqual({ id: "#7", name: "Alice" });
    expect(d?.dailySeries).toHaveLength(3);
    expect(d?.recentCalls).toHaveLength(1);
    expect(d?.recentCalls[0]?.outcome).toBe("ok");
    expect(d?.recentCalls[0]?.actor).toEqual({ id: "#7", name: "Alice" });
    expect(d?.recentCalls[0]?.npc).toBeNull();
  });

  it("an empty-but-valid dashboard (a quiet server) still parses", () => {
    const d = parseDashboardPayload({
      kill_switch_engaged: true,
      budgets: [],
      top_actors: [],
      top_npcs: [],
      daily_series: [],
      recent_calls: [],
    });
    expect(d).not.toBeNull();
    expect(d?.killSwitchEngaged).toBe(true);
    expect(d?.budgets).toEqual([]);
    expect(d?.recentCalls).toEqual([]);
  });

  it("rejects a fundamentally non-object payload", () => {
    expect(parseDashboardPayload(null)).toBeNull();
    expect(parseDashboardPayload("nope")).toBeNull();
    expect(parseDashboardPayload(42)).toBeNull();
  });

  it("NEVER surfaces prompt/response text or a key even if the payload carried it", () => {
    // A hypothetical malformed/regressed server response leaking the raw
    // audit fields the dashboard must never carry (dashboard.rs's module
    // doc / the assignment's hard requirement). The parser only ever reads
    // named fields, so anything else is silently dropped on the way in —
    // this is the client-side backstop, not the only line of defense.
    const leaky = {
      ...fullPayload,
      recent_calls: [
        {
          ...fullPayload.recent_calls[0],
          prompt_sha256: "deadbeefdeadbeef",
          prompt_summary: "the builder's actual secret prose",
          response_summary: "the model's actual generated room text",
          api_key: "sk-super-secret",
        },
      ],
    };
    const d = parseDashboardPayload(leaky);
    expect(d).not.toBeNull();
    const call = d?.recentCalls[0];
    expect(call).toBeDefined();
    // Only the known RecentCall fields exist on the parsed object.
    expect(Object.keys(call as object).sort()).toEqual(
      [
        "actor",
        "costUsd",
        "feature",
        "inputTokens",
        "latencyMs",
        "model",
        "npc",
        "outcome",
        "outputTokens",
        "provider",
        "ts",
      ].sort(),
    );
    const rendered = JSON.stringify(d);
    expect(rendered).not.toContain("secret");
    expect(rendered).not.toContain("prompt_summary");
    expect(rendered).not.toContain("response_summary");
    expect(rendered).not.toContain("sk-super-secret");
  });

  it("a malformed budget/spend/call row is dropped, not fatal to the rest", () => {
    const d = parseDashboardPayload({
      ...fullPayload,
      budgets: [{ scope: "global", input_tokens: 5 }, "not an object", 42],
      top_actors: [{ input_tokens: 1 }, fullPayload.top_actors[0]], // missing `obj` -> dropped
    });
    expect(d?.budgets).toHaveLength(1);
    expect(d?.topActors).toHaveLength(1);
  });
});

describe("budget row helpers", () => {
  it("isFeatureBudget / featureLabel / scopeLabel", () => {
    expect(isFeatureBudget({ scope: "feature:worldgen_room" } as never)).toBe(true);
    expect(isFeatureBudget({ scope: "global" } as never)).toBe(false);
    expect(featureLabel("feature:worldgen_room")).toBe("worldgen_room");
    expect(featureLabel("global")).toBe("global");
    expect(scopeLabel("actor_default")).toBe("per-builder default");
    expect(scopeLabel("npc_default")).toBe("per-NPC default");
    expect(scopeLabel("feature:ambience")).toBe("ambience");
  });

  it("budgetMeterPct clamps to 0..100 and is null with no cap", () => {
    expect(
      budgetMeterPct({
        scope: "global",
        inputTokens: 500,
        outputTokens: 500,
        costUsd: 0,
        tokensCap: 1000,
        costCap: null,
        tokensRemaining: 0,
        costRemaining: null,
      }),
    ).toBe(100);
    expect(
      budgetMeterPct({
        scope: "global",
        inputTokens: 250,
        outputTokens: 0,
        costUsd: 0,
        tokensCap: 1000,
        costCap: null,
        tokensRemaining: 750,
        costRemaining: null,
      }),
    ).toBe(25);
    // Over cap clamps to 100, never renders a >100% bar.
    expect(
      budgetMeterPct({
        scope: "global",
        inputTokens: 2000,
        outputTokens: 0,
        costUsd: 0,
        tokensCap: 1000,
        costCap: null,
        tokensRemaining: 0,
        costRemaining: null,
      }),
    ).toBe(100);
    expect(
      budgetMeterPct({
        scope: "actor_default",
        inputTokens: 0,
        outputTokens: 0,
        costUsd: 0,
        tokensCap: null,
        costCap: null,
        tokensRemaining: null,
        costRemaining: null,
      }),
    ).toBeNull();
  });

  it("budgetMeterPct drives the meter off the cost axis for a cost-only cap", () => {
    // The shipped defaults (per_actor_default, worldgen_room) are cost-only:
    // no token cap, so the meter reads the cost axis instead of vanishing.
    expect(
      budgetMeterPct({
        scope: "actor_default",
        inputTokens: 900,
        outputTokens: 300,
        costUsd: 0.5,
        tokensCap: null,
        costCap: 1.0,
        tokensRemaining: null,
        costRemaining: 0.5,
      }),
    ).toBe(50);
    // A token cap still wins when both axes are capped.
    expect(
      budgetMeterPct({
        scope: "global",
        inputTokens: 100,
        outputTokens: 0,
        costUsd: 0.9,
        tokensCap: 1000,
        costCap: 1.0,
        tokensRemaining: 900,
        costRemaining: 0.1,
      }),
    ).toBe(10);
  });

  it("budgetOverCap flags a row at/over its cap", () => {
    expect(
      budgetOverCap({
        scope: "global",
        inputTokens: 1000,
        outputTokens: 0,
        costUsd: 0,
        tokensCap: 1000,
        costCap: null,
        tokensRemaining: 0,
        costRemaining: null,
      }),
    ).toBe(true);
    expect(
      budgetOverCap({
        scope: "global",
        inputTokens: 10,
        outputTokens: 0,
        costUsd: 0,
        tokensCap: 1000,
        costCap: null,
        tokensRemaining: 990,
        costRemaining: null,
      }),
    ).toBe(false);
    // A cost-only scope at its cost cap is also flagged over.
    expect(
      budgetOverCap({
        scope: "worldgen_room",
        inputTokens: 0,
        outputTokens: 0,
        costUsd: 2.0,
        tokensCap: null,
        costCap: 2.0,
        tokensRemaining: null,
        costRemaining: 0,
      }),
    ).toBe(true);
  });
});

describe("daily series -> by-provider bars", () => {
  const series: DaySlice[] = parseDashboardPayload(fullPayload)?.dailySeries ?? [];

  it("latestDay picks the last (most recent) day in the server-sorted series", () => {
    expect(latestDay(series)).toBe("2026-08-29");
    expect(latestDay([])).toBeNull();
  });

  it("providerTotalsForDay folds every feature's row for that day, sorted by cost", () => {
    const totals = providerTotalsForDay(series, "2026-08-29");
    expect(totals).toEqual([
      { provider: "openrouter", inputTokens: 900, outputTokens: 300, costUsd: 0.22, calls: 4 },
      { provider: "anthropic", inputTokens: 200, outputTokens: 100, costUsd: 0.05, calls: 3 },
    ]);
    // Yesterday's row is excluded.
    expect(providerTotalsForDay(series, "2026-08-28")).toEqual([
      { provider: "openrouter", inputTokens: 400, outputTokens: 150, costUsd: 0.1, calls: 2 },
    ]);
    expect(providerTotalsForDay(series, null)).toEqual([]);
    expect(providerTotalsForDay(series, "2099-01-01")).toEqual([]);
  });
});

describe("formatting helpers", () => {
  it("relativeBarPct is proportional to the group max, clamped", () => {
    expect(relativeBarPct(50, 100)).toBe(50);
    expect(relativeBarPct(100, 100)).toBe(100);
    expect(relativeBarPct(0, 100)).toBe(0);
    expect(relativeBarPct(10, 0)).toBe(0); // no group max -> no bar, not Infinity/NaN
  });

  it("formatCost always shows 4 decimals", () => {
    expect(formatCost(0.0421)).toBe("$0.0421");
    expect(formatCost(1)).toBe("$1.0000");
    expect(formatCost(0)).toBe("$0.0000");
  });

  it("formatTokens groups thousands", () => {
    expect(formatTokens(1234)).toBe("1,234");
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(1_000_000)).toBe("1,000,000");
  });

  it("shortDayLabel renders an ISO day as a short month/day", () => {
    expect(shortDayLabel("2026-08-29")).toBe("Aug 29");
    expect(shortDayLabel("2026-01-05")).toBe("Jan 05");
    // Defensive: an unexpected shape passes through rather than throwing.
    expect(shortDayLabel("not-a-date")).toBe("not-a-date");
  });
});
