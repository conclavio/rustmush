// The replay on the web (16b cut I.2; 16c N9, Q37): a finished match drawn on
// screen 0, both sides, scrubbed through at the arena's own clock.
//
// `SIM REPLAY <match>` (16b cut I.1) PUSHES one `grid` window on `arena.replay`
// — the line client's picture, and the web's trigger: `GameApp` opens the
// replay on it and asks the parameter-less `arena.replay` query ONCE for the
// whole document (`schema.shape == "replay"`) and `arena.map` for the ground
// under the cockpit's own latch. Everything after that is this file, over the
// document the client holds: `frameAt` places every unit at any arena
// millisecond, `composeReplayWindow` draws them on the ground, `replayView`
// words it.
//
// **Interpolation is a renderer's drawing, and the screen says so** (N9's
// default). The arena recorded where every unit stood every `summary_ms`; in
// between, `frameAt` moves each unit in a straight line and turns it along the
// shorter arc, and the label under the canvas reads `Between keyframes 19 and
// 20: drawn, not recorded.` — never a keyframe's own sentence. The TUI and the
// line client draw keyframes only.
//
// **Both sides are shown, and the screen says so** (Q37): the document names
// every tank by the callsign the arena gave it, and carries no object number,
// no pilot and no contact id; nothing here reads any other store.
//
// The fixture set is `fixtures/replay/`: `replay.json` and `replay_window.json`
// (blessed by `crates/tacsim/tests/replay.rs`, the one blesser), and
// `replay_grid.txt` (blessed by `mush-ansi`'s golden from the line client's
// print of the window) — the byte target this file's `composeReplayWindow` and
// the TUI's replay map both equal at 39 by 20 cells — and
// `replay_view_model.txt`, this file's own golden (`REPLAY_BLESS=1`).
//
// **The live picture (16b cut I.4; 16c N10).** `@arena/watch <match>` makes a
// staffer a watcher of a RUNNING match, and the world pushes them its whole
// field on `arena.observer` at most once an arena second. `decodeObserver`
// reads one; `liveAsReplay` turns it into a one-keyframe `Replay` so the same
// `frameAt` and `composeReplayWindow` draw it (units as a replay draws them:
// `o` red, `*` blue, `%` a wreck, a unit with no cell not drawn); `liveView`
// words it. A live picture is objective and a STAFF surface: its rows name
// object numbers on purpose.

import {
  windowGround,
  type ArenaMap,
  type ComposedWindow,
  type Marker,
  type MarkerKind,
  markerLegend,
} from "./cockpit";

/** The replay feed (16b cut I.1's `arena_bridge::REPLAY_FEED`). */
export const REPLAY_FEED = "arena.replay";

/** The glyphs a replay draws its units with (16b cut I.1's). */
export const RED_GLYPH = "o";
export const BLUE_GLYPH = "*";
export const WRECK_GLYPH = "%";

/** What the screen says while the document is on its way. */
export const WAITING_FOR_THE_REPLAY = "Waiting for the replay...";

/** The staff picture's feed (16b cut I.4's `arena_bridge::OBSERVER_FEED`). */
export const OBSERVER_FEED = "arena.observer";

/** The label under a live picture's canvas (16b cut I.4). */
export const LIVE_LABEL = "Live: the arena's own positions, at most once an arena second.";

/** The live status lines' second and last (16b cut I.4; the TUI's too). */
export const LIVE_CADENCE = "Live, at most once a second.";
export const LIVE_STOP = "@arena/watch off stops it.";

/** One unit of the roster: the callsign every viewer calls it by, and its
 * side. No object number and no pilot. */
export interface ReplayRoster {
  callsign: string;
  team: "red" | "blue";
}

/** Where one unit stood at one keyframe: an arena cell and a heading in whole
 * degrees clockwise from north. */
export interface KeyframeUnit {
  x: number;
  y: number;
  heading_deg: number;
}

