// The replay store (16b cut I.2; 16c N9, Q37): a finished match open on screen
// 0 — the document the client holds, the ground it is drawn on, and where the
// viewer is in it.
//
// It owns the rules the TUI's `follow_the_replay` owns on its side, and they
// are the same rules because there is one design:
//
// * **The trigger is the pushed grid** (`applyGrid`). `SIM REPLAY <match>`
//   pushes one `grid` window on `arena.replay`; a grid of a match this viewer
//   has not dismissed opens a replay of it (the caller keeps a SEATED pilot on
//   the cockpit and does not call this). A grid replayed out of the resume ring
//   carries the seq it was first sent with, so a grid whose seq is at or under
//   the one a dismissal saw does NOT re-open the match; a fresh `SIM REPLAY`'s
//   grid has a newer seq, which clears the mark — keyed on the envelope seq,
//   never on payload equality. The seqs are one attach's: a fresh attach (the
//   resume gap outgrew the ring) numbers from 1 again, so `beginSegment`
//   forgets every seq and every mark — the new ring holds no grid of the old
//   attach to replay.
// * **The document is asked ONCE** (`needsDocument`): up to 256 KiB
//   (`MAP_MAX_BYTES`), so never on a timer — a 2 s re-ask would queue copies on
//   a slow socket and trip `mush-net-web`'s 10 s send timeout. It is asked once
//   per open until answered, again after `clearForRehydrate` (the hello pass)
//   and after `pull` (`p`), and never while an ask is outstanding (`docAsked`,
//   cleared by the answer) — `pull` included. An ask counts as outstanding for
//   `REPLAY_ASK_MS` and is then presumed dropped, so `p` can recover from an
//   answer that never came without a key mashed on a slow link queueing a
//   copy per press.
// * **The ground is asked under the cockpit's latch** (`needsMap`,
//   `MAP_ASK_MS`, keyed by match) until it is held; a map of another match is
//   dropped (`applyMap`).
// * **Any `available: false` answer closes the picture and dismisses the
//   match** (`applyDocument`): the viewer left the centre, typed a refused
//   `SIM REPLAY`, or the server half restarted and the cursor is gone — the
//   picture ends instead of asking again.
// * **`close` dismisses** (`x`, a seated frame, `available: false`): both feeds
//   forgotten, the flags cleared, the match marked.
//
// Nothing here reads a clock of its own: `needsMap`, `needsDocument` and `pull`
// are given one.
//
// **The live picture (16b cut I.4; 16c N10)** rides the same store in LIVE
// mode (`live`, `applyObserver`): a staffer's `@arena/watch` pushes the whole
// field on `arena.observer` at most once an arena second, and the first
// picture opens the store live — **closing an open replay** (the picture is
// replaced and the replay's document and ground are forgotten). The picture
// is held as a one-keyframe document (`liveAsReplay`) so the screen draws it
// as it draws a keyframe; it is never asked for as a document (`needsDocument`
// is `false` live) — every picture is pushed — and `observerOwed` is what the
// hello pass and `p` re-ask `arena.observer` on, beside the staleness heal
// (`LIVE_STALE_MS`, `needsObserver`) that catches a server-half restart a web
// socket rides with no hello. Nothing steps it
// (`stepKeyframe` and `seek` do nothing live) and only the server closes it:
// `watching: false` closes it and dismisses the match (I.2's rule for
// `available: false`, applied), under the same seq rule a grid is held to.

import type { ArenaMap } from "../cockpit";
import { decodeArenaMap } from "../cockpit";
import {
  decodeObserver,
  decodeReplay,
  decodeReplayGrid,
  isObserverCleared,
  liveAsReplay,
  type ObserverPicture,
  type Replay,
} from "../replay";
import { MAP_ASK_MS } from "./cockpit.svelte";

/**
 * **How long an `arena.replay` document ask stays outstanding** (16b cut I.2):
 * the TUI's `REPLAY_ASK_LATCH_S`. Nothing asks the document (up to 256 KiB)
 * again while one ask is in flight, `p` included; past this the ask is presumed
 * dropped and `p` may ask again. Ten seconds is `mush-net-web`'s send timeout:
 * an answer that has not landed by then is not coming on this socket.
 */
export const REPLAY_ASK_MS = 10_000;

/**
 * **How long a live picture may go without a keyframe before the heal asks
 * `arena.observer` again** (16b cut I.4), ms — and the least time between two
 * such asks: the TUI's `LIVE_STALE_S`. The world pushes a picture at most once
 * an arena second but only once a pulse — every `quiet_ms` (5,000 by default)
 * at `Quiet`, as seldom as `MAX_TEMPO_FLOOR_MS` (10,000) under a staff floor —
 * so fifteen seconds is above every healthy gap, and a quiet that long means a
 * lagging match or a server half that restarted and forgot the watch. A web
 * socket rides such a restart with NO hello, so without this the last picture
 * would stay up under the live label until `p`; the answer is the picture
 * again, or `watching: false`, which closes it.
 */
