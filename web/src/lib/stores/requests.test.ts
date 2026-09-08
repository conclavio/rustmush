// RequestStore (interface plan I10): parsing, the pending queue, duplicate
// replacement, removal on answer, and deadline expiry.

import { describe, expect, it } from "vitest";

import { parseRequest, RequestStore } from "./requests.svelte";

const choice = (id: number, deadline?: number) => ({
  id,
  kind: "choice",
  prompt: [{ text: "Aim at whom?" }],
  options: [
    { id: "razor", label: "Razor" },
    { id: "mara", label: "Mara" },
  ],
  ...(deadline !== undefined ? { deadline_ms: deadline } : {}),
});

describe("parseRequest", () => {
  it("parses a well-formed request and clamps options", () => {
    const many = Array.from({ length: 50 }, (_, i) => ({ id: `o${i}`, label: `O${i}` }));
    const r = parseRequest({ id: 1, kind: "choice", prompt: [{ text: "?" }], options: many });
    expect(r?.kind).toBe("choice");
    expect(r?.options.length).toBe(32);
    expect(r?.deadlineMs).toBeNull();
  });

  it("keeps a deadline and empty options for a confirm", () => {
    const r = parseRequest({ id: 2, kind: "confirm", prompt: [{ text: "Spend WP?" }], deadline_ms: 999 });
    expect(r?.kind).toBe("confirm");
    expect(r?.options).toEqual([]);
    expect(r?.deadlineMs).toBe(999);
  });

  it("rejects a malformed request", () => {
    expect(parseRequest({ id: 1, kind: "grid", prompt: [] })).toBeNull();
    expect(parseRequest({ id: 1, kind: "text" })).toBeNull();
    expect(parseRequest("no")).toBeNull();
  });
});

describe("RequestStore", () => {
  it("queues requests front-to-back and answers by id", () => {
    const s = new RequestStore();
    expect(s.current).toBeNull();
    expect(s.push(choice(1))).toBe(true);
    expect(s.push(choice(2))).toBe(true);
    expect(s.current?.id).toBe(1);
    s.remove(1);
    expect(s.current?.id).toBe(2);
    s.remove(2);
    expect(s.current).toBeNull();
  });

  it("keeps the oldest as current when a second arrives, and advances on answer", () => {
    // The cross-client contract (interface plan I10): a second request never
    // displaces the one the player is already looking at, and answering the
    // first promotes the next in line. The SSH TUI's request overlay is being
    // changed to match this; pinned here so the web half cannot drift.
    const s = new RequestStore();
    s.push(choice(1));
    s.push(choice(2));
    expect(s.current?.id).toBe(1);
    s.remove(1);
    expect(s.current?.id).toBe(2);
  });

  it("replaces a duplicate id IN PLACE rather than stacking or demoting it", () => {
    // D-I-29's position rule, shared with the SSH TUI overlay: a re-sent id
    // is the same ask restated, so it keeps its place in the queue — the
    // player looking at request 1 sees the new prompt, not request 2. (The
    // rule earns its keep on a remint collision after a world restart: the
    // fresh ask replaces the zombie at the front instead of queueing behind
    // it.) The web store used to demote to the back — `[2, 1]` — which is
    // the drift this pin exists to stop.
    const s = new RequestStore();
    s.push(choice(1));
    s.push(choice(2));
    s.push({ ...choice(1), prompt: [{ text: "again" }] });
    expect(s.pending.map((r) => r.id)).toEqual([1, 2]);
    expect(s.current?.prompt).toEqual([{ text: "again" }]);
  });

  it("ignores a malformed push", () => {
    const s = new RequestStore();
    expect(s.push({ id: 1, kind: "grid" })).toBe(false);
    expect(s.pending).toEqual([]);
  });

  it("expires requests past their deadline and leaves the rest", () => {
    const s = new RequestStore();
    s.push(choice(1, 1000));
    s.push(choice(2, 5000));
    s.push(choice(3)); // no deadline
    expect(s.expireDue(2000)).toEqual([1]);
    expect(s.pending.map((r) => r.id)).toEqual([2, 3]);
    expect(s.expireDue(9000)).toEqual([2]);
    expect(s.pending.map((r) => r.id)).toEqual([3]);
    s.clear();
    expect(s.pending).toEqual([]);
  });
});

it("a new epoch discards all queued offers from the prior engine", () => {
  const store = new RequestStore();
  store.push({...choice(1), epoch:"old"});
  store.push({...choice(2), epoch:"old"});
  store.push({...choice(1), epoch:"new"});
  expect(store.pending.map(r=>[r.id,r.epoch])).toEqual([[1,"new"]]);
});
