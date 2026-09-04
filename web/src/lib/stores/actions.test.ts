import { describe, expect, it } from "vitest";
import type { ViewEnvelope } from "../proto";
import { LegalActionsStore } from "./actions.svelte";

function keyframe(sequence: number): ViewEnvelope {
  return {
    view: "legal.actions",
    schema_version: 1,
    generation: 7,
    sequence,
    data: [
      {
        offer: "opaque-token",
        subject: { kind: "self_actor" },
        parameters: { parameters: [] },
        targets: { kind: "none" },
        required_channels: [],
        presentation: { verb: "brace", label: "Brace", help: "Adopt a stance." },
      },
    ],
  };
}

describe("LegalActionsStore", () => {
  it("keeps only the public projected action shape", () => {
    const store = new LegalActionsStore();
    expect(store.apply(keyframe(1))).toBe("applied");
    expect(store.actions).toHaveLength(1);
    expect(store.actions[0]!.presentation.verb).toBe("brace");
    expect(store.actions[0]!.offer).toBe("opaque-token");
  });

  it("drops stale controls and coalesces resync after a delta gap", () => {
    const store = new LegalActionsStore();
    store.apply(keyframe(1));
    const gap: ViewEnvelope = {
      ...keyframe(3),
      base_sequence: 2,
      data: { remove: 0 },
    };
    expect(store.apply(gap)).toBe("resync");
    expect(store.actions).toEqual([]);
    expect(store.resyncPending).toBe(true);
    expect(store.requireResync()).toBe(false);
    expect(store.apply(keyframe(4))).toBe("applied");
    expect(store.resyncPending).toBe(false);
  });
});
