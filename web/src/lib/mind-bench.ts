import { cockpitView, cockpitViewLines, decodeCockpitFrame } from "./cockpit";
import { decodeMindTrace } from "./mind-trace";
import type { MindCard } from "./mind-card";
import { mindReference } from "./mind-bulk";

export interface BenchView {
  state: string; watches: string[];
  picture: { unit: number; event: number; atMs: number; stateHash: string; complete: boolean; lines: string[]; card: MindCard | null; traces: string[]; canEvaluate: boolean; armed: boolean;
    privateRun: boolean; possessed: boolean; timeline: BenchTimeline; refusals: string[]; diff: BenchDiff | null } | null;
}
export type BenchTimeline = { kind: "reference" } | { kind: "fork"; serial: string; parentEvent: number; fromMs: number };
export interface BenchDiff {
  mind: string; fromMs: number; thinks: number; differences: number; omitted: number;
  rows: { thinkSeq: number; atMs: number; referenceIntents: number; candidateIntents: number; candidateFault: boolean }[];
}
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const natural = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const bytes = (value: string) => new TextEncoder().encode(value).length;
const boundedText = (value: unknown, limit: number): value is string => typeof value === "string" && bytes(value) <= limit && !/[\u0000-\u001f\u007f]/.test(value);

function decodeDiff(value: unknown): BenchDiff | null {
  if (!object(value) || !boundedText(value.mind, 256) || !natural(value.from_ms) || !natural(value.thinks)
    || !natural(value.differences) || !natural(value.omitted) || value.differences > value.thinks
    || !Array.isArray(value.rows) || value.rows.length > 64 || value.rows.length + value.omitted !== value.differences) return null;
  const rows: BenchDiff["rows"] = [];
  for (const row of value.rows) {
    if (!object(row) || !natural(row.think_seq) || !natural(row.at_ms) || !natural(row.reference_intents)
      || !natural(row.candidate_intents) || row.reference_intents > 16 || row.candidate_intents > 16
      || typeof row.candidate_fault !== "boolean") return null;
    rows.push({ thinkSeq: row.think_seq, atMs: row.at_ms, referenceIntents: row.reference_intents,
      candidateIntents: row.candidate_intents, candidateFault: row.candidate_fault });
  }
  return { mind: value.mind, fromMs: value.from_ms, thinks: value.thinks, differences: value.differences, omitted: value.omitted, rows };
}

