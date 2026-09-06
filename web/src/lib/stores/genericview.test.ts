// GenericViewStore: holds the latest parsed view per `.view` feed, ignores
// a malformed keyframe, and clears (interface plan I9).

import { describe, expect, it } from "vitest";

import { GenericViewStore, isViewFeed, VIEW_FEED_SUFFIX } from "./genericview.svelte";

const listView = {
  schema: { id: "jobs", version: 1, shape: "list", columns: [{ key: "id" }, { key: "title" }] },
  rows: [{ id: 1, title: "Docks" }],
};

describe("GenericViewStore", () => {
  it("recognises a .view feed", () => {
    expect(VIEW_FEED_SUFFIX).toBe(".view");
    expect(isViewFeed("jobs.view")).toBe(true);
    expect(isViewFeed("wod.vitals")).toBe(false);
  });

  it("applies a keyframe, keeps it by feed, and clears", () => {
    const s = new GenericViewStore();
    expect(s.apply("jobs.view", listView)).toBe(true);
    expect(s.get("jobs.view")?.schema.id).toBe("jobs");
    expect(s.get("jobs.view")?.rows).toHaveLength(1);
    expect(s.get("staff.view")).toBeNull();
    expect(s.apply("other.view", listView)).toBe(true);
    expect(s.feeds).toEqual(["jobs.view", "other.view"]);
    s.clear();
    expect(s.get("jobs.view")).toBeNull();
    expect(s.feeds).toEqual([]);
  });

  it("ignores a malformed keyframe rather than throwing", () => {
    const s = new GenericViewStore();
    expect(s.apply("bad.view", { rows: [] })).toBe(false);
    expect(s.apply("bad.view", { schema: { id: "x", version: 1, shape: "grid" } })).toBe(false);
    expect(s.get("bad.view")).toBeNull();
  });
});
