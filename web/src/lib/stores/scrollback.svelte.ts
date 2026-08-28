// Scrollback — the capped ring of rendered output blocks.
//
// Entries are pre-rendered (`RLine[]` via $lib/render) at push time so the
// component stays dumb and the logic stays unit-testable. Virtualization is
// deliberately simple (documented in web/README.md): only the last
// `windowSize` blocks render; a "show older" expander widens the window.
// The ring caps at 5000 blocks.
//
// Scroll-lock: the component reports whether the view is pinned to the
// bottom (`setPinned`); while unpinned, pushes accumulate `unseen` for the
// "N new lines" pill.

import type { RLine } from "../render";

/** One scrollback block: one event's rendered lines. */
export interface SbBlock {
  /** Monotonic render key. */
  id: number;
  /** Unix ms (the envelope's `ts`, or local time for client lines). */
  ts: number;
  /** Row class, e.g. `"ev-narrative"`, `"ev-sys"`, `"ev-echo"`. */
  cls: string;
  /** Rendered lines (a Room block has several). */
  lines: RLine[];
}

export const SCROLLBACK_CAP = 5000;
export const WINDOW_STEP = 300;

export class ScrollbackStore {
  entries = $state<SbBlock[]>([]);
  /** How many trailing blocks the component renders. */
  windowSize = $state(WINDOW_STEP);
  /** Whether the view is pinned to the bottom (autoscroll on push). */
  pinned = $state(true);
  /** Blocks pushed while unpinned (drives the "N new lines" pill). */
  unseen = $state(0);

  #nextId = 1;
  readonly #cap: number;

  constructor(cap = SCROLLBACK_CAP) {
    this.#cap = cap;
  }

  /** The slice the component renders (last `windowSize` blocks). */
  get visible(): SbBlock[] {
    return this.entries.length > this.windowSize
      ? this.entries.slice(this.entries.length - this.windowSize)
      : this.entries;
  }

  /** Blocks hidden above the window (drives the "show older" expander). */
  get hiddenCount(): number {
    return Math.max(0, this.entries.length - this.windowSize);
  }

  push(cls: string, lines: RLine[], ts = Date.now()): void {
    this.entries.push({ id: this.#nextId++, ts, cls, lines });
    if (this.entries.length > this.#cap) {
      this.entries.splice(0, this.entries.length - this.#cap);
    }
    if (!this.pinned) {
      this.unseen += 1;
    }
  }

  /** Convenience: one plain single-span line (client-local notices). */
  pushText(cls: string, text: string, ts = Date.now()): void {
    this.push(cls, [[{ text, cls: "", css: "", cmd: null }]], ts);
  }

  /** Widen the render window ("show older"). */
  showOlder(step = WINDOW_STEP): void {
    this.windowSize = Math.min(this.entries.length, this.windowSize + step);
  }

  /** The component reports scroll position; pinning clears the pill. */
  setPinned(pinned: boolean): void {
    this.pinned = pinned;
    if (pinned) {
      this.unseen = 0;
    }
  }

  clear(): void {
    this.entries = [];
    this.windowSize = WINDOW_STEP;
    this.unseen = 0;
  }
}