/** One keyframe: every roster unit at one arena millisecond, POSITIONAL by
 * roster index, `null` for a unit not in it. */
export interface Keyframe {
  t_ms: number;
  units: (KeyframeUnit | null)[];
}

/** What became of one shot. */
export type ShotOutcome = "flying" | "hit" | "miss";

/** One shot: when it was fired, by and at which roster entries, and what
 * became of it. */
export interface ReplayShot {
  t_ms: number;
  shooter: number;
  target: number;
  outcome: ShotOutcome;
}

/** One replay document, decoded (16b cut I.1's `Replay::to_document`). */
export interface Replay {
  match: number;
  arena: string;
  cell_cm: number;
  width: number;
  height: number;
  summary_ms: number;
  end_ms: number;
  outcome: string;
  reason: string;
  /** **The rule the match was fought under** (16b cut O.1): the document's
   * `mode` word, or `""` where it has none — every replay written before the
   * cut. A word other than `skirmish` is tagged on the header (` [hold]`). */
  mode: string;
  roster: ReplayRoster[];
  keyframes: Keyframe[];
  shots: ReplayShot[];
  /** `[t_ms, roster index]`, in tape order. */
  destroyed: [number, number][];
  truncated: boolean;
  /** Why: `roster`, `keyframes`, `shots`, `page`, each once. */
  cut: string[];
}

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function int(raw: unknown, fallback: number): number {
  return typeof raw === "number" && Number.isFinite(raw) ? Math.trunc(raw) : fallback;
}

function nat(raw: unknown): number | null {
  return typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? Math.trunc(raw) : null;
}

/** Server text: control characters out, cut to `max`. */
function clean(raw: unknown, fallback: string, max = 32): string {
  if (typeof raw !== "string") {
    return fallback;
  }
  const stripped = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, "").trim();
  return stripped.length === 0 ? fallback : [...stripped].slice(0, max).join("");
}

/** A `[x, y, heading]` slot, or `null` for anything else. */
function decodeSlot(raw: unknown): KeyframeUnit | null {
  if (!Array.isArray(raw)) {
    return null;
  }
  const x = nat(raw[0]);
  const y = nat(raw[1]);
  const h = nat(raw[2]);
  return x !== null && y !== null && h !== null ? { x, y, heading_deg: h % 360 } : null;
}

/**
 * **Read one `arena.replay` answer** (16b cut I.2): the document under
 * `schema.shape == "replay"`, tolerant of missing and extra keys — or `null`
 * for `available: false`, for the pushed grid, and for anything with no match.
 *
 * Total over what the wire might carry, as the server's own reader is: a
 * missing number reads `0`, a word empty; a roster entry's unreadable side
 * reads red (the rows are positional, so it is kept); each keyframe is padded
 * or cut to the roster's length, a slot that is not three small numbers reading
 * `null`; a malformed keyframe, shot or wreck — or one naming an index past the
 * roster — is skipped.
 */
