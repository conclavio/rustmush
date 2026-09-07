// Unit panel state (interface cut I8): one `*.unit` keyframe — an object,
// a schema identity, a revision, sections, cells, components, capabilities
// and a mobility summary — plus the projected journal deltas a mutating
// staff command publishes against it.
//
// Kept module-generic in exactly the way `stores/sheet.svelte.ts` and
// `stores/vitals.svelte.ts` are: this file never names a part, a pool or a
// judgement. Every label, id and `condition` word is printed as the frame
// gives it. The ONLY words this client interprets are the four gauge states
// (`ok` / `warn` / `crit` / `down`) a ruleset publishes as `state` —
// rulesets judge, clients render (docs/design/10b §4 rule 1). A `condition`
// word is opaque: it is printed verbatim and never mapped to a glyph, a
// colour or an ordering.
//
// Shape follows `SheetStore`: a revision watermark plus a boolean resync
// storm guard released by ANY reply to a unit Req. The coalescing
// `setTimeout` that turns a burst of deltas into ONE keyframe request lives
// in `GameApp` (`#scheduleUnitResync`), so this store stays pure and
// testable; `clear()` releases the guard so a cleared store can never leave
// a refresh latched.
//
// Everything is read defensively at the JSON boundary: a malformed payload
// yields `null` / `false` and never throws, and no key the parsers do not
// name is ever carried through (so a definition body could not leak into
// the client even if a producer sent one).

/** Rows the journal ring holds, newest first. */
export const JOURNAL_RING = 200;
/** Rows one delta may contribute (49 plus one elided row). */
export const JOURNAL_ROWS_PER_DELTA = 50;
/** How long a burst of deltas is coalesced before one keyframe request. */
export const UNIT_RESYNC_MS = 250;

/** The contacts feed the unit screen embeds (a fixed module feed id, like
 * `comsys` / `scenes` — not derived by suffix). */
export const CONTACTS_FEED = "tactics.contacts";

/** What the panel says when the held keyframe data is `null`. */
export const NO_UNIT_LINE =
  "No tactical unit: take a station, or @unit/show #n if you control one.";

/** The client-internal journal kind standing in for the rows one oversized
 * delta could not carry. It never appears on the wire. */
export const ELIDED_KIND = "elided";

/** Address scopes (`addr` and `install` are flat `/`-joined paths). */
const UNIT_SCOPE = "unit";
const SECTION_SCOPE = "section";
const FACET_SCOPE = "facet";
const COMPONENT_SCOPE = "component";
const SLOT_SCOPE = "slot";

/** An em dash for an absent value. */
const DASH = "—";

/** The only `state` words this client interprets (rule 1). Anything else is
 * neutral: an unknown word must never reach a class name, because the app's
 * global stylesheet is unscoped and a value carrying whitespace would land
 * a second, attacker-chosen class token on the element. */
const GAUGE_STATES: readonly string[] = ["ok", "warn", "crit", "down"];

/** A `state` word, or `""` for anything this client does not know. */
export function gaugeState(v: unknown): string {
  return typeof v === "string" && GAUGE_STATES.includes(v) ? v : "";
}

/** Which half of the screen is showing (D-I8-4: one screen, two layouts). */
export type UnitLayout = "unit" | "damage";

/** What applying a journal delta asks the caller to do. */
export type JournalOutcome = "resync" | "stale" | "foreign" | "malformed";

export interface UnitObject {
  id: string;
  name: string;
}

export interface UnitCell {
  addr: string;
  label: string;
  current: number;
  min: number;
  max: number;
  /** One of the four gauge states, or `""` for anything else. */
  state: string;
}

export interface UnitFacet {
  id: string;
  cells: string[];
}

export interface UnitSection {
  id: string;
  /** The ruleset's opaque word — printed, never interpreted. */
  condition: string;
  state: string;
  facets: UnitFacet[];
  slots: string[];
}

export interface UnitComponent {
  id: string;
  definition: string;
  install: string;
  condition: string;
  state: string;
  asset: boolean;
}

export interface UnitMobilitySwitch {
  from: string;
  to: string;
  ready_at: number;
}

export interface UnitMobilityMode {
  id: string;
  locomotion: string;
  mediums: string[];
  tags: string[];
  nominal_speed_cms: number;
  transition_s: number;
}

export interface UnitMobility {
  mode: string | null;
  switching: UnitMobilitySwitch | null;
  modes: UnitMobilityMode[];
}

