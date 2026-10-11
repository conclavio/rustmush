// The cockpit on the web (16b cut Q.5; doc 16 §5): the wire types of the two
// arena feeds, their decoders, the ONE map composition every renderer runs,
// and the view model `CockpitScreen.svelte` paints.
//
// Everything here is pure and DOM-free, so vitest pins what the player sees
// without a browser: the canvas draws from `composeWindow`'s output and the
// screen prints `cockpitView`'s strings, and nothing in the component decides
// either. That is what makes the parity golden (`fixtures/cockpit/view_grid.txt`)
// an assertion about the web client and not about a test helper. The golden is
// `VIEW`'s window, 39 by 20 cells, the line client printing each cell twice
// (16b cut B.4); the file holds one glyph per cell, which is what this client
// composes.
//
// **The observer rule** (doc 16 §1, §5) is enforced by the SHAPE of the types
// below rather than by care at each print site: `CockpitFrame` carries no
// contact `id`, no `lock` digits, no `unit` object number and no notice `text`.
// A renderer cannot leak a field its decoder never kept, and a notice's own
// sentence already reaches the scrollback as a `Sys` line (16b cut F.2's
// one-owner-per-sentence contract), so the ring here is composed from `kind` +
// `designator` + `was` + `section` and the refusal's parts, and never from the
// notice's own prose. (The store appends the pilot's `Sys` replies to the same
// ring while seated — 16b cut B.1 — because on screen 0 it is the only reply
// surface; those lines are already plain text and are not composed here. The
// bridge's `Sys` copy of a notice wears `NOTICE_SENTENCE_TAG` and is never
// rung, so the one-owner rule holds on the ring too.)
//
// The payload is a `serde_json::Value` on the server — outside ts-rs — so these
// types are hand-written, and every decoder is tolerant of a missing key and of
// an unknown one: the frame evolves additively (16b cut Q.1 and Q.2 both added
// fields at `FRAME_VERSION` 1, and 17b cut M1a.3's fields at 2 — `aspect`,
// `reads_dead`, `odds` and `fatal` per contact, `bands_cm` per weapon, `t_ms` and
// `order` at the top, and no `weapons` key at all on a hull with no weapons
// station; 17b cut M1b.3's at 3 — `armed` per weapon and `odds_floor_permille`
// at the top; 17b cut M5.4's `mind` at the top, added without a version and
// decoded since 17b cut M5.5), and neither an older server nor a newer one may
// make this client throw. The version is not read.

/** The per-pulse cockpit keyframe's feed (`tacsim::arena_bridge::FRAME_FEED`). */
export const COCKPIT_FEED = "arena.cockpit";

/** The whole arena's terrain, pulled and never pushed (`MAP_FEED`). */
export const ARENA_MAP_FEED = "arena.map";

/** The span tag on the `Sys` copy of a frame's own notice sentence
 * (`tacsim::arena_bridge::NOTICE_SENTENCE_TAG`, 16b cut B.1). The pilot's
 * replies carry the arena's category, `arena`, instead. */
export const NOTICE_SENTENCE_TAG = "arena-notice";

/**
 * Whether a `Sys` line is the bridge's copy of a notice the next frame carries
 * (16b cut B.1). The notices ring composes that notice from the frame's fields
 * (`noticeLine`), so ringing the sentence as a reply too would say every notice
 * twice — cut F.2's one owner per sentence. Only the tag decides, never the
 * words: the first span carries it, as `isRollNarration` reads `roll`.
 */
export function isNoticeSentence(text: readonly { tag?: string | null }[]): boolean {
  return text[0]?.tag === NOTICE_SENTENCE_TAG;
}

/** The default square viewport of the web's map, in arena cells: a sensor's
 * sixty reach every way from the middle and still leaves margin. 64 would hide
 * the 330–600 m band the sensor's sight covers; a hostile is told apart by its
 * label, not by the zoom (16b cut B.1, `B-09`: see `labelledAt`). */
export const COCKPIT_VIEW_CELLS = 128;

/** The viewport sizes the zoom control steps through, in arena cells. */
export const COCKPIT_ZOOM_CELLS: readonly number[] = [32, 64, 128, 256];

/** How many composed notice lines the ring keeps. */
export const COCKPIT_NOTICES = 50;

/** The empty state of a viewer who is not in a cockpit. Byte-identical to the
 * TUI's, and it says nothing about whether a match is being fought. */
export const NOT_IN_A_COCKPIT = "Not in a cockpit.";

/** The map pane while the pulled terrain has not arrived. */
export const WAITING_FOR_THE_MAP = "Waiting for the arena map...";

/** A scan band letter, as `Condition::letters` spells it (16b cut Q.2), and
 * `unknown` for a letter this build predates (16b cut B.1, `T-58`): it is
 * uncoloured rather than painted green, which would say "whole". */
export type ScanBand = "green" | "yellow" | "red" | "destroyed" | "unknown";

/** What a marker is, which is the whole of what the canvas colours it by.
 * `spot` (16b cut V.1) is one of the ground's named places — surveyed ground,
 * never a unit. `red`, `blue` and `wreck` (16b cut I.2) are a replay's units —
 * both sides of a finished match, drawn by `composeReplayWindow` and never by
 * the cockpit's own `composeWindow`. */
export type MarkerKind =
  | "you"
  | "unknown"
  | "friendly"
  | "hostile"
  | "spot"
  | "red"
  | "blue"
  | "wreck";

export interface CockpitCondition {
  /** The index into the frame's `scan_tables`. */
  table: number;
  /** One letter a section, `g` / `y` / `r` / `d`. */
  bands: string;
  /** The age of the LOOK, which is older than the plot. */
  age_ms: number;
}

export interface CockpitContact {
  designator: string;
  alias: string | null;
  /** The classification word, printed and never interpreted. */
  class: string;
  bearing_deg: number;
  relative_deg: number;
  band: string;
  range_m: number;
  age_ms: number;
  age_s: number;
  /** The ABSOLUTE arena cell the contact last plotted at, or `null`. */
  cell: [number, number] | null;
  friendly: boolean;
  condition: CockpitCondition | null;
  /** The FACET word the contact's plot was showing — `front`, `left`, `right`
   * or `rear` (17b cut M1a.3); `front` from a server that does not send it. The
   * row prints its side word (`sideWord`). */
  aspect: string;
  /** Whether this platform has reason to believe the contact is dead (17b cut
   * M1a.3), or `null` on a hull with no weapons station — and from a server
   * that does not send the key. */
  reads_dead: boolean | null;
  /** The chances against this contact, the default weapon first (17b cut
   * M1a.3); `[]` for a contact the fire control is not resolving (since 17b
   * cut M1b.3 every friend, an unclassified return and a hostile past the lock
   * and the nearest), on an unarmed hull, and from an older server. */
  odds: CockpitOdds[];
  /** The target chassis's fatal section words beside a held reading (17b cut
   * M1a.3), or `null`. */
  fatal: string[] | null;
}

/** One weapon's advertised chance against one contact (17b cut M1a.3). */
export interface CockpitOdds {
  weapon: string;
  p_permille: number;
  percent: number;
}

/** A standing order as the keyframe spells it (17b cut M1a.3): `target` and
 * `of` are the designators of the hull's own rows, never an id. Every piloted
 * hull's frame carries `order: null`. */
export interface CockpitOrder {
  squad: string;
  objective: string;
  target: string | null;
  to: [number, number] | null;
  facing_mdeg: number | null;
  of: string | null;
  hold_fire: boolean;
  max_range_band: string | null;
  expires_in_ms: number;
}

/** The numbers a refusal's sentence carries (`RefusalDetail`), or `null`. */
export interface CockpitNoticeDetail {
  /** Seconds until the weapon is ready, on a `cycling` refusal. */
  secs: number | null;
  /** What the platform is carrying, on an `overheated` one. */
  heat: number | null;
  /** Its ceiling. */
  max: number | null;
  /** **17b cut M1b.4**: the Compact floor an `unlawful` refusal was held to,
   * per mille — present only when the wire carries it as an integer, so every
   * other detail decodes to exactly the three keys above. */
  floor_permille?: number;
}

/** **Why a slot held a shot** (17b cut M1b.4; doc 17 §5), off a `slot_held`
 * notice's `detail`: `why` is `odds`, `aspect`, `range` or `rounds`, and each
 * other key is that why's own number (or facet word), `null` elsewhere. */
export interface CockpitHeldWhy {
  why: string;
  percent: number | null;
  floor: number | null;
  aspect: string | null;
  range_m: number | null;
  min_m: number | null;
  max_m: number | null;
  rounds: number | null;
  keeps: number | null;
}

/** **A slot notice's own parts** (17b cut M1b.4; D-UM-14): the slot number,
 * and on a `slot_fired` the chance and the facet the shot read, on a
 * `slot_held` the reason. */
export interface CockpitSlotNotice {
  slot: number;
  percent: number | null;
  aspect: string | null;
  held: CockpitHeldWhy | null;
}

export interface CockpitNotice {
  /** The wire word (`NoticeKind::as_str`). */
  kind: string;
  designator: string | null;
  /** The old name an `identified` or `unidentified again` line quotes, or
   * `null` (16b cut B.1, `T-86`). A designator this platform issued or read. */
  was: string | null;
  /** The section the notice is about — the target's on the gunner's kinds,
   * the pilot's own on the victim's (16b cut B.1 widened it); `kind` says
   * whose. */
  section: string | null;
  /** Why motion stopped (`BlockedCause::as_str`), or `null`. 16b cut D.2. */
  cause: string | null;
  /** Why the order was refused (`Refusal::as_str`), or `null`. */
  refusal: string | null;
  /** The refused order's own word (`heading`, `fire`, …), or `null`. */
  order: string | null;
  /** The weapon it named, or `null`. */
  weapon: string | null;
  /** The numbers that refusal's sentence carries, or `null`. */
  detail: CockpitNoticeDetail | null;
  /** The contact this notice names is on the platform's own side. */
  friendly: boolean;
  /** The round found a section that was already destroyed. */
  spent: boolean;
  /** **17b cut M1b.4**: a `slot_fired` or `slot_held` notice's own parts —
   * present on those two kinds alone, so every other notice decodes to
   * exactly the keys above. */
  slot?: CockpitSlotNotice;
}

