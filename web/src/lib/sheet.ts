// Sheet schema/doc — pure types + rendering helpers for the schema-driven
// character sheet panel (crates/mush-ruleset/src/schema.rs +
// crates/ruleset-wod/src/sheet_cmds.rs's `wod.sheet` query).
//
// ## Why hand-written types, not ts-rs
//
// `SheetSchema`/`SheetDoc` live in mush-ruleset (not mush-proto), and the
// sheet is explicitly designed to be walked generically — `SheetSchema` IS
// the wire description (`SheetSchema::describe()` just serializes itself;
// see the schema module docs). Adding `#[cfg_attr(feature = "ts", derive(TS))]`
// there would be a cross-crate touch outside this agent's web/-only scope,
// and would fight the framework's own "one declarative source, walked at
// render time" design (recursive `FieldType::{List,Map}` boxes are exactly
// the kind of shape ts-rs mirrors awkwardly). So: a light hand-written
// interface here, validated defensively at the JSON boundary (mirrors
// mail.svelte.ts's `str`/`num` guards) — deliberately loose where the
// schema is recursive (`FieldType` for `List`/`Map` element types), strict
// where it matters for rendering. Documented per the assignment's explicit
// "runtime-walked JSON with a hand-written light interface" option.
//
// Wire shapes below mirror serde's defaults exactly (see schema.rs/doc.rs):
//   SectionLayout   -> "list" | "tracks" | "map" | { grid: { rows, cols } }
//   FieldType       -> internally tagged on "kind" (snake_case variant names)
//   TrackSize       -> { fixed: number } | { derived: unknown }
//   FieldDef.staff_only -> present (true) only when the field is staff-only
//   ObjId (target)  -> serializes via Display as "#123" (a String), NOT a
//                      bare number, wherever the Rust side called
//                      `.to_string()` first (both `target` and the roll
//                      panel's `who` do; see roll.ts)

/** A track's slot count: fixed, or derived from sibling fields (already
 * resolved into a concrete array by the time `data` arrives — this client
 * never evaluates `derived`). */
export type TrackSize = { fixed: number } | { derived: unknown };

/** One level of a damage/resource track (`schema::DamageLevelDef`). */
export interface DamageLevelDef {
  id: string;
  label: string;
  penalty: number;
}

/** A field's type + constraints (`schema::FieldType`, tag = "kind"). */
export type FieldType =
  | { kind: "dots"; min: number; max: number }
  | { kind: "number"; min: number; max: number }
  | { kind: "text"; max_len: number }
  | { kind: "enum"; options: string[] }
  | { kind: "track"; size: TrackSize; levels: DamageLevelDef[] }
  | { kind: "list"; of: FieldType }
  | { kind: "map"; value: FieldType };

/** One sheet field (`schema::FieldDef`). */
export interface FieldDef {
  id: string;
  label: string;
  ty: FieldType;
  category?: string;
  default: unknown;
  /** Present (and `true`) only for staff-only fields; non-staff viewers
   * never see the key in `data` at all (the server strips it — see
   * `strip_staff_only` in sheet_cmds.rs), so a field can be `staff_only`
   * here yet simply absent from `data`. */
  staff_only?: boolean;
}

/** A section's rendering hint (`schema::SectionLayout`). */
export type SectionLayout = "list" | "tracks" | "map" | { grid: { rows: number; cols: number } };

/** One sheet section (`schema::SheetSection`). */
export interface SheetSection {
  id: string;
  label: string;
  layout: SectionLayout;
  fields: FieldDef[];
}

/** The whole declarative sheet schema (`schema::SheetSchema`). */
export interface SheetSchema {
  ruleset: string;
  version: number;
  sections: SheetSection[];
}

/** The approval-state summary the `<ruleset>.sheet` query attaches
 * (`sheet_cmds::approval_json`). `detail` mirrors `doc::ApprovalState`
 * (externally tagged: `"unapproved"` | `{pending: jobId}` |
 * `{approved: {by, at}}`) but is not modeled further here — the panel only
 * ever needs `state` (a ready-made label) and `approved`. */
export interface SheetApproval {
  state: string;
  approved: boolean;
  detail?: unknown;
}