export interface UnitCapability {
  id: string;
  source: string | null;
}

/** The frame's TRUE totals — exactly these four keys, so a producer's extra
 * key cannot ride into the store unread (the byte-budget walk reports what
 * it could not carry through the gaps between these and the held lists). */
export interface UnitCounts {
  sections: number;
  cells: number;
  components: number;
  capabilities: number;
}

export interface UnitView {
  object: UnitObject;
  /** A definition identity (`owner:kind:id@version#<hash>`), never a body —
   * held so a future pane can show provenance; no pane renders it today. */
  schema: string;
  revision: number;
  state: string;
  record: string | null;
  mobility: UnitMobility | null;
  sections: UnitSection[];
  cells: UnitCell[];
  components: UnitComponent[];
  capabilities: UnitCapability[];
  counts: UnitCounts;
  truncated: boolean;
}

/** One ring row: the projected entry plus the delta header it came from. */
export interface JournalRow {
  object: string;
  rev: number;
  cause: string | null;
  at: number;
  entry: Record<string, unknown>;
}

// -- defensive readers -------------------------------------------------------

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function strList(v: unknown): string[] {
  return Array.isArray(v) ? v.flatMap((s) => str(s) ?? []) : [];
}

/** A scalar as display text; anything else is an empty cell. */
function scalar(v: unknown): string {
  if (typeof v === "string") {
    return v;
  }
  if (typeof v === "number" || typeof v === "boolean") {
    return String(v);
  }
  return "";
}

function join(parts: readonly string[]): string {
  return parts.filter((p) => p.length > 0).join(" ");
}

function parseObject(raw: unknown): UnitObject | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  return id === null ? null : { id, name: str(raw.name) ?? id };
}

function parseCell(raw: unknown): UnitCell | null {
  if (!isObj(raw)) {
    return null;
  }
  const addr = str(raw.addr);
  const current = num(raw.current);
  const min = num(raw.min);
  const max = num(raw.max);
  if (addr === null || current === null || min === null || max === null) {
    return null;
  }
  return { addr, label: str(raw.label) ?? addr, current, min, max, state: gaugeState(raw.state) };
}

function parseFacet(raw: unknown): UnitFacet | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  return id === null ? null : { id, cells: strList(raw.cells) };
}

function parseSection(raw: unknown): UnitSection | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  if (id === null) {
    return null;
  }
  return {
    id,
    condition: str(raw.condition) ?? "",
    state: gaugeState(raw.state),
    facets: Array.isArray(raw.facets) ? raw.facets.flatMap((f) => parseFacet(f) ?? []) : [],
    slots: strList(raw.slots),
  };
}

function parseComponent(raw: unknown): UnitComponent | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  if (id === null) {
    return null;
  }
  return {
    id,
    definition: str(raw.definition) ?? "",
    install: str(raw.install) ?? "",
    condition: str(raw.condition) ?? "",
    state: gaugeState(raw.state),
    asset: raw.asset === true,
  };
}

function parseSwitching(raw: unknown): UnitMobilitySwitch | null {
  if (!isObj(raw)) {
    return null;
  }
  const from = str(raw.from);
  const to = str(raw.to);
  const readyAt = num(raw.ready_at);
  if (from === null || to === null || readyAt === null) {
    return null;
  }
  return { from, to, ready_at: readyAt };
}

function parseMode(raw: unknown): UnitMobilityMode | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  if (id === null) {
    return null;
  }
  return {
    id,
    locomotion: str(raw.locomotion) ?? "",
    mediums: strList(raw.mediums),
    tags: strList(raw.tags),
    nominal_speed_cms: num(raw.nominal_speed_cms) ?? 0,
    transition_s: num(raw.transition_s) ?? 0,
  };
}

function parseMobility(raw: unknown): UnitMobility | null {
  if (!isObj(raw)) {
    return null;
  }
  return {
    mode: str(raw.mode),
    switching: parseSwitching(raw.switching),
    modes: Array.isArray(raw.modes) ? raw.modes.flatMap((m) => parseMode(m) ?? []) : [],
  };
}

function parseCapability(raw: unknown): UnitCapability | null {
  if (!isObj(raw)) {
    return null;
  }
  const id = str(raw.id);
  return id === null ? null : { id, source: str(raw.source) };
}

