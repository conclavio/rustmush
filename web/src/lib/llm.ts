// The staff-only LLM cost dashboard — the `llm.dashboard` panel feed
// (`crates/mush-worldgen/src/commands.rs`'s `dashboard_json`, aggregate
// shape from `crates/mush-llm/src/dashboard.rs`'s `DashboardSnapshot`).
//
// Hand-written types walked defensively at runtime, same convention as
// `sheet.ts`/`worldgen.ts`/`roll.ts`: this is opaque module-panel JSON to
// `mush-proto`, not a ts-rs type, and the aggregate shape is small and
// stable enough that a light interface here (validated at the JSON
// boundary) beats a cross-crate ts-rs derive on a type that lives in
// `mush-llm` for a web-only concern.
//
// **Safety contract carried through from the server (see dashboard.rs's
// module doc and worldgen's `dashboard_json`): this payload NEVER carries
// prompt or response text, a key, or any per-player data beyond the
// actor/NPC references it already needs to attribute spend.** The parse
// functions below only ever pick the specific fields listed here — even if
// a future server response accidentally included a `prompt_summary` or
// similar, it would be silently dropped on the way into `DashboardPayload`,
// never reaching the renderer. `llm.test.ts` asserts exactly that.
//
// ```jsonc
// {
//   "kill_switch_engaged": false,
//   "budgets": [
//     { "scope": "global", "input_tokens": 1200, "output_tokens": 400,
//       "cost_usd": 0.31, "tokens_cap": 5000000, "cost_cap": null,
//       "tokens_remaining": 4998400 },
//     { "scope": "feature:worldgen_room", "input_tokens": 900, … }
//   ],
//   "top_actors": [ { "obj": { "id": "#7", "name": "Alice" }, "input_tokens": 900, … } ],
//   "top_npcs": [ … same shape … ],
//   "daily_series": [
//     { "day": "2026-08-29", "feature": "worldgen_room", "provider": "openrouter",
//       "calls": 4, "refusals": 0, "budget_trips": 0, "disabled": 0, "errors": 0,
//       "input_tokens": 900, "output_tokens": 300, "cost_usd": 0.22 }
//   ],
//   "recent_calls": [
//     { "ts": 1756500000, "feature": "worldgen_room", "provider": "openrouter",
//       "model": "z-ai/glm-5.3", "actor": { "id": "#7", "name": "Alice" }, "npc": null,
//       "input_tokens": 120, "output_tokens": 60, "cost_usd": 0.01, "latency_ms": 1400,
//       "outcome": "ok" }
//   ]
// }
// ```

/** `{id, name}` — the standard `ObjRef` wire shape (see `../lib/proto/ObjRef.ts`;
 * built by hand server-side here rather than reused as a ts-rs import, to
 * keep this module's only dependency the JSON shape itself). */
export interface ObjRefLike {
  id: string;
  name: string;
}

/** One budget scope's today-vs-cap snapshot. */
export interface BudgetStatus {
  scope: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  tokensCap: number | null;
  costCap: number | null;
  tokensRemaining: number | null;
  costRemaining: number | null;
}

/** One builder or NPC's spend today — a "top spenders" row. */
export interface SpendRow {
  obj: ObjRefLike;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
}

/** One UTC-day, one (feature, provider) aggregate slice. */
export interface DaySlice {
  day: string;
  feature: string;
  provider: string;
  calls: number;
  refusals: number;
  budgetTrips: number;
  disabled: number;
  errors: number;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
}

/** One recent call — summary fields only (see the module doc). */
export interface RecentCall {
  ts: number;
  feature: string;
  provider: string;
  model: string;
  actor: ObjRefLike | null;
  npc: ObjRefLike | null;
  inputTokens: number;
  outputTokens: number;
  costUsd: number | null;
  latencyMs: number;
  outcome: string;
}

/** The full `llm.dashboard` panel payload. */
export interface DashboardPayload {
  killSwitchEngaged: boolean;
  budgets: BudgetStatus[];
  topActors: SpendRow[];
  topNpcs: SpendRow[];
  dailySeries: DaySlice[];
  recentCalls: RecentCall[];
}

// ---------------------------------------------------------------------------
// Defensive parsing (the JSON boundary) — only ever reads the named fields
// below; anything else in the payload (there should never be anything else)
// is silently dropped here and never reaches `DashboardPayload`.
// ---------------------------------------------------------------------------

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function num(v: unknown): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

