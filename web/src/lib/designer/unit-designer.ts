// The staff unit designer's engine (doc 31 cut UD.2): pure, DOM-free data —
// the shipped catalogue (units, weapons, sensors, cores, looks, models) as
// templates, a draft (a unit's `def` and its look), the edits the studio
// makes, an undo/redo history, the saved draft, the test hit, and the two
// files a checked design exports. The rules are checked by the server
// (`tacsim::design`, the `unit_design` invoke) and the look by the picture
// module (`mush_pixel::design`); nothing here decides whether a design holds.

/** Any JSON value. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
/** A JSON object. */
export type JsonObject = { [key: string]: Json };

/** One shipped rules pack, as the files carry it. */
interface PackFile {
  manifest: { id: string; version: number };
  entries: Record<string, { kind: string; def: JsonObject }>;
}

/** One model in the catalogue's index (`crates/mush-pixel/models/index.json`). */
export interface ModelEntry {
  id: string;
  file: string;
  size_cm: [number, number];
  asset_pack: string;
  prefab: string;
}

/** A shipped unit offered as a starting point. */
export interface UnitTemplate {
  /** Its definition id (`tacsim.unit.line_tank`). */
  id: string;
  /** Its `def`. */
  def: JsonObject;
  /** The shipped look that draws it, if any. */
  look: JsonObject | null;
}

/** A named entry of the catalogue (a weapon, sensor or core). */
export interface CatalogueEntry {
  id: string;
  label: string;
}

const UNIT_FILES = import.meta.glob<PackFile>("./content/units/*.json", { eager: true, import: "default" });
const WEAPON_FILES = import.meta.glob<PackFile>("./content/weapons/*.json", { eager: true, import: "default" });
const SENSOR_FILES = import.meta.glob<PackFile>("./content/sensors/*.json", { eager: true, import: "default" });
const CORE_FILES = import.meta.glob<PackFile>("./content/cores/*.json", { eager: true, import: "default" });
const LOOK_FILES = import.meta.glob<JsonObject>("./content/looks/*.json", { eager: true, import: "default" });
const MODEL_FILES = import.meta.glob<{ models: ModelEntry[] }>("./content/models.json", {
  eager: true,
  import: "default",
});

/** The prefix a designed unit's id carries. */
export const UNIT_PREFIX = "tacsim.unit.";

function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

