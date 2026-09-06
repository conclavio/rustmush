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
//
// Narrative backlog (interface plan I12): a narrative block records its
// per-session envelope `seq`; `prependOlder` splices a server page in below
// the current session's blocks. A full rehydrate starts a new SESSION
// SEGMENT (`beginSegment`): the old blocks stay visible but lose their seqs
// (a fresh session restarts the seq namespace, so old seqs must never act as
// the cursor or the de-dup key), the seam is recorded, and pages for the new
// session land at the seam — between the old history and the live tail —
// so order stays chronological.

import type { RLine } from "../render";
import type { RollEntry } from "./rolls.svelte";

/** The actor a grouped block belongs to. */
export interface SbSpeaker {
  id: string;
  name: string;
}

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
  /** The speaker, for a block that groups under a name header (I5). */
  speaker: SbSpeaker | null;
  /** The lines without the speaker, shown inside a group. */
  body: RLine[] | null;
  /** Whether the block may join a speaker group. */
  groupable: boolean;
  /** The roller's dice card (a `.roll` frame for the viewer's own roll). */
  roll: RollEntry | null;
  /** The per-session envelope `seq` for a narrative block (interface plan
   * I12: the backlog cursor + the de-dup key against a paged reply). `null`
   * for room/sys/roll/client-local blocks, and for every block from a
   * previous session segment. */
  seq: number | null;
}

/** What `push` may say about a block beyond its lines. */
export interface SbMeta {
  speaker?: SbSpeaker | null;
  body?: RLine[] | null;
  groupable?: boolean;
  /** The narrative envelope `seq` (interface plan I12). */
  seq?: number;
}

/** A pre-rendered older block from a narrative backlog page (interface plan
 * I12), spliced in at the session seam. */
export type SbOlderBlock = Omit<SbBlock, "id" | "roll">;

/** Consecutive blocks by one speaker within this window merge under one
 * name header (interface plan I5, plan §7). */
export const GROUP_WINDOW_S = 300;

/** A run of blocks under one header (or a single ungrouped block). */
export interface SbGroup {
  id: number;
  speaker: SbSpeaker | null;
  blocks: SbBlock[];
}

/**
 * Speaker grouping (interface plan I5): a block joins the group before it
 * when both are groupable, by the same actor, and it follows the previous
 * block within `windowMs`; any other block — a room, a system line, an
 * emit, an OOC line, a roll — stands alone and breaks the run, so a line
 * is never attributed across an intervening event.
 */