export function decodeReplay(data: unknown): Replay | null {
  if (!isObj(data) || data.available === false) {
    return null;
  }
  // The shape tells the document from the pushed grid on the one feed; a
  // payload with no shape at all is read as a document by its `record` (the
  // stored form, `Replay::to_document` before the query adds its schema).
  const shape = isObj(data.schema) ? data.schema.shape : undefined;
  if (shape !== "replay" && !(shape === undefined && data.record === "tacsim.replay")) {
    return null;
  }
  const match = nat(data.match);
  if (match === null) {
    return null;
  }
  const roster: ReplayRoster[] = (Array.isArray(data.roster) ? data.roster : []).map((e) => ({
    callsign: clean(isObj(e) ? e.callsign : undefined, "?", 8),
    team: isObj(e) && e.team === "blue" ? "blue" : "red",
  }));
  const n = roster.length;
  const keyframes: Keyframe[] = (Array.isArray(data.keyframes) ? data.keyframes : []).flatMap(
    (kf): Keyframe[] => {
      if (!Array.isArray(kf)) {
        return [];
      }
      const t = nat(kf[0]);
      if (t === null) {
        return [];
      }
      const slots = Array.isArray(kf[1]) ? kf[1] : [];
      return [{ t_ms: t, units: roster.map((_, i) => decodeSlot(slots[i])) }];
    },
  );
  const outcomes: readonly string[] = ["flying", "hit", "miss"];
  const shots: ReplayShot[] = (Array.isArray(data.shots) ? data.shots : []).flatMap(
    (s): ReplayShot[] => {
      if (!Array.isArray(s)) {
        return [];
      }
      const t = nat(s[0]);
      const shooter = nat(s[1]);
      const target = nat(s[2]);
      const outcome = typeof s[3] === "string" && outcomes.includes(s[3]) ? (s[3] as ShotOutcome) : null;
      return t !== null && shooter !== null && target !== null && outcome !== null && shooter < n && target < n
        ? [{ t_ms: t, shooter, target, outcome }]
        : [];
    },
  );
  const destroyed: [number, number][] = (Array.isArray(data.destroyed) ? data.destroyed : []).flatMap(
    (d): [number, number][] => {
      if (!Array.isArray(d)) {
        return [];
      }
      const t = nat(d[0]);
      const at = nat(d[1]);
      return t !== null && at !== null && at < n ? [[t, at]] : [];
    },
  );
  const cut = (Array.isArray(data.cut) ? data.cut : []).filter(
    (c): c is string => typeof c === "string" && ["roster", "keyframes", "shots", "page"].includes(c),
  );
  return {
    match,
    arena: clean(data.arena, "", 40),
    cell_cm: int(data.cell_cm, 100),
    width: Math.max(0, int(data.width, 0)),
    height: Math.max(0, int(data.height, 0)),
    summary_ms: Math.max(0, int(data.summary_ms, 0)),
    end_ms: Math.max(0, int(data.end_ms, 0)),
    outcome: clean(data.outcome, "", 16),
    reason: clean(data.reason, "", 24),
    mode: clean(data.mode, "", 16),
    roster,
    keyframes,
    shots,
    destroyed,
    truncated: data.truncated === true,
    cut,
  };
}

/** **The trigger** (16b cut I.2): the match a pushed `grid` window on
 * `arena.replay` is of, and the keyframe it shows (0-based) — or `null` for
 * anything that is not one (the document, `available: false`). */
export function decodeReplayGrid(data: unknown): { match: number; keyframe: number } | null {
  if (!isObj(data) || !isObj(data.schema) || data.schema.shape !== "grid") {
    return null;
  }
  const match = nat(data.match);
  if (match === null) {
    return null;
  }
  const keyframe = isObj(data.keyframe) ? (nat(data.keyframe.index) ?? 0) : 0;
  return { match, keyframe };
}

/** One unit of a live picture (16b cut I.4): the payload's positional row
 * `[callsign, team letter, x | null, y | null, heading, object number, flags]`. */
export interface ObserverUnit {
  callsign: string;
  team: "red" | "blue";
  /** Its cell, or `null` off the window. */
  cell: [number, number] | null;
  heading_deg: number;
  /** Its object number — a staff surface names it on purpose. */
  id: number;
  /** `w` wreck, `p` power online, `s` starting, `m` machine, `h` held, in
   * that order. */
  flags: string;
}

/** One `arena.observer` picture, decoded (16b cut I.4's `observer_frame`). */
export interface ObserverPicture {
  match: number;
  now_ms: number;
  tempo: string;
  /** The staff tempo floor, milliseconds, while one stands. */
  floor_ms: number | null;
  cell_cm: number;
  width: number;
  height: number;
  in_flight: number;
  units: ObserverUnit[];
}

/** **Read one `arena.observer` keyframe** (16b cut I.4), tolerant as
 * `decodeReplay` is — or `null` for `watching: false` and for anything with no
 * match. */