export interface CockpitWeapon {
  id: string;
  ready: boolean;
  ready_in_ms: number;
  rounds: number;
  rounds_max: number;
  /** This weapon's own chance against the lock, or `null` without one. */
  percent: number | null;
  /** What one shot adds to the platform's heat; `0` for a cold gun (16b cut
   * D.2), which is what an overheated tank may still fire. */
  heat_per_shot: number;
  /** The weapon's band edges, centimetres (17b cut M1a.3); `[]` from an older
   * server. */
  bands_cm: number[];
  /** The condition slot this weapon is armed in (17b cut M1b.3), or `null` —
   * a weapon no slot holds, and every weapon from an older server. */
  armed: CockpitArming | null;
}

/**
 * **One weapon's armed slot** (17b cut M1b.3; doc 17 §3, §5, D-UM-4): the
 * pilot's own slot number and its terms. `target` is the KIND — `lock`,
 * `best` or `contact` — and never a designator (the server sends none).
 * `aspects` are the facet words in the mask (`front`, `left`, `right`,
 * `rear`); `keeps` the rounds the slot keeps back; a `null` range bound is
 * none.
 */
export interface CockpitArming {
  slot: number;
  target: string;
  min_permille: number;
  percent: number;
  aspects: string[];
  keeps: number;
  min_range_m: number | null;
  max_range_m: number | null;
}

/**
 * **The hull's sealed mind** (17b cut M5.4's keyframe object, decoded by 17b
 * cut M5.5; doc 17 §12, OQ-8): what the pilot's own seal says, for the person
 * in the seat alone — never a seal id, never a card. Both classes are read
 * live by the arena (a degraded core reads as it is now).
 */
export interface CockpitMind {
  /** `Standard Doctrine`, or `null` for a seal that is not Standard Doctrine
   * (the pilot's own mind, whose name the frame does not carry). */
  name: string | null;
  /** The sealed profiles, in sealed order — one to four of the six preset
   * names (`MAX_MIND_PROFILES`). */
  profiles: string[];
  /** The index into `profiles` of the one flown; `-1` when the frame names
   * none of the profiles kept. */
  active: number;
  /** The class the mind thinks at — `I`, `II` or `III` — or `0`, where the ROM
   * flies the hull (no core, a core at Class 0, a seal capped there). */
  class: string;
  /** The class of the hull's core (`0` with no core). */
  core: string;
  /** The channels the pilot holds: `helm`, `sensors` and, on an armed hull,
   * `fire`. */
  channels: string[];
  /** The ABSOLUTE arena millisecond from which the next `MIND PROFILE` switch
   * is taken, or `null`. Decoded and never drawn: a quiet frame would leave a
   * drawn countdown stale (line mode's `MIND` prints it off the mirror's
   * clock). */
  next_switch_ms: number | null;
  /** The hull was dropped to the ROM for the match. */
  off: boolean;
}

/** The most profiles a seal carries (the arena's `ProfileSet`, four codes). */
export const MAX_MIND_PROFILES = 4;

export interface CockpitFacet {
  id: string;
  armor: number;
  armor_max: number;
}

export interface CockpitSection {
  id: string;
  /** The ruleset's own condition word. */
  condition: string;
  internals: number;
  internals_max: number;
  facets: CockpitFacet[];
}

export interface CockpitHeat {
  heat: number;
  max: number;
  overheated: boolean;
}

/** The platform's own receiver while it is degraded (16b cut D.4's `receiver`
 * key, decoded by 16b cut B.1): the attenuation budget its last pass ran with,
 * that budget unjammed, and what is left of the reach as a whole per cent.
 * Three integers about the pilot's own machine and nothing about the source. */
export interface CockpitReceiver {
  budget: number;
  budget_max: number;
  reach_percent: number;
}

/** One section's words on a catch-up (16b cut Y.4): the condition word, the
 * facets' armour summed, the internals. */
export interface CockpitSectionWords {
  condition: string;
  armour: number;
  internals: number;
}

/**
 * **The catch-up report** (16b cut Y.4; doc 07 §1.3 step 5): what happened
 * while the pilot could not see, on the first frame after a reclaim and `null`
 * on every other. The pilot's own tank, plus the platform's own designators for
 * the contacts it gained and lost — nothing about a unit it never held.
 */
export interface CockpitCatchup {
  /** Whole seconds, the floor of the arena's milliseconds. */
  away_s: number;
  cell_then: [number, number] | null;
  cell_now: [number, number] | null;
  power_then: string;
  power_now: string;
  wreck: boolean;
  contacts_new: string[];
  contacts_lost: string[];
  contacts_more: number;
  sections: { id: string; then: CockpitSectionWords; now: CockpitSectionWords }[];
  sections_more: number;
  rounds_spent: { weapon: string; rounds: number }[];
  rounds_more: number;
  machine: { state: string; goal: [number, number] | null; objective: string | null } | null;
}

/** The most new or lost contacts a catch-up names, each list (the arena's
 * `MAX_CATCHUP_CONTACTS`). */
export const MAX_CATCHUP_CONTACTS = 12;
/** The most changed sections a catch-up names (`MAX_CATCHUP_SECTIONS`). */
export const MAX_CATCHUP_SECTIONS = 6;
/** The most weapons a catch-up names rounds spent for (`MAX_CATCHUP_WEAPONS`). */
export const MAX_CATCHUP_WEAPONS = 6;

/**
 * One `arena.cockpit` keyframe, decoded.
 *
 * `seated` is `false` for the cleared frame the bridge sends once when a pilot
 * is unbound (`cockpit_cleared()`); it is the LAST frame that pilot gets, and
 * a client must treat it as "clear the panel and drop the map".
 */
export interface CockpitFrame {
  seated: boolean;
  /** The match this frame belongs to, or `null` on a cleared frame. */
  match: number | null;
  callsign: string;
  /** The platform's own arena cell, or `null` before it has one. */
  cell: [number, number] | null;
  power: string;
  /** Motion is held (the `HELD` mark). */
  blocked: boolean;
  /** The tank is a wreck and takes no order (16b cut D.2). */
  wreck: boolean;
  /** The tank cannot move (16b cut D.2). */
  immobile: boolean;
  speed_kph: number;
  heading_deg: number;
  heat: CockpitHeat | null;
  /** The receiver, or `null` on a clean one (and from a server that does not
   * send the key). The row's presence is what says "degraded". */
  receiver: CockpitReceiver | null;
  /** The platform's own jamming pod's switch (16b cut O.2, 16c N12): `"on"`
   * or `"off"`, and `null` for a platform that carries no pod — and from a
   * server that does not send the key, or a word this build predates. The
   * status block's scope row carries it as a suffix. */
  ecm: "on" | "off" | null;
  /** The arena's one global cadence word. The status block does NOT print it
   * (16b cut D.2), and no shipped client reads it today; it stays decoded
   * because `T-59` is closed for the printed word alone. */
  tempo: string;
  /** THIS platform's own alert state — `clear`, `unidentified` or `hostile`
   * (16b cut D.1) — which is what the status block prints. */
  scope: string;
  lock_designator: string | null;
  lock_range_m: number | null;
  lock_percent: number | null;
  sections: CockpitSection[];
  weapons: CockpitWeapon[];
  contacts: CockpitContact[];
  /** The distinct section-id lists a `condition.table` indexes (16b cut Q.2). */
  scan_tables: string[][];
  notices: CockpitNotice[];
  /** The catch-up report (16b cut Y.4), or `null` on every frame but a
   * reclaim's first — and from a server that does not send the key. */
  catchup: CockpitCatchup | null;
  /** The arena millisecond the frame was built at (17b cut M1a.3), or `null`
   * from a server that does not send it. */
  t_ms: number | null;
  /** The hull's standing order (17b cut M1a.3), or `null` — every piloted
   * hull, and an older server. */
  order: CockpitOrder | null;
  /** The match's Compact floor, per mille (17b cut M1b.3), captured at
   * check-out — or `null` from a server that does not send the key. */
  odds_floor_permille: number | null;
  /** Whether the frame carries the `weapons` key (17b cut M1a.3): a hull with
   * no weapons station carries none, and its contact rows end at the aspect. */
  armed: boolean;
  /** The hull's sealed mind (17b cut M5.4's key, decoded by 17b cut M5.5), or
   * `null` — a hull no seal reached, and an older server. */
  mind: CockpitMind | null;
}

/** One of the ground's named places, as `arena.map` carries it (16b cut V.1):
 * the marker letter it is drawn as, its authored name, and its ABSOLUTE arena
 * cell. */
export interface ArenaSpot {
  letter: string;
  name: string;
  cell: [number, number];
}

/** One `arena.map` keyframe, decoded. Row `0` of `rows` is the SOUTH edge. */
export interface ArenaMap {
  seated: boolean;
  match: number | null;
  surface: string;
  cell_cm: number;
  width: number;
  height: number;
  origin_cell: [number, number];
  rows: [string, number][][];
  legend: [string, string][];
  /** The ground's named spots in authored order (16b cut V.1); `[]` from a
   * server that sends none, and from the unseated answer. */
  spots: ArenaSpot[];
}

/** A marker drawn on the composed window. */
export interface Marker {
  /** The window column. */
  x: number;
  /** The window ROW FROM THE SOUTH, so `grid[h - 1 - y]` is the line it is on. */
  y: number;
  glyph: string;
  kind: MarkerKind;
  /** The designator (or the pilot's callsign for their own marker, or — 16b
   * cut V.1 — a spot's authored name). It is the marker's label. */
  designator: string;
}

/** What `composeWindow` answers: the window's corner and size, the glyph rows
 * NORTH-UP with the markers already written into them, and the markers that
 * landed, in the order they were drawn. */
export interface ComposedWindow {
  x0: number;
  y0: number;
  w: number;
  h: number;
  /** `h` rows of exactly `w` glyphs, `grid[0]` the NORTH row, WITH the markers
   * written in — what a text client prints, and the parity golden's subject. */
  grid: string[];
  /** The same rows with the markers left off: the ground alone, so a canvas
   * that paints terrain by glyph never reads `+` as a door or `x` as rubble
   * because a contact happened to be standing there. */
  terrain: string[];
  markers: Marker[];
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** Server text on a pane border and in a prefill: control characters out, cut
 * to `max`. The TUI's `clean` + `truncate_chars`, in the same places. */
function clean(raw: unknown, fallback: string, max = 32): string {
  if (typeof raw !== "string") {
    return fallback;
  }
  const stripped = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, "").trim();
  return stripped.length === 0 ? fallback : [...stripped].slice(0, max).join("");
}

function int(raw: unknown, fallback: number): number {
  return typeof raw === "number" && Number.isFinite(raw) ? Math.trunc(raw) : fallback;
}

function optInt(raw: unknown): number | null {
  return typeof raw === "number" && Number.isFinite(raw) ? Math.trunc(raw) : null;
}

