// The cockpit on the web (16b cut Q.5; doc 16 §5): the wire types of the two
// arena feeds, their decoders, the ONE map composition every renderer runs,
// and the view model `CockpitScreen.svelte` paints.
//
// Everything here is pure and DOM-free, so vitest pins what the player sees
// without a browser: the canvas draws from `composeWindow`'s output and the
// screen prints `cockpitView`'s strings, and nothing in the component decides
// either. That is what makes the parity golden (`fixtures/cockpit/view_grid.txt`)
// an assertion about the web client and not about a test helper.
//
// **The observer rule** (doc 16 §1, §5) is enforced by the SHAPE of the types
// below rather than by care at each print site: `CockpitFrame` carries no
// contact `id`, no `lock` digits, no `unit` object number and no notice `text`.
// A renderer cannot leak a field its decoder never kept, and a notice's own
// sentence already reaches the scrollback as a `Sys` line (16b cut F.2's
// one-owner-per-sentence contract), so the ring here is composed from `kind` +
// `designator` + `section` and never from the server's prose.
//
// The payload is a `serde_json::Value` on the server — outside ts-rs — so these
// types are hand-written, and every decoder is tolerant of a missing key and of
// an unknown one: the frame evolves additively (16b cut Q.1 and Q.2 both added
// fields at `FRAME_VERSION` 1), and neither an older server nor a newer one may
// make this client throw.

/** The per-pulse cockpit keyframe's feed (`tacsim::arena_bridge::FRAME_FEED`). */
export const COCKPIT_FEED = "arena.cockpit";

/** The whole arena's terrain, pulled and never pushed (`MAP_FEED`). */
export const ARENA_MAP_FEED = "arena.map";

/** The default square viewport of the web's map, in arena cells: a sensor's
 * sixty reach every way from the middle and still leaves margin. */
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

/** A scan band letter, as `Condition::letters` spells it (16b cut Q.2). */
export type ScanBand = "green" | "yellow" | "red" | "destroyed";

/** What a marker is, which is the whole of what the canvas colours it by. */
export type MarkerKind = "you" | "unknown" | "friendly" | "hostile";

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
}

export interface CockpitNotice {
  /** The wire word (`NoticeKind::as_str`). */
  kind: string;
  designator: string | null;
  section: string | null;
}

export interface CockpitWeapon {
  id: string;
  ready: boolean;
  ready_in_ms: number;
  rounds: number;
  rounds_max: number;
  /** This weapon's own chance against the lock, or `null` without one. */
  percent: number | null;
}

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
  speed_kph: number;
  heading_deg: number;
  heat: CockpitHeat | null;
  tempo: string;
  lock_designator: string | null;
  lock_range_m: number | null;
  lock_percent: number | null;
  sections: CockpitSection[];
  weapons: CockpitWeapon[];
  contacts: CockpitContact[];
  /** The distinct section-id lists a `condition.table` indexes (16b cut Q.2). */
  scan_tables: string[][];
  notices: CockpitNotice[];
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
}

/** A marker drawn on the composed window. */
export interface Marker {
  /** The window column. */
  x: number;
  /** The window ROW FROM THE SOUTH, so `grid[h - 1 - y]` is the line it is on. */
  y: number;
  glyph: string;
  kind: MarkerKind;
  /** The designator (or the pilot's callsign for their own marker). */
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
  };
}