/** Direct connection-only projection. Memory never enters retained UI state. */
export function decodeBenchView(value: unknown): BenchView | null {
  if (!object(value) || !object(value.schema) || value.schema.id !== "mind.bench" || value.schema.version !== 1
    || !["loading", "ready", "running", "evaluating", "closing", "closed"].includes(String(value.state))
    || !Array.isArray(value.watches) || value.watches.length > 8
    || value.watches.some(row => typeof row !== "string" || bytes(row) > 2048 || /[\u0000-\u001f\u007f]/.test(row))) return null;
  const result: BenchView = { state: String(value.state), watches: value.watches as string[], picture: null };
  if (value.state === "closed") return result;
  if (value.picture === null) return result;
  const p = value.picture;
  if (!object(p) || !natural(p.unit) || p.unit > 0xffffffff || !natural(p.event) || !natural(p.at_ms) || !object(p.view)
    || typeof p.state_hash !== "string" || !/^[0-9a-f]{16}$/.test(p.state_hash)
    || !Array.isArray(p.traces) || p.traces.length > 64 || typeof p.private_run !== "boolean" || typeof p.possessed !== "boolean"
    || !object(p.timeline) || !["reference", "fork"].includes(String(p.timeline.kind))
    || !Array.isArray(p.refusals) || p.refusals.length > 8 || p.refusals.some(row => !boundedText(row, 512))) return null;
  let timeline: BenchTimeline = { kind: "reference" };
  if (p.timeline.kind === "fork") {
    if (typeof p.timeline.serial !== "string" || !/^[1-9]\d{0,19}$/.test(p.timeline.serial)
      || BigInt(p.timeline.serial) > 18446744073709551615n || !natural(p.timeline.parent_event) || !natural(p.timeline.from_ms)) return null;
    timeline = { kind: "fork", serial: p.timeline.serial, parentEvent: p.timeline.parent_event, fromMs: p.timeline.from_ms };
  } else if (p.possessed) return null;
  const diff = p.diff === null ? null : decodeDiff(p.diff);
  if (p.diff !== null && diff === null) return null;
  // Reuse the cockpit's closed projection, with bounded input and explicit
  // truncation at the display boundary. No unknown raw fields are retained.
  let omitted = 0;
  const take = (key: string, cap: number) => {
    const list = p.view && object(p.view) ? p.view[key] : null;
    if (!Array.isArray(list)) return [];
    omitted += Math.max(0, list.length - cap);
    return list.slice(0, cap);
  };
  const frame = decodeCockpitFrame({ ...p.view, contacts: take("contacts", 24), weapons: take("weapons", 64),
    sections: take("sections", 64), notices: take("notices", 16), rules: take("rules", 6) });
  if (!frame) return null;
  const all = cockpitViewLines(cockpitView(frame, null, []));
  const lines = all.slice(0, 511).map(line => [...line].slice(0, 512).join(""));
  if (omitted || all.length > 511) lines.push(`Display bounds omitted ${omitted} rows and ${Math.max(0, all.length - 511)} lines.`);
  result.picture = { unit: p.unit, event: p.event, atMs: p.at_ms, stateHash: p.state_hash, complete: p.complete === true, lines,
    card: frame.mind?.card ?? null, traces: decodeMindTrace({ schema: { id: "arena.mind", version: 1 }, available: true, unit: p.unit, traces: p.traces })?.lines ?? [],
    canEvaluate: object(p.memory), armed: frame.weapons.length > 0, privateRun: p.private_run,
    possessed: p.possessed, timeline, refusals: p.refusals as string[], diff };
  return result;
}

export function benchStepCommand(verb: "STEP" | "BACK", count: number): string {
  if (!Number.isInteger(count) || count < 1 || count > 1000) throw new Error("Choose 1–1000 steps.");
  return `BENCH ${verb} ${count}`;
}
export function benchTimeCommand(time: string): string {
  if (!/^\d{1,6}:[0-5]\d$/.test(time)) throw new Error("Enter replay time as minutes:seconds.");
  return `BENCH RUN TO ${time}`;
}
export function benchExpressionCommand(verb: "EVAL" | "WATCH", expression: string): string {
  if (!expression.trim() || bytes(expression) > 2048 || /[\u0000-\u001f\u007f]/.test(expression)) throw new Error("Use one expression of at most 2048 UTF-8 bytes.");
  return `BENCH ${verb} ${expression}`;
}

export function benchCandidateCommand(verb: "DIFF" | "SWAP", reference: string): string {
  return `BENCH ${verb} ${mindReference(reference)}`;
}
export function benchStationCommand(command: string): string {
  if (!command.trim() || bytes(command) > 1024 || /[\u0000-\u001f\u007f]/.test(command)) throw new Error("Use one station command of at most 1024 UTF-8 bytes.");
  return `BENCH DO ${command}`;
}
export function benchActAsCommand(callsign: string): string {
  if (!/^[A-Za-z0-9-]{1,16}$/.test(callsign)) throw new Error("Enter a callsign from this private proving run.");
  return `BENCH ACT AS ${callsign}`;
}
export function benchLoadMatchCommand(match: string, callsign: string): string {
  if (!/^\d{1,20}$/.test(match) || BigInt(match) > 18446744073709551615n) throw new Error("Enter a kept match number.");
  const selected = benchActAsCommand(callsign).slice("BENCH ACT AS ".length);
  return `BENCH LOAD MATCH ${BigInt(match)} ${selected}`;
}
