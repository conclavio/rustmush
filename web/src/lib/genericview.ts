// Generic semantic-view schema (interface plan I9): the payload convention
// that lets a module ship a view and get a list, a detail pane or a
// read-only form rendered in every client with no bespoke code. It is a
// convention inside a view's `data` (or any panel keyframe), walked at
// render time exactly like the sheet schema (`sheet.ts`) — deliberately not
// a proto type, so it adds no wire and never touches the authoritative
// `ViewEnvelope` payload semantics.
//
// A producer sends `{ schema: {...}, rows: [...] }` for a list, or
// `{ schema: {...}, record: {...} }` for a detail or form. Every value is
// validated defensively at the JSON boundary; a malformed schema yields
// `null` and the renderer shows nothing rather than throwing.
//
// D-I-18: a row or a form never fires a mutation on its own. A row's action
// (named by `actionKey`) is always a Prefill — it opens the composer with a
// line ready, and the user confirms with Enter. The generic renderer holds
// no Run allowlist of its own.

export type ViewShape = "list" | "detail" | "form";

/** A list column. */
export interface ViewColumn {
  key: string;
  label: string;
  align: "left" | "right";
}

export type ViewFieldKind = "text" | "number" | "rich" | "bool";

/** A detail/form field. */
export interface ViewField {
  key: string;
  label: string;
  kind: ViewFieldKind;
}

/** The declarative schema a view carries. */
export interface ViewSchema {
  id: string;
  version: number;
  shape: ViewShape;
  /** List columns (empty for detail/form). */
  columns: ViewColumn[];
  /** Detail/form fields (empty for a list). */
  fields: ViewField[];
  /** The row/record field whose string value is the Prefill line for that
   * row (D-I-18); `null` when the view offers no action. */
  actionKey: string | null;
}

/** A parsed generic view: the schema plus its rows (list) or record
 * (detail/form). */
export interface GenericView {
  schema: ViewSchema;
  rows: Record<string, unknown>[];
  record: Record<string, unknown> | null;
}

const SHAPES: readonly ViewShape[] = ["list", "detail", "form"];
const FIELD_KINDS: readonly ViewFieldKind[] = ["text", "number", "rich", "bool"];

/** A row/record cap, mirroring the legal-actions bound. */
export const MAX_VIEW_ROWS = 256;
/** A column/field cap. */
export const MAX_VIEW_FIELDS = 32;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function parseColumn(raw: unknown): ViewColumn | null {
  if (!isObj(raw) || typeof raw.key !== "string") {
    return null;
  }
  return {
    key: raw.key,
    label: str(raw.label) ?? raw.key,
    align: raw.align === "right" ? "right" : "left",
  };
}

function parseField(raw: unknown): ViewField | null {
  if (!isObj(raw) || typeof raw.key !== "string") {
    return null;
  }
  const kind = FIELD_KINDS.find((k) => k === raw.kind) ?? "text";
  return { key: raw.key, label: str(raw.label) ?? raw.key, kind };
}

/**
 * First wins on a repeated `key` (D-I-4, LB-4).
 *
 * Both renderers key their `#each` block by the column's or field's key, and
 * Svelte throws `each_key_duplicate` out of the WHOLE flush on a repeat: a
 * producer that ships two columns called `id` would take the client's render
 * down, not just its own view. The keys are producer-chosen ids, so they are
 * not trustworthy — the same class the I8 fold fixed in `KeyValue.svelte` by
 * keying on the index.
 *
 * Deduped BEFORE the cap: a run of duplicates must not squeeze real columns
 * out of the first `MAX_VIEW_FIELDS`.
 */
function dedupByKey<T extends { key: string }>(items: readonly T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const item of items) {
    if (!seen.has(item.key)) {
      seen.add(item.key);
      out.push(item);
    }
  }
  return out;
}

/** Parse a `ViewSchema`; `null` when it is fundamentally malformed. */
export function parseViewSchema(raw: unknown): ViewSchema | null {
  if (!isObj(raw) || typeof raw.id !== "string" || typeof raw.version !== "number") {
    return null;
  }
  const shape = SHAPES.find((s) => s === raw.shape);
  if (!shape) {
    return null;
  }
  const columns = Array.isArray(raw.columns)
    ? dedupByKey(raw.columns.flatMap((c) => parseColumn(c) ?? [])).slice(0, MAX_VIEW_FIELDS)
    : [];
  const fields = Array.isArray(raw.fields)
    ? dedupByKey(raw.fields.flatMap((f) => parseField(f) ?? [])).slice(0, MAX_VIEW_FIELDS)
    : [];
  return {
    id: raw.id,
    version: raw.version,
    shape,
    columns,
    fields,
    actionKey: str(raw.actionKey),
  };
}

/** Parse a full generic-view payload; `null` when the schema does not
 * parse (nothing is renderable without it). */
export function parseGenericView(raw: unknown): GenericView | null {
  if (!isObj(raw)) {
    return null;
  }
  const schema = parseViewSchema(raw.schema);
  if (!schema) {
    return null;
  }
  const rows = Array.isArray(raw.rows)
    ? raw.rows.flatMap((r) => (isObj(r) ? [r] : [])).slice(0, MAX_VIEW_ROWS)
    : [];
  const record = isObj(raw.record) ? raw.record : null;
  return { schema, rows, record };
}

/** A cell's display text: a string as-is, a number or boolean stringified,
 * anything else (or absent) an empty cell. Never throws. */
export function cellText(row: Record<string, unknown>, key: string): string {
  const v = row[key];
  if (v === null || v === undefined) {
    return "";
  }
  if (typeof v === "string") {
    return v;
  }
  if (typeof v === "number" || typeof v === "boolean") {
    return String(v);
  }
  return "";
}

/** A row's Prefill line (D-I-18), read from the schema's `actionKey`;
 * `null` when the view declares no action or the field is empty. */
export function rowActionLine(row: Record<string, unknown>, schema: ViewSchema): string | null {
  if (schema.actionKey === null) {
    return null;
  }
  const line = str(row[schema.actionKey]);
  return line !== null && line.length > 0 ? line : null;
}