export function groupBlocks(blocks: readonly SbBlock[], windowMs = GROUP_WINDOW_S * 1000): SbGroup[] {
  const groups: SbGroup[] = [];
  for (const b of blocks) {
    const last = groups[groups.length - 1];
    const prev = last?.blocks[last.blocks.length - 1];
    if (
      last &&
      prev &&
      b.groupable &&
      prev.groupable &&
      last.speaker !== null &&
      b.speaker !== null &&
      last.speaker.id === b.speaker.id &&
      b.ts - prev.ts <= windowMs &&
      b.ts >= prev.ts
    ) {
      last.blocks.push(b);
      continue;
    }
    groups.push({ id: b.id, speaker: b.groupable ? b.speaker : null, blocks: [b] });
  }
  return groups;
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
  /** Whether the server may hold older narrative below what is here
   * (interface plan I12). `true` until a backlog page reports `more: false`
   * or the local cap can hold no more — drives the "load older history"
   * affordance. Re-armed by `beginSegment` (a new session has a new ring). */
  moreBacklog = $state(true);
  /** A backlog page is in flight (one at a time). Set only once a request
   * actually went out; reset on every `hello`. */
  fetchingBacklog = $state(false);
  /** The server reported older history rolled off its ring (`evicted`), so
   * "no more" means "gone", not "none ever" (interface plan I12). */
  backlogEvicted = $state(false);
  /** Bumped only when a backlog page actually added blocks — the component
   * restores its scroll anchor on this, never on an empty/all-duplicate
   * page or an unrelated live push. */
  prependGen = $state(0);

  #nextId = 1;
  /** Descending render keys for prepended older blocks (0, -1, -2 …), a
   * namespace disjoint from the ascending live ids so `#each` keys never
   * collide (interface plan I12). */
  #prevId = 0;
  /** The `entries` index where the current session segment begins: older
   * pages splice in here. 0 until a rehydrate starts a new segment. */
  #seam = 0;
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

  push(cls: string, lines: RLine[], ts = Date.now(), meta: SbMeta = {}): void {
    this.#append({
      id: this.#nextId++,
      ts,
      cls,
      lines,
      speaker: meta.speaker ?? null,
      body: meta.body ?? null,
      groupable: meta.groupable === true && (meta.speaker ?? null) !== null,
      roll: null,
      seq: meta.seq ?? null,
    });
  }

  /** The viewer's own roll as a dice card (interface plan I5). */
  pushRoll(entry: RollEntry, ts = Date.now()): void {
    this.#append({
      id: this.#nextId++,
      ts,
      cls: "ev-roll-card",
      lines: [],
      speaker: null,
      body: null,
      groupable: false,
      roll: entry,
      seq: null,
    });
  }

  /** The oldest narrative `seq` held in the CURRENT session segment — the
   * `before_seq` cursor for the next backlog page (interface plan I12).
   * `null` when none is held yet (the first page starts from the newest).
   * Blocks from a previous segment carry no seq, so a stale namespace can
   * never become the cursor. */
  get oldestNarrativeSeq(): number | null {
    let min: number | null = null;
    for (const b of this.entries) {
      if (b.seq !== null && (min === null || b.seq < min)) {
        min = b.seq;
      }
    }
    return min;
  }

  /** Start a new session segment after a full rehydrate (interface plan
   * I12): the server fell back to a fresh attach, whose ring and seq
   * namespace start over. Keep the old blocks on screen but drop their
   * seqs (never a cursor or de-dup key again), record the seam, and re-arm
   * the backlog so the new session's history can be pulled. */
  beginSegment(): void {
    let changed = false;
    for (const b of this.entries) {
      if (b.seq !== null) {
        b.seq = null;
        changed = true;
      }
    }
    if (changed) {
      this.entries = [...this.entries];
    }
    this.#seam = this.entries.length;
    this.moreBacklog = true;
    this.fetchingBacklog = false;
    this.backlogEvicted = false;
  }

  /** Splice a narrative backlog page in at the session seam, oldest-first,
   * deduped by `seq` against what the current segment already holds
   * (interface plan I12). `more` is the server's "another page exists below
   * this" flag; `evicted` its "older history rolled off". Older blocks take
   * descending ids so `#each` keys stay unique. When the local cap cannot
   * hold the whole page the backlog is marked exhausted (`moreBacklog =
   * false`) — there is nowhere to put more, and re-fetching the same page
   * on every scroll would loop. Bumps `prependGen` only when blocks were
   * actually added. Returns that count. */
  prependOlder(older: SbOlderBlock[], more: boolean, evicted = false): number {
    this.fetchingBacklog = false;
    this.backlogEvicted = evicted;
    const held = new Set<number>();
    for (const b of this.entries) {
      if (b.seq !== null) {
        held.add(b.seq);
      }
    }
    const fresh = older.filter((b) => b.seq === null || !held.has(b.seq));
    const room = Math.max(0, this.#cap - this.entries.length);
    // Keep the NEWEST of the older batch when capped (never drop the live tail).
    const kept = fresh.length > room ? fresh.slice(fresh.length - room) : fresh;
    // The cap truncated the page (or held none of it): nothing older can be
    // shown, so stop offering — else the same page is re-fetched forever.
    this.moreBacklog = kept.length < fresh.length ? false : more;
    if (kept.length === 0) {
      return 0;
    }
    const blocks: SbBlock[] = kept.map((b) => ({ ...b, id: this.#prevId--, roll: null }));
    const seam = Math.min(this.#seam, this.entries.length);
    this.entries.splice(seam, 0, ...blocks);
    // Keep the just-revealed history on screen.
    this.windowSize = Math.min(this.entries.length, this.windowSize + kept.length);
    this.prependGen += 1;
    return kept.length;
  }

  /** The visible window as speaker groups. */
  get visibleGroups(): SbGroup[] {
    return groupBlocks(this.visible);
  }

  #append(block: SbBlock): void {
    this.entries.push(block);
    if (this.entries.length > this.#cap) {
      const excess = this.entries.length - this.#cap;
      this.entries.splice(0, excess);
      // The head rolled off: the seam moves with it.
      this.#seam = Math.max(0, this.#seam - excess);
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
    this.moreBacklog = true;
    this.fetchingBacklog = false;
    this.backlogEvicted = false;
    this.#seam = 0;
  }
}
