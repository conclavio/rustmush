import { describe, expect, it } from "vitest";
import type { ViewEnvelope } from "./proto";
import { classifyViewFrame, cursorAfter } from "./view";

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