function parseCounts(raw: unknown): UnitCounts {
  const o = isObj(raw) ? raw : {};
  return {
    sections: num(o.sections) ?? 0,
    cells: num(o.cells) ?? 0,
    components: num(o.components) ?? 0,
    capabilities: num(o.capabilities) ?? 0,
  };
}

/**
 * Parse a unit keyframe payload; `null` when it is not one (a `null`
 * payload — the honest no-subject frame — is handled by the store, not
 * here). Only the keys named below are carried through, so nothing a
 * producer adds can reach the panel unread.
 */
export function parseUnitView(raw: unknown): UnitView | null {
  if (!isObj(raw)) {
    return null;
  }
  const object = parseObject(raw.object);
  const revision = num(raw.revision);
  if (object === null || revision === null) {
    return null;
  }
  return {
    object,
    schema: str(raw.schema) ?? "",
    revision,
    state: str(raw.state) ?? "",
    record: str(raw.record),
    mobility: parseMobility(raw.mobility),
    sections: Array.isArray(raw.sections) ? raw.sections.flatMap((s) => parseSection(s) ?? []) : [],
    cells: Array.isArray(raw.cells) ? raw.cells.flatMap((c) => parseCell(c) ?? []) : [],
    components: Array.isArray(raw.components)
      ? raw.components.flatMap((c) => parseComponent(c) ?? [])
      : [],
    capabilities: Array.isArray(raw.capabilities)
      ? raw.capabilities.flatMap((c) => parseCapability(c) ?? [])
      : [],
    counts: parseCounts(raw.counts),
    truncated: raw.truncated === true,
  };
}

/** The delta header the ring rows are tagged with. */
interface DeltaHeader {
  object: string;
  /** The revision the delta applied ON TOP of; `0` means it is the first
   * apply against a freshly created subject, whose watermark restarted. */
  fromRevision: number | null;
  toRevision: number;
  cause: string | null;
  at: number;
}

function deltaHeader(raw: unknown): DeltaHeader | null {
  if (!isObj(raw)) {
    return null;
  }
  const object = parseObject(raw.object);
  const toRevision = num(raw.to_revision);
  if (object === null || toRevision === null) {
    return null;
  }
  return {
    object: object.id,
    fromRevision: num(raw.from_revision),
    toRevision,
    cause: str(raw.cause),
    at: num(raw.now) ?? 0,
  };
}

/** A parsed delta: its header and the ring rows it contributes. */
function parseDelta(raw: unknown): { head: DeltaHeader; rows: JournalRow[] } | null {
  const head = deltaHeader(raw);
  if (head === null || !isObj(raw) || !Array.isArray(raw.entries)) {
    return null;
  }
  const entries = raw.entries.flatMap((e) => (isObj(e) ? [e] : []));
  const row = (entry: Record<string, unknown>): JournalRow => ({
    object: head.object,
    rev: head.toRevision,
    cause: head.cause,
    at: head.at,
    entry,
  });
  if (entries.length <= JOURNAL_ROWS_PER_DELTA) {
    return { head, rows: entries.map(row) };
  }
  const kept = JOURNAL_ROWS_PER_DELTA - 1;
  return {
    head,
    rows: [
      ...entries.slice(0, kept).map(row),
      row({ kind: ELIDED_KIND, omitted: entries.length - kept }),
    ],
  };
}

/**
 * The ring rows one journal delta contributes, oldest entry first; `null`
 * when the payload is not a delta. A delta carrying more than
 * `JOURNAL_ROWS_PER_DELTA` entries contributes its first 49 plus one
 * client-internal `elided` row counting the rest.
 */
export function journalRows(raw: unknown): JournalRow[] | null {
  return parseDelta(raw)?.rows ?? null;
}

// -- pane helpers ------------------------------------------------------------

/**
 * The rows of the NEWEST-ARRIVED delta the pane can report on: the ring is
 * newest first, so that is simply its first matching row's `(object, rev)`.
 *
 * Newest-ARRIVED, never the highest revision: a subject recreated in place
 * restarts at revision 0, and picking the maximum would keep reporting the
 * dead subject's last damage forever. With nothing held (an unstationed
 * staffer reading their own after-action report) the newest row's own
 * object is the subject, so the pane still fills.
 */
export function afterAction(view: UnitView | null, ring: readonly JournalRow[]): JournalRow[] {
  const mine = view === null ? ring : ring.filter((r) => r.object === view.object.id);
  const newest = mine[0];
  if (newest === undefined) {
    return [];
  }
  return mine.filter((r) => r.object === newest.object && r.rev === newest.rev);
}