/** The full `<ruleset>.sheet` panel payload (`sheet_cmds::SheetQuery::query`). */
export interface SheetPayload {
  target: string;
  schema: SheetSchema;
  approved: SheetApproval;
  data: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Defensive parsing (the JSON boundary)
// ---------------------------------------------------------------------------

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

/** A `FieldType`, validated shallowly (the `kind` tag + its own scalar
 * fields); recursive `of`/`value` element types are trusted as-is once the
 * outer shape checks out — the renderer degrades gracefully (see
 * `typedValue`'s fallback branches) if a nested type is ever malformed. */
function parseFieldType(raw: unknown): FieldType | null {
  if (!isObj(raw) || typeof raw.kind !== "string") {
    return null;
  }
  switch (raw.kind) {
    case "dots":
    case "number":
      return typeof raw.min === "number" && typeof raw.max === "number"
        ? { kind: raw.kind, min: raw.min, max: raw.max }
        : null;
    case "text":
      return typeof raw.max_len === "number" ? { kind: "text", max_len: raw.max_len } : null;
    case "enum":
      return Array.isArray(raw.options)
        ? { kind: "enum", options: raw.options.flatMap((o) => (typeof o === "string" ? [o] : [])) }
        : null;
    case "track": {
      const size = isObj(raw.size)
        ? typeof raw.size.fixed === "number"
          ? { fixed: raw.size.fixed }
          : { derived: raw.size.derived }
        : { fixed: 0 };
      const levels = Array.isArray(raw.levels)
        ? raw.levels.flatMap((l) =>
            isObj(l) && typeof l.id === "string" && typeof l.penalty === "number"
              ? [{ id: l.id, label: str(l.label) ?? l.id, penalty: l.penalty }]
              : [],
          )
        : [];
      return { kind: "track", size, levels };
    }
    case "list": {
      const of = parseFieldType(raw.of) ?? { kind: "text", max_len: 0 };
      return { kind: "list", of };
    }
    case "map": {
      const value = parseFieldType(raw.value) ?? { kind: "text", max_len: 0 };
      return { kind: "map", value };
    }
    default:
      return null;
  }
}

function parseFieldDef(raw: unknown): FieldDef | null {
  if (!isObj(raw) || typeof raw.id !== "string" || typeof raw.label !== "string") {
    return null;
  }
  const ty = parseFieldType(raw.ty);
  if (!ty) {
    return null;
  }
  const out: FieldDef = { id: raw.id, label: raw.label, ty, default: raw.default };
  if (typeof raw.category === "string") {
    out.category = raw.category;
  }
  if (raw.staff_only === true) {
    out.staff_only = true;
  }
  return out;
}

function parseLayout(raw: unknown): SectionLayout {
  if (raw === "list" || raw === "tracks" || raw === "map") {
    return raw;
  }
  if (isObj(raw) && isObj(raw.grid)) {
    const rows = typeof raw.grid.rows === "number" ? raw.grid.rows : 1;
    const cols = typeof raw.grid.cols === "number" ? raw.grid.cols : 1;
    return { grid: { rows, cols } };
  }
  return "list";
}

function parseSection(raw: unknown): SheetSection | null {
  if (!isObj(raw) || typeof raw.id !== "string" || typeof raw.label !== "string") {
    return null;
  }
  const fields = Array.isArray(raw.fields) ? raw.fields.flatMap((f) => parseFieldDef(f) ?? []) : [];
  return { id: raw.id, label: raw.label, layout: parseLayout(raw.layout), fields };
}

/** Parse a `SheetSchema`; `null` on a fundamentally malformed payload. */
export function parseSheetSchema(raw: unknown): SheetSchema | null {
  if (!isObj(raw) || typeof raw.ruleset !== "string" || typeof raw.version !== "number") {
    return null;
  }
  const sections = Array.isArray(raw.sections)
    ? raw.sections.flatMap((s) => parseSection(s) ?? [])
    : [];
  return { ruleset: raw.ruleset, version: raw.version, sections };
}

/** Parse the approval summary; a permissive default when absent/malformed
 * (never blocks rendering the rest of the sheet). */
export function parseApproval(raw: unknown): SheetApproval {
  if (!isObj(raw)) {
    return { state: "unknown", approved: false };
  }
  return {
    state: str(raw.state) ?? "unknown",
    approved: raw.approved === true,
    detail: raw.detail,
  };
}

/** Parse a full `<ruleset>.sheet` keyframe payload; `null` if the schema
 * itself doesn't parse (a malformed schema means nothing is renderable). */
export function parseSheetPayload(raw: unknown): SheetPayload | null {
  if (!isObj(raw)) {
    return null;
  }
  const schema = parseSheetSchema(raw.schema);
  if (!schema) {
    return null;
  }
  return {
    target: str(raw.target) ?? "",
    schema,
    approved: parseApproval(raw.approved),
    data: isObj(raw.data) ? raw.data : {},
  };
}

// ---------------------------------------------------------------------------
// Layout dispatch
// ---------------------------------------------------------------------------

/** Normalize a `SectionLayout` to its discriminant, for `{#if}` dispatch. */
export function layoutKind(l: SectionLayout): "grid" | "list" | "tracks" | "map" {
  return typeof l === "string" ? l : "grid";
}

// ---------------------------------------------------------------------------
// Dots / pip pools
// ---------------------------------------------------------------------------

/** A `Number` field renders as a pip row (like `Dots`) when its span is this
 * small or smaller — covers Willpower (1..10), Rage/Gnosis (0..10), Renown
 * tracks (0..10), Generation (4..15), Humanity (0..10), Merit/Flaw points
 * (0..7). Wider pools (Blood Pool 0..50, Quintessence/Paradox 0..100, XP)
 * fall back to a numeric value + proportional bar in `isPipRange` — driven
 * purely by the field's declared range, never by field id, so it needs no
 * per-splat knowledge. */
export const PIP_MAX_SPAN = 20;

export function isPipRange(min: number, max: number): boolean {
  return Math.max(0, max - Math.min(min, 0)) <= PIP_MAX_SPAN;
}

/** `value` many filled pips out of `max` total (ignoring `min` — a dot row's
 * *length* is its ceiling, not its chargen floor; matches the classic sheet
 * convention of drawing all 5 attribute dots even though the minimum is 1). */
export function pipRow(value: number, max: number): boolean[] {
  const n = Math.max(0, Math.trunc(max));
  const v = Math.max(0, Math.min(n, Math.trunc(value)));
  return Array.from({ length: n }, (_, i) => i < v);
}

/** Fill percentage (0..100) for the wide-range numeric bar fallback. */
export function barPct(value: number, min: number, max: number): number {
  const span = max - min;
  if (span <= 0) {
    return 0;
  }
  const v = Math.max(min, Math.min(max, value));
  return Math.round(((v - min) / span) * 100);
}

// ---------------------------------------------------------------------------
// Tracks (Health, and any future damage/resource track)
// ---------------------------------------------------------------------------

/** A track's mark string -> the classic single-character glyph. Unknown
 * marks (a future resource track's own vocabulary) fall back to their
 * first letter, uppercased, so the box still shows *something* meaningful
 * without this module needing to know every possible mark. */
export function trackMarkSymbol(mark: string | null): string {
  if (mark === null) {
    return "";
  }
  switch (mark) {
    case "bashing":
      return "/";
    case "lethal":
      return "X";
    case "aggravated":
      return "*";
    default:
      return mark.charAt(0).toUpperCase();
  }
}

/** A track field's raw `data[field.id]` array -> validated marks (`null` =
 * empty slot). Non-conforming entries default to empty rather than
 * throwing. */
export function trackCells(raw: unknown): (string | null)[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw.map((v) => (typeof v === "string" ? v : null));
}

/** The `DamageLevelDef` currently in effect (deepest filled slot), or `null`
 * for an empty track / a track with no declared levels. Mirrors
 * `SheetStore::wound_penalty`'s slot->level mapping. */
export function deepestLevel(
  marks: (string | null)[],
  levels: DamageLevelDef[],
): DamageLevelDef | null {
  const filled = marks.filter((m) => m !== null).length;
  if (filled === 0 || levels.length === 0) {
    return null;
  }
  const idx = Math.min(filled - 1, levels.length - 1);
  return levels[idx] ?? null;
}

// ---------------------------------------------------------------------------
// Maps (Backgrounds, Disciplines/Spheres/Gifts, Merits/Flaws, Specialties, …)
// ---------------------------------------------------------------------------

export interface MapEntry {
  key: string;
  value: unknown;
}

/** A `Map` field's `data[field.id]` entries, alphabetically by key (stable,
 * readable ordering — the server stores a plain JSON object, which has no
 * guaranteed order of its own). */
export function mapEntries(data: Record<string, unknown>, field: FieldDef): MapEntry[] {
  const raw = data[field.id];
  if (!isObj(raw)) {
    return [];
  }
  return Object.entries(raw)
    .map(([key, value]) => ({ key, value }))
    .sort((a, b) => a.key.localeCompare(b.key));
}

/** A `List` field's `data[field.id]` elements. */
export function listItems(data: Record<string, unknown>, field: FieldDef): unknown[] {
  const raw = data[field.id];
  return Array.isArray(raw) ? raw : [];
}

/** A section's fields that are actually present in `data` — the panel's
 * staff_only gate. The server already strips staff-only fields from a
 * non-staff viewer's `data` (see `strip_staff_only` in sheet_cmds.rs); this
 * is just "don't render a field that isn't there" for *any* reason (a
 * partial delta, a schema/data version mismatch mid-migration), so a
 * staff-only field a player can't see is naturally omitted without this
 * function ever checking `staff_only` itself. */
export function visibleFields(data: Record<string, unknown>, section: SheetSection): FieldDef[] {
  return section.fields.filter((f) => data[f.id] !== undefined);
}

// ---------------------------------------------------------------------------
// Small value coercions used by the panel's field renderer
// ---------------------------------------------------------------------------

export function asNumber(v: unknown): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

export function asText(v: unknown): string {
  if (typeof v === "string") {
    return v;
  }
  if (typeof v === "number" || typeof v === "boolean") {
    return String(v);
  }
  return "";
}

export function asStringList(v: unknown): string[] {
  return Array.isArray(v) ? v.flatMap((x) => (typeof x === "string" ? [x] : [])) : [];
}

/** First-letter capitalization for enum values (`"vampire"` -> `"Vampire"`) —
 * the schema only carries raw option ids, no separate display labels. */
export function capitalize(s: string): string {
  return s.length > 0 ? s.charAt(0).toUpperCase() + s.slice(1) : s;
}

// ---------------------------------------------------------------------------
// Approval banner
// ---------------------------------------------------------------------------

export type ApprovalKind = "approved" | "pending" | "unapproved";

export function approvalKind(approval: SheetApproval): ApprovalKind {
  if (approval.approved) {
    return "approved";
  }
  return approval.state.startsWith("pending") ? "pending" : "unapproved";
}
