// Server-initiated interaction (interface plan I10, D-I-21): the pending
// `Request` events the server has posed, front to back. The server owns the
// effect; the client only reports the viewer's choice by the request id, or
// lets the request expire (the server applies its own default). This store
// holds the queue; `GameApp.respond` sends the answer and removes it.

import type { RequestKind, RequestOption, RichText } from "../proto";

/** The most options a Choice request renders; mirrors the server's
 * `MAX_REQUEST_OPTIONS` (a Rust const, not a ts-rs type). */
export const MAX_REQUEST_OPTIONS = 32;

/** One pending server request. */
export interface PendingRequest {
  id: number;
  kind: RequestKind;
  prompt: RichText;
  options: RequestOption[];
  /** Unix ms when it expires, or `null` for the server's default. */
  deadlineMs: number | null;
}

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

const KINDS: readonly RequestKind[] = ["confirm", "choice", "target", "text"];

/** Parse a `request` envelope's payload into a `PendingRequest`, or `null`
 * when it is malformed (options clamped to the cap). */
export function parseRequest(d: unknown): PendingRequest | null {
  if (!isObj(d) || typeof d.id !== "number" || typeof d.kind !== "string") {
    return null;
  }
  const kind = KINDS.find((k) => k === d.kind);
  if (!kind || !Array.isArray(d.prompt)) {
    return null;
  }
  const options: RequestOption[] = Array.isArray(d.options)
    ? d.options
        .flatMap((o) =>
          isObj(o) && typeof o.id === "string" && typeof o.label === "string"
            ? [{ id: o.id, label: o.label }]
            : [],
        )
        .slice(0, MAX_REQUEST_OPTIONS)
    : [];
  return {
    id: d.id,
    kind,
    prompt: d.prompt as RichText,
    options,
    deadlineMs: typeof d.deadline_ms === "number" ? d.deadline_ms : null,
  };
}

export class RequestStore {
  pending = $state<PendingRequest[]>([]);

  /** The request to show now (the oldest outstanding one). */
  get current(): PendingRequest | null {
    return this.pending[0] ?? null;
  }

  /** Enqueue a parsed request; a duplicate id replaces the old one IN PLACE
   * (D-I-29 — the rule the SSH TUI overlay already follows). Position is
   * part of the contract: a re-sent id is the same ask restated, so it must
   * not be demoted behind asks the player has not seen yet, and after a
   * world restart the id counter remints from 1, where a collision means the
   * front entry is a zombie the fresh ask should REPLACE, not queue behind. */
  push(d: unknown): boolean {
    const req = parseRequest(d);
    if (!req) {
      return false;
    }
    this.pending = this.pending.some((r) => r.id === req.id)
      ? this.pending.map((r) => (r.id === req.id ? req : r))
      : [...this.pending, req];
    return true;
  }

  /** Remove the request with `id` (answered or expired). */
  remove(id: number): void {
    this.pending = this.pending.filter((r) => r.id !== id);
  }

  /** Drop every request whose deadline has passed; the server applies its
   * own default for each. Returns the ids dropped. */
  expireDue(now = Date.now()): number[] {
    const due = this.pending.filter((r) => r.deadlineMs !== null && r.deadlineMs <= now);
    if (due.length > 0) {
      const ids = new Set(due.map((r) => r.id));
      this.pending = this.pending.filter((r) => !ids.has(r.id));
    }
    return due.map((r) => r.id);
  }

  clear(): void {
    this.pending = [];
  }
}
