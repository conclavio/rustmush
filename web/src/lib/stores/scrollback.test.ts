// ScrollbackStore: ring cap, the simple virtualization window, and the
// scroll-lock / unseen-pill accounting.

import { describe, expect, it } from "vitest";

import { ScrollbackStore, WINDOW_STEP } from "./scrollback.svelte";

function fill(sb: ScrollbackStore, n: number): void {
  for (let i = 0; i < n; i++) {
    sb.pushText("ev-sys", `line ${i}`);
  }
}

describe("ScrollbackStore", () => {
  it("caps the ring and keeps the newest entries", () => {
    const sb = new ScrollbackStore(10);
    fill(sb, 25);
    expect(sb.entries).toHaveLength(10);
    expect(sb.entries[0]!.lines[0]![0]!.text).toBe("line 15");
    expect(sb.entries[9]!.lines[0]![0]!.text).toBe("line 24");
  });

  it("renders only the trailing window, expandable via showOlder", () => {
    const sb = new ScrollbackStore();
    fill(sb, WINDOW_STEP + 50);
    expect(sb.visible).toHaveLength(WINDOW_STEP);
    expect(sb.hiddenCount).toBe(50);
    expect(sb.visible[0]!.lines[0]![0]!.text).toBe("line 50");

    sb.showOlder();
    expect(sb.visible).toHaveLength(WINDOW_STEP + 50);
    expect(sb.hiddenCount).toBe(0);
  });

  it("showOlder never exceeds the entry count", () => {
    const sb = new ScrollbackStore();
    fill(sb, 5);
    sb.showOlder();
    expect(sb.windowSize).toBeGreaterThanOrEqual(5);
    expect(sb.visible).toHaveLength(5);
  });

  it("counts unseen pushes while unpinned; pinning clears them", () => {
    const sb = new ScrollbackStore();
    fill(sb, 3);
    expect(sb.unseen).toBe(0); // pinned by default

    sb.setPinned(false);
    fill(sb, 4);
    expect(sb.unseen).toBe(4);

    sb.setPinned(true);
    expect(sb.unseen).toBe(0);
    fill(sb, 2);
    expect(sb.unseen).toBe(0);
  });

  it("clear resets entries, window and unseen", () => {
    const sb = new ScrollbackStore();
    fill(sb, 10);
    sb.setPinned(false);
    fill(sb, 2);
    sb.showOlder();
    sb.clear();
    expect(sb.entries).toHaveLength(0);
    expect(sb.unseen).toBe(0);
    expect(sb.windowSize).toBe(WINDOW_STEP);
  });

  it("blocks carry monotonically increasing ids (stable render keys)", () => {
    const sb = new ScrollbackStore(3);
    fill(sb, 6);
    const ids = sb.entries.map((b) => b.id);
    expect(ids).toEqual([...ids].sort((a, z) => a - z));
    expect(new Set(ids).size).toBe(ids.length);
  });
});