export const LIVE_STALE_MS = 15_000;

export class ReplayStore {
  /** Whether a replay is open. */
  open = $state(false);
  /** The match it is of. */
  match = $state<number | null>(null);
  /** Its document, once the answer has landed. */
  doc = $state<Replay | null>(null);
  /** The ground it was fought on, once pulled. */
  map = $state<ArenaMap | null>(null);
  /** The arena millisecond the screen draws. */
  cursorMs = $state(0);
  /** The roster index the window is centred on. */
  focus = $state(0);
  /** Whether it is playing at 1× arena time. */
  playing = $state(false);
  /** Whether this is a live picture (16b cut I.4's); `false` for a replay. */
  live = $state(false);
  /** The live picture, while `live` (16b cut I.4). */
  observer = $state<ObserverPicture | null>(null);
  /** The live picture is owed an `arena.observer` ask (the hello pass, `p`). */
  observerOwed = false;
  /** When the last live picture landed, and when the last `arena.observer`
   * ask went out — what the staleness heal measures `LIVE_STALE_MS` from. */
  #observerSeenAt: number | null = null;
  #observerAskedAt: number | null = null;
  /** The matches this viewer closed; see the header. */
  dismissed = new Set<number>();
  /** An `arena.replay` ask is outstanding. */
  docAsked = false;

  /** The highest grid seq seen per match, and the seq each dismissal saw. */
  #gridSeq = new Map<number, number>();
  #dismissedAt = new Map<number, number>();
  /** The document is owed an ask (opened, re-hydrated or pulled). */
  #docOwed = false;
  /** When the outstanding document ask went out (meaningful while `docAsked`). */
  #docAskedAt: number | null = null;
  /** When the last `arena.map` ask went out, and for which match. */
  #mapAskedAt: number | null = null;
  #mapAskedFor: number | null = null;
  /** The keyframe the grid showed, taken as the cursor when the document lands. */
  #openAt: number | null = null;

  /** Whether the held ground is the open match's. */
  get holdsTheGround(): boolean {
    return this.open && this.map !== null && this.map.match === this.match;
  }

  /** The last arena millisecond the scrubber reaches. */
  get endMs(): number {
    return this.doc?.end_ms ?? 0;
  }

  /**
   * Apply one pushed `grid` keyframe (seq `seq`). Answers whether it OPENED a
   * picture — `true` once per match opened, so the caller switches the screen
   * exactly then. A grid of the match already open keeps the document, the
   * cursor and the focus.
   */
  applyGrid(data: unknown, seq: number): boolean {
    const grid = decodeReplayGrid(data);
    if (grid === null) {
      return false;
    }
    const seen = this.#dismissedAt.get(grid.match);
    if (seen !== undefined && seq > seen) {
      // A fresh `SIM REPLAY`: the mark goes.
      this.dismissed.delete(grid.match);
      this.#dismissedAt.delete(grid.match);
    }
    this.#gridSeq.set(grid.match, Math.max(seq, this.#gridSeq.get(grid.match) ?? seq));
    if (this.dismissed.has(grid.match)) {
      return false;
    }
    if (this.open && this.match === grid.match) {
      return false;
    }
    const keepMap = this.map !== null && this.map.match === grid.match;
    this.open = true;
    this.match = grid.match;
    this.doc = null;
    this.map = keepMap ? this.map : null;
    this.cursorMs = 0;
    this.focus = 0;
    this.playing = false;
    this.docAsked = false;
    this.#docAskedAt = null;
    this.#docOwed = true;
    this.#openAt = grid.keyframe;
    return true;
  }

