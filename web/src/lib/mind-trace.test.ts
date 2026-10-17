import { describe, expect, it } from "vitest";
import { decodeMindTrace } from "./mind-trace";
describe("source-free mind trace", () => {
  it("renders actual fees, rules, outcomes and omitted counts while dropping unknown data", () => {
    const trace = decodeMindTrace({ schema: { id: "arena.mind", version: 1 }, available: true, unit: 7, omitted_thinks: 2,
      source: "PRIVATE SOURCE", memory: { private: "PRIVATE MEMORY" }, traces: [{ think_seq: 4, t_ms: 500,
        outcome: { state: "applied" }, diagnostics: { ops: 123, fees: 4, kernels: [{ kernel: "odds", fee: 4 }],
          rules: [{ slot: 1, description: "in range", matches: null, terms: [{ description: "range", value: false }] }],
          omitted: { author_chars: 7, private: "PRIVATE OMITTED" } }, author: ["seek\ncover"],
        intents: [{ action: "drive", detail: "public cell", outcome: { state: "refused", reason: "epoch" }, memory: "PRIVATE INTENT" }] }] });
    const words = trace!.lines.join("\n");
    expect(words).toContain("Operations 123; host fees 4");
    expect(words).toContain("S1 [unknown] in range");
    expect(words).toContain("[no] range");
    expect(words).toContain("drive public cell: refused: epoch");
    expect(words).toContain("Omitted 7 author chars");
    expect(words).toContain("seek cover");
    expect(words).not.toContain("PRIVATE");
  });
  it("revokes a previous view explicitly and bounds the complete display", () => {
    expect(decodeMindTrace({ schema: { id: "arena.mind", version: 1 }, available: false, traces: [{ source: "private" }] })).toEqual({ available: false, unit: null, lines: [] });
    expect(decodeMindTrace({ schema: { id: "arena.mind", version: 2 }, available: true })).toBeNull();
    expect(decodeMindTrace({ schema: { id: "arena.mind", version: 1 }, available: true, unit: 0x100000000 })).toBeNull();
    const trace = decodeMindTrace({ schema: { id: "arena.mind", version: 1 }, available: true, unit: 7,
      traces: Array.from({ length: 80 }, () => ({ author: Array(50).fill("x".repeat(200)), diagnostics: { kernels: Array(80).fill({ kernel: "odds", fee: 4 }) } })) });
    expect(trace!.lines).toHaveLength(513);
    expect(trace!.lines.at(-1)).toContain("additional display lines omitted");
    expect(trace!.lines.some(line => line.includes("x".repeat(121)))).toBe(false);
  });
});