export function decodeObserver(data: unknown): ObserverPicture | null {
  if (!isObj(data) || data.watching === false) {
    return null;
  }
  const match = nat(data.match);
  if (match === null) {
    return null;
  }
  const units: ObserverUnit[] = (Array.isArray(data.units) ? data.units : []).flatMap(
    (row): ObserverUnit[] => {
      if (!Array.isArray(row)) {
        return [];
      }
      const x = nat(row[2]);
      const y = nat(row[3]);
      return [
        {
          callsign: clean(row[0], "?", 8),
          team: row[1] === "B" ? "blue" : "red",
          cell: x !== null && y !== null ? [x, y] : null,
          heading_deg: (nat(row[4]) ?? 0) % 360,
          id: nat(row[5]) ?? 0,
          flags: clean(row[6], "", 8),
        },
      ];
    },
  );
  return {
    match,
    now_ms: Math.max(0, int(data.now_ms, 0)),
    tempo: clean(data.tempo, "?", 8),
    floor_ms: nat(data.floor_ms),
    cell_cm: int(data.cell_cm, 100),
    width: Math.max(0, int(data.width, 0)),
    height: Math.max(0, int(data.height, 0)),
    in_flight: nat(data.in_flight) ?? 0,
    units,
  };
}

/** Whether an `arena.observer` keyframe says there is nothing to watch. */
export function isObserverCleared(data: unknown): boolean {
  return isObj(data) && data.watching === false;
}

/** **A live picture as a one-keyframe replay** (16b cut I.4): the roster its
 * units, one keyframe at its `now_ms`, its wrecks wrecked at that moment — so
 * `frameAt` and `composeReplayWindow` draw it exactly as a replay's keyframe is
 * drawn. It carries no shot and no outcome. */
export function liveAsReplay(pic: ObserverPicture): Replay {
  return {
    match: pic.match,
    arena: "",
    cell_cm: pic.cell_cm,
    width: pic.width,
    height: pic.height,
    summary_ms: 0,
    end_ms: pic.now_ms,
    outcome: "",
    reason: "",
    mode: "",
    roster: pic.units.map((u) => ({ callsign: u.callsign, team: u.team })),
    keyframes: [
      {
        t_ms: pic.now_ms,
        units: pic.units.map((u) =>
          u.cell === null ? null : { x: u.cell[0], y: u.cell[1], heading_deg: u.heading_deg },
        ),
      },
    ],
    shots: [],
    destroyed: pic.units.flatMap((u, i): [number, number][] =>
      u.flags.includes("w") ? [[pic.now_ms, i]] : [],
    ),
    truncated: false,
    cut: [],
  };
}

/** A live unit's words after its object number: `machine` or `pilot`, then
 * `wreck`, `held` and `starting` where they apply (the TUI's). */
function liveWords(u: ObserverUnit): string {
  const words = [u.flags.includes("m") ? "machine" : "pilot"];
  for (const [flag, word] of [
    ["w", "wreck"],
    ["h", "held"],
    ["s", "starting"],
  ] as const) {
    if (u.flags.includes(flag)) {
      words.push(word);
    }
  }
  return words.join("  ");
}

/** **The live picture's view model** (16b cut I.4): every string the screen
 * prints for one picture drawn in the window `win` — the heading, the label
 * that says what it is, the clock, the legend, one row a unit (its cell and
 * heading or `off the window`, its object number, its words) and the four
 * status lines the TUI's status pane prints. No shots and no wrecks: the
 * picture carries none. */
export function liveView(pic: ObserverPicture, win: ComposedWindow | null): ReplayView {
  const floor = pic.floor_ms === null ? "" : `  floor ${pic.floor_ms}`;
  return {
    header: `Watching ${pic.match} — ${clockText(pic.now_ms)} (live)`,
    label: LIVE_LABEL,
    clock: clockText(pic.now_ms),
    legend: markerLegend(win),
    shots: [],
    wrecks: [],
    units: pic.units.map((u) => ({
      callsign: u.callsign,
      side: u.team,
      where: `${
        u.cell === null
          ? "off the window"
          : `(${u.cell[0]}, ${u.cell[1]})  ${String(u.heading_deg).padStart(3, "0")}°`
      }  #${u.id}  ${liveWords(u)}`,
    })),
    truncation: [],
    status: [`Match ${pic.match}  ${pic.tempo}${floor}`, LIVE_CADENCE, `In flight: ${pic.in_flight}`, LIVE_STOP],
  };
}