function decodeNotice(raw: unknown): CockpitNotice | null {
  if (!isObj(raw) || typeof raw.kind !== "string") {
    return null;
  }
  // `text` is deliberately NOT read: the arena already sent that sentence as a
  // `Sys` line and one sentence has one owner (16b cut F.2).
  return {
    kind: clean(raw.kind, "?"),
    designator: optStr(raw.designator, 8),
    section: optStr(raw.section, 24),
  };
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
    speed_kph: int(data.speed_kph, 0),
    heading_deg: int(data.heading_deg, 0),
    heat: decodeHeat(data.heat),
    tempo: clean(data.tempo, "?"),
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
 * **The map composition** (16b cut Q.5; doc 16 §5), the rule the TUI's
 * `render_cockpit_map_rows` and the server's `view_window` both run, differing
 * only in the SIZE asked for:
 *
 * 1. the window is `w = min(map.width, cols)` by `h = min(map.height, rows)`,
 *    centred on the platform's own `cell` — the map's middle when it has none —
 *    and CLAMPED to the arena, so a tank in a corner is drawn at its own cell;
 * 2. the terrain comes out of the run-length `rows`, whose row `0` is the SOUTH
 *    edge, clipped to the window and padded with spaces;
 * 3. the markers go down in the order 16b cut Q.3 fixed — the pilot's own `@`
 *    FIRST, then the contacts in the frame's own order — each at its ABSOLUTE
 *    cell less the window origin, a later marker overwriting an earlier one;
 * 4. a contact whose `cell` is `null`, or which falls outside the window, is not
 *    drawn (it is still in the contacts table);
 * 5. `grid` prints NORTH-UP: `grid[0]` is the window's north row.
 *
 * Every glyph is the payload's own. There is no second table anywhere on this
 * path, which is why composing the committed `arena_map.json` and
 * `cockpit_frame.json` at exactly 78 by 20 has to equal `view_grid.txt` byte
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

  // Only the rows the window shows are expanded: a shipped arena is 256 by 256
  // and a viewport of it has no business paying for the other sixty thousand
  // cells on every redraw (the TUI's `mush_ansi::window_rows` makes the same
  // trade for the same reason).
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
 * the payload did not already carry. */
const NOTICE_WORDS: Readonly<Record<string, string>> = {
  acquired: "new contact",
  classified: "identified",
  lost: "contact lost",
  lock_lost: "lock broken",
  locked: "locked",
  hit: "hit",
  miss: "miss",
  kill: "destroyed",
  struck: "hit taken",
  section_lost: "section gone",
  target_section_lost: "section destroyed",
  refused: "refused",
  blocked: "held",
  power_online: "power online",
};

/**
 * One line of the notices ring, composed from `kind` + `designator` + `section`
 * and NEVER from the notice's own `text` (16b cut F.2: one sentence, one owner
 * — the arena already sent that sentence to the scrollback as a `Sys` line, and
 * a panel that printed it too would say everything twice).
 */
export function noticeLine(notice: CockpitNotice): string {
  const words = NOTICE_WORDS[notice.kind] ?? notice.kind.replace(/_/g, " ");
  const head = notice.designator === null ? words : `${notice.designator} ${words}`;
  return notice.section === null ? head : `${head}: ${notice.section}`;
}

/** Every notice of one frame, as ring lines. */
export function noticeLines(notices: readonly CockpitNotice[]): string[] {
  return notices.map(noticeLine);
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
   * contact marker landed. Excludes the pilot and INCLUDES a marker a later one
   * overwrote, exactly as the TUI's legend does. */
  legend: string;
  /** The status block: callsign, power, speed/heading, heat, tempo, lock. */
  status: string[];
  damage: CockpitDamageRow[];
  /** One line a weapon, or the single `(no weapon)` line. */
  weapons: string[];
  contacts: CockpitContactRow[];
  /** The ring, oldest first. */
  notices: string[];
}

const BAND_OF: Readonly<Record<string, ScanBand>> = {
  g: "green",
  y: "yellow",
  r: "red",
  d: "destroyed",
};

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
    chips.push({ word: table[i] ?? "?", letter, band: BAND_OF[letter] ?? "green" });
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
  const lines = [
    `Callsign ${frame.callsign}`,
    `power ${frame.power}${frame.blocked ? "  HELD" : ""}`,
    `${frame.speed_kph} kph  heading ${deg3(frame.heading_deg)}°`,
  ];
  if (frame.heat !== null) {
    lines.push(
      `heat ${frame.heat.heat}/${frame.heat.max}${frame.heat.overheated ? "  OVERHEATED" : ""}`,
    );
  }
  lines.push(`tempo ${frame.tempo}`);
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

/** The weapons rows: readiness, rounds, and each weapon's OWN chance. */
export function weaponLines(frame: CockpitFrame): string[] {
  if (frame.weapons.length === 0) {
    return ["(no weapon)"];
  }
  return frame.weapons.map((w) => {
    const ready = w.ready ? "ready" : `${Math.ceil(Math.max(0, w.ready_in_ms) / 1000)} s`;
    const chance = w.percent === null ? "" : `  ${w.percent}%`;
    return `${w.id}  ${ready}  ${w.rounds}/${w.rounds_max}${chance}`;
  });
}

/** The contact table's rows, in the frame's own order. */
export function contactRows(frame: CockpitFrame): CockpitContactRow[] {
  return frame.contacts.map((c) => ({
    designator: c.designator,
    detail: `${c.class}  ${deg3(c.bearing_deg)}°  ${c.band}  ${c.range_m} m  ${c.age_s} s`,
    chips: scanChips(c, frame.scan_tables),
  }));
}

/** One contact row as the line a terminal prints. */
export function contactText(row: CockpitContactRow): string {
  return `${row.designator.padEnd(4)}${row.detail}`;
}

/** The marker legend: every contact marker that landed, in frame order. */
export function markerLegend(win: ComposedWindow | null): string {
  return win === null
    ? ""
    : win.markers
        .filter((m) => m.kind !== "you")
        .map((m) => `${m.glyph} ${m.designator}`)
        .join("  ");
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
    weapons: weaponLines(frame),
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
