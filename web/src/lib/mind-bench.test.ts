import { describe, expect, it } from "vitest";
import { benchExpressionCommand, benchStepCommand, benchTimeCommand, decodeBenchView, benchCandidateCommand, benchStationCommand, benchActAsCommand, benchLoadMatchCommand } from "./mind-bench";
import frame from "./fixtures/cockpit/cockpit_frame.json";

const packet = () => ({ schema: { id: "mind.bench", version: 1 }, state: "ready", watches: ["pic.speed_kph"],
  picture: { unit: 7, event: 3, at_ms: 1000, state_hash: "0123456789abcdef", complete: false, view: { ...frame, source: "hidden source" },
    private_run: true, possessed: false, timeline: { kind: "reference" }, refusals: [], diff: null,
    card: { private: "hidden raw card" }, memory: { private: "hidden memory" }, traces: [] } });
describe("private replay bench projection", () => {
  it("reuses the real cockpit picture without retaining raw memory or unknown fields", () => {
    const result = decodeBenchView(packet());
    expect(result?.picture?.atMs).toBe(1000);
    expect(result?.picture?.stateHash).toBe("0123456789abcdef");
    expect(result?.picture?.canEvaluate).toBe(true);
    expect(result?.picture?.lines.length).toBeGreaterThan(5);
    expect(JSON.stringify(result)).not.toContain("hidden");
  });
  it("clears a closed session and refuses malformed or excessive transport fields", () => {
    expect(decodeBenchView({ ...packet(), state: "closed" })?.picture).toBeNull();
    expect(decodeBenchView({ ...packet(), watches: Array(9).fill("1") })).toBeNull();
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, traces: Array(65).fill({}) } })).toBeNull();
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, at_ms: -1 } })).toBeNull();
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, state_hash: "not a hash" } })).toBeNull();
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, unit: 0x100000000 } })).toBeNull();
  });
  it("prepares bounded commands and refuses line injection", () => {
    expect(benchStepCommand("BACK", 2)).toBe("BENCH BACK 2");
    expect(benchTimeCommand("1:30")).toBe("BENCH RUN TO 1:30");
    expect(benchExpressionCommand("EVAL", "mem.state")).toBe("BENCH EVAL mem.state");
    expect(() => benchStepCommand("STEP", 1001)).toThrow();
    expect(() => benchTimeCommand("1:99")).toThrow();
    expect(() => benchExpressionCommand("WATCH", "1\n@mind/new wrong")).toThrow();
    expect(() => benchExpressionCommand("EVAL", "é".repeat(1025))).toThrow();
  });
  it("labels forks and bounds comparison rows without retaining unknown candidate data", () => {
    const diff = { mind: "mine@2/Line", from_ms: 1000, thinks: 2, differences: 1, omitted: 0,
      rows: [{ think_seq: 4, at_ms: 1500, reference_intents: 1, candidate_intents: 2, candidate_fault: false, source: "hidden" }] };
    const withDiff = { ...packet(), picture: { ...packet().picture, diff } };
    expect(decodeBenchView(withDiff)?.picture?.diff?.rows[0]?.candidateIntents).toBe(2);
    expect(JSON.stringify(decodeBenchView(withDiff))).not.toContain("hidden");
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, diff: { ...diff, rows: Array(65).fill(diff.rows[0]) } } })).toBeNull();
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, diff: { ...diff, differences: 3 } } })).toBeNull();
    const fork = { ...packet(), picture: { ...packet().picture, possessed: true,
      timeline: { kind: "fork", serial: "18446744073709551615", parent_event: 3, from_ms: 1000 } } };
    expect(decodeBenchView(fork)?.picture?.timeline).toEqual({ kind: "fork", serial: "18446744073709551615", parentEvent: 3, fromMs: 1000 });
    expect(decodeBenchView({ ...packet(), picture: { ...packet().picture, possessed: true } })).toBeNull();
    expect(decodeBenchView({ ...fork, picture: { ...fork.picture, refusals: Array(9).fill("Refused") } })).toBeNull();
  });
  it("prepares private experiments with exact references and no line injection", () => {
    expect(benchCandidateCommand("SWAP", 'my "patrol"@3/Line')).toBe('BENCH SWAP "my \\"patrol\\"@3/Line"');
    expect(benchActAsCommand("RA")).toBe("BENCH ACT AS RA");
    expect(benchLoadMatchCommand("18446744073709551615", "RA")).toBe("BENCH LOAD MATCH 18446744073709551615 RA");
    expect(benchStationCommand("SPEED 20")).toBe("BENCH DO SPEED 20");
    expect(() => benchCandidateCommand("DIFF", "mine\n@destroy")).toThrow();
    expect(() => benchActAsCommand("RA RELEASE")).toThrow();
    expect(() => benchStationCommand("SPEED 20\nFIRE ALL")).toThrow();
    expect(() => benchStationCommand("é".repeat(513))).toThrow();
    expect(() => benchLoadMatchCommand("18446744073709551616", "RA")).toThrow();
  });
});