/** What the before/after pane heads with when the rows are not the held
 * subject's: the object they belong to, their revision and the cause. */
export function afterActionSubject(rows: readonly JournalRow[]): string {
  const head = rows[0];
  return head === undefined ? "" : join([head.object, `rev ${head.rev}`, head.cause ?? DASH]);
}

/** Cells addressed under one section (its own and its facets') or one
 * component, over a bare cell list. */
export function cellsIn(
  cells: readonly UnitCell[],
  scope: "section" | "component",
  id: string,
): UnitCell[] {
  if (scope === "component") {
    const prefix = `${COMPONENT_SCOPE}/${id}/`;
    return cells.filter((c) => c.addr.startsWith(prefix));
  }
  const own = `${SECTION_SCOPE}/${id}/`;
  const facet = `${FACET_SCOPE}/${id}/`;
  return cells.filter((c) => c.addr.startsWith(own) || c.addr.startsWith(facet));
}

/** `cellsIn` over a held view. */
export function cellsUnder(
  view: UnitView,
  scope: "section" | "component",
  id: string,
): UnitCell[] {
  return cellsIn(view.cells, scope, id);
}

/** A section's facet-scope cells only (the compact list's inline gauges). */
export function facetCells(cells: readonly UnitCell[], id: string): UnitCell[] {
  const prefix = `${FACET_SCOPE}/${id}/`;
  return cells.filter((c) => c.addr.startsWith(prefix));
}

/** The object's own cells (the `plant` pane). */
export function unitCells(cells: readonly UnitCell[]): UnitCell[] {
  const prefix = `${UNIT_SCOPE}/`;
  return cells.filter((c) => c.addr.startsWith(prefix));
}

/** One before/after row of the after-action table. A type alias, not an
 * interface, so it carries the implicit index signature `Table`'s
 * `Record<string, unknown>` rows want. */
export type BeforeAfterRow = {
  addr: string;
  before: string;
  after: string;
};

/** The after-action table: the two-sided entries of the newest revision. */
export function beforeAfterRows(rows: readonly JournalRow[]): BeforeAfterRow[] {
  return rows.flatMap((r) => {
    const e = r.entry;
    const before = scalar(e.from);
    const after = scalar(e.to);
    switch (str(e.kind)) {
      case "cell": {
        const addr = str(e.addr);
        return addr === null ? [] : [{ addr, before, after }];
      }
      case SECTION_SCOPE: {
        const id = str(e.section);
        return id === null ? [] : [{ addr: `${SECTION_SCOPE}/${id}`, before, after }];
      }
      case COMPONENT_SCOPE: {
        const id = str(e.component);
        return id === null ? [] : [{ addr: `${COMPONENT_SCOPE}/${id}`, before, after }];
      }
      default:
        return [];
    }
  });
}

function transition(from: string, to: string): string {
  return from.length === 0 && to.length === 0 ? "" : `${from}→${to}`;
}

/**
 * One journal entry as text. Known kinds print their target and, where the
 * entry has two sides, the transition; an unknown kind prints its own
 * `kind` and nothing else (additive evolution — a client that predates a
 * kind still shows that something happened).
 */
export function journalEntryText(entry: Record<string, unknown>): string {
  const kind = str(entry.kind) ?? "";
  const both = transition(scalar(entry.from), scalar(entry.to));
  switch (kind) {
    case "cell":
      return join([kind, scalar(entry.addr), both]);
    case SECTION_SCOPE:
      return join([kind, scalar(entry.section), both]);
    case COMPONENT_SCOPE:
      return join([kind, scalar(entry.component), both]);
    case "asset":
      return join([kind, scalar(entry.component), entry.attached === true ? "attached" : "detached"]);
    case "capability_granted":
    case "capability_revoked":
      return join([kind, scalar(entry.id), scalar(entry.source)]);
    case ELIDED_KIND:
      return `… ${num(entry.omitted) ?? 0} more entries`;
    default:
      return kind;
  }
}

/** One journal table row (`rev · cause · entry`); a type alias for the same
 * reason as `BeforeAfterRow`. */
export type JournalTableRow = {
  rev: string;
  cause: string;
  entry: string;
};

/** The journal pane's rows. A row belonging to another object prints that
 * object too, so a stray frame is never read as this one's history. */
