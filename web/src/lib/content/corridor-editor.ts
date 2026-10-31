// Pure data models, entry filters, and JSON patchers for Cut CS.4 — Corridor
// (`lane/ceres_phobos.json`) & Relays (`relays/luna_ceres.json`,
// `relays/manufacture-v1.json`) Studio (doc 34 §5.2, D-34-6).
import type { JsonValue } from "../proto";

export type CorridorFilter =
  | "all"
  | "port"
  | "route"
  | "location"
  | "body"
  | "relay";

export interface CorridorEntrySummary {
  id: string;
  kind: string;
  label: string;
  subtitle: string;
  readOnly: boolean;
  def: Record<string, unknown>;
}

function asRecord(val: unknown): Record<string, unknown> {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : {};
}

export function extractCorridorEntries(
  doc: JsonValue | null,
): CorridorEntrySummary[] {
  const root = asRecord(doc);
  const entries = asRecord(root.entries);
  const out: CorridorEntrySummary[] = [];

  for (const [id, raw] of Object.entries(entries)) {
    const entry = asRecord(raw);
    const kind = typeof entry.kind === "string" ? entry.kind : "entry";
    const def = asRecord(entry.def);

    if (kind === "port") {
      const venue = typeof def.venue === "string" ? def.venue : id;
      const area = typeof def.area === "string" ? def.area : "";
      const vendors = Array.isArray(def.vendors) ? def.vendors.length : 0;
      const orgs = Array.isArray(def.orgs) ? def.orgs.length : 0;
      const hasSite = Boolean(def.site);
      out.push({
        id,
        kind,
        label: venue,
        subtitle: `${area} · ${orgs} orgs · ${vendors} vendors${hasSite ? " · mining site" : ""}`,
        readOnly: false,
        def,
      });
    } else if (kind === "route") {
      const origin = typeof def.origin === "string" ? def.origin : "?";
      const dest = typeof def.destination === "string" ? def.destination : "?";
      const durS =
        typeof def.duration_seconds === "number" ? def.duration_seconds : 0;
      const days = (durS / 86400).toFixed(1);
      out.push({
        id,
        kind,
        label: `${origin} → ${dest}`,
        subtitle: `${days} IC days (${durS} s)`,
        readOnly: false,
        def,
      });
    } else if (kind === "location") {
      const role = typeof def.role === "string" ? def.role : "place";
      const region = typeof def.region === "string" ? def.region : "sol";
      out.push({
        id,
        kind,
        label: id,
        subtitle: `role: ${role} · region: ${region}`,
        readOnly: false,
        def,
      });
    } else if (kind === "body") {
      const elements = Array.isArray(def.elements) ? def.elements.length : 0;
      out.push({
        id,
        kind,
        label: id,
        subtitle: `JPL Horizons ephemeris (${elements} epochs, read-only)`,
        readOnly: true,
        def,
      });
    } else {
      // Relay network (`network`), recipe (`private_pair`), or `region` (`sol`)
      const nodes = Object.keys(asRecord(def.nodes)).length;
      const pairs = Object.keys(asRecord(def.pairs)).length;
      const label =
        typeof def.label === "string"
          ? def.label
          : nodes > 0
            ? `Relay Network (${nodes} nodes, ${pairs} pairs)`
            : id;
      out.push({
        id,
        kind:
          nodes > 0 || kind === "recipe" || id === "network" ? "relay" : kind,
        label,
        subtitle: `kind: ${kind}`,
        readOnly: false,
        def,
      });
    }
  }
  return out;
}

/** Update a single entry's `def` inside `lane/ceres_phobos.json` or `relays/*.json`. */
export function updateCorridorEntryDef(
  doc: JsonValue,
  entryId: string,
  nextDef: Record<string, unknown>,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const entries = asRecord(clone.entries);
  const existing = asRecord(entries[entryId]);
  entries[entryId] = {
    ...existing,
    def: nextDef,
  };
  clone.entries = entries;
  return clone as JsonValue;
}