/** Where every unit is drawn at one arena millisecond. */
export interface ReplayFrame {
  /** The arena millisecond asked for. */
  t_ms: number;
  /** One per roster unit: its cell (rounded to the nearest) and heading, or
   * `null` when neither keyframe around `t_ms` holds it. */
  units: (KeyframeUnit | null)[];
  /** Whether `t_ms` is NOT a keyframe's own time — so what is drawn is the
   * renderer's, not the arena's at that time: interpolated strictly between
   * two keyframes, or `held` before the first and after the last. */
  between: boolean;
  /** Whether the frame is a keyframe HELD past its own time: `t_ms` before
   * the first keyframe or after the last (the scrubber runs to `end_ms`,
   * usually after the last keyframe — the arena writes no summary at the end),
   * so the positions drawn are that keyframe's, not the arena's at `t_ms`.
   * `before === after` then. */
  held: boolean;
  /** The keyframe at or before `t_ms` (the first when before it). */
  before: number;
  /** The keyframe at or after `t_ms` (the last when after it). */
  after: number;
}

/** A heading, `a` to `b` by `f` in `[0, 1]`, along the SHORTER arc: 350° to
 * 10° passes through 0°, never 180°. */
function turn(a: number, b: number, f: number): number {
  const delta = ((((b - a) % 360) + 540) % 360) - 180;
  return (((Math.round(a + delta * f) % 360) + 360) % 360);
}

/**
 * **Every unit at arena millisecond `t_ms`** (16b cut I.2).
 *
 * On a keyframe's own time it is that keyframe. Strictly between two, each unit
 * moves in a straight line from the one to the other (cells rounded to the
 * nearest) and turns along the shorter arc — a drawing, flagged `between`. A
 * unit missing from one of the two is drawn where the other has it; from both,
 * not at all. Before the first keyframe and after the last the frame is
 * clamped to that keyframe, and flagged `between` AND `held`: the positions are
 * that keyframe's, held, not the arena's at `t_ms`.
 */
export function frameAt(replay: Replay, t_ms: number): ReplayFrame {
  const kfs = replay.keyframes;
  const last = kfs.length - 1;
  const first = kfs[0];
  const final = kfs[last];
  if (first === undefined || final === undefined) {
    return { t_ms, units: replay.roster.map(() => null), between: false, held: false, before: 0, after: 0 };
  }
  const at = (i: number, held = false): ReplayFrame => ({
    t_ms,
    units: [...(kfs[i]?.units ?? [])],
    between: held,
    held,
    before: i,
    after: i,
  });
  if (t_ms <= first.t_ms) {
    return at(0, t_ms < first.t_ms);
  }
  if (t_ms >= final.t_ms) {
    return at(last, t_ms > final.t_ms);
  }
  let after = kfs.findIndex((kf) => kf.t_ms >= t_ms);
  if (after < 0) {
    after = last;
  }
  const b = kfs[after];
  if (b !== undefined && b.t_ms === t_ms) {
    return at(after);
  }
  const before = Math.max(0, after - 1);
  const a = kfs[before];
  if (a === undefined || b === undefined) {
    return at(before);
  }
  const f = (t_ms - a.t_ms) / Math.max(1, b.t_ms - a.t_ms);
  const units = replay.roster.map((_, i): KeyframeUnit | null => {
    const from = a.units[i] ?? null;
    const to = b.units[i] ?? null;
    if (from === null) {
      return to;
    }
    if (to === null) {
      return from;
    }
    return {
      x: Math.round(from.x + (to.x - from.x) * f),
      y: Math.round(from.y + (to.y - from.y) * f),
      heading_deg: turn(from.heading_deg, to.heading_deg, f),
    };
  });
  return { t_ms, units, between: true, held: false, before, after };
}

