/** These controls prepare readable commands; the server resolves all authority and scenarios. */
export const mindReference = (value: string): string => {
  if (!value || new TextEncoder().encode(value).length > 256 || /[\u0000-\u001f\u007f]/.test(value)) throw new Error("Use a mind reference of at most 256 bytes without control characters.");
  return JSON.stringify(value);
};
export function proveCommand(own: string, opponent: string, pairs: number, scenario: string, red: number, blue: number, seed: string): string {
  if (!Number.isInteger(pairs) || pairs < 10 || pairs > 100 || pairs % 2 !== 0) throw new Error("Choose an even match count from 10–100; each seed is flown from both sides.");
  if (!["DUEL", "MUSTER", "HOLD"].includes(scenario)) throw new Error("Choose a proving scenario.");
  if (![red, blue].every(value => Number.isInteger(value) && value >= 1 && value <= 6)) throw new Error("Choose 1–6 units per side.");
  if (red !== blue) throw new Error("Use equal sides from 1v1 through 6v6.");
  if (!/^\d{1,20}$/.test(seed) || BigInt(seed) > 18446744073709551615n) throw new Error("Seed must be an unsigned 64-bit integer.");
  return `@mind/prove ${mindReference(own)} VS ${mindReference(opponent)} ${pairs} ${scenario} ${red}v${blue} SEED ${BigInt(seed)}`;
}
export function benchLoadCommand(run: string, game: number): string {
  if (!/^\d{1,20}$/.test(run) || BigInt(run) > 18446744073709551615n || !Number.isInteger(game) || game < 1 || game > 100) throw new Error("Enter a run number and game 1–100.");
  return `BENCH LOAD ${BigInt(run)}/${game}`;
}

export interface ProvingRun { run: number; state: string; games: number; scenario: string; perSide: number; charged: number; canCancel: boolean; canForget: boolean }
export interface ProvingContext {
  scenario: string; ground: string; chassis: string; scenarioDigest: string; groundDigest: string; configDigest: string;
  startupMs: number; maxDurationMs: number; substepMs: number;
  candidate: { class: string; profile: string; digest: string };
  opponent: { class: string; profile: string; digest: string }; seeds: string[];
}
export interface ProvingView { runs: ProvingRun[]; report: { run: number; page: number; pages: number; lines: string[]; context: ProvingContext | null } | null }
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const natural = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const label = (value: unknown): value is string => typeof value === "string" && value.length > 0 && new TextEncoder().encode(value).length <= 128 && !/[\u0000-\u001f\u007f]/.test(value);
const digest = (value: unknown): value is string => typeof value === "string" && /^[0-9a-fA-F]{64}$/.test(value);
function decodeContext(value: unknown): ProvingContext | null {
  if (!object(value) || !["duel", "muster", "hold"].includes(String(value.scenario))
    || !label(value.ground) || !label(value.chassis) || !digest(value.scenario_content_sha256) || !digest(value.ground_sha256) || !digest(value.config_sha256)
    || !natural(value.startup_ms) || !natural(value.max_duration_ms) || value.max_duration_ms === 0 || !natural(value.native_substep_ms) || value.native_substep_ms === 0
    || !object(value.candidate) || !object(value.opponent) || !Array.isArray(value.pair_seeds) || value.pair_seeds.length < 5 || value.pair_seeds.length > 50) return null;
  for (const selection of [value.candidate, value.opponent]) {
    if (!["0", "I", "II", "III"].includes(String(selection.class)) || !label(selection.profile) || !digest(selection.source_sha256)) return null;
  }
  const seeds: string[] = [];
  for (const seed of value.pair_seeds) {
    if (typeof seed !== "string" || !/^\d{1,20}$/.test(seed) || BigInt(seed) > 18446744073709551615n) return null;
    seeds.push(BigInt(seed).toString());
  }
  return { scenario: String(value.scenario), ground: value.ground, chassis: value.chassis, scenarioDigest: value.scenario_content_sha256,
    groundDigest: value.ground_sha256, configDigest: value.config_sha256, startupMs: value.startup_ms, maxDurationMs: value.max_duration_ms,
    substepMs: value.native_substep_ms, candidate: { class: String(value.candidate.class), profile: String(value.candidate.profile), digest: String(value.candidate.source_sha256) },
    opponent: { class: String(value.opponent.class), profile: String(value.opponent.profile), digest: String(value.opponent.source_sha256) }, seeds };
}
export function decodeProvingView(value: unknown): ProvingView | null {
  if (!object(value) || !object(value.schema) || value.schema.id !== "mind.bulk" || value.schema.version !== 1 || !Array.isArray(value.runs) || value.runs.length > 100) return null;
  const runs: ProvingRun[] = [];
  for (const row of value.runs) {
    if (!object(row) || !natural(row.run) || !natural(row.games) || !natural(row.per_side) || !natural(row.charged_unit_ms)
      || !["queued", "running", "cancel_requested", "complete", "cancelled", "interrupted", "failed"].includes(String(row.state))
      || !["duel", "muster", "hold"].includes(String(row.scenario))) return null;
    runs.push({ run: row.run, state: String(row.state), games: row.games, scenario: String(row.scenario), perSide: row.per_side,
      charged: row.charged_unit_ms, canCancel: row.can_cancel === true, canForget: row.can_forget === true });
  }
  const r = value.report;
  let report: ProvingView["report"] = null;
  if (r !== null) {
    if (!object(r) || !natural(r.run) || !natural(r.page) || !natural(r.pages) || r.page < 1 || r.page > r.pages
      || !Array.isArray(r.lines) || r.lines.length > 15 || r.lines.some(line => typeof line !== "string" || new TextEncoder().encode(line).length > 512 || /[\u0000-\u001f\u007f]/.test(line))) return null;
    const context = r.context == null ? null : decodeContext(r.context);
    if (r.context != null && context === null) return null;
    report = { run: r.run, page: r.page, pages: r.pages, lines: r.lines as string[], context };
  }
  return { runs, report };
}
