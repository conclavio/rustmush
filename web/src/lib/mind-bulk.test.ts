import { describe, expect, it } from "vitest";
import { proveCommand, benchLoadCommand, decodeProvingView } from "./mind-bulk";
describe("explicit proving command preparation", () => {
  it("quotes references exactly and preserves full unsigned seed precision", () => {
    expect(proveCommand('my "patrol"@7/line with spaces', 'Standard/Hunter', 30, 'MUSTER', 2, 2, '18446744073709551615'))
      .toBe('@mind/prove "my \\"patrol\\"@7/line with spaces" VS "Standard/Hunter" 30 MUSTER 2v2 SEED 18446744073709551615');
    expect(benchLoadCommand('7', 2)).toBe('BENCH LOAD 7/2');
  });
  it("refuses malformed control inputs before preparing anything", () => {
    expect(() => proveCommand('mine\n@destroy', 'mirror', 30, 'MUSTER', 1, 1, '1')).toThrow();
    expect(() => proveCommand('mine', 'mirror', 101, 'MUSTER', 1, 1, '1')).toThrow();
    expect(() => proveCommand('mine', 'mirror', 31, 'MUSTER', 1, 1, '1')).toThrow();
    expect(() => proveCommand('mine', 'mirror', 30, 'MUSTER', 1, 2, '1')).toThrow();
    expect(() => proveCommand('mine', 'mirror', 30, 'MUSTER', 7, 1, '1')).toThrow();
    expect(() => proveCommand('mine', 'mirror', 30, 'MUSTER', 1, 1, '18446744073709551616')).toThrow();
    expect(() => benchLoadCommand('7/PIC', 1)).toThrow();
  });
  it("keeps only bounded source-free run/report fields", () => {
    const raw = { schema: { id: 'mind.bulk', version: 1 }, source: 'private', runs: [{ run: 2, state: 'complete', games: 60,
      scenario: 'muster', per_side: 2, charged_unit_ms: 12000, can_cancel: false, can_forget: true, source: 'private' }],
      report: { run: 2, page: 1, pages: 2, lines: ['wins 12; draws 4; losses 14'], source: 'private' } };
    const view = decodeProvingView(raw);
    expect(view?.runs[0]?.charged).toBe(12000);
    expect(view?.report?.lines).toEqual(raw.report.lines);
    expect(JSON.stringify(view)).not.toContain('private');
    expect(decodeProvingView({ ...raw, report: { ...raw.report, page: 0 } })).toBeNull();
    expect(decodeProvingView({ ...raw, report: { ...raw.report, lines: ['x'.repeat(513)] } })).toBeNull();
    expect(decodeProvingView({ ...raw, runs: Array(101).fill(raw.runs[0]) })).toBeNull();
  });
  it("preserves frozen seed precision and omits unexpected context fields", () => {
    const digest = 'a'.repeat(64);
    const context = { scenario: 'muster', ground: 'dunes', chassis: 'line', scenario_content_sha256: digest, ground_sha256: digest,
      config_sha256: digest, startup_ms: 1000, max_duration_ms: 900000, native_substep_ms: 100,
      candidate: { class: 'II', profile: 'Patrol', source_sha256: digest, source: 'secret code' },
      opponent: { class: 'I', profile: 'Line', source_sha256: digest }, pair_seeds: ['18446744073709551615', '2', '3', '4', '5'], source: 'secret code' };
    const raw = { schema: { id: 'mind.bulk', version: 1 }, runs: [], report: { run: 1, page: 1, pages: 1, context, lines: [] } };
    const decoded = decodeProvingView(raw);
    expect(decoded?.report?.context?.seeds[0]).toBe('18446744073709551615');
    expect(JSON.stringify(decoded)).not.toContain('secret code');
    expect(decodeProvingView({ ...raw, report: { ...raw.report, context: { ...context, pair_seeds: [Number.MAX_SAFE_INTEGER + 1, '2', '3', '4', '5'] } } })).toBeNull();
  });
});