  /**
   * **Apply one `arena.observer` keyframe** (16b cut I.4), seq `seq`. Answers
   * whether it OPENED a live picture — `true` once per match opened, so the
   * caller switches the screen exactly then.
   *
   * `watching: false` closes a LIVE picture and dismisses its match (a replay
   * is left alone). A picture of a match whose dismissal saw a seq at or above
   * this one is a replay of the resume ring and opens nothing; a newer one
   * clears the mark. A picture of the live match already open replaces what is
   * drawn; any other opens it live, closing whatever was open — a replay
   * included — and keeping a held ground only when it is the same match's.
   */
  applyObserver(data: unknown, seq: number, now: number): boolean {
    if (isObserverCleared(data)) {
      if (this.open && this.live) {
        this.close();
      }
      return false;
    }
    const pic = decodeObserver(data);
    if (pic === null) {
      return false;
    }
    const seen = this.#dismissedAt.get(pic.match);
    if (seen !== undefined && seq > seen) {
      this.dismissed.delete(pic.match);
      this.#dismissedAt.delete(pic.match);
    }
    this.#gridSeq.set(pic.match, Math.max(seq, this.#gridSeq.get(pic.match) ?? seq));
    if (this.dismissed.has(pic.match)) {
      return false;
    }
    const already = this.open && this.live && this.match === pic.match;
    if (!already) {
      const keepMap = this.map !== null && this.map.match === pic.match;
      const map = keepMap ? this.map : null;
      if (this.open) {
        this.close();
      }
      this.open = true;
      this.live = true;
      this.match = pic.match;
      this.map = map;
      this.focus = 0;
      this.playing = false;
      this.docAsked = false;
      this.#docAskedAt = null;
      this.#docOwed = false;
      this.#openAt = null;
    }
    this.observer = pic;
    this.#observerSeenAt = now;
    this.doc = liveAsReplay(pic);
    this.cursorMs = pic.now_ms;
    if (this.focus >= pic.units.length) {
      this.focus = 0;
    }
    return !already;
  }