function optStr(raw: unknown, max = 32): string | null {
  return typeof raw === "string" ? clean(raw, "", max) || null : null;
}

/** A `[x, y]` cell pair, or `null` for anything else (the key is nullable). */
function cellPair(raw: unknown): [number, number] | null {
  if (!Array.isArray(raw) || raw.length < 2) {
    return null;
  }
  const x = optInt(raw[0]);
  const y = optInt(raw[1]);
  return x !== null && y !== null && x >= 0 && y >= 0 ? [x, y] : null;
}

function decodeCondition(raw: unknown): CockpitCondition | null {
  if (!isObj(raw)) {
    return null;
  }
  const bands = typeof raw.bands === "string" ? raw.bands : "";
  return bands.length === 0
    ? null
    : { table: Math.max(0, int(raw.table, 0)), bands, age_ms: int(raw.age_ms, 0) };
}

function decodeContact(raw: unknown): CockpitContact | null {
  if (!isObj(raw)) {
    return null;
  }
  return {
    // The designator is the row key and the word a prefill carries, so it is
    // cut to what a designator can be (the TUI cuts it to eight too).
    designator: clean(raw.designator, "?", 8),
    alias: optStr(raw.alias, 8),
    class: clean(raw.class, "?"),
    bearing_deg: int(raw.bearing_deg, 0),
    relative_deg: int(raw.relative_deg, 0),
    band: clean(raw.band, "?"),
    range_m: int(raw.range_m, 0),
    age_ms: int(raw.age_ms, 0),
    age_s: int(raw.age_s, 0),
    cell: cellPair(raw.cell),
    friendly: raw.friendly === true,
    condition: decodeCondition(raw.condition),
    // 17b cut M1a.3: every one tolerant of absence (an older server).
    aspect: clean(raw.aspect, "front", 8),
    reads_dead: typeof raw.reads_dead === "boolean" ? raw.reads_dead : null,
    odds: (Array.isArray(raw.odds) ? raw.odds : []).flatMap((o) => {
      const odds = decodeOdds(o);
      return odds ? [odds] : [];
    }),
    fatal: Array.isArray(raw.fatal)
      ? raw.fatal.flatMap((word) => {
          const named = optStr(word, 24);
          return named === null ? [] : [named];
        })
      : null,
  };
}

/** One odds entry (17b cut M1a.3); `null` without a weapon. */
function decodeOdds(raw: unknown): CockpitOdds | null {
  if (!isObj(raw)) {
    return null;
  }
  const weapon = optStr(raw.weapon, 24);
  if (weapon === null) {
    return null;
  }
  const p_permille = Math.max(0, int(raw.p_permille, 0));
  return {
    weapon,
    p_permille,
    percent: Math.max(0, int(raw.percent, Math.trunc(p_permille / 10))),
  };
}

/** The facet words a slot's mask may carry, in mask order (17b cut M1b.3). */
const FACET_WORDS: readonly string[] = ["front", "left", "right", "rear"];

/** One weapon's `armed` object (17b cut M1b.3), bounded and cleaned as
 * `decodeOdds` is: `null` for anything not an object and for one without a
 * slot; a target word this build predates is kept (cleaned, cut short) and
 * prints nothing; facet words it does not know are dropped. */
function decodeArming(raw: unknown): CockpitArming | null {
  if (!isObj(raw)) {
    return null;
  }
  const slot = optInt(raw.slot);
  if (slot === null || slot < 0) {
    return null;
  }
  const min_permille = Math.max(0, int(raw.min_permille, 0));
  const bound = (v: unknown): number | null => {
    const n = optInt(v);
    return n === null || n < 0 ? null : n;
  };
  return {
    slot,
    target: clean(raw.target, "lock", 8),
    min_permille,
    percent: Math.max(0, int(raw.percent, Math.trunc(min_permille / 10))),
    aspects: (Array.isArray(raw.aspects) ? raw.aspects : []).flatMap((word) =>
      typeof word === "string" && FACET_WORDS.includes(word) ? [word] : [],
    ),
    keeps: Math.max(0, int(raw.keeps, 0)),
    min_range_m: bound(raw.min_range_m),
    max_range_m: bound(raw.max_range_m),
  };
}

/** A standing order (17b cut M1a.3), or `null` for anything not an object. */
function decodeOrder(raw: unknown): CockpitOrder | null {
  if (!isObj(raw)) {
    return null;
  }
  return {
    squad: clean(raw.squad, "?", 16),
    objective: clean(raw.objective, "?", 16),
    target: optStr(raw.target, 8),
    to: cellPair(raw.to),
    facing_mdeg: optInt(raw.facing_mdeg),
    of: optStr(raw.of, 8),
    hold_fire: raw.hold_fire === true,
    max_range_band: optStr(raw.max_range_band, 16),
    expires_in_ms: Math.max(0, int(raw.expires_in_ms, 0)),
  };
}

/**
 * Decode the frame's `mind` object (17b cut M5.5); `null` for anything that is
 * not an object. A profile that is not a string is dropped and `active` still
 * names the profile the wire's index named (`-1` when that one was dropped);
 * the set is cut to `MAX_MIND_PROFILES`, each name to 16, each class word to
 * 8, the channels to four of 16.
 */
export function decodeMind(raw: unknown): CockpitMind | null {
  if (!isObj(raw)) {
    return null;
  }
  const flown = optInt(raw.active);
  const profiles: string[] = [];
  let active = -1;
  (Array.isArray(raw.profiles) ? raw.profiles : []).forEach((entry, at) => {
    const name = optStr(entry, 16);
    if (name === null || profiles.length >= MAX_MIND_PROFILES) {
      return;
    }
    if (at === flown) {
      active = profiles.length;
    }
    profiles.push(name);
  });
  return {
    name: optStr(raw.name, 32),
    profiles,
    active,
    class: clean(raw.class, "?", 8),
    core: clean(raw.core, "?", 8),
    channels: names(raw.channels, 16, 4),
    next_switch_ms: optInt(raw.next_switch_ms),
    off: raw.off === true,
  };
}

function decodeDetail(raw: unknown): CockpitNoticeDetail | null {
  if (!isObj(raw)) {
    return null;
  }
  const detail: CockpitNoticeDetail = {
    secs: optInt(raw.secs),
    heat: optInt(raw.heat),
    max: optInt(raw.max),
  };
  // 17b cut M1b.4: the floor an `unlawful` refusal names, only when it is there.
  const floor = optInt(raw.floor_permille);
  return floor === null ? detail : { ...detail, floor_permille: floor };
}

/** A slot notice's parts (17b cut M1b.4), or `undefined` for any other kind
 * and for a slot notice with no integer `slot` (which then reads as its kind's
 * words). */
function decodeSlot(raw: Record<string, unknown>, kind: string): CockpitSlotNotice | undefined {
  if (kind !== "slot_fired" && kind !== "slot_held") {
    return undefined;
  }
  const slot = optInt(raw.slot);
  if (slot === null) {
    return undefined;
  }
  const detail = raw.detail;
  const held: CockpitHeldWhy | null =
    kind === "slot_held" && isObj(detail) && typeof detail.why === "string"
      ? {
          why: clean(detail.why, "?", 24),
          percent: optInt(detail.percent),
          floor: optInt(detail.floor),
          aspect: optStr(detail.aspect, 24),
          range_m: optInt(detail.range_m),
          min_m: optInt(detail.min_m),
          max_m: optInt(detail.max_m),
          rounds: optInt(detail.rounds),
          keeps: optInt(detail.keeps),
        }
      : null;
  return { slot, percent: optInt(raw.percent), aspect: optStr(raw.aspect, 24), held };
}

function decodeNotice(raw: unknown): CockpitNotice | null {
  if (!isObj(raw) || typeof raw.kind !== "string") {
    return null;
  }
  // `text` is deliberately NOT read: the arena already sent that sentence as a
  // `Sys` line and one sentence has one owner (16b cut F.2). 16b cut D.2 is what
  // made that possible to honour — until it the sentence's parts were not on the
  // wire at all, so the ring printed a bare `refused` with no order and no
  // reason, which is `B-07`.
  const kind = clean(raw.kind, "?");
  const notice: CockpitNotice = {
    kind,
    designator: optStr(raw.designator, 8),
    // 16b cut B.1 (`T-86`): cut to what a designator can be, like `designator`.
    was: optStr(raw.was, 8),
    section: optStr(raw.section, 24),
    cause: optStr(raw.cause, 24),
    refusal: optStr(raw.refusal, 24),
    order: optStr(raw.order, 16),
    weapon: optStr(raw.weapon, 24),
    detail: decodeDetail(raw.detail),
    friendly: raw.friendly === true,
    spent: raw.spent === true,
  };
  // 17b cut M1b.4: a slot notice's own parts, on those two kinds alone.
  const slot = decodeSlot(raw, kind);
  return slot === undefined ? notice : { ...notice, slot };
}

function decodeWeapon(raw: unknown): CockpitWeapon | null {
  if (!isObj(raw)) {
    return null;
  }
  return {
    id: clean(raw.id, "?", 24),
    ready: raw.ready === true,
    ready_in_ms: int(raw.ready_in_ms, 0),
    rounds: int(raw.rounds, 0),
    rounds_max: int(raw.rounds_max, 0),
    percent: optInt(raw.percent),
    heat_per_shot: Math.max(0, int(raw.heat_per_shot, 0)),
    // 17b cut M1a.3.
    bands_cm: (Array.isArray(raw.bands_cm) ? raw.bands_cm : []).flatMap((edge) => {
      const cm = optInt(edge);
      return cm === null ? [] : [cm];
    }),
    // 17b cut M1b.3.
    armed: decodeArming(raw.armed),
  };
}

function decodeSection(raw: unknown): CockpitSection | null {
  if (!isObj(raw)) {
    return null;
  }
  const facets: CockpitFacet[] = (Array.isArray(raw.facets) ? raw.facets : []).flatMap((f) =>
    isObj(f)
      ? [{ id: clean(f.id, "?", 24), armor: int(f.armor, 0), armor_max: int(f.armor_max, 0) }]
      : [],
  );
  return {
    id: clean(raw.id, "?", 24),
    condition: clean(raw.condition, "?"),
    internals: int(raw.internals, 0),
    internals_max: int(raw.internals_max, 0),
    facets,
  };
}

function decodeHeat(raw: unknown): CockpitHeat | null {
  return isObj(raw)
    ? { heat: int(raw.heat, 0), max: int(raw.max, 0), overheated: raw.overheated === true }
    : null;
}