export function journalTableRows(
  ring: readonly JournalRow[],
  heldObject: string | null,
): JournalTableRow[] {
  return ring.map((r) => ({
    rev: String(r.rev),
    cause: r.cause ?? DASH,
    entry:
      r.object === heldObject
        ? journalEntryText(r.entry)
        : join([r.object, journalEntryText(r.entry)]),
  }));
}

/** The one mobility line: the effective mode, a switch in progress, or none. */
export function mobilityLine(mobility: UnitMobility | null): string {
  if (mobility === null) {
    return "mobility none";
  }
  if (mobility.switching !== null) {
    const s = mobility.switching;
    return `mobility ${s.from} → ${s.to} (ready ${s.ready_at})`;
  }
  return mobility.mode === null ? "mobility none" : `mobility ${mobility.mode}`;
}

/**
 * The footer a frame that could not carry everything gets, or `null` when
 * what is held matches what the frame says it counted.
 *
 * Two ways a member goes missing, and both are reported: the producer's
 * byte-budget walk stopped (`truncated`, and it stops on the CAPABILITY
 * list last, so a capability-only cut is the common case), or a member was
 * dropped here for being unreadable — in which case the frame calls itself
 * whole and only the count gap knows better, so the footer says
 * "incomplete" rather than claiming the producer truncated it.
 */
export function truncatedFooter(view: UnitView): string | null {
  const gap = (counted: number, held: number): number => Math.max(0, counted - held);
  const cells = gap(view.counts.cells, view.cells.length);
  const components = gap(view.counts.components, view.components.length);
  const capabilities = gap(view.counts.capabilities, view.capabilities.length);
  if (!view.truncated && cells === 0 && components === 0 && capabilities === 0) {
    return null;
  }
  const how = view.truncated ? "truncated" : "incomplete";
  return `view ${how}: ${cells} cells, ${components} components, ${capabilities} capabilities not shown`;
}

/** One key/value row of a grouped list. `state` drives the glyph and the
 * colour; `value` is the ruleset's own opaque word, printed verbatim. */
export interface UnitGroupRow {
  key: string;
  value: string;
  state: string;
  detail: string | null;
}

/** Components (and the capabilities they carry) under one install scope. */
export interface UnitGroup {
  id: string;
  label: string;
  rows: UnitGroupRow[];
}

/** The section an `install` path names, or `null` for the object's own
 * scope (`unit`, or anything this client does not recognise). */
export function installSection(install: string): string | null {
  const parts = install.split("/");
  if (parts[0] === SECTION_SCOPE || parts[0] === SLOT_SCOPE) {
    return parts[1] ?? null;
  }
  return null;
}

/** One row per key: the key/value list is keyed on it, so a capability
 * sourced twice inside one group must not appear twice. */
function dedupe(rows: readonly UnitGroupRow[]): UnitGroupRow[] {
  const seen = new Set<string>();
  const out: UnitGroupRow[] = [];
  for (const r of rows) {
    if (!seen.has(r.key)) {
      seen.add(r.key);
      out.push(r);
    }
  }
  return out;
}

/**
 * Components grouped by the section they are installed in, the object's own
 * group first (it carries the capabilities with no source). Each component
 * prints its opaque condition word with its `state` driving the glyph, is
 * tagged when it is an asset in its own right, and is followed by the
 * capabilities it sources.
 */
export function componentGroups(view: UnitView): UnitGroup[] {
  const capsOf = (source: string | null): UnitGroupRow[] =>
    view.capabilities
      .filter((c) => c.source === source)
      .map((c) => ({ key: c.id, value: "", state: "", detail: null }));
  const rowsFor = (c: UnitComponent): UnitGroupRow[] => [
    { key: c.id, value: c.condition, state: c.state, detail: c.asset ? "asset" : null },
    ...capsOf(c.id),
  ];
  const order = [null as string | null, ...view.sections.map((s) => s.id)];
  for (const c of view.components) {
    const section = installSection(c.install);
    if (section !== null && !order.includes(section)) {
      order.push(section);
    }
  }
  return order.flatMap((section) => {
    const rows = dedupe([
      ...view.components.filter((c) => installSection(c.install) === section).flatMap(rowsFor),
      ...(section === null ? capsOf(null) : []),
    ]);
    if (rows.length === 0) {
      return [];
    }
    // The group id is what the render loop keys on, so it is SCOPED: a
    // section is free to be named `unit`, and an id collision with the
    // object's own group would throw out of the keyed block.
    return [
      {
        id: section === null ? UNIT_SCOPE : `${SECTION_SCOPE}/${section}`,
        label: section ?? UNIT_SCOPE,
        rows,
      },
    ];
  });
}