  /** Whether the live picture is owed an `arena.observer` ask at `now` — the
   * hello pass's or `p`'s debt, or the staleness heal's: no picture and no ask
   * for `LIVE_STALE_MS` — and, when it is, takes the debt and stamps the ask,
   * so a caller asks exactly as often as this answers `true`. */
  needsObserver(now: number): boolean {
    if (!this.open || !this.live) {
      return false;
    }
    const aged = (at: number | null): boolean => at === null || now - at >= LIVE_STALE_MS || now < at;
    if (!this.observerOwed && !(aged(this.#observerSeenAt) && aged(this.#observerAskedAt))) {
      return false;
    }
    this.observerOwed = false;
    this.#observerAskedAt = now;
    return true;
  }

  /**
   * Apply one `arena.replay` document answer. `available: false` closes the
   * picture and dismisses the match; the document of the open match is held
   * (the cursor on the keyframe the grid showed, the focus inside the
   * roster); one of any other match is dropped. Either way the outstanding ask
   * is answered.
   */
  applyDocument(data: unknown): void {
    this.docAsked = false;
    // 16b cut I.4: a live picture has no document; an answer on the replay
    // feed is not its to take.
    if (this.live) {
      return;
    }
    if (typeof data === "object" && data !== null && (data as Record<string, unknown>).available === false) {
      if (this.open) {
        this.close();
      }
      return;
    }
    const doc = decodeReplay(data);
    if (doc === null || !this.open || doc.match !== this.match) {
      return;
    }
    this.doc = doc;
    if (this.#openAt !== null) {
      this.cursorMs = doc.keyframes[Math.min(this.#openAt, doc.keyframes.length - 1)]?.t_ms ?? 0;
      this.#openAt = null;
    }
    this.cursorMs = Math.min(Math.max(0, this.cursorMs), doc.end_ms);
    if (this.focus >= doc.roster.length) {
      this.focus = 0;
    }
  }

  /** Apply one `arena.map` keyframe: the open match's ground is held, any
   * other (another match's, the unseated answer) is dropped. */
  applyMap(data: unknown): void {
    const map = decodeArenaMap(data);
    if (map === null || !this.open || map.match !== this.match) {
      return;
    }
    if (map.seated && map.width > 0 && map.height > 0) {
      this.map = map;
    }
  }

  /**
   * Whether the document is owed an ask now — and, when it is, ARMS
   * `docAsked` (at `now`), so a caller asks exactly as often as this answers
   * `true`: once per open until answered, again after `clearForRehydrate` or
   * `pull`, and never while an ask is outstanding (see `docOutstanding`).
   */
  needsDocument(now: number): boolean {
    if (!this.open || this.live || !this.#docOwed || this.docOutstanding(now)) {
      return false;
    }
    this.#docOwed = false;
    this.docAsked = true;
    this.#docAskedAt = now;
    return true;
  }

  /** Whether a document ask is still outstanding at `now`: one went out less
   * than `REPLAY_ASK_MS` ago and nothing has answered it. */
  docOutstanding(now: number): boolean {
    if (!this.docAsked || this.#docAskedAt === null) {
      return false;
    }
    const since = now - this.#docAskedAt;
    return since >= 0 && since < REPLAY_ASK_MS;
  }

  /** Whether an `arena.map` ask is due now — and, when it is, ARMS the latch
   * (`MAP_ASK_MS`, keyed by match), until the ground is held. */
  needsMap(now: number): boolean {
    const id = this.match;
    if (!this.open || id === null || this.holdsTheGround) {
      return false;
    }
    const since = this.#mapAskedAt === null ? null : now - this.#mapAskedAt;
    if (this.#mapAskedFor === id && since !== null && since >= 0 && since < MAP_ASK_MS) {
      return false;
    }
    this.#mapAskedAt = now;
    this.#mapAskedFor = id;
    return true;
  }

  /** Record an `arena.map` ask this store did not call for (the hello pass's,
   * which an unseated viewer's cursor answers with this ground): it arms the
   * latch, so the ground is not pulled twice. */
  noteMapAsked(now: number): void {
    this.#mapAskedAt = now;
    this.#mapAskedFor = this.match;
  }

  /** `p`: the document asked again — unless an ask is still outstanding at
   * `now`, which `p` does not duplicate (the ground while dark goes through
   * `needsMap`'s latch). */
  pull(now: number): void {
    // 16b cut I.4: a live picture's pull is the picture itself.
    if (this.open && this.live) {
      this.observerOwed = true;
      return;
    }
    if (this.open && !this.docOutstanding(now)) {
      this.#docOwed = true;
    }
  }

  /** Step to the previous (`-1`) or next (`+1`) keyframe from the cursor: from
   * between two, to the one on that side. Stops playing. */
  stepKeyframe(delta: number): void {
    const doc = this.doc;
    this.playing = false;
    // 16b cut I.4: nothing steps a live picture.
    if (doc === null || doc.keyframes.length === 0 || this.live) {
      return;
    }
    const times = doc.keyframes.map((kf) => kf.t_ms);
    const t = this.cursorMs;
    let target: number | undefined;
    // With no keyframe on that side the cursor stays where it is: past the
    // last keyframe (the scrubber runs to `end_ms`, usually after it) `next`
    // must not step BACK to it, nor `prev` forward before the first.
    if (delta < 0) {
      target = [...times].reverse().find((kt) => kt < t);
    } else {
      target = times.find((kt) => kt > t);
    }
    this.cursorMs = target ?? t;
  }

  /** Move the cursor (the scrubber), clamped to the replay. */
  seek(ms: number): void {
    // 16b cut I.4: nor scrubs one.
    if (this.live) {
      return;
    }
    this.cursorMs = Math.min(Math.max(0, Math.trunc(ms)), this.endMs);
  }

  /** The focus to the next roster unit, wrapping. */
  nextFocus(): void {
    const n = this.doc?.roster.length ?? 0;
    if (n > 0) {
      this.focus = (this.focus + 1) % n;
    }
  }

  /** Close the picture: its match dismissed, both feeds forgotten, the flags
   * cleared. */
  close(): void {
    const id = this.match;
    if (id !== null) {
      this.dismissed.add(id);
      this.#dismissedAt.set(id, this.#gridSeq.get(id) ?? 0);
    }
    this.open = false;
    this.match = null;
    this.doc = null;
    this.map = null;
    this.cursorMs = 0;
    this.focus = 0;
    this.playing = false;
    this.docAsked = false;
    this.#docAskedAt = null;
    this.#docOwed = false;
    this.#openAt = null;
    this.#mapAskedAt = null;
    this.#mapAskedFor = null;
    // 16b cut I.4: and the live picture.
    this.live = false;
    this.observer = null;
    this.observerOwed = false;
    this.#observerSeenAt = null;
    this.#observerAskedAt = null;
  }

  /** **A re-hydrate** (the hello pass): the picture stays open on its match,
   * and the document and the ground are dropped so the pass asks for both
   * again. */
  clearForRehydrate(): void {
    if (!this.open) {
      return;
    }
    if (this.live) {
      // 16b cut I.4: a live picture keeps drawing its last picture and is
      // owed a re-ask of `arena.observer` — the picture again, or `watching:
      // false` (a server-half restart forgot the watch), which closes it.
      this.map = null;
      this.observerOwed = true;
      this.#mapAskedAt = null;
      this.#mapAskedFor = null;
      return;
    }
    this.doc = null;
    this.map = null;
    this.docAsked = false;
    this.#docAskedAt = null;
    this.#docOwed = true;
    this.#mapAskedAt = null;
    this.#mapAskedFor = null;
    this.playing = false;
  }

  /** **A fresh attach** (`onFullRehydrate`: the resume gap outgrew the ring):
   * a new seq namespace. Every grid seq and every dismissal mark belonged to
   * the old one — a new grid numbered from 1 must still open a closed match,
   * and the new ring holds no grid of the old attach to replay — so all are
   * forgotten. An open picture stays open (the hello pass re-hydrates it). */
  beginSegment(): void {
    this.dismissed.clear();
    this.#dismissedAt.clear();
    this.#gridSeq.clear();
  }

  /** The full reset seam (a new session): nothing open, nothing dismissed. */
  clear(): void {
    this.close();
    this.dismissed.clear();
    this.#dismissedAt.clear();
    this.#gridSeq.clear();
  }
}