function decodeReceiver(raw: unknown): CockpitReceiver | null {
  return isObj(raw)
    ? {
        budget: int(raw.budget, 0),
        budget_max: int(raw.budget_max, 0),
        reach_percent: int(raw.reach_percent, 0),
      }
    : null;
}

/** A catch-up count: a whole number, never below nought. */
function count(raw: unknown): number {
  return Math.max(0, int(raw, 0));
}

/** The strings of one catch-up list, cleaned and cut to `max`, a blank or
 * non-string entry dropped, the list cut to `cap`. */
function names(raw: unknown, max: number, cap: number): string[] {
  return (Array.isArray(raw) ? raw : [])
    .flatMap((entry) => {
      const name = optStr(entry, max);
      return name === null ? [] : [name];
    })
    .slice(0, cap);
}

function decodeWords(raw: unknown): CockpitSectionWords | null {
  return isObj(raw)
    ? {
        condition: clean(raw.condition, "?", 16),
        armour: int(raw.armour, 0),
        internals: int(raw.internals, 0),
      }
    : null;
}

/**
 * Decode the frame's `catchup` object (16b cut Y.4); `null` for anything that
 * is not an object. Every string is cleaned and cut the way the TUI's
 * `catchup_words` cuts it — a designator to 8, a section or weapon id to 24,
 * the state, the objective and every condition and power word to 16 — each list
 * is cut to what the arena sends at most, and a malformed entry is dropped.
 */
export function decodeCatchup(raw: unknown): CockpitCatchup | null {
  if (!isObj(raw)) {
    return null;
  }
  const machine = isObj(raw.machine)
    ? {
        state: clean(raw.machine.state, "?", 16),
        goal: cellPair(raw.machine.goal),
        objective: optStr(raw.machine.objective, 16),
      }
    : null;
  return {
    away_s: count(raw.away_s),
    cell_then: cellPair(raw.cell_then),
    cell_now: cellPair(raw.cell_now),
    power_then: clean(raw.power_then, "?", 16),
    power_now: clean(raw.power_now, "?", 16),
    wreck: raw.wreck === true,
    contacts_new: names(raw.contacts_new, 8, MAX_CATCHUP_CONTACTS),
    contacts_lost: names(raw.contacts_lost, 8, MAX_CATCHUP_CONTACTS),
    contacts_more: count(raw.contacts_more),
    sections: (Array.isArray(raw.sections) ? raw.sections : [])
      .flatMap((s) => {
        if (!isObj(s)) {
          return [];
        }
        const id = optStr(s.id, 24);
        const then = decodeWords(s.then);
        const now = decodeWords(s.now);
        return id === null || then === null || now === null ? [] : [{ id, then, now }];
      })
      .slice(0, MAX_CATCHUP_SECTIONS),
    sections_more: count(raw.sections_more),
    rounds_spent: (Array.isArray(raw.rounds_spent) ? raw.rounds_spent : [])
      .flatMap((r) => {
        if (!isObj(r)) {
          return [];
        }
        const weapon = optStr(r.weapon, 24);
        const rounds = optInt(r.rounds);
        return weapon === null || rounds === null ? [] : [{ weapon, rounds }];
      })
      .slice(0, MAX_CATCHUP_WEAPONS),
    rounds_more: count(raw.rounds_more),
    machine,
  };
}

function decodeTables(raw: unknown): string[][] {
  return (Array.isArray(raw) ? raw : []).map((table) =>
    (Array.isArray(table) ? table : []).map((word) => clean(word, "?", 24)),
  );
}

/**
 * Decode one `arena.cockpit` payload; `null` when it is not an object at all.
 *
 * Missing keys take their defaults and unknown ones are ignored, so an older
 * server (no `scan_tables`) and a newer one (a field this build predates) both
 * decode. A `VIEW` composite — the same payload with `schema.shape === "grid"`
 * and the window's `rows`, `legend`, `units`, `you`, `origin` added — decodes as
 * one more status frame: the grid keys are simply not read, because the web
 * composes its own window at its own size (doc 16 §5's one rule, one parameter).
 */
export function decodeCockpitFrame(data: unknown): CockpitFrame | null {
  if (!isObj(data)) {
    return null;
  }
  return {
    seated: data.seated !== false,
    match: optInt(data.match),
    callsign: clean(data.callsign, "?", 8),
    cell: cellPair(data.cell),
    power: clean(data.power, "?"),
    blocked: data.blocked === true,
    wreck: data.wreck === true,
    immobile: data.immobile === true,
    speed_kph: int(data.speed_kph, 0),
    heading_deg: int(data.heading_deg, 0),
    heat: decodeHeat(data.heat),
    receiver: decodeReceiver(data.receiver),
    // 16b cut O.2: the pod's switch, or `null` (absent, null or unknown alike).
    ecm: data.ecm === "on" || data.ecm === "off" ? data.ecm : null,
    tempo: clean(data.tempo, "?"),
    scope: clean(data.scope, "?"),
    lock_designator: optStr(data.lock_designator, 8),
    lock_range_m: optInt(data.lock_range_m),
    lock_percent: optInt(data.lock_percent),
    sections: (Array.isArray(data.sections) ? data.sections : []).flatMap((s) => {
      const section = decodeSection(s);
      return section ? [section] : [];
    }),
    weapons: (Array.isArray(data.weapons) ? data.weapons : []).flatMap((w) => {
      const weapon = decodeWeapon(w);
      return weapon ? [weapon] : [];
    }),
    contacts: (Array.isArray(data.contacts) ? data.contacts : []).flatMap((c) => {
      const contact = decodeContact(c);
      return contact ? [contact] : [];
    }),
    scan_tables: decodeTables(data.scan_tables),
    notices: (Array.isArray(data.notices) ? data.notices : []).flatMap((n) => {
      const notice = decodeNotice(n);
      return notice ? [notice] : [];
    }),
    catchup: decodeCatchup(data.catchup),
    // 17b cut M1a.3.
    t_ms: optInt(data.t_ms),
    order: decodeOrder(data.order),
    armed: Array.isArray(data.weapons),
    // 17b cut M1b.3.
    odds_floor_permille: optInt(data.odds_floor_permille),
    // 17b cut M5.5 (the key is 17b cut M5.4's).
    mind: decodeMind(data.mind),
  };
}

/**
 * Decode one `arena.map` payload; `null` when it is not an object.
 *
 * The unseated answer (`map_unseated()`) is two keys — `schema` and
 * `seated: false` — and decodes as a map with `seated` false and no ground,
 * which the store drops rather than draws.
 */
export function decodeArenaMap(data: unknown): ArenaMap | null {
  if (!isObj(data)) {
    return null;
  }
  const rows: [string, number][][] = (Array.isArray(data.rows) ? data.rows : []).map((row) =>
    (Array.isArray(row) ? row : []).flatMap((run): [string, number][] =>
      Array.isArray(run) && typeof run[0] === "string" && typeof run[1] === "number"
        ? [[run[0], Math.max(0, Math.trunc(run[1]))]]
        : [],
    ),
  );
  const legend: [string, string][] = (Array.isArray(data.legend) ? data.legend : []).flatMap(
    (e): [string, string][] =>
      Array.isArray(e) && typeof e[0] === "string" && typeof e[1] === "string"
        ? [[e[0], clean(e[1], "", 24)]]
        : [],
  );
  // 16b cut V.1: the named spots. An entry with no one-character `letter` (a
  // control character is none), no `name` or no cell pair is dropped; the name
  // is cleaned to the forty the server's load-time check bounds it to, so the
  // three legends agree.
  const spots: ArenaSpot[] = (Array.isArray(data.spots) ? data.spots : []).flatMap(
    (e): ArenaSpot[] => {
      if (!isObj(e) || typeof e.letter !== "string") {
        return [];
      }
      const letter = clean(e.letter, "", 1);
      const name = clean(e.name, "", 40);
      const cell = cellPair(e.cell);
      return letter.length > 0 && letter === e.letter && name.length > 0 && cell !== null
        ? [{ letter, name, cell }]
        : [];
    },
  );
  return {
    seated: data.seated !== false,
    match: optInt(data.match),
    surface: clean(data.surface, "", 40),
    cell_cm: int(data.cell_cm, 100),
    width: Math.max(0, int(data.width, 0)),
    height: Math.max(0, int(data.height, 0)),
    origin_cell: cellPair(data.origin_cell) ?? [0, 0],
    rows,
    legend,
    spots,
  };
}

// ---------------------------------------------------------------------------
// The one composition (doc 16 §5)
// ---------------------------------------------------------------------------

/** A contact's glyph: `?` nothing has classified, `+` your side, `x` theirs. */
export function contactGlyph(contact: CockpitContact): string {
  return contact.class === "unknown" ? "?" : contact.friendly ? "+" : "x";
}

/** The same fact as a kind, which is what the canvas colours by. */
export function contactKind(contact: CockpitContact): MarkerKind {
  return contact.class === "unknown" ? "unknown" : contact.friendly ? "friendly" : "hostile";
}

/**
 * **The ground under one window** (16b cut I.2 factored it out of
 * `composeWindow`, which the replay's `composeReplayWindow` shares it with):
 * `h` rows of exactly `w` glyphs, row `0` the window's SOUTH row, expanded out
 * of the map's run-length `rows` and padded with spaces.
 *
 * Only the rows the window shows are expanded: a shipped arena is 256 by 256
 * and a viewport of it has no business paying for the other sixty thousand
 * cells on every redraw (the TUI's `mush_ansi::window_rows` makes the same
 * trade for the same reason).
 */
export function windowGround(
  map: ArenaMap,
  x0: number,
  y0: number,
  w: number,
  h: number,
): string[][] {
  const width = Math.max(0, Math.trunc(map.width));
  const height = Math.max(0, Math.trunc(map.height));
  const south: string[][] = [];
  for (let y = y0; y < y0 + h; y += 1) {
    const row: string[] = new Array<string>(w).fill(" ");
    const runs = y < height ? map.rows[y] : undefined;
    if (runs) {
      let col = 0;
      for (const [glyph, n] of runs) {
        if (col >= width) {
          break;
        }
        const g = glyph.charAt(0) || " ";
        const end = Math.min(col + n, width);
        for (let c = Math.max(col, x0); c < Math.min(end, x0 + w); c += 1) {
          row[c - x0] = g;
        }
        col = end;
      }
    }
    south.push(row);
  }
  return south;
}