/** Whether roster unit `i` was a wreck by arena millisecond `t_ms`. */
export function wreckedBy(replay: Replay, i: number, t_ms: number): boolean {
  return replay.destroyed.some(([when, at]) => at === i && when <= t_ms);
}

/**
 * **The replay's map** (16b cut I.2): `composeWindow`'s rule and clamp — a
 * window of `min(width, cols)` by `min(height, rows)` cells centred on the
 * FOCUS unit's cell in `frame` (the ground's middle when it is not in it) and
 * clamped to the arena — and ONE marker order: the ground's named spots first,
 * in authored order, as their letters; then every roster unit in `frame`, in
 * roster order, `o` red, `*` blue, `%` a unit wrecked by `frame.t_ms` (kinds
 * `red`, `blue`, `wreck`, designator the callsign), a later marker overwriting
 * an earlier one; a unit off the window, or `null`, not drawn.
 *
 * `composeReplayWindow(arena_map, replay, frameAt(replay, 5000), 0, 39, 20).grid`
 * over the committed fixtures IS `replay_grid.txt`, byte for byte, and so is
 * the TUI's replay map at a 39 by 22 pane — the parity doc 16 §5 asks for.
 */
export function composeReplayWindow(
  map: ArenaMap,
  replay: Replay,
  frame: ReplayFrame,
  focus: number,
  cols: number,
  rows: number,
): ComposedWindow {
  const width = Math.max(0, Math.trunc(map.width));
  const height = Math.max(0, Math.trunc(map.height));
  const w = Math.min(width, Math.max(1, Math.trunc(cols)));
  const h = Math.min(height, Math.max(1, Math.trunc(rows)));
  const centre = frame.units[focus] ?? null;
  const [cx, cy] = centre === null ? [Math.floor(width / 2), Math.floor(height / 2)] : [centre.x, centre.y];
  const x0 = Math.min(Math.max(cx - Math.floor(w / 2), 0), width - w);
  const y0 = Math.min(Math.max(cy - Math.floor(h / 2), 0), height - h);
  const south = windowGround(map, x0, y0, w, h);
  const terrain: string[] = [];
  for (let y = h - 1; y >= 0; y -= 1) {
    terrain.push((south[y] ?? []).join(""));
  }
  const markers: Marker[] = [];
  const put = (cell: [number, number], glyph: string, kind: MarkerKind, designator: string): void => {
    const x = cell[0] - x0;
    const y = cell[1] - y0;
    if (x < 0 || y < 0 || x >= w || y >= h) {
      return;
    }
    const row = south[y];
    if (!row) {
      return;
    }
    row[x] = glyph;
    markers.push({ x, y, glyph, kind, designator });
  };
  for (const spot of map.spots) {
    put(spot.cell, spot.letter, "spot", spot.name);
  }
  replay.roster.forEach((unit, i) => {
    const at = frame.units[i] ?? null;
    if (at === null) {
      return;
    }
    const wreck = wreckedBy(replay, i, frame.t_ms);
    const kind: MarkerKind = wreck ? "wreck" : unit.team;
    const glyph = wreck ? WRECK_GLYPH : unit.team === "blue" ? BLUE_GLYPH : RED_GLYPH;
    put([at.x, at.y], glyph, kind, unit.callsign);
  });
  const grid: string[] = [];
  for (let y = h - 1; y >= 0; y -= 1) {
    grid.push((south[y] ?? []).join(""));
  }
  return { x0, y0, w, h, grid, terrain, markers };
}

