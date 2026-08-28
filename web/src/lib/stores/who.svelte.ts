// WHO rail — the latest `who` listing plus client-side idle ticking.
//
// The server reports `idle_s`/`conn_s` as of the moment the listing was
// built; between refreshes the rail ticks both forward locally from
// `fetchedAt` (a 1 s interval in the component supplies `nowMs`).

import type { WhoEntry } from "../proto";

export class WhoStore {
  entries = $state<WhoEntry[]>([]);
  /** Local receipt time (unix ms) of the current listing; 0 = never. */
  fetchedAt = $state(0);

  get count(): number {
    return this.entries.length;
  }

  replace(entries: WhoEntry[], atMs = Date.now()): void {
    this.entries = entries;
    this.fetchedAt = atMs;
  }

  /** Seconds since the elapsed-time baseline (0 before any listing). */
  #elapsed(nowMs: number): number {
    return this.fetchedAt > 0 ? Math.max(0, (nowMs - this.fetchedAt) / 1000) : 0;
  }

  /** Idle seconds for an entry as of `nowMs` (server value + local drift). */
  idleOf(entry: WhoEntry, nowMs: number): number {
    return entry.idle_s + this.#elapsed(nowMs);
  }

  /** Connected seconds for an entry as of `nowMs`. */
  connOf(entry: WhoEntry, nowMs: number): number {
    return entry.conn_s + this.#elapsed(nowMs);
  }

  clear(): void {
    this.entries = [];
    this.fetchedAt = 0;
  }
}