/**
 * **The map composition** (16b cut Q.5; doc 16 §5), the rule the TUI's
 * `render_cockpit_map_rows` and the server's `view_window` both run, differing
 * only in the SIZE asked for:
 *
 * 1. the window is `w = min(map.width, cols)` by `h = min(map.height, rows)`,
 *    centred on the platform's own `cell` — the map's middle when it has none —
 *    and CLAMPED to the arena, so a tank in a corner is drawn at its own cell;
 * 2. the terrain comes out of the run-length `rows`, whose row `0` is the SOUTH
 *    edge, clipped to the window and padded with spaces;
 * 3. the markers go down in the order 16b cut V.1 fixed — the map's named
 *    `spots` FIRST, in authored order, each as its letter; then the pilot's own
 *    `@`; then the contacts in the frame's own order — each at its ABSOLUTE
 *    cell less the window origin, a later marker overwriting an earlier one, so
 *    a contact or the `@` standing on a spot hides its letter (the spot is
 *    ground, the contact is what the pilot holds);
 * 4. a spot or a contact whose `cell` is `null`, or which falls outside the
 *    window, is not drawn (a contact is still in the contacts table);
 * 5. `grid` prints NORTH-UP: `grid[0]` is the window's north row.
 *
 * **The naming order is the reverse of the drawing** (16b cut V.1):
 * `markerLegend` names the contacts that own their cell first and the spots
 * that own theirs after them, and `labelPlan` places the contacts' labels
 * before any spot's — so a narrow legend or a crowded canvas keeps the hostile
 * a pilot aims by before the name of a field.
 *
 * Every glyph is the payload's own. There is no second table anywhere on this
 * path, which is why composing the committed `arena_map.json` and
 * `cockpit_frame.json` at exactly 39 by 20 has to equal `view_grid.txt` byte
 * for byte — the parity assertion doc 16 §5 asks for.
 */
