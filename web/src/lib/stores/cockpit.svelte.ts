// The cockpit store (16b cut Q.5; doc 16 §5): the two arena feeds held for the
// cockpit screen — the per-pulse `arena.cockpit` keyframe, the pulled
// `arena.map` terrain under it, and the notices ring beside them.
//
// It owns three rules the TUI's `follow_the_cockpit` owns on its side, and they
// are the same rules because there is one design:
//
// * **The map is pulled, never pushed** (16b cut Q.3). Nothing publishes
//   `arena.map`, so it is never in the session's resume ring either: a client
//   that holds a cockpit frame of match M and no map of M asks for one, at most
//   once every `MAP_ASK_MS`, until it has it. A map of another match — or the
//   unseated answer — is DROPPED rather than drawn, because stale ground under
//   live markers is worse than none, and dropping it does NOT re-arm the latch:
//   an answer that keeps coming back wrong would otherwise buy a fresh ask on
//   every pulse, which is the storm the latch exists to stop.
// * **The screen opens itself once per match id.** A pilot who then switches
//   away is left alone; the next pulse does not drag them back.
// * **A cleared frame** (`seated: false`, the last one a pilot gets) drops
//   everything and re-arms both, so the next match starts clean.
//
// Nothing here reads a clock of its own: `needsMap(now)` is given one, so the
// latch is pinned without fake timers.

import {
  COCKPIT_NOTICES,
  decodeArenaMap,
  decodeCockpitFrame,
  noticeLines,
  type ArenaMap,
  type CockpitFrame,
} from "../cockpit";

/** At most one `arena.map` ask per this many milliseconds (the TUI's
 * `MAP_HEAL_LATCH_S`, in the units the browser counts in). */
export const MAP_ASK_MS = 2000;

export class CockpitStore {
  /** The last SEATED frame, or `null` when this viewer is not in a cockpit. */
  frame = $state<CockpitFrame | null>(null);
  /** The terrain of `frame`'s match, or `null` while it has not arrived. */
  map = $state<ArenaMap | null>(null);
  /** The composed notice lines, oldest first, at most `COCKPIT_NOTICES`. */
  notices = $state<string[]>([]);

  /** When the last `arena.map` ask went out, and which match it was for. */
  #askedAt: number | null = null;
  #askedFor: number | null = null;

  /** The match id the screen has already been opened for (once per match). */
  #switchedFor: number | null = null;

  /** Whether this viewer is in a cockpit at all. */
  get seated(): boolean {
    return this.frame !== null;
  }

  /** The match the held frame belongs to. */
  get matchId(): number | null {
    return this.frame?.match ?? null;
  }

  /** Whether the held terrain is the terrain of the held frame's match. One
   * pulled map serves every redraw of a match, so this is what both the ask
   * and the canvas are gated on. */
  get holdsTheGround(): boolean {
    const id = this.matchId;
    return id !== null && this.map !== null && this.map.seated && this.map.match === id;
  }

  /**
   * Apply one `arena.cockpit` keyframe.
   *
   * Answers whether the screen should be switched to — `true` exactly once per
   * match id, on that match's FIRST seated frame — so the caller does the
   * switching and this store does no navigation.
   */
  applyFrame(data: unknown): boolean {
    const frame = decodeCockpitFrame(data);
    if (frame === null) {
      return false;
    }
    if (!frame.seated || frame.match === null) {
      // The seat is gone: nothing of it stays on screen, and both latches
      // re-arm so the next match starts clean.
      this.clear();
      return false;
    }
    this.frame = frame;
    if (!this.holdsTheGround && this.map !== null) {
      // Another match's terrain, or the unseated answer. The latch is NOT
      // touched here — a wrong answer is the case it guards.
      this.map = null;
    }
    if (frame.notices.length > 0) {
      // Per-pulse frames carry only that pulse's notices, and the mirror a read
      // verb re-emits is stored with them cleared, so nothing is rung twice.
      this.notices = [...this.notices, ...noticeLines(frame.notices)].slice(-COCKPIT_NOTICES);
    }
    const open = this.#switchedFor !== frame.match;
    this.#switchedFor = frame.match;
    return open;
  }

  /** Apply one `arena.map` keyframe. A map of another match, and the unseated
   * answer, are held as nothing: `holdsTheGround` is the only reader. */
  applyMap(data: unknown): void {
    const map = decodeArenaMap(data);
    if (map === null) {
      return;
    }
    this.map = map.seated && map.width > 0 && map.height > 0 ? map : null;
  }

  /**
   * Record an `arena.map` ask this store did not itself call for.
   *
   * The hello pass pulls the ground before any frame has named a match — it
   * has to, because the map is never ring-recorded and a re-attach therefore
   * holds none — and that ask covers whichever match the first frame turns out
   * to be. Without it the first frame would find an unarmed latch and pull the
   * WHOLE arena's terrain a second time, on a just-reopened socket draining its
   * resume ring: the one moment the pull design exists to keep clear.
   */
  noteMapAsked(now: number): void {
    this.#askedAt = now;
    this.#askedFor = this.matchId;
  }

  /**
   * Whether an `arena.map` query is due now — and, when it is, ARMS the latch,
   * so a caller asks exactly as often as this answers `true`.
   *
   * A fresh match asks on this very frame rather than waiting out a latch armed
   * for the old one; otherwise the latch ages, so an ask that was never
   * answered (a throttled query is dropped in silence by the bridge) costs one
   * stale window and not the session.
   */
  needsMap(now: number): boolean {
    const id = this.matchId;
    if (id === null || this.holdsTheGround) {
      return false;
    }
    // An ask armed for ANOTHER match is spent, so a fresh match asks on this
    // very frame. An ask armed before any frame had named a match — the hello
    // pass, through `noteMapAsked` — is a wildcard and not a third state: the
    // server answers it with the map of whatever match this pilot is seated
    // in, which is the one the first frame names.
    const spent = this.#askedFor !== null && this.#askedFor !== id;
    const since = this.#askedAt === null ? null : now - this.#askedAt;
    if (!spent && since !== null && since >= 0 && since < MAP_ASK_MS) {
      return false;
    }
    this.#askedAt = now;
    this.#askedFor = id;
    return true;
  }

  clear(): void {
    this.frame = null;
    this.map = null;
    this.notices = [];
    this.#askedAt = null;
    this.#askedFor = null;
    this.#switchedFor = null;
  }
}