/** An arena millisecond as `m:ss` (the server's `lobby::clock_text`). */
export function clockText(ms: number): string {
  const s = Math.floor(Math.max(0, ms) / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** `4 m 32 s` (the server's `record::duration_words`). */
function durationWords(ms: number): string {
  const secs = Math.floor(Math.max(0, ms) / 1000);
  const hours = Math.floor(secs / 3600);
  const minutes = Math.floor((secs % 3600) / 60);
  const seconds = secs % 60;
  if (hours > 0) {
    return `${hours} h ${minutes} m ${seconds} s`;
  }
  if (minutes > 0) {
    return `${minutes} m ${seconds} s`;
  }
  return `${seconds} s`;
}

/** **How a match ended, in public words** — the rows of the server's
 * `record::outcome_table`, copied (a `Map`, so an inherited key is no word). */
const OUTCOME_WORDS: ReadonlyMap<string, string> = new Map([
  ["red/elimination", "red won (blue eliminated)"],
  ["blue/elimination", "blue won (red eliminated)"],
  ["red/conceded", "red won (blue conceded)"],
  ["blue/conceded", "blue won (red conceded)"],
  // 16b cut O.1 (16c N11).
  ["red/hold", "red won (red held the key spot)"],
  ["blue/hold", "blue won (blue held the key spot)"],
  ["draw/spent", "draw (both sides out of ammunition)"],
  ["draw/elimination", "draw (both sides eliminated)"],
  ["timeout/unattended", "timed out (every pilot had left)"],
  ["timeout/timeout", "timed out (the clock ran out)"],
  ["abort/abort", "aborted"],
  ["abort/restart", "aborted (the server restarted)"],
  ["void/void", "void"],
  ["void/ledger_overflow", "void (the tape overflowed)"],
]);

/** The outcome words for a pair; any other pair as `outcome (reason)`. */
export function outcomeWords(outcome: string, reason: string): string {
  return OUTCOME_WORDS.get(`${outcome}/${reason}`) ?? (reason ? `${outcome} (${reason})` : outcome);
}

/** **The mode's tag** (16b cut O.1): ` [hold]` for a word that is neither empty
 * nor `skirmish`, so a match fought under another rule is never read as a
 * plain fight; nothing for a skirmish or a document with no `mode`. */
export function modeTag(mode: string): string {
  return mode === "" || mode === "skirmish" ? "" : ` [${mode}]`;
}

/** One row of the units table. */
export interface ReplayUnitRow {
  callsign: string;
  side: "red" | "blue";
  /** `(6, 4)  090°`, `wreck`, or `off the window`. */
  where: string;
}

/** The whole of what the replay screen prints. */
export interface ReplayView {
  /** `Replay of match 12 — red won (blue eliminated) — both sides are shown:
   * this match is over.` */
  header: string;
  /** The label under the canvas: a keyframe's own sentence, the
   * between-keyframes one, or a held keyframe's (`After keyframe 3 (0:10): the
   * last recorded positions, held.`). */
  label: string;
  /** `1:30 of 4:32`. */
  clock: string;
  /** The markers that own their cell, then the spots (`markerLegend`). */
  legend: string;
  /** The shots fired up to the cursor, newest first: `1:27  RA at BA: hit`. */
  shots: string[];
  /** The wrecks up to the cursor, newest first: `1:25  BB wrecked`. */
  wrecks: string[];
  /** Every roster unit where the frame draws it. */
  units: ReplayUnitRow[];
  /** One sentence per cut, the line client's words. */
  truncation: string[];
  /** **A live picture's status lines** (16b cut I.4, `liveView`); empty for a
   * replay. */
  status: string[];
}

const SHOT_WORDS: ReadonlyMap<string, string> = new Map([
  ["flying", "in flight"],
  ["hit", "hit"],
  ["miss", "miss"],
]);

/** **The replay screen's view model** (16b cut I.2): every string the screen
 * prints for `replay` at `cursorMs`, drawn as `frame` in the window `win`.
 * Pure, so vitest pins it (`replay_view_model.txt`) and the component decides
 * none of its words itself. */
export function replayView(
  replay: Replay,
  frame: ReplayFrame,
  win: ComposedWindow | null,
  cursorMs: number,
): ReplayView {
  const id = replay.match;
  const callsign = (i: number): string => replay.roster[i]?.callsign ?? "?";
  const n = replay.keyframes.length;
  const kfClock = clockText(replay.keyframes[frame.before]?.t_ms ?? 0);
  // Three sentences, never one for another: the arena's own positions only ON
  // a keyframe; a keyframe held past its time says it is held (the clock reads
  // on while the units stand); between two says the renderer drew it.
  const label = frame.held
    ? frame.t_ms < (replay.keyframes[0]?.t_ms ?? 0)
      ? `Before keyframe 1 (${kfClock}): the first recorded positions, held.`
      : `After keyframe ${frame.before + 1} (${kfClock}): the last recorded positions, held.`
    : frame.between
      ? `Between keyframes ${frame.before + 1} and ${frame.after + 1}: drawn, not recorded.`
      : `Keyframe ${frame.before + 1} of ${n} (${kfClock}): the arena's own positions.`;
  const shots = replay.shots
    .filter((s) => s.t_ms <= cursorMs)
    .map((s) => `${clockText(s.t_ms)}  ${callsign(s.shooter)} at ${callsign(s.target)}: ${SHOT_WORDS.get(s.outcome) ?? "?"}`)
    .reverse();
  const wrecks = replay.destroyed
    .filter(([when]) => when <= cursorMs)
    .map(([when, at]) => `${clockText(when)}  ${callsign(at)} wrecked`)
    .reverse();
  const units: ReplayUnitRow[] = replay.roster.map((unit, i) => {
    const at = frame.units[i] ?? null;
    const where = wreckedBy(replay, i, frame.t_ms)
      ? "wreck"
      : at === null
        ? "off the window"
        : `(${at.x}, ${at.y})  ${String(at.heading_deg).padStart(3, "0")}°`;
    return { callsign: unit.callsign, side: unit.team, where };
  });
  const lastKeyframe = replay.keyframes[n - 1]?.t_ms ?? 0;
  const truncation = [...replay.cut];
  if (replay.truncated && truncation.length === 0) {
    truncation.push("keyframes");
  }
  return {
    header: `Replay of match ${id} — ${outcomeWords(replay.outcome, replay.reason)}${modeTag(replay.mode)} — both sides are shown: this match is over.`,
    label,
    clock: `${clockText(cursorMs)} of ${clockText(replay.end_ms)}`,
    legend: markerLegend(win),
    shots,
    wrecks,
    units,
    truncation: truncation.map((cut) => {
      switch (cut) {
        case "roster":
          return `The replay names the first ${replay.roster.length} units of match ${id}; the tape has the rest.`;
        case "keyframes":
          return `The replay holds the first ${durationWords(lastKeyframe)} of match ${id}; the tape has the rest.`;
        case "shots":
          return `The replay keeps the shots of match ${id} to ${clockText(replay.shots[replay.shots.length - 1]?.t_ms ?? 0)}; the tape has the rest.`;
        default:
          return `A page of the tape of match ${id} could not be read; the replay is missing what it held.`;
      }
    }),
    status: [],
  };
}

/** Every line of a view, flattened — the golden's subject and the observer
 * rule's lens: a word not in here is not on the screen. */
export function replayViewLines(view: ReplayView): string[] {
  const lines = [view.header, view.label, view.clock];
  if (view.legend.length > 0) {
    lines.push(view.legend);
  }
  lines.push(...view.units.map((u) => `${u.callsign.padEnd(4)} ${u.side.padEnd(5)} ${u.where}`));
  lines.push(...(view.shots.length > 0 ? view.shots : ["No shots yet."]));
  lines.push(...(view.wrecks.length > 0 ? view.wrecks : ["No wrecks yet."]));
  lines.push(...view.truncation);
  // 16b cut I.4: a live picture's status lines (a replay has none, and its
  // golden is unchanged).
  lines.push(...view.status);
  return lines;
}