export function composeWindow(
  map: ArenaMap,
  frame: CockpitFrame,
  cols: number,
  rows: number,
): ComposedWindow {
  const width = Math.max(0, Math.trunc(map.width));
  const height = Math.max(0, Math.trunc(map.height));
  const w = Math.min(width, Math.max(1, Math.trunc(cols)));
  const h = Math.min(height, Math.max(1, Math.trunc(rows)));
  const [cx, cy] = frame.cell ?? [Math.floor(width / 2), Math.floor(height / 2)];
  const x0 = Math.min(Math.max(cx - Math.floor(w / 2), 0), width - w);
  const y0 = Math.min(Math.max(cy - Math.floor(h / 2), 0), height - h);
  const south = windowGround(map, x0, y0, w, h);

  // The ground alone, taken before a single marker goes down (north-up).
  const terrain: string[] = [];
  for (let y = h - 1; y >= 0; y -= 1) {
    terrain.push((south[y] ?? []).join(""));
  }

  const markers: Marker[] = [];
  const put = (
    cell: [number, number],
    glyph: string,
    kind: MarkerKind,
    designator: string,
  ): void => {
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
  // 16b cut V.1: the ground's named spots first — under everything.
  for (const spot of map.spots) {
    put(spot.cell, spot.letter, "spot", spot.name);
  }
  if (frame.cell) {
    put(frame.cell, "@", "you", frame.callsign);
  }
  for (const contact of frame.contacts) {
    if (contact.cell) {
      put(contact.cell, contactGlyph(contact), contactKind(contact), contact.designator);
    }
  }

  const grid: string[] = [];
  for (let y = h - 1; y >= 0; y -= 1) {
    grid.push((south[y] ?? []).join(""));
  }
  return { x0, y0, w, h, grid, terrain, markers };
}

// ---------------------------------------------------------------------------
// The view model — what the screen prints, decided here so vitest pins it
// ---------------------------------------------------------------------------

/** Three digits, the way every heading and bearing on this surface reads. */
function deg3(n: number): string {
  const v = ((Math.trunc(n) % 360) + 360) % 360;
  return String(v).padStart(3, "0");
}

/** The words one notice kind is said in. A kind this build predates falls back
 * to its own wire word with the underscores opened out, which can name nothing
 * the payload did not already carry.
 *
 * A `Map` and not an object (16b cut B.1, `T-86`): a plain object answers an
 * inherited key, so a kind named `constructor` printed a function's source. The
 * same holds for the two tables below. */
const NOTICE_WORDS: ReadonlyMap<string, string> = new Map([
  ["acquired", "new contact"],
  ["classified", "identified"],
  // 16b cut B.1 (`T-86`): with `was` on the wire the line reads
  // `BA unidentified again: X2`, the sentence's own two names.
  ["unidentified", "unidentified again"],
  ["lost", "contact lost"],
  ["lock_lost", "lock broken"],
  ["locked", "locked"],
  ["hit", "hit"],
  ["miss", "miss"],
  ["kill", "destroyed"],
  ["struck", "hit taken"],
  ["struck_spent", "hit on a dead section"],
  ["section_lost", "section gone"],
  ["target_section_lost", "section destroyed"],
  ["refused", "refused"],
  ["blocked", "held"],
  ["power_online", "power online"],
  ["destroyed", "your tank is destroyed"],
  ["disabled", "your tank is out of the fight"],
  // 16b cut O.2 (16c N12): the pilot's own pod's switch, short.
  ["emcon_on", "EMCON on"],
  ["emcon_off", "EMCON off: pod silent"],
  // 17b cut M1a.4: the motor's two answers to the pilot's own `DRIVE`, whole —
  // each is already short, and there is nothing in it to compose.
  ["drive_arrived", "DRIVE: arrived."],
  ["drive_no_route", "DRIVE: no route from here; holding."],
  // 17b cut M1b.4: a slot's shot and a slot's held shot. Both lines are
  // composed whole from the notice's fields (`slotLine`); these are the
  // fallbacks a line missing its parts would fall to.
  ["slot_fired", "fires by"],
  ["slot_held", "held"],
]);

/** Why an order was refused, in the pilot's words (`Refusal::as_str`). A word
 * this build predates prints with its underscores opened out, which can name
 * nothing the payload did not already carry. */
const REFUSAL_WORDS: ReadonlyMap<string, string> = new Map([
  ["power_not_online", "power is not online"],
  ["queue_full", "too many orders at once"],
  ["wreck", "the tank is destroyed"],
  ["motion_disabled", "the tank cannot move"],
  ["no_contact", "no such contact"],
  ["no_lock", "no lock"],
  ["cycling", "still cycling"],
  ["empty", "empty"],
  ["overheated", "overheated"],
  ["no_weapon", "no such weapon"],
  ["epoch", "the seat changed hands"],
  // 16b cut O.2: `EMCON` on a tank with no pod.
  ["no_jammer", "no jamming pod"],
  // 17b cut M1b.1's three words (said here since 17b cut M1b.3): a shot with
  // no warrant, a slot count full, an arming under the Compact floor.
  ["unwarranted", "no warrant"],
  ["no_slot", "every slot armed"],
  ["unlawful", "under the floor"],
  // 17b cut M5.2: a `MIND PROFILE` the arena could not take.
  ["profile", "that profile cannot be switched to now"],
]);

/** Why motion stopped, in the pilot's words (`BlockedCause::as_str`). */
const CAUSE_WORDS: ReadonlyMap<string, string> = new Map([
  ["impassable", "the ground will not take the tank"],
  ["occupied", "something is in the way"],
  ["edge", "the arena's edge"],
  ["too_steep", "too steep: drive round"],
]);

/** A wire word with its underscores opened out — the fallback for a kind, a
 * refusal or a cause this build predates. */
function opened(word: string): string {
  return word.replace(/_/g, " ");
}

/** The clause a refusal's own numbers add: `, 3 s` or `, 60 of 60`. */
function detailWords(detail: CockpitNoticeDetail | null): string {
  if (detail === null) {
    return "";
  }
  if (detail.secs !== null) {
    return `, ${detail.secs} s`;
  }
  if (detail.heat !== null && detail.max !== null) {
    return `, ${detail.heat} of ${detail.max}`;
  }
  return "";
}

/**
 * One line of the notices ring, composed from the notice's own FIELDS and NEVER
 * from its `text` (16b cut F.2: one sentence, one owner — the arena already sent
 * that sentence to the scrollback as a `Sys` line, and a panel that printed it
 * too would say everything twice). That is also why the ring's reply half
 * (16b cut B.1, `CockpitStore.pushReply`) skips the `Sys` line tagged
 * `NOTICE_SENTENCE_TAG`: it is this notice's sentence, not an answer.
 *
 * **16b cut D.2** (`B-07`) is what made that rule survivable. A refused order
 * and a blocked tank carried their whole sentence in `text` and nothing else, so
 * this ring printed `refused` and `held` — no order, no weapon, no reason — and
 * a pilot watching the panel could not tell a cold tank from a cycling gun. The
 * parts are on the wire now, so the line is composed here.
 *
 * **16b cut B.1** (`T-86`): an `identified` or `unidentified again` line that
 * carries its old name reads `X2 identified: BA` / `BA unidentified again: X2`
 * — the old name first, the name the row reads now after the colon, which is
 * the order the pilot met them in; a block with no cause reads `held` alone
 * rather than guessing `occupied`'s words; and the victim's `struck`,
 * `struck_spent` and `section_lost` gain `: section` through the common clause
 * now that the wire carries their section. `fixtures/cockpit/notice_lines.json`
 * pins every shape, and 16b cut B.3 holds the TUI's ring to the same file.
 */
export function noticeLine(notice: CockpitNotice): string {
  if (notice.kind === "refused") {
    const order = (notice.order ?? "order").toUpperCase();
    const weapon = notice.weapon === null ? "" : ` ${notice.weapon}`;
    // 17b cut M1b.4: an arming under the Compact floor carrying its floor reads
    // the line-mode sentence itself (less its full stop), as the TUI's ring does.
    const floor = notice.detail?.floor_permille;
    if (notice.refusal === "unlawful" && floor !== undefined) {
      return `${order}${weapon} refused. The Compact floor is ${Math.trunc(floor / 10)}%; a condition may not ask for less`;
    }
    const why =
      notice.refusal === null
        ? "refused"
        : (REFUSAL_WORDS.get(notice.refusal) ?? opened(notice.refusal));
    return `${order}${weapon} refused: ${why}${detailWords(notice.detail)}`;
  }
  if (notice.kind === "blocked") {
    return notice.cause === null
      ? "held"
      : `held: ${CAUSE_WORDS.get(notice.cause) ?? opened(notice.cause)}`;
  }
  const words = NOTICE_WORDS.get(notice.kind) ?? opened(notice.kind);
  // 16b cut D.2 (`B-18`): friendly fire is said and never gated, on the panel
  // exactly as in the scrollback.
  const named = notice.designator === null ? null : `${notice.designator}${notice.friendly ? " (FRIENDLY)" : ""}`;
  if (notice.slot !== undefined) {
    const line = slotLine(notice, notice.slot, named);
    if (line !== null) {
      return line;
    }
  }
  if (
    (notice.kind === "classified" || notice.kind === "unidentified") &&
    notice.was !== null
  ) {
    // The mark goes with the name the row reads now (`X3 identified: RB
    // (FRIENDLY)`): an own-side tank told apart must not read as a hostile's.
    return named === null ? `${notice.was} ${words}` : `${notice.was} ${words}: ${named}`;
  }
  const head = named === null ? words : `${named} ${words}`;
  const line = notice.section === null ? head : `${head}: ${notice.section}`;
  // 16b cut D.2 (`B-15`): the round found a section that was already gone.
  return notice.kind === "hit" && notice.spent ? `${line} (already destroyed)` : line;
}

/**
 * **A slot notice's line** (17b cut M1b.4; doc 17 §5, D-UM-14) — exactly the
 * line-mode sentence (`notice_line`) less its full stop, from the notice's own
 * fields, as the TUI's `slot_words` composes it: `{Weapon} fires at
 * {designator}{ (FRIENDLY)} by S{slot} ({percent}%, {side})` (`{Weapon} fires
 * by S{slot} (…)` with no designator) and `S{slot} held the {weapon}: {why}` —
 * `{percent}% under your {floor}%`, `{designator} shows its {side}`,
 * `{designator} at {range_m} m, outside {min} to {max}` (`under {max}` with no
 * nearest, `past {min}` with no furthest), `{rounds} rounds, keeps {keeps}`.
 * `null` for a notice missing a part, which then reads as its kind's words.
 */
function slotLine(
  notice: CockpitNotice,
  parts: CockpitSlotNotice,
  named: string | null,
): string | null {
  if (notice.weapon === null) {
    return null;
  }
  if (notice.kind === "slot_fired") {
    if (parts.percent === null || parts.aspect === null) {
      return null;
    }
    const gun = `${notice.weapon.charAt(0).toUpperCase()}${notice.weapon.slice(1)}`;
    const tail = `by S${parts.slot} (${parts.percent}%, ${sideWord(parts.aspect)})`;
    return named === null ? `${gun} fires ${tail}` : `${gun} fires at ${named} ${tail}`;
  }
  const held = parts.held;
  if (held === null) {
    return null;
  }
  let why: string | null = null;
  if (held.why === "odds" && held.percent !== null && held.floor !== null) {
    why = `${held.percent}% under your ${held.floor}%`;
  } else if (held.why === "aspect" && notice.designator !== null && held.aspect !== null) {
    why = `${notice.designator} shows its ${sideWord(held.aspect)}`;
  } else if (
    held.why === "range" &&
    notice.designator !== null &&
    held.range_m !== null &&
    held.min_m !== null &&
    held.max_m !== null
  ) {
    const bounds =
      held.min_m === 0
        ? `under ${held.max_m}`
        : held.max_m === 4294967295
          ? `past ${held.min_m}`
          : `${held.min_m} to ${held.max_m}`;
    why = `${notice.designator} at ${held.range_m} m, outside ${bounds}`;
  } else if (held.why === "rounds" && held.rounds !== null && held.keeps !== null) {
    why = `${held.rounds} rounds, keeps ${held.keeps}`;
  }
  return why === null ? null : `S${parts.slot} held the ${notice.weapon}: ${why}`;
}

/** Every notice of one frame, as ring lines. */
export function noticeLines(notices: readonly CockpitNotice[]): string[] {
  return notices.map(noticeLine);
}

/** The machine's state in the pilot's words (16b cut Y.4). A `Map`, for the
 * reason `NOTICE_WORDS` is one. **17b cut M5.3 (D14)**: `unknown` — a hull
 * flown by a mind that is not Standard Doctrine — reads `flying on its own
 * mind`, as the server's `machine_state_words` and the TUI's `catchup_words`
 * say it, rather than the opened word. */
const MACHINE_STATE_WORDS: ReadonlyMap<string, string> = new Map([
  ["startup", "starting up"],
  ["patrol", "patrolling"],
  ["approach", "closing on a contact"],
  ["engage", "engaging"],
  ["withdraw", "withdrawing toward its entry zone"],
  ["disabled", "disabled"],
  ["unknown", "flying on its own mind"],
]);

/** The standing order's objective in the pilot's words (16b cut Y.4). */
const MACHINE_ORDER_WORDS: ReadonlyMap<string, string> = new Map([
  ["hold", "a hold order"],
  ["advance", "an advance order"],
  ["engage", "an engage order"],
  ["withdraw", "a withdraw order"],
]);

/** A length in words over whole seconds — the server's `duration_words`. */
function durationWords(secs: number): string {
  const s = Math.max(0, Math.trunc(secs));
  const hours = Math.trunc(s / 3600);
  const minutes = Math.trunc((s % 3600) / 60);
  const seconds = s % 60;
  if (hours > 0) {
    return `${hours} h ${minutes} m ${seconds} s`;
  }
  if (minutes > 0) {
    return `${minutes} m ${seconds} s`;
  }
  return `${seconds} s`;
}

/** `none new` / `new BA, X3` / `new BA, X3 and 3 more`. */
function listedClause(word: string, list: readonly string[], more: number): string {
  if (list.length === 0) {
    return more === 0 ? `none ${word}` : `${word} ${more} more`;
  }
  return more === 0 ? `${word} ${list.join(", ")}` : `${word} ${list.join(", ")} and ${more} more`;
}

/** `a, b` or `a, b and 2 more`, or `none` when there is nothing at all. */
function listedOr(parts: readonly string[], separator: string, more: number, none: string): string {
  if (parts.length === 0) {
    return more === 0 ? none : `${more} more`;
  }
  return more === 0 ? parts.join(separator) : `${parts.join(separator)} and ${more} more`;
}

function cellWords(cell: [number, number]): string {
  return `(${cell[0]}, ${cell[1]})`;
}

/**
 * **The catch-up report, as the notices ring prints it** (16b cut Y.4; doc 07
 * §1.3 step 5): the server's `catchup_lines` and the TUI's `catchup_words`,
 * line for line, composed from the frame's decoded `catchup` — the line client
 * was sent the server's own words as one `Sys` block tagged
 * `NOTICE_SENTENCE_TAG`, which `pushReply`'s caller leaves out. The contacts'
 * one overflow count goes after the one list that is full (only a full list
 * can have overflowed); when both are full it cannot say whose it was and is
 * its own clause, `3 more not named`. `fixtures/cockpit/catchup_lines.json`
 * pins all three.
 */
export function catchupLines(callsign: string, catchup: CockpitCatchup): string[] {
  const lines = [`Back aboard ${callsign} after ${durationWords(catchup.away_s)} away.`];
  const machine = catchup.machine;
  if (machine === null) {
    lines.push("  The machine had no state.");
  } else {
    const state = MACHINE_STATE_WORDS.get(machine.state) ?? opened(machine.state);
    const order =
      machine.objective === null
        ? "no order"
        : (MACHINE_ORDER_WORDS.get(machine.objective) ?? `a ${opened(machine.objective)} order`);
    lines.push(`  The machine was ${state} under ${order}.`);
  }
  const then = catchup.cell_then;
  const now = catchup.cell_now;
  if (now === null) {
    lines.push("  Position: unknown.");
  } else if (then === null) {
    lines.push(`  Position: cell unknown then, ${cellWords(now)} now.`);
  } else if (then[0] === now[0] && then[1] === now[1]) {
    lines.push(`  Position: cell ${cellWords(now)}, unchanged.`);
  } else {
    lines.push(`  Position: cell ${cellWords(then)} then, ${cellWords(now)} now.`);
  }
  const more = catchup.contacts_more;
  if (catchup.contacts_new.length === 0 && catchup.contacts_lost.length === 0 && more === 0) {
    lines.push("  Contacts: none new, none lost.");
  } else {
    const newFull = catchup.contacts_new.length >= MAX_CATCHUP_CONTACTS;
    const lostFull = catchup.contacts_lost.length >= MAX_CATCHUP_CONTACTS;
    const onNew = more > 0 && newFull && !lostFull;
    const onLost = more > 0 && lostFull && !newFull;
    const unnamed = more > 0 && !onNew && !onLost ? more : 0;
    const clauses = `${listedClause("new", catchup.contacts_new, onNew ? more : 0)}; ${listedClause(
      "lost",
      catchup.contacts_lost,
      onLost ? more : 0,
    )}`;
    lines.push(
      unnamed === 0 ? `  Contacts: ${clauses}.` : `  Contacts: ${clauses}; ${unnamed} more not named.`,
    );
  }
  const sections = catchup.sections.map(({ id, then: was, now: is }) => {
    if (was.condition !== is.condition) {
      return `${id} ${was.condition} then, ${is.condition} now`;
    }
    const clauses: string[] = [];
    if (is.armour < was.armour) {
      clauses.push(`armour ${was.armour} then, ${is.armour} now`);
    }
    if (is.internals < was.internals) {
      clauses.push(`internals ${was.internals} then, ${is.internals} now`);
    }
    return clauses.length === 0 ? id : `${id} ${clauses.join(", ")}`;
  });
  lines.push(`  Sections: ${listedOr(sections, "; ", catchup.sections_more, "unchanged")}.`);
  const rounds = catchup.rounds_spent.map(({ weapon, rounds: spent }) => `${weapon} ${spent}`);
  lines.push(`  Rounds spent: ${listedOr(rounds, ", ", catchup.rounds_more, "none")}.`);
  lines.push(`  Power ${catchup.power_now}.`);
  if (catchup.wreck) {
    lines.push("  Your tank is a wreck.");
  }
  return lines;
}

export interface ScanChip {
  word: string;
  letter: string;
  band: ScanBand;
}

export interface CockpitContactRow {
  designator: string;
  /** Everything after the designator, as the TUI's row prints it. */
  detail: string;
  /** The scan of this contact, empty when this platform holds no reading. */
  chips: ScanChip[];
}

export interface CockpitDamageRow {
  id: string;
  condition: string;
  /** `internals/internals_max`. */
  internals: string;
  /** How much of the internals is left, 0 to 100 — arithmetic on the payload's
   * own two numbers, so the bar can never say anything the fraction does not. */
  pct: number;
  facets: { id: string; armor: string; pct: number }[];
}

export interface CockpitView {
  /** The map's own heading line. */
  heading: string;
  /** The marker legend, `glyph designator` joined by two spaces; `""` when no
   * contact marker landed. Excludes the pilot, and names only the markers that
   * OWN their cell — the last one written there, whose glyph is the one on the
   * map — exactly as the TUI's legend does (16b cut B.3, `B-62`). */
  legend: string;
  /** The status block: callsign, power, speed/heading (and the tank's own
   * limit), heat, the degraded receiver, scope (and the pod's switch), lock. */
  status: string[];
  damage: CockpitDamageRow[];
  /** One line a weapon, or the single `(no weapon)` line — and (17b cut M5.5)
   * none on a hull with no weapons station whose frame carries a mind: the
   * mind's rows take that line's place. */
  weapons: string[];
  /** **The weapons section's tail** (17b cut M5.5): `mindLines`, four lines
   * under the weapons, or none when the frame carries no `mind`. */
  mind: string[];
  /** The tail's profile row in its parts (17b cut M5.5): `mindProfiles`, one
   * part a sealed profile, which the screen makes a button each; joined they
   * are `mind[1]`, and empty when `mind` is. */
  profiles: CockpitMindProfile[];
  contacts: CockpitContactRow[];
  /** The ring, oldest first. */
  notices: string[];
}

/** A band letter's band. A `Map`, read with `.get`, so a letter this build
 * predates — or an inherited key — is `unknown` and never green (16b cut B.1,
 * `T-58`). */
const BAND_OF: ReadonlyMap<string, ScanBand> = new Map([
  ["g", "green"],
  ["y", "yellow"],
  ["r", "red"],
  ["d", "destroyed"],
]);

/** One contact's scan chips, or `[]` when this platform holds no reading of it.
 * The section words come out of `scan_tables[condition.table]` (16b cut Q.2) —
 * the frame's own vocabulary, never a table of this client's. */
export function scanChips(contact: CockpitContact, tables: readonly string[][]): ScanChip[] {
  const condition = contact.condition;
  if (condition === null) {
    return [];
  }
  const table = tables[condition.table];
  if (!table) {
    return [];
  }
  const chips: ScanChip[] = [];
  for (let i = 0; i < table.length && i < condition.bands.length; i += 1) {
    const letter = condition.bands.charAt(i);
    chips.push({ word: table[i] ?? "?", letter, band: BAND_OF.get(letter) ?? "unknown" });
  }
  return chips;
}

/** The scan chips as the one text row a terminal prints, so the golden and the
 * TUI say the same thing: a word that is not green carries its band letter. */
export function scanText(chips: readonly ScanChip[]): string {
  return `  ${chips.map((c) => (c.letter === "g" ? `${c.word} ` : `${c.word}/${c.letter} `)).join("")}`;
}

/** The status block (the TUI's status pane, line for line). */
export function statusLines(frame: CockpitFrame): string[] {
  // 16b cut B.1 (`T-86`): the pilot's own two limits (16b cut D.2) on the speed
  // line — a wreck says only DESTROYED, because it is immobile as well.
  const limit = frame.wreck ? "  DESTROYED" : frame.immobile ? "  IMMOBILE" : "";
  const lines = [
    `Callsign ${frame.callsign}`,
    `power ${frame.power}${frame.blocked ? "  HELD" : ""}`,
    `${frame.speed_kph} kph  heading ${deg3(frame.heading_deg)}°${limit}`,
  ];
  if (frame.heat !== null) {
    lines.push(
      `heat ${frame.heat.heat}/${frame.heat.max}${frame.heat.overheated ? "  OVERHEATED" : ""}`,
    );
  }
  // 16b cut B.1 (`T-86`): the degraded receiver, after the heat row. The row's
  // presence is what says "degraded" (the frame carries `null` on a clean
  // one); `receiver 15/40  60%` is 19 characters, so it never wraps the TUI's
  // 22-column pane, whose row is the same words in the same place.
  if (frame.receiver !== null) {
    const r = frame.receiver;
    lines.push(`receiver ${r.budget}/${r.budget_max}  ${r.reach_percent}%`);
  }
  // 16b cut D.2 (16c `T-59`, the panel half): this platform's own scope, not the
  // arena's one global tempo word — a pilot with an empty scope read `engaged`
  // off somebody else's fight. `frame.tempo` is still decoded, but nothing in
  // this client reads it: `T-59` is closed for the printed word alone.
  // 16b cut O.2 (16c N12): the pod's switch rides this row — `scope hostile
  // EMCON on` — and a platform with no pod prints the row unchanged, exactly
  // as the TUI's does.
  const pod = frame.ecm === null ? "" : `  EMCON ${frame.ecm}`;
  lines.push(`scope ${frame.scope}${pod}`);
  lines.push(lockLine(frame));
  return lines;
}

/** The lock line: what is held, how far and what it is worth. */
export function lockLine(frame: CockpitFrame): string {
  if (frame.lock_designator === null) {
    return "No lock";
  }
  let line = `Lock ${frame.lock_designator}`;
  if (frame.lock_range_m !== null) {
    line += `  ${frame.lock_range_m} m`;
  }
  if (frame.lock_percent !== null) {
    line += `  ${frame.lock_percent}%`;
  }
  return line;
}

/** The damage rows: a pilot's own tank is outside the observer rule entirely
 * (doc 16 §3.6), so this is the whole of it. */
export function damageRows(frame: CockpitFrame): CockpitDamageRow[] {
  const share = (current: number, max: number): number =>
    max <= 0 ? 0 : Math.max(0, Math.min(100, Math.round((current * 100) / max)));
  return frame.sections.map((s) => ({
    id: s.id,
    condition: s.condition,
    internals: `${s.internals}/${s.internals_max}`,
    pct: share(s.internals, s.internals_max),
    facets: s.facets.map((f) => ({
      id: f.id,
      armor: `${f.armor}/${f.armor_max}`,
      pct: share(f.armor, f.armor_max),
    })),
  }));
}

/** One damage row as the line a terminal prints. */
export function damageText(row: CockpitDamageRow): string {
  return `${row.id}  ${row.condition}  ${row.internals}${row.facets
    .map((f) => `  ${f.id} ${f.armor}`)
    .join("")}`;
}

/** The weapons rows: readiness, rounds, and each weapon's OWN chance — and,
 * under an armed weapon (17b cut M1b.3), one more line: two spaces and
 * `armingWords`' line, the one line mode's `WEAPONSTATUS` and the TUI pane
 * print under the row. */
export function weaponLines(frame: CockpitFrame): string[] {
  if (frame.weapons.length === 0) {
    return ["(no weapon)"];
  }
  return frame.weapons.flatMap((w) => {
    const ready = w.ready ? "ready" : `${Math.ceil(Math.max(0, w.ready_in_ms) / 1000)} s`;
    const chance = w.percent === null ? "" : `  ${w.percent}%`;
    const row = `${w.id}  ${ready}  ${w.rounds}/${w.rounds_max}${chance}`;
    return w.armed === null ? [row] : [row, `  ${armingWords(w.armed)}`];
  });
}

/** One sealed profile on the mind tail's profile row (17b cut M5.5): its name,
 * whether it is the one flown, and the text before it — one space after the
 * previous part, then `> ` for the flown profile or two spaces for another. */
export interface CockpitMindProfile {
  lead: string;
  name: string;
  active: boolean;
}

/** The profile row's parts, every sealed profile in sealed order (17b cut
 * M5.5); `[]` when the frame carries no mind. */
export function mindProfiles(frame: CockpitFrame): CockpitMindProfile[] {
  const mind = frame.mind;
  if (mind === null) {
    return [];
  }
  return mind.profiles.map((name, at) => ({
    lead: `${at === 0 ? "" : " "}${at === mind.active ? "> " : "  "}`,
    name,
    active: at === mind.active,
  }));
}

/**
 * **The mind's four lines, the weapons section's tail** (17b cut M5.5; doc 17
 * §12, OQ-8; the M5 design note's M5.5 item 2) — the TUI's weapons pane tail
 * (`render_cockpit_mind_tail`) line for line, held equal for the same scene by
 * `tui_golden.rs`' `m5_the_tui_and_the_web_draw_the_same_mind_tail`, which
 * reads `web_view_model.txt` beside the TUI's own rows:
 *
 * ```text
 * mind Standard Doctrine · Class I
 * > Sniper   Line
 * dormant while you fly
 * you: helm sensors fire
 * ```
 *
 * Line for line for every object the arena's `mind_json` sends (one to four
 * preset names, the class words): `decodeMind`'s tolerance of anything else
 * (its cuts, its cap of four, a blank dropped, `?` for an empty class) is the
 * web's alone, so an object no server sends may read otherwise on the TUI.
 *
 * The mind (`mind: your own · Class I` for a seal that is not Standard
 * Doctrine) and the class it thinks at; every sealed profile, the flown one
 * marked `> `; `dormant while you fly`, or `off for the match: on the ROM`
 * where the ROM flies the hull — one dropped to it (`off`), or one whose mind
 * class reads `0` (no core, a core at Class 0, a seal capped there), for which
 * line mode's `MIND` names the ROM too (a live class only falls during a
 * match); the channels the pilot holds. No countdown (the frame's
 * `next_switch_ms` is absolute, and a quiet frame would leave one stale), no
 * card and no rule glyph (M6's). `[]` when the frame carries no mind.
 */
export function mindLines(frame: CockpitFrame): string[] {
  const mind = frame.mind;
  if (mind === null) {
    return [];
  }
  const head =
    mind.name === null
      ? `mind: your own · Class ${mind.class}`
      : `mind ${mind.name} · Class ${mind.class}`;
  return [
    head,
    mindProfiles(frame)
      .map((p) => `${p.lead}${p.name}`)
      .join(""),
    mind.off || mind.class === "0" ? "off for the match: on the ROM" : "dormant while you fly",
    `you: ${mind.channels.join(" ")}`.trimEnd(),
  ];
}

/**
 * **The arming line** (17b cut M1b.3) — the web's copy of the arena's ONE
 * formatter (`tacsim::match_::cockpit::arming_words`), byte for byte, pinned
 * equal by `web_view_model.txt` against the TUI golden's rows for the same
 * scene: `S{slot}:` then, one space apart, `best` for a `best` target
 * (`contact` for a `contact` one; `lock`, the default, prints nothing),
 * `>={percent}%`, the facets as `front`/`side`/`rear` joined `/` only when the
 * mask is not all four, the range as `{a}-{b}m`, `under {b}m` or `past {a}m`
 * only when bound, and `keeps {k}` only above nought —
 * `S1: >=60% side/rear keeps 8`, `S1: best >=10% under 420m`.
 */
export function armingWords(a: CockpitArming): string {
  const terms = [`S${a.slot}:`];
  if (a.target === "best" || a.target === "contact") {
    terms.push(a.target);
  }
  terms.push(`>=${a.percent}%`);
  const has = (word: string): boolean => a.aspects.includes(word);
  if (!FACET_WORDS.every(has)) {
    const words = [
      has("front") ? "front" : null,
      has("left") || has("right") ? "side" : null,
      has("rear") ? "rear" : null,
    ].filter((word): word is string => word !== null);
    if (words.length > 0) {
      terms.push(words.join("/"));
    }
  }
  const near = a.min_range_m !== null && a.min_range_m > 0 ? a.min_range_m : null;
  const far = a.max_range_m;
  if (near !== null && far !== null) {
    terms.push(`${near}-${far}m`);
  } else if (far !== null) {
    terms.push(`under ${far}m`);
  } else if (near !== null) {
    terms.push(`past ${near}m`);
  }
  if (a.keeps > 0) {
    terms.push(`keeps ${a.keeps}`);
  }
  return terms.join(" ");
}

/** **The side word a facet word prints as** (17b cut M1a.3; the arena's
 * `Aspect::side_word`): `left` and `right` are `side`, `rear` is `rear`, and
 * everything else — `front`, and a word this build predates — is `front`. */
export function sideWord(facet: string): string {
  switch (facet) {
    case "left":
    case "right":
      return "side";
    case "rear":
      return "rear";
    default:
      return "front";
  }
}

/** The contact table's rows, in the frame's own order.
 *
 * **17b cut M1a.3** (doc 17 §3): each detail ends in the aspect's side word,
 * then — only on a frame that carries `weapons` — the first odds entry (the
 * default weapon's, which the server puts first) as `cannon 62%`, or `-` for a
 * contact the fire control is not resolving. An unarmed hull's row ends at the
 * side word. */
export function contactRows(frame: CockpitFrame): CockpitContactRow[] {
  return frame.contacts.map((c) => {
    const first = c.odds[0];
    const odds = !frame.armed ? "" : first ? `  ${first.weapon} ${first.percent}%` : "  -";
    return {
      designator: c.designator,
      detail: `${c.class}  ${deg3(c.bearing_deg)}°  ${c.band}  ${c.range_m} m  ${c.age_s} s  ${sideWord(c.aspect)}${odds}`,
      chips: scanChips(c, frame.scan_tables),
    };
  });
}

/** One contact row as the line a terminal prints. */
export function contactText(row: CockpitContactRow): string {
  return `${row.designator.padEnd(4)}${row.detail}`;
}

/**
 * **The key the canvas's terrain cache is held under** (16b cut B.2, `T-33`):
 * the window's corner and size and the cell size in pixels — everything the
 * painted ground depends on but the map itself, which the component compares
 * by identity beside it. Never the frame: the ground does not move when a
 * contact does, so a pulse that leaves the window where it was repaints the
 * markers alone.
 */
export function terrainCacheKey(win: ComposedWindow, px: number): string {
  return `${win.x0},${win.y0},${win.w},${win.h},${px}`;
}

/**
 * **Which markers own their cell** (16b cut V.1): one flag per marker, `true`
 * when no later marker landed on the same cell. The last write wins, in the
 * grid and here — so `markerLegend`, `labelPlan` and the canvas all name and
 * draw exactly what the grid shows.
 */
export function markerOwners(markers: readonly Marker[]): boolean[] {
  const owner = new Map<string, number>();
  markers.forEach((m, i) => owner.set(`${m.x},${m.y}`, i));
  return markers.map((m, i) => owner.get(`${m.x},${m.y}`) === i);
}

/**
 * **The marker legend** (16b cut B.3, `B-62`): the contact markers that OWN
 * their cell, in frame order — the TUI's `render_cockpit_map_rows` rule. The
 * last marker written to a cell owns it, so two contacts on one cell name the
 * later, whose glyph is the one drawn; the earlier is still a row of the
 * contacts table. The pilot's own `@` is written before the contacts, so a
 * contact on the pilot's cell owns it and is named, and the `@` itself is never
 * listed.
 *
 * **16b cut V.1**: THEN the named spots that own their cell, in authored order,
 * as `A The Ford`. A spot is written first of all, so one under a contact or
 * under the `@` owns nothing and is named nowhere. The contacts lead because a
 * narrow legend is read from the front.
 */
export function markerLegend(win: ComposedWindow | null): string {
  if (win === null) {
    return "";
  }
  const owns = markerOwners(win.markers);
  const named = (spots: boolean): string[] =>
    win.markers
      .filter((m, i) => m.kind !== "you" && (m.kind === "spot") === spots && owns[i] === true)
      .map((m) => `${m.glyph} ${m.designator}`);
  return [...named(false), ...named(true)].join("  ");
}

/** The cell size, in canvas pixels, from which every marker is labelled — and
 * below which only a hostile and the lock are (16b cut B.1). */
const LABEL_ALL_PX = 6;

/**
 * **Whether a marker's designator is drawn beside it at this cell size** (16b
 * cut B.1, `B-09`).
 *
 * A hostile and the lock are labelled at EVERY zoom: at the default 128-cell
 * view of a full arena a cell is 4 px, and two unlabelled hostiles there were
 * two identical red dots — the pilot could not tell which one to lock. Your own
 * marker, a friendly and an unknown keep the old gate (`px >= 6`); they are not
 * what a pilot aims at, and labelling them too would bury the ones that are.
 * A named spot (16b cut V.1) keeps the friendly's rule: ground, labelled from
 * 6 px and never below.
 */
export function labelledAt(
  kind: MarkerKind,
  designator: string,
  lock: string | null,
  px: number,
): boolean {
  if (kind === "spot") {
    return px >= LABEL_ALL_PX;
  }
  return kind === "hostile" || designator === lock || px >= LABEL_ALL_PX;
}

/**
 * **One flag per marker, in frame order: is its label painted?** (16b cut B.1,
 * `B-09`). The canvas reads this once per frame and decides nothing itself.
 *
 * A marker `labelledAt` this size is labelled unless its text box overlaps a
 * box already placed — so a knot of hostiles at 2 px reads as the first one's
 * name rather than as an unreadable smear — except that the LOCK's label is
 * always placed, and placed first, so it is never the one skipped. The box is
 * the text's own: it starts `px * 1.3` right of the cell's west edge (the paint
 * anchor, `cx + px * 0.8`), runs `designator.length` glyphs of six tenths of the
 * font the paint uses — 5 px under 6 px, where the font is a fixed 8 px, and
 * `max(8, px) * 0.6` from 6 px, so a 6 or 7 px cell, whose font is still 8 px,
 * is not measured narrower than it is painted — and is 8 px tall about the
 * row's middle. Boxes that only touch do not overlap.
 *
 * **16b cut V.1**: the order is the lock, then every marker that is not a spot
 * in order, and the spots LAST — so a spot's label never displaces a
 * contact's. `B-09`'s promise is that a hostile and the lock are labelled at
 * every zoom, and the name of a field must not undo it. A spot that does not
 * own its cell (a later marker landed on it, so it is drawn nowhere) is never
 * labelled and places no box.
 */
export function labelPlan(markers: readonly Marker[], lock: string | null, px: number): boolean[] {
  const glyph = px < LABEL_ALL_PX ? 5 : Math.max(8, px) * 0.6;
  const box = (m: Marker): [number, number, number, number] => {
    const x0 = m.x * px + px * 1.3;
    const yMid = m.y * px + px / 2;
    return [x0, yMid - 4, x0 + m.designator.length * glyph, yMid + 4];
  };
  const placed: [number, number, number, number][] = [];
  const overlaps = (b: [number, number, number, number]): boolean =>
    placed.some(([x0, y0, x1, y1]) => b[0] < x1 && x0 < b[2] && b[1] < y1 && y0 < b[3]);
  const flags = markers.map(() => false);
  // The lock's first, and unconditionally. A spot is never the lock.
  markers.forEach((m, i) => {
    if (lock !== null && m.kind !== "you" && m.kind !== "spot" && m.designator === lock) {
      flags[i] = true;
      placed.push(box(m));
    }
  });
  const owns = markerOwners(markers);
  const place = (m: Marker, i: number): void => {
    if (flags[i] || !labelledAt(m.kind, m.designator, lock, px)) {
      return;
    }
    // A covered spot is drawn nowhere, so it is labelled nowhere.
    if (m.kind === "spot" && owns[i] !== true) {
      return;
    }
    const b = box(m);
    if (!overlaps(b)) {
      flags[i] = true;
      placed.push(b);
    }
  };
  // Every other marker in order, then the spots (16b cut V.1).
  markers.forEach((m, i) => {
    if (m.kind !== "spot") {
      place(m, i);
    }
  });
  markers.forEach((m, i) => {
    if (m.kind === "spot") {
      place(m, i);
    }
  });
  return flags;
}

/** The whole of what the screen prints for one frame. `win` is the composed
 * window the canvas is drawing (`null` while the map has not arrived). */
export function cockpitView(
  frame: CockpitFrame,
  win: ComposedWindow | null,
  notices: readonly string[],
): CockpitView {
  return {
    heading: `Map — heading ${deg3(frame.heading_deg)}°`,
    legend: markerLegend(win),
    status: statusLines(frame),
    damage: damageRows(frame),
    // 17b cut M5.5: on a hull with no weapons station, the mind's rows take
    // `(no weapon)`'s place.
    weapons: frame.weapons.length === 0 && frame.mind !== null ? [] : weaponLines(frame),
    mind: mindLines(frame),
    profiles: mindProfiles(frame),
    contacts: contactRows(frame),
    notices: [...notices],
  };
}

/**
 * Every line of a view, flattened — the golden's subject and the observer
 * rule's lens. If a word is not in here it is not on the screen, because the
 * component renders these same values and decides none of them itself.
 */
export function cockpitViewLines(view: CockpitView): string[] {
  const lines = [view.heading];
  if (view.legend.length > 0) {
    lines.push(view.legend);
  }
  lines.push(...view.status);
  lines.push(...view.damage.map(damageText));
  lines.push(...view.weapons);
  // 17b cut M5.5: the weapons section's tail.
  lines.push(...view.mind);
  for (const row of view.contacts) {
    lines.push(contactText(row));
    if (row.chips.length > 0) {
      lines.push(scanText(row.chips));
    }
  }
  if (view.contacts.length === 0) {
    lines.push("(no contacts)");
  }
  lines.push(...view.notices);
  return lines;
}