/**
 * The capability pane's rows: ONE row per capability id, listing every
 * source that grants it in frame order (`—` for one the object carries in
 * its own right). The same id may legitimately arrive twice with different
 * sources, and the list is keyed on the id — so this must both collapse
 * the duplicate and keep the second source, which is the only thing the
 * pane prints besides the id.
 */
export function capabilityRows(view: UnitView): UnitGroupRow[] {
  const bySource = new Map<string, string[]>();
  for (const c of view.capabilities) {
    const sources = bySource.get(c.id);
    if (sources === undefined) {
      bySource.set(c.id, [c.source ?? DASH]);
    } else {
      sources.push(c.source ?? DASH);
    }
  }
  return [...bySource].map(([key, sources]) => ({
    key,
    value: sources.join(", "),
    state: "",
    detail: null,
  }));
}

/** A pane with no producer at all yet: the title is the pane's name and the
 * body declares what is missing (the same bytes the TUI prints). */
export interface UnitPlaceholder {
  id: string;
  title: string;
  body: string;
}

export const UNIT_PLACEHOLDERS: readonly UnitPlaceholder[] = [
  { id: "grid", title: "grid", body: "awaiting a geometry projection (plan §9)" },
  { id: "weapons", title: "weapons", body: "awaiting a weapons-cycle projection (plan §9)" },
  { id: "comms", title: "comms", body: "awaiting a station comms projection (plan §9)" },
];

/** The contacts pane while its feed has not answered. It is not one of the
 * declared placeholders above: that list is the panes with no producer
 * anywhere, and this one's producer exists — the server either has not
 * answered yet or does not publish the feed. */
export const CONTACTS_PLACEHOLDER: UnitPlaceholder = {
  id: "contacts",
  title: "contacts",
  body: "awaiting tactics.contacts (plan §9)",
};

/** The headings of the panes that have a producer. */
export const UNIT_PANE_TITLES: Readonly<Record<string, string>> = {
  sections: "sections",
  sectionmap: "sections",
  plant: "plant",
  components: "components",
  capabilities: "capabilities",
  afteraction: "after action",
  journal: "journal",
  contacts: "contacts",
};

/**
 * The panes of a layout, in render order, placeholders LAST.
 *
 * The narrow-width rule hides a placeholder by its `placeholder` CLASS,
 * wherever it sits — so this order is not what makes a phone drop them.
 * What it does is set the wide layout's reading sequence: the panes with
 * real data first, the declarations of what is still missing after them. A
 * live contacts table is a real pane and sits with them; without its frame
 * it joins the declarations at the end.
 */
export function unitPaneOrder(layout: UnitLayout, contactsLive: boolean): string[] {
  const core =
    layout === "damage"
      ? ["sectionmap", "afteraction", "journal"]
      : ["sections", "plant", "components", "capabilities"];
  const declared = UNIT_PLACEHOLDERS.map((p) => p.id);
  return contactsLive
    ? [...core, CONTACTS_PLACEHOLDER.id, ...declared]
    : [...core, ...declared, CONTACTS_PLACEHOLDER.id];
}

export class UnitStore {
  view = $state<UnitView | null>(null);
  /** Which layout the one screen shows (D-I8-4); `t` flips it. */
  layout = $state<UnitLayout>("unit");
  /** The revision watermark the held keyframe seeded. */
  revision = $state(0);
  /** The journal ring, newest first, capped at `JOURNAL_RING`. */
  journal = $state<JournalRow[]>([]);
  /** A keyframe has been applied and a view is held. */
  loaded = $state(false);
  /** The server has answered the subscribe at least once since the last
   * `clear()` — with a view, with the honest no-subject frame, or with a
   * refusal. Until it has, the panel knows nothing and must say so rather
   * than asserting the viewer has no unit. */
  answered = $state(false);
  /** The server's last answer was the honest no-subject frame. A delta
   * arriving now is somebody else's history: ring it, but do not spend a
   * round trip on a keyframe request that can only come back `null` again.
   * Cleared by any keyframe that names a subject. */
  noSubject = $state(false);
  /** A keyframe re-request is armed or in flight — the storm guard that
   * bounds a burst of deltas to one request. Released by ANY reply. */
  resyncPending = $state(false);
  /** The server answered with a refusal rather than a frame, and nothing
   * was held before it; the panel renders the message itself. */
  unavailable = $state<string | null>(null);

