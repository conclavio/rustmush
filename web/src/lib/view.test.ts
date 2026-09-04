import { describe, expect, it } from "vitest";
import type { ViewEnvelope } from "./proto";
import { classifyViewFrame, cursorAfter, parseResolutionOutcome } from "./view";

const keyframe: ViewEnvelope = {
  view: "legal.actions",
  schema_version: 1,
  generation: 7,
  sequence: 3,
  data: [],
};

describe("semantic view continuity", () => {
  it("accepts keyframes and exact-base deltas", () => {
    expect(classifyViewFrame(keyframe, "legal.actions", 1, null)).toBe("replace");
    const cursor = cursorAfter(keyframe);
    expect(
      classifyViewFrame(
        { ...keyframe, sequence: 4, base_sequence: 3, data: { remove: 0 } },
        "legal.actions",
        1,
        cursor,
      ),
    ).toBe("apply_delta");
  });

  it("requires resync after a gap, generation change, or unknown schema", () => {
    const cursor = cursorAfter(keyframe);
    expect(
      classifyViewFrame(
        { ...keyframe, sequence: 5, base_sequence: 4 },
        "legal.actions",
        1,
        cursor,
      ),
    ).toBe("resync");
    expect(
      classifyViewFrame(
        { ...keyframe, generation: 8, sequence: 4, base_sequence: 3 },
        "legal.actions",
        1,
        cursor,
      ),
    ).toBe("resync");
    expect(classifyViewFrame(keyframe, "legal.actions", 2, cursor)).toBe("resync");
  });

  it("rejects integers JSON cannot represent exactly", () => {
    expect(
      classifyViewFrame(
        { ...keyframe, generation: Number.MAX_SAFE_INTEGER + 1 },
        "legal.actions",
        1,
        null,
      ),
    ).toBe("resync");
  });
});

describe("resolution outcome decoding", () => {
  const commit = "0123456789abcdef0123456789abcdef";
  const outcome: ViewEnvelope = {
    view: "resolution.outcome",
    schema_version: 1,
    generation: Number.parseInt(commit.slice(0, 13), 16),
    sequence: 1,
    commit,
    data: {
      schema_version: 1,
      status: "k5:toy:brace:0000000000000007:01010101010101010101010101010101",
      commit,
      result: {
        kind: "brace",
        summary: "Brace committed.",
        data: { braced: true, revision: 1 },
      },
    },
  };

  it("accepts the exact bounded v1 receipt", () => {
    expect(parseResolutionOutcome(outcome)).toEqual({
      status: "k5:toy:brace:0000000000000007:01010101010101010101010101010101",
      commit,
      result: {
        kind: "brace",
        summary: "Brace committed.",
        data: { braced: true, revision: 1 },
      },
    });
  });

  it("rejects mismatched commits, forged cursors, and control text", () => {
    expect(parseResolutionOutcome({ ...outcome, sequence: 2 })).toBeNull();
    expect(parseResolutionOutcome({ ...outcome, generation: outcome.generation + 1 })).toBeNull();
    expect(
      parseResolutionOutcome({
        ...outcome,
        data: { ...(outcome.data as object), commit: "f".repeat(32) },
      }),
    ).toBeNull();
    expect(
      parseResolutionOutcome({
        ...outcome,
        data: {
          ...(outcome.data as object),
          result: { kind: "brace", summary: "ok\nforged", data: {} },
        },
      }),
    ).toBeNull();
  });
});
