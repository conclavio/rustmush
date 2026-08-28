// HistoryBuffer: readline-style up/down with draft preservation.

import { describe, expect, it } from "vitest";

import { HistoryBuffer } from "./history";

describe("HistoryBuffer", () => {
  it("browses up through history and back down to the draft", () => {
    const h = new HistoryBuffer();
    h.push("look");
    h.push("say hi");

    expect(h.up("wip")).toBe("say hi");
    expect(h.up("say hi")).toBe("look");
    expect(h.up("look")).toBe("look"); // pinned at the oldest
    expect(h.down()).toBe("say hi");
    expect(h.down()).toBe("wip"); // back to the draft
    expect(h.down()).toBeNull(); // not browsing any more
  });

  it("returns null with no history", () => {
    const h = new HistoryBuffer();
    expect(h.up("x")).toBeNull();
    expect(h.down()).toBeNull();
  });

  it("collapses consecutive duplicates and skips empty lines", () => {
    const h = new HistoryBuffer();
    h.push("look");
    h.push("look");
    h.push("");
    h.push("who");
    expect(h.lines).toEqual(["look", "who"]);
  });

  it("caps at the configured size", () => {
    const h = new HistoryBuffer(3);
    for (const l of ["a", "b", "c", "d", "e"]) {
      h.push(l);
    }
    expect(h.lines).toEqual(["c", "d", "e"]);
  });

  it("push resets the browse cursor", () => {
    const h = new HistoryBuffer();
    h.push("one");
    h.push("two");
    expect(h.up("")).toBe("two");
    h.push("three");
    expect(h.up("")).toBe("three");
  });
});
