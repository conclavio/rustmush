/** Keep only the source-free trace contract. Unknown fields cannot enter a view. */
export interface MindTrace { available: boolean; unit: number | null; lines: string[] }
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const count = (value: unknown): number | null => Number.isSafeInteger(value) && Number(value) >= 0 ? Number(value) : null;
const text = (value: unknown): string => typeof value === "string" ? [...value].slice(0, 120).join("").replace(/[\u0000-\u001f\u007f]/g, " ") : "";
const rows = (value: unknown, max: number): unknown[] => Array.isArray(value) ? value.slice(0, max) : [];
const truth = (value: unknown): string => value === true ? "yes" : value === false ? "no" : "unknown";
function outcome(value: unknown): string {
  if (!object(value)) return "unknown";
  switch (value.state) {
    case "pending": case "applied": return value.state;
    case "refused": case "cancelled": return `${value.state}: ${text(value.reason)}`;
    case "faulted": return `faulted: ${text(value.fault)}`;
    default: return "unknown";
  }
}
export function decodeMindTrace(value: unknown): MindTrace | null {
  if (!object(value) || !object(value.schema) || value.schema.id !== "arena.mind" || value.schema.version !== 1) return null;
  if (value.available !== true) return { available: false, unit: null, lines: [] };
  const unit = count(value.unit);
  if (unit === null || unit > 0xffffffff) return null;
  const lines: string[] = [];
  let omittedLines = 0;
  const push = (line: string) => { if (lines.length < 512) lines.push(line); else omittedLines++; };
  const omitted = count(value.omitted_thinks);
  if (omitted) push(`${omitted} earlier thinks omitted by the server's payload bound.`);
  for (const raw of rows(value.traces, 64)) {
    if (!object(raw)) continue;
    push(`Think ${count(raw.think_seq) ?? "?"} · ${count(raw.t_ms) ?? "?"} ms · ${raw.rom === true ? "ROM" : "mind"} · ${outcome(raw.outcome)}`);
    if (raw.landed_ms !== null && count(raw.landed_ms) !== null) push(`Landed at ${count(raw.landed_ms)} ms`);
    if (raw.rom_drop === "core" || raw.rom_drop === "strikes") push(`Dropped to ROM: ${raw.rom_drop}`);
    if (object(raw.diagnostics)) {
      const d = raw.diagnostics;
      push(`Operations ${count(d.ops) ?? "unmeasured"}; host fees ${count(d.fees) ?? "unmeasured"}`);
      for (const kernel of rows(d.kernels, 32)) if (object(kernel)) push(`Kernel ${text(kernel.kernel)}: ${count(kernel.fee) ?? "?"} fee`);
      for (const rule of rows(d.rules, 6)) if (object(rule)) {
        push(`S${count(rule.slot) ?? "?"} [${truth(rule.matches)}] ${text(rule.description)}`);
        for (const term of rows(rule.terms, 4)) if (object(term)) push(`  [${truth(term.value)}] ${text(term.description)}`);
      }
      if (object(d.omitted)) for (const key of ["author_lines", "author_chars", "kernel_calls", "rules", "rule_terms", "detail_chars", "intents"]) {
        const amount = count(d.omitted[key]);
        if (amount) push(`Omitted ${amount} ${key.replaceAll("_", " ")}`);
      }
    }
    for (const author of rows(raw.author, 8)) push(`Author: ${text(author)}`);
    for (const intent of rows(raw.intents, 16)) if (object(intent)) push(`${text(intent.action)}${intent.detail ? ` ${text(intent.detail)}` : ""}: ${outcome(intent.outcome)}`);
  }
  if (omittedLines) lines.push(`${omittedLines} additional display lines omitted; request fewer thinks for detail.`);
  return { available: true, unit, lines };
}
