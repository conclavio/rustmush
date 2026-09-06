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

  // ---- I12 narrative backlog prepend --------------------------------------

  const older = (seq: number, ts = seq * 10) => ({
    seq,
    ts,
    cls: "ev-narrative",
    lines: [[{ text: `n${seq}`, cls: "", css: "", cmd: null }]],
    speaker: null,
    body: null,
    groupable: false,
  });

  it("prependOlder inserts older-first at the head with descending, unique ids", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative", [], 100, { seq: 5 }); // a live block, seq 5
    const added = sb.prependOlder([older(2), older(3), older(4)], true);
    expect(added).toBe(3);
    // Order: the three older (2,3,4) then the live (5).
    expect(sb.entries.map((b) => b.seq)).toEqual([2, 3, 4, 5]);
    // Prepended ids are descending and disjoint from the live positive id.
    const prepIds = sb.entries.slice(0, 3).map((b) => b.id);
    expect(prepIds).toEqual([0, -1, -2]);
    expect(new Set(sb.entries.map((b) => b.id)).size).toBe(4);
    expect(sb.moreBacklog).toBe(true);
    expect(sb.fetchingBacklog).toBe(false);
  });

  it("prependOlder dedups by seq against what is already held", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative", [], 100, { seq: 3 });
    sb.push("ev-narrative", [], 110, { seq: 4 });
    // A page racing a live delivery re-sends seq 3; only seq 1,2 are fresh.
    const added = sb.prependOlder([older(1), older(2), older(3)], false);
    expect(added).toBe(2);
    expect(sb.entries.map((b) => b.seq)).toEqual([1, 2, 3, 4]);
    expect(sb.moreBacklog).toBe(false);
  });

  it("oldestNarrativeSeq is the cursor and ignores seq-less blocks", () => {
    const sb = new ScrollbackStore();
    expect(sb.oldestNarrativeSeq).toBeNull();
    sb.pushText("ev-sys", "no seq");
    expect(sb.oldestNarrativeSeq).toBeNull();
    sb.push("ev-narrative", [], 100, { seq: 7 });
    sb.push("ev-narrative", [], 110, { seq: 9 });
    expect(sb.oldestNarrativeSeq).toBe(7);
    sb.prependOlder([older(4)], true);
    expect(sb.oldestNarrativeSeq).toBe(4);
  });
});

// ---- I12 verify fold: session segments, the cap loop, the prepend generation --

describe("ScrollbackStore — I12 verify fold", () => {
  const older = (seq: number, ts = seq * 10) => ({
    seq,
    ts,
    cls: "ev-narrative",
    lines: [[{ text: `n${seq}`, cls: "", css: "", cmd: null }]],
    speaker: null,
    body: null,
    groupable: false,
  });

  it("beginSegment nulls old seqs, re-arms the backlog, and pages splice at the seam", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative", [], 100, { seq: 40 }); // the old session
    sb.push("ev-narrative", [], 110, { seq: 41 });
    sb.prependOlder([], false); // its ring is exhausted
    expect(sb.moreBacklog).toBe(false);
    sb.beginSegment();
    expect(sb.entries.every((b) => b.seq === null)).toBe(true);
    expect(sb.oldestNarrativeSeq).toBeNull(); // an old seq is never the cursor
    expect(sb.moreBacklog).toBe(true);
    // The new session: a live block (seq 3), then a page (1, 2) that must land
    // at the seam — after the old blocks, before the live tail.
    sb.push("ev-narrative", [], 200, { seq: 3 });
    expect(sb.prependOlder([older(1), older(2)], false)).toBe(2);
    expect(sb.entries.map((b) => b.seq)).toEqual([null, null, 1, 2, 3]);
    expect(sb.oldestNarrativeSeq).toBe(1);
  });

  it("an old-segment seq never dedups a new-session line with the same number", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative", [], 100, { seq: 2 }); // the old session's seq 2
    sb.beginSegment();
    sb.push("ev-narrative", [], 200, { seq: 5 });
    expect(sb.prependOlder([older(2)], false)).toBe(1); // a real, different line
    expect(sb.entries.map((b) => b.seq)).toEqual([null, 2, 5]);
  });

  it("a cap-truncated page ends the backlog instead of looping", () => {
    const sb = new ScrollbackStore(4);
    sb.push("ev-narrative", [], 100, { seq: 10 });
    sb.push("ev-narrative", [], 110, { seq: 11 });
    // room = 2: a page of 3 is truncated -> exhausted even though the server
    // said more; the newest of the batch is what is kept.
    expect(sb.prependOlder([older(7), older(8), older(9)], true)).toBe(2);
    expect(sb.entries.map((b) => b.seq)).toEqual([8, 9, 10, 11]);
    expect(sb.moreBacklog).toBe(false);
    // A full cap holds nothing more: still exhausted, nothing added.
    expect(sb.prependOlder([older(6)], true)).toBe(0);
    expect(sb.moreBacklog).toBe(false);
  });

  it("prependGen bumps only on a real prepend", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative", [], 100, { seq: 5 });
    expect(sb.prependGen).toBe(0);
    sb.prependOlder([], false); // an empty page
    expect(sb.prependGen).toBe(0);
    sb.prependOlder([older(5)], false); // all duplicates
    expect(sb.prependGen).toBe(0);
    sb.prependOlder([older(4)], false);
    expect(sb.prependGen).toBe(1);
  });

  it("prependOlder records the server's evicted flag and clears fetching", () => {
    const sb = new ScrollbackStore();
    sb.fetchingBacklog = true;
    sb.prependOlder([], false, true);
    expect(sb.fetchingBacklog).toBe(false);
    expect(sb.backlogEvicted).toBe(true);
  });

  it("the cap rolling the head off keeps the seam in place", () => {
    const sb = new ScrollbackStore(3);
    sb.push("ev-narrative", [], 100, { seq: 1 });
    sb.beginSegment(); // seam after the one old block
    sb.push("ev-narrative", [], 200, { seq: 10 });
    sb.push("ev-narrative", [], 210, { seq: 11 });
    sb.push("ev-narrative", [], 220, { seq: 12 }); // rolls the old block off
    expect(sb.entries.map((b) => b.seq)).toEqual([10, 11, 12]);
    // The seam moved to 0 with the head: a page (seq 9) lands at the front.
    sb.prependOlder([older(9)], false); // room 0 -> nothing added, exhausted
    expect(sb.entries.map((b) => b.seq)).toEqual([10, 11, 12]);
    expect(sb.moreBacklog).toBe(false);
  });
});