function entriesOf(files: Record<string, PackFile>, kind: string): CatalogueEntry[] {
  const out: CatalogueEntry[] = [];
  for (const pack of Object.values(files)) {
    for (const [id, entry] of Object.entries(pack.entries ?? {})) {
      if (entry.kind !== kind) continue;
      const label = typeof entry.def.label === "string" ? entry.def.label : id;
      out.push({ id, label });
    }
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** Every shipped unit, with the look that draws it, sorted by id. */
export function templates(): UnitTemplate[] {
  const looks = Object.values(LOOK_FILES);
  const out: UnitTemplate[] = [];
  for (const pack of Object.values(UNIT_FILES)) {
    for (const [id, entry] of Object.entries(pack.entries ?? {})) {
      if (entry.kind !== "unit") continue;
      const look = looks.find((l) => Array.isArray(l.units) && (l.units as Json[]).includes(id)) ?? null;
      out.push({ id, def: entry.def, look });
    }
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** The weapons a component may fit. */
export function weapons(): CatalogueEntry[] {
  return entriesOf(WEAPON_FILES, "weapon");
}
/** The sensors a component may fit. */
export function sensors(): CatalogueEntry[] {
  return entriesOf(SENSOR_FILES, "sensor");
}
/** The cores a component may fit. */
export function cores(): CatalogueEntry[] {
  return entriesOf(CORE_FILES, "core");
}
/** The model catalogue. */
export function models(): ModelEntry[] {
  return Object.values(MODEL_FILES)[0]?.models ?? [];
}

/** **A design in progress**: the unit's id and pack version, its rules (the
 * `def`) and its look. */
export interface Draft {
  id: string;
  version: number;
  rules: JsonObject;
  look: JsonObject;
}

/** `name` as a designed unit's id slug: lower case, `_` for anything else. */
export function slugOf(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

/** **A new draft from `template`**, named `name`: its rules copied, its look
 * copied (or a bare one on the template's model-less size) and pointed at
 * the new id. */
export function draftFrom(template: UnitTemplate, name: string): Draft {
  const id = `${UNIT_PREFIX}${slugOf(name) || "design"}`;
  const rules = clone(template.def);
  rules.label = name;
  const look: JsonObject = template.look
    ? clone(template.look)
    : { units: [], label: name, short: "??", size_cm: [500, 250] };
  look.units = [id];
  look.label = name;
  return { id, version: 1, rules, look };
}

function sectionsOf(d: Draft): JsonObject[] {
  return Array.isArray(d.rules.sections) ? (d.rules.sections as JsonObject[]) : [];
}

function componentsOf(d: Draft): JsonObject[] {
  return Array.isArray(d.rules.components) ? (d.rules.components as JsonObject[]) : [];
}

/** The draft with its id renamed from `name` (rules label and look kept). */
export function rename(d: Draft, name: string): Draft {
  const n = clone(d);
  n.id = `${UNIT_PREFIX}${slugOf(name) || "design"}`;
  n.look.units = [n.id];
  return n;
}

/** The draft with its label (rules and look) set. */
export function setLabel(d: Draft, label: string): Draft {
  const n = clone(d);
  n.rules.label = label;
  n.look.label = label;
  return n;
}

/** The draft with its look's short name set. */
export function setShort(d: Draft, short: string): Draft {
  const n = clone(d);
  n.look.short = short;
  return n;
}

/** The draft drawn from model `model`, its look's `size_cm` the model's
 * height with the look's own width kept within the matrix check's bounds. */
export function setModel(d: Draft, model: ModelEntry): Draft {
  const n = clone(d);
  n.look.model = model.id;
  const [w] = Array.isArray(n.look.size_cm) ? (n.look.size_cm as number[]) : [model.size_cm[0]];
  const [mw, mh] = model.size_cm;
  const width = Math.min(Math.max(w ?? mw, Math.ceil(mw / 2)), mw * 2);
  n.look.size_cm = [width, mh];
  return n;
}

/** The draft with facet `facet` of section `section` at `armor`. */
export function setFacetArmor(d: Draft, section: string, facet: string, armor: number): Draft {
  const n = clone(d);
  for (const s of sectionsOf(n)) {
    if (s.id !== section || !Array.isArray(s.facets)) continue;
    for (const f of s.facets as JsonObject[]) if (f.id === facet) f.armor = Math.max(0, Math.round(armor));
  }
  return n;
}

/** The draft with section `section`'s internals at `internals`. */
export function setInternals(d: Draft, section: string, internals: number): Draft {
  const n = clone(d);
  for (const s of sectionsOf(n)) if (s.id === section) s.internals = Math.max(0, Math.round(internals));
  return n;
}

/** The draft with weapon component `component` fitting `weapon`. */
export function setWeapon(d: Draft, component: string, weapon: string): Draft {
  const n = clone(d);
  for (const c of componentsOf(n)) {
    const role = c.role as JsonObject | undefined;
    if (c.id === component && role?.kind === "weapon") role.weapon = weapon;
  }
  return n;
}

/** The draft with weapon component `component`'s short name `short` on its
 * look (empty removes it). */
export function setWeaponName(d: Draft, component: string, short: string): Draft {
  const n = clone(d);
  const names = (n.look.weapons as JsonObject | undefined) ?? {};
  if (short.trim() === "") delete names[component];
  else names[component] = short;
  n.look.weapons = names;
  return n;
}

/** The draft with its mobility's top speed at `cms` (cm/s). */
export function setMaxSpeed(d: Draft, cms: number): Draft {
  const n = clone(d);
  const mobility = n.rules.mobility as JsonObject | undefined;
  if (mobility) {
    for (const v of Object.values(mobility)) {
      if (v && typeof v === "object" && !Array.isArray(v) && "max_speed_cms" in v) {
        (v as JsonObject).max_speed_cms = Math.max(0, Math.round(cms));
      }
    }
  }
  return n;
}

/** The draft's doll rows replaced by `rows` (one string a row). */
export function setDollRows(d: Draft, rows: string[]): Draft {
  const n = clone(d);
  const doll = (n.look.doll as JsonObject | undefined) ?? null;
  if (doll) doll.rows = rows;
  return n;
}

/** Every weapon component: its id, section and weapon. */
export function weaponComponents(d: Draft): { id: string; section: string; weapon: string }[] {
  return componentsOf(d)
    .filter((c) => (c.role as JsonObject | undefined)?.kind === "weapon")
    .map((c) => ({
      id: String(c.id),
      section: String(c.section),
      weapon: String((c.role as JsonObject).weapon ?? ""),
    }));
}

/** Every section: its id, internals and facets' armour. */
export function sections(d: Draft): { id: string; internals: number; facets: { id: string; armor: number }[] }[] {
  return sectionsOf(d).map((s) => ({
    id: String(s.id),
    internals: Number(s.internals ?? 0),
    facets: (Array.isArray(s.facets) ? (s.facets as JsonObject[]) : []).map((f) => ({
      id: String(f.id),
      armor: Number(f.armor ?? 0),
    })),
  }));
}

/** **The test hit** (doc 31 §6): what destroying section `section` would
 * cost, in words, from the section's `consequences` — the check the server
 * cannot give, so an authoring mistake shows before a match finds it. */
export function testHit(d: Draft, section: string): string[] {
  const s = sectionsOf(d).find((x) => x.id === section);
  if (!s) return [];
  const out: string[] = [];
  for (const c of (Array.isArray(s.consequences) ? s.consequences : []) as JsonObject[]) {
    switch (c.kind) {
      case "destroy_unit":
        out.push("the unit is destroyed");
        break;
      case "remove_weapons": {
        const lost = weaponComponents(d).filter((w) => w.section === c.section);
        out.push(
          `every weapon on ${String(c.section)} is lost${lost.length ? ` (${lost.map((w) => w.id).join(", ")})` : " (none fitted there)"}`,
        );
        break;
      }
      case "slow":
        out.push(`top speed falls ${String(c.percent)} %`);
        break;
      case "no_turn_toward":
        out.push(`it cannot turn toward its ${String(c.side)}`);
        break;
      case "disable_motion":
        out.push("it cannot move");
        break;
      case "disable_motion_when_all_of":
        out.push(`it cannot move once ${(c.sections as Json[]).join(" and ")} are all gone`);
        break;
      case "degrade_core":
        out.push("its core degrades");
        break;
      case "destroy_core":
        out.push("its core is destroyed");
        break;
      default:
        out.push(String(c.kind));
    }
  }
  return out;
}

/** Numbers to set beside a template's (doc 31 §6's compare pane): armour by
 * section, weapons, top speed. Not a balance verdict. */
export function figures(d: Draft): { armour: number; internals: number; weapons: string[]; speedCms: number | null } {
  let armour = 0;
  let internals = 0;
  for (const s of sections(d)) {
    internals += s.internals;
    for (const f of s.facets) armour += f.armor;
  }
  let speed: number | null = null;
  const mobility = d.rules.mobility as JsonObject | undefined;
  for (const v of Object.values(mobility ?? {})) {
    if (v && typeof v === "object" && !Array.isArray(v) && typeof (v as JsonObject).max_speed_cms === "number") {
      speed = (v as JsonObject).max_speed_cms as number;
    }
  }
  return { armour, internals, weapons: weaponComponents(d).map((w) => w.weapon), speedCms: speed };
}

/** **A bounded undo/redo history** of drafts, the map studio's habit. */
export class History {
  #past: Draft[] = [];
  #future: Draft[] = [];
  constructor(readonly limit = 100) {}

  /** Remember `before`, the draft an edit replaced; the redo branch goes. */
  push(before: Draft): void {
    this.#past.push(clone(before));
    if (this.#past.length > this.limit) this.#past.shift();
    this.#future = [];
  }

  /** The draft before `current`, or `null`. */
  undo(current: Draft): Draft | null {
    const prev = this.#past.pop();
    if (!prev) return null;
    this.#future.push(clone(current));
    return prev;
  }

  /** The draft an undo left, or `null`. */
  redo(current: Draft): Draft | null {
    const next = this.#future.pop();
    if (!next) return null;
    this.#past.push(clone(current));
    return next;
  }

  get canUndo(): boolean {
    return this.#past.length > 0;
  }
  get canRedo(): boolean {
    return this.#future.length > 0;
  }
}

/** The `localStorage` key the working draft is kept under. */
export const DRAFT_KEY = "mush.unitDesigner.draft";

/** Keep `d` as the working draft (a closed tab costs nothing); storage that
 * refuses is ignored. */
export function saveDraft(storage: Pick<Storage, "setItem"> | null, d: Draft): void {
  try {
    storage?.setItem(DRAFT_KEY, JSON.stringify(d));
  } catch {
    // Private mode or a full store: the draft lives on in the page.
  }
}

/** The working draft kept by {@link saveDraft}, or `null`. */
export function loadDraft(storage: Pick<Storage, "getItem"> | null): Draft | null {
  try {
    const text = storage?.getItem(DRAFT_KEY);
    if (!text) return null;
    const v = JSON.parse(text) as Draft;
    if (typeof v.id !== "string" || typeof v.version !== "number" || !v.rules || !v.look) return null;
    return v;
  } catch {
    return null;
  }
}

/** One exported file. */
export interface ExportFile {
  /** Where it goes in the tree. */
  path: string;
  /** Its text. */
  text: string;
}

/** **The two files a checked design exports** (doc 31 D-31-8): the server's
 * finished rules pack for `crates/tacsim/content/units/` and the look for
 * `crates/mush-pixel/looks/`, as the tree formats JSON (two spaces, a
 * newline at the end). */
export function exportFiles(d: Draft, pack: Json): ExportFile[] {
  const slug = d.id.startsWith(UNIT_PREFIX) ? d.id.slice(UNIT_PREFIX.length) : slugOf(d.id);
  return [
    { path: `crates/tacsim/content/units/${slug}.json`, text: `${JSON.stringify(pack, null, 2)}\n` },
    { path: `crates/mush-pixel/looks/${slug}.json`, text: `${JSON.stringify(d.look, null, 2)}\n` },
  ];
}
