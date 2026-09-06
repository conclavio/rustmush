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

  it("replaces a duplicate id rather than stacking it", () => {
    const s = new RequestStore();
    s.push(choice(1));
    s.push(choice(2));
    s.push({ ...choice(1), prompt: [{ text: "again" }] });
    expect(s.pending.map((r) => r.id)).toEqual([2, 1]);
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
