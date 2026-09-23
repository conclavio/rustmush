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
//   once every `MAP_ASK_MS`, until it has it. While a frame is held, a map of
//   another match — or the unseated answer — is DROPPED on arrival rather than
//   drawn (`applyMap`, 16b cut B.1), because stale ground under live markers is
//   worse than none, and dropping it does NOT re-arm the latch: an answer that
//   keeps coming back wrong would otherwise buy a fresh ask on every pulse,
//   which is the storm the latch exists to stop. A map that arrives BEFORE any
//   frame (the hello pass's wildcard ask) is held, and judged by
//   `holdsTheGround` when the first frame names the match. `GameApp` asks on
//   every frame AND on a `MAP_ASK_MS` timer (16b cut B.2), because the arena
//   pushes a frame only when one changes: a pilot sitting still would
//   otherwise wait for the first thing to move.
// * **The notices ring is also the reply surface** (16b cut B.1): while seated,
//   every `Sys` line lands in it through `pushReply`, because a pilot kept on
//   screen 0 has no scrollback in front of them — every one but the bridge's
//   copy of a frame's own notice, which the ring composes from the frame.
// * **The screen opens itself once per match id.** A pilot who then switches
//   away is left alone; the next pulse does not drag them back.
// * **The ring starts each match clean** (16b cut B.2): the frame that names a
//   new match empties it before its own notices land, and a full rehydrate
//   empties it through `clearForRehydrate`, which keeps the spent switch.
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
  /** The composed notice lines — the frames' notices and, while seated, the
   * pilot's own replies — oldest first, at most `COCKPIT_NOTICES`. */
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
    // The change of match is decided BEFORE a notice lands (16b cut B.2,
    // `B-80`): a new match starts an empty ring, so the last twelve lines of
    // match 7 never sit above match 8's first ones. The cleared frame that
    // ends a seat is NOT relied on for this — a resumed tab may never be sent
    // it (`cockpit_cleared`'s doc) — so the frame that names a new match is
    // what resets. The map latches are left alone; `needsMap` already treats
    // an ask armed for another match as spent.
    const open = this.#switchedFor !== frame.match;
    this.#switchedFor = frame.match;
    if (open) {
      this.notices = [];
    }
    if (frame.notices.length > 0) {
      // Per-pulse frames carry only that pulse's notices, and the mirror a read
      // verb re-emits is stored with them cleared, so nothing is rung twice.
      this.notices = [...this.notices, ...noticeLines(frame.notices)].slice(-COCKPIT_NOTICES);
    }
    return open;
  }

  /**
   * Apply one `arena.map` keyframe.
   *
   * While a frame is held, a map of ANOTHER match is dropped and the held one
   * kept (16b cut B.1, `B-61`): a late answer to an ask made for the previous
   * match must never replace the ground under this one. A map that arrives
   * before any frame — the hello pass's wildcard ask, see `noteMapAsked` — is
   * held, and `holdsTheGround` judges it when the first frame names the match.
   * The unseated answer, and a map with no size, are held as nothing.
   */
  applyMap(data: unknown): void {
    const map = decodeArenaMap(data);
    if (map === null) {
      return;
    }
    if (this.matchId !== null && map.match !== this.matchId) {
      return;
    }
    this.map = map.seated && map.width > 0 && map.height > 0 ? map : null;
  }

  /**
   * Append one reply line — the plain text of a `Sys` line — to the notices
   * ring, while seated; nothing when not (16b cut B.1).
   *
   * On screen 0 this ring is the only reply surface: the scrollback is mounted
   * by the scene screen alone, and a pilot who typed `lock BA` on the cockpit
   * expects `Locking BA.` there. Every `Sys` line goes, not a chosen subset of
   * replies, and the scrollback keeps its own copy of the sentence for screen 1
   * — Q20 lets two screens word one event. Same ring, same `COCKPIT_NOTICES`
   * cap. The one line the caller leaves out is the bridge's copy of a frame's
   * own notice (`isNoticeSentence`): the ring composes that notice from the
   * frame, and taking the sentence too would ring every notice twice.
   */
  pushReply(text: string): void {
    if (!this.seated) {
      return;
    }
    this.notices = [...this.notices, text].slice(-COCKPIT_NOTICES);
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

  /**
   * **A full rehydrate** (16b cut B.2, `B-80`): the resume gap outgrew the
   * ring, so nothing held here can be trusted to be the world's — drop the
   * frame, the ground and the ring, and let the hello pass's re-asks refill
   * them. Two things are KEPT, and that is the difference from `clear()`:
   *
   * * **The spent switch** (`#switchedFor`). A rehydrate re-answers the same
   *   match's frame, and the screen opens the moment a switch is owed (16b cut
   *   B.1), so forgetting it would drag a pilot who had walked off to the scene
   *   back onto the cockpit on every Wi-Fi blip. A new match still opens it.
   * * **The hello pass's map ask**, as the wildcard it is. On the wire the
   *   hello — and with it `#hydrate`'s `noteMapAsked` — arrives BEFORE the
   *   `resumed { replayed: false }` that runs this (`mush-net-web`'s pump sends
   *   the two in that order), so clearing the latch here would make the first
   *   frame pull the whole terrain a second time on the socket that has just
   *   reopened. Its time stands; the match it was stamped with — the old
   *   frame's, still held when the hello landed — is forgotten, because the
   *   server answers the ask with whatever match this pilot is seated in now.
   */
  clearForRehydrate(): void {
    this.frame = null;
    this.map = null;
    this.notices = [];
    this.#askedFor = null;
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