function numOrNull(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function bool(v: unknown): boolean {
  return v === true;
}

function parseObjRef(v: unknown): ObjRefLike | null {
  if (!isObj(v)) {
    return null;
  }
  const id = str(v.id);
  if (id.length === 0) {
    return null;
  }
  return { id, name: str(v.name) };
}

function parseBudgetStatus(v: unknown): BudgetStatus | null {
  if (!isObj(v) || typeof v.scope !== "string") {
    return null;
  }
  return {
    scope: v.scope,
    inputTokens: num(v.input_tokens),
    outputTokens: num(v.output_tokens),
    costUsd: num(v.cost_usd),
    tokensCap: numOrNull(v.tokens_cap),
    costCap: numOrNull(v.cost_cap),
    tokensRemaining: numOrNull(v.tokens_remaining),
    costRemaining: numOrNull(v.cost_remaining),
  };
}

function parseSpendRow(v: unknown): SpendRow | null {
  if (!isObj(v)) {
    return null;
  }
  const obj = parseObjRef(v.obj);
  if (obj === null) {
    return null;
  }
  return {
    obj,
    inputTokens: num(v.input_tokens),
    outputTokens: num(v.output_tokens),
    costUsd: num(v.cost_usd),
  };
}

function parseDaySlice(v: unknown): DaySlice | null {
  if (!isObj(v) || typeof v.day !== "string") {
    return null;
  }
  return {
    day: v.day,
    feature: str(v.feature),
    provider: str(v.provider),
    calls: num(v.calls),
    refusals: num(v.refusals),
    budgetTrips: num(v.budget_trips),
    disabled: num(v.disabled),
    errors: num(v.errors),
    inputTokens: num(v.input_tokens),
    outputTokens: num(v.output_tokens),
    costUsd: num(v.cost_usd),
  };
}

function parseRecentCall(v: unknown): RecentCall | null {
  if (!isObj(v)) {
    return null;
  }
  return {
    ts: num(v.ts),
    feature: str(v.feature),
    provider: str(v.provider),
    model: str(v.model),
    actor: parseObjRef(v.actor),
    npc: parseObjRef(v.npc),
    inputTokens: num(v.input_tokens),
    outputTokens: num(v.output_tokens),
    costUsd: numOrNull(v.cost_usd),
    latencyMs: num(v.latency_ms),
    outcome: str(v.outcome),
  };
}

/** Parse one `llm.dashboard` panel keyframe. `null` when the payload isn't a
 * usable dashboard at all (not an object) — a dashboard with every list
 * empty is still valid (a quiet server), so emptiness alone never fails
 * parsing. */
export function parseDashboardPayload(raw: unknown): DashboardPayload | null {
  if (!isObj(raw)) {
    return null;
  }
  const budgets = Array.isArray(raw.budgets)
    ? raw.budgets.flatMap((b) => parseBudgetStatus(b) ?? [])
    : [];
  const topActors = Array.isArray(raw.top_actors)
    ? raw.top_actors.flatMap((r) => parseSpendRow(r) ?? [])
    : [];
  const topNpcs = Array.isArray(raw.top_npcs)
    ? raw.top_npcs.flatMap((r) => parseSpendRow(r) ?? [])
    : [];
  const dailySeries = Array.isArray(raw.daily_series)
    ? raw.daily_series.flatMap((d) => parseDaySlice(d) ?? [])
    : [];
  const recentCalls = Array.isArray(raw.recent_calls)
    ? raw.recent_calls.flatMap((c) => parseRecentCall(c) ?? [])
    : [];
  return {
    killSwitchEngaged: bool(raw.kill_switch_engaged),
    budgets,
    topActors,
    topNpcs,
    dailySeries,
    recentCalls,
  };
}

// ---------------------------------------------------------------------------
// Pure rendering helpers (bars/meters/table shaping) — kept out of the
// Svelte component so they're directly unit-testable, same convention as
// `sheet.ts`'s `barPct`/`pipRow`.
// ---------------------------------------------------------------------------

/** Whether a budget row is a per-feature one (as opposed to `"global"`,
 * `"actor_default"`, or `"npc_default"`). */
export function isFeatureBudget(b: BudgetStatus): boolean {
  return b.scope.startsWith("feature:");
}

/** `"feature:worldgen_room"` -> `"worldgen_room"`; any other scope label
 * passes through unchanged. */
export function featureLabel(scope: string): string {
  return scope.startsWith("feature:") ? scope.slice("feature:".length) : scope;
}

/** A human label for the fixed non-feature scopes; passes feature/global
 * scopes through `featureLabel` unchanged. */
export function scopeLabel(scope: string): string {
  switch (scope) {
    case "actor_default":
      return "per-builder default";
    case "npc_default":
      return "per-NPC default";
    default:
      return featureLabel(scope);
  }
}

/** A budget meter's fill percentage (0..100, clamped) against whichever axis
 * the scope is capped on; `null` when the scope has no cap at all (render "no
 * cap" instead of a meter). A token cap drives the meter when present;
 * otherwise a cost cap does — the shipped example's most important caps are
 * cost-only (`per_actor_default`, `worldgen_room`), and those scopes must still
 * show an approach-the-cap meter (M10c), not render as bare numbers. */
export function budgetMeterPct(b: BudgetStatus): number | null {
  if (b.tokensCap !== null && b.tokensCap > 0) {
    const used = b.inputTokens + b.outputTokens;
    return Math.max(0, Math.min(100, Math.round((used / b.tokensCap) * 100)));
  }
  if (b.costCap !== null && b.costCap > 0) {
    return Math.max(0, Math.min(100, Math.round((b.costUsd / b.costCap) * 100)));
  }
  return null;
}

/** Whether a meter is at or past its cap (styling hook — render it as a
 * warning/danger fill). */
export function budgetOverCap(b: BudgetStatus): boolean {
  const pct = budgetMeterPct(b);
  return pct !== null && pct >= 100;
}

/** The most recent UTC day present in a daily series (the server sorts
 * oldest-first), or `null` for an empty series — "today" for the
 * by-provider bars, which have no other source (the budget-vs-cap rows
 * don't carry a provider dimension). */
export function latestDay(series: DaySlice[]): string | null {
  return series.at(-1)?.day ?? null;
}

/** One provider's totals for a single day, folded across every feature
 * that used it that day. */
export interface ProviderTotal {
  provider: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  calls: number;
}

/** Fold a daily series' rows for `day` into per-provider totals, sorted by
 * cost descending (ties by tokens). Empty when `day` is `null` or matches
 * nothing. */
export function providerTotalsForDay(series: DaySlice[], day: string | null): ProviderTotal[] {
  if (day === null) {
    return [];
  }
  const byProvider = new Map<string, ProviderTotal>();
  for (const s of series) {
    if (s.day !== day) {
      continue;
    }
    const cur = byProvider.get(s.provider) ?? {
      provider: s.provider,
      inputTokens: 0,
      outputTokens: 0,
      costUsd: 0,
      calls: 0,
    };
    cur.inputTokens += s.inputTokens;
    cur.outputTokens += s.outputTokens;
    cur.costUsd += s.costUsd;
    cur.calls += s.calls;
    byProvider.set(s.provider, cur);
  }
  return [...byProvider.values()].sort(
    (a, b) =>
      b.costUsd - a.costUsd || b.inputTokens + b.outputTokens - (a.inputTokens + a.outputTokens),
  );
}

/** A bar's fill percentage (0..100) relative to the largest value in its
 * group — the shared shape for both the by-feature and by-provider bar
 * charts (each computes its own group max and calls this per row). */
export function relativeBarPct(value: number, max: number): number {
  if (max <= 0) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round((value / max) * 100)));
}

/** `$0.0421` — 4 decimal places (LLM per-call costs are often sub-cent;
 * 2 decimals would round most of them to `$0.00`). */
export function formatCost(usd: number): string {
  return `$${usd.toFixed(4)}`;
}

/** `1,234` — thousands-grouped token counts. */
export function formatTokens(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

/** `"Aug 29"`-style label from an ISO `"YYYY-MM-DD"` day string, for chart
 * axis ticks; the raw string passes through unchanged if it isn't the
 * expected shape (defensive, never throws). */
export function shortDayLabel(day: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) {
    return day;
  }
  const months = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  const monthIdx = Number(m[2]) - 1;
  return `${months[monthIdx] ?? m[2]} ${m[3]}`;
}