  /** The held object's id, or `null` when nothing is held. */
  get objectId(): string | null {
    return this.view?.object.id ?? null;
  }

  /**
   * Apply a keyframe. A `null` payload is the honest no-subject frame: the
   * view is dropped and the panel says so, without an error. Returns
   * whether the frame was understood — `false` only for a payload that is
   * neither `null` nor a unit view, which leaves state untouched.
   *
   * The ring is cleared whenever the SUBJECT changes, and losing a subject
   * is a change: held → none clears, so an intervening no-subject frame
   * cannot smuggle one object's rows into the next object's journal. A
   * first keyframe over nothing held clears nothing — the fanned delta
   * legitimately arrives before the resync's keyframe.
   */
  applyKeyframe(raw: unknown): boolean {
    if (raw === null || raw === undefined) {
      if (this.view !== null) {
        this.journal = [];
      }
      this.view = null;
      this.loaded = false;
      this.revision = 0;
      this.unavailable = null;
      this.resyncPending = false;
      this.answered = true;
      this.noSubject = true;
      return true;
    }
    const parsed = parseUnitView(raw);
    if (parsed === null) {
      return false;
    }
    const previous = this.objectId;
    if (previous !== null && previous !== parsed.object.id) {
      this.journal = [];
    }
    this.view = parsed;
    this.revision = parsed.revision;
    this.loaded = true;
    this.unavailable = null;
    this.resyncPending = false;
    this.answered = true;
    this.noSubject = false;
    return true;
  }

  /**
   * Apply a journal delta. Parseable rows are ALWAYS ringed — a delta for
   * another object, or one that arrives before any keyframe, is still real
   * history — and the outcome says what the caller should do:
   * - `"malformed"` — not a delta; nothing ringed, nothing to do;
   * - `"foreign"` — tagged with another object, or arriving while the
   *   server has told us there is no subject at all; ringed only;
   * - `"stale"` — at or behind the watermark; ringed only;
   * - `"resync"` — ahead of the watermark, or nothing held: ask for a
   *   fresh keyframe (the caller coalesces and storm-guards that).
   *
   * Rows keep the delta's own order (entry 0 first) and each delta is
   * prepended whole, so the ring reads newest DELTA first with the entries
   * of one delta in the order the producer applied them.
   */
  applyJournal(raw: unknown): JournalOutcome {
    const delta = parseDelta(raw);
    if (delta === null) {
      return "malformed";
    }
    if (delta.rows.length > 0) {
      this.journal = [...delta.rows, ...this.journal].slice(0, JOURNAL_RING);
    }
    if (this.noSubject) {
      // The last answer was "you have no subject"; a keyframe request would
      // only be answered `null` again. The rows are still real history and
      // the panes render them from the ring, each named by its object.
      return "foreign";
    }
    if (!this.loaded || this.view === null) {
      return "resync";
    }
    if (delta.head.object !== this.view.object.id) {
      return "foreign";
    }
    if (delta.head.fromRevision === 0) {
      // A from-zero delta is a subject's FIRST apply: it was recreated in
      // place and its watermark restarted, so the held one is meaningless
      // and the comparison below would read the new frame as stale.
      return "resync";
    }
    return delta.head.toRevision <= this.revision ? "stale" : "resync";
  }

  /** Flip between the two layouts (the screen's `t`). */
  toggleLayout(): void {
    this.layout = this.layout === "unit" ? "damage" : "unit";
  }

  /** The query answered with a refusal instead of a frame. Releases the
   * storm guard, so a later delta can ask again rather than being dropped
   * forever. Losing a held subject clears the ring here for the same
   * reason it does in `applyKeyframe` (unreachable in practice: the caller
   * only refuses into a panel that holds nothing). */
  markUnavailable(reason: string): void {
    if (this.view !== null) {
      this.journal = [];
    }
    this.view = null;
    this.loaded = false;
    this.unavailable = reason;
    this.resyncPending = false;
    this.answered = true;
  }

  clear(): void {
    this.view = null;
    this.layout = "unit";
    this.revision = 0;
    this.journal = [];
    this.loaded = false;
    this.answered = false;
    this.noSubject = false;
    this.resyncPending = false;
    this.unavailable = null;
  }
}
