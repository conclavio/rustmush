// DOM-free structured editor and validator for `space/hulls.json`
// (`SF.16`, `SF.19` — canonical space hulls and slot-stable refit packages).
import type { JsonValue } from "../proto";

export interface SpaceHullSpecDraft {
  id: string;
  short_id: string;
  label: string;
  refit_family: string;
  pixel_model: string;
  dry_mass_t: number;
  max_accel_mps2: number;
  propellant_kg: number;
  delta_v_mps: number;
  return_reserve_dv_mps: number;
  cooling_rate_permille_s: number;
  hit_radius_m: number;
  decoys: number;
  hull_armor: [number, number, number, number, number];
  hull_internals: number;
  hold_internals: number;
  drive_internals: number;
  torpedo_tubes: number;
  torpedo_magazine: number;
  laser_turrets: number;
  railgun_rounds: number;
  pdc_mounts: number;
  pdc_ammo: number;
}

export interface SpaceRefitPackageDraft {
  id: string;
  label: string;
  refit_family: string;
  target_hull_id: string;
  cost_cr: number;
  work_hours: number;
  add_delta_v_mps: number;
  add_propellant_kg: number;
  add_laser_turrets: number;
  add_torpedo_magazine: number;
  add_railgun_rounds: number;
  add_pdc_ammo: number;
  add_decoys: number;
  add_cooling_permille_s: number;
}

export interface SpaceHullsDocDraft {
  schema_id: string;
  version: number;
  hulls: SpaceHullSpecDraft[];
  refit_packages: SpaceRefitPackageDraft[];
}

export const ARMOR_FACET_LABELS = [
  "Front",
  "Left",
  "Right",
  "Rear",
  "Top",
] as const;

export const PIXEL_SHIP_MODELS = [
  "freighter",
  "corvette",
  "raider",
  "drone",
  "torpedo",
  "asteroid",
  "derelict",
  "shipyard",
] as const;

function asInt(val: unknown, fallback: number): number {
  return typeof val === "number" && Number.isFinite(val) ? Math.trunc(val) : fallback;
}

function asStr(val: unknown, fallback: string): string {
  return typeof val === "string" ? val : fallback;
}

function parseHullArmor(raw: unknown): [number, number, number, number, number] {
  if (!Array.isArray(raw) || raw.length < 5) {
    return [120, 80, 80, 60, 60];
  }
  return [
    asInt(raw[0], 120),
    asInt(raw[1], 80),
    asInt(raw[2], 80),
    asInt(raw[3], 60),
    asInt(raw[4], 60),
  ];
}

export function parseSpaceHullsDoc(doc: JsonValue | null): SpaceHullsDocDraft {
  if (!doc || typeof doc !== "object" || Array.isArray(doc)) {
    return {
      schema_id: "tacsim.space.hulls",
      version: 1,
      hulls: [],
      refit_packages: [],
    };
  }
  const obj = doc as Record<string, unknown>;
  const schemaObj =
    obj.schema && typeof obj.schema === "object" && !Array.isArray(obj.schema)
      ? (obj.schema as Record<string, unknown>)
      : {};
  const hullsRaw = Array.isArray(obj.hulls) ? obj.hulls : [];
  const pkgsRaw = Array.isArray(obj.refit_packages) ? obj.refit_packages : [];

  const hulls: SpaceHullSpecDraft[] = hullsRaw
    .filter((h): h is Record<string, unknown> => Boolean(h && typeof h === "object"))
    .map((h) => ({
      id: asStr(h.id, "tacsim.unit.new_hull"),
      short_id: asStr(h.short_id, "new_hull"),
      label: asStr(h.label, "New Space Hull"),
      refit_family: asStr(h.refit_family, "dray_hull"),
      pixel_model: asStr(h.pixel_model, "freighter"),
      dry_mass_t: asInt(h.dry_mass_t, 240),
      max_accel_mps2: asInt(h.max_accel_mps2, 25),
      propellant_kg: asInt(h.propellant_kg, 80000),
      delta_v_mps: asInt(h.delta_v_mps, 4500),
      return_reserve_dv_mps: asInt(h.return_reserve_dv_mps, 900),
      cooling_rate_permille_s: asInt(h.cooling_rate_permille_s, 30),
      hit_radius_m: asInt(h.hit_radius_m, 60),
      decoys: asInt(h.decoys, 4),
      hull_armor: parseHullArmor(h.hull_armor),
      hull_internals: asInt(h.hull_internals, 600),
      hold_internals: asInt(h.hold_internals, 400),
      drive_internals: asInt(h.drive_internals, 250),
      torpedo_tubes: asInt(h.torpedo_tubes, 0),
      torpedo_magazine: asInt(h.torpedo_magazine, 0),
      laser_turrets: asInt(h.laser_turrets, 1),
      railgun_rounds: asInt(h.railgun_rounds, 0),
      pdc_mounts: asInt(h.pdc_mounts, 1),
      pdc_ammo: asInt(h.pdc_ammo, 2000),
    }));

  const refit_packages: SpaceRefitPackageDraft[] = pkgsRaw
    .filter((p): p is Record<string, unknown> => Boolean(p && typeof p === "object"))
    .map((p) => ({
      id: asStr(p.id, "refit.new_package"),
      label: asStr(p.label, "New Refit Package"),
      refit_family: asStr(p.refit_family, "dray_hull"),
      target_hull_id: asStr(p.target_hull_id, "tacsim.unit.dray"),
      cost_cr: asInt(p.cost_cr, 18500),
      work_hours: asInt(p.work_hours, 12),
      add_delta_v_mps: asInt(p.add_delta_v_mps, 0),
      add_propellant_kg: asInt(p.add_propellant_kg, 0),
      add_laser_turrets: asInt(p.add_laser_turrets, 0),
      add_torpedo_magazine: asInt(p.add_torpedo_magazine, 0),
      add_railgun_rounds: asInt(p.add_railgun_rounds, 0),
      add_pdc_ammo: asInt(p.add_pdc_ammo, 0),
      add_decoys: asInt(p.add_decoys, 0),
      add_cooling_permille_s: asInt(p.add_cooling_permille_s, 0),
    }));

  return {
    schema_id: asStr(schemaObj.id, "tacsim.space.hulls"),
    version: asInt(schemaObj.version ?? obj.version, 1),
    hulls,
    refit_packages,
  };
}

export function serializeSpaceHullsDoc(draft: SpaceHullsDocDraft): JsonValue {
  return {
    schema: {
      id: draft.schema_id,
      version: draft.version,
    },
    hulls: draft.hulls.map((h) => ({
      id: h.id,
      short_id: h.short_id,
      label: h.label,
      refit_family: h.refit_family,
      pixel_model: h.pixel_model,
      dry_mass_t: h.dry_mass_t,
      max_accel_mps2: h.max_accel_mps2,
      propellant_kg: h.propellant_kg,
      delta_v_mps: h.delta_v_mps,
      return_reserve_dv_mps: h.return_reserve_dv_mps,
      cooling_rate_permille_s: h.cooling_rate_permille_s,
      hit_radius_m: h.hit_radius_m,
      decoys: h.decoys,
      hull_armor: [...h.hull_armor],
      hull_internals: h.hull_internals,
      hold_internals: h.hold_internals,
      drive_internals: h.drive_internals,
      torpedo_tubes: h.torpedo_tubes,
      torpedo_magazine: h.torpedo_magazine,
      laser_turrets: h.laser_turrets,
      railgun_rounds: h.railgun_rounds,
      pdc_mounts: h.pdc_mounts,
      pdc_ammo: h.pdc_ammo,
    })),
    refit_packages: draft.refit_packages.map((p) => ({
      id: p.id,
      label: p.label,
      refit_family: p.refit_family,
      target_hull_id: p.target_hull_id,
      cost_cr: p.cost_cr,
      work_hours: p.work_hours,
      add_delta_v_mps: p.add_delta_v_mps,
      add_propellant_kg: p.add_propellant_kg,
      add_laser_turrets: p.add_laser_turrets,
      add_torpedo_magazine: p.add_torpedo_magazine,
      add_railgun_rounds: p.add_railgun_rounds,
      add_pdc_ammo: p.add_pdc_ammo,
      add_decoys: p.add_decoys,
      add_cooling_permille_s: p.add_cooling_permille_s,
    })),
  };
}

export function updateHullSpec(
  doc: JsonValue | null,
  index: number,
  patch: Partial<SpaceHullSpecDraft>,
): JsonValue {
  const parsed = parseSpaceHullsDoc(doc);
  if (index < 0 || index >= parsed.hulls.length) {
    return serializeSpaceHullsDoc(parsed);
  }
  const current = parsed.hulls[index]!;
  parsed.hulls[index] = {
    ...current,
    ...patch,
    hull_armor: patch.hull_armor ? [...patch.hull_armor] : [...current.hull_armor],
  };
  return serializeSpaceHullsDoc(parsed);
}

export function updateRefitPackage(
  doc: JsonValue | null,
  index: number,
  patch: Partial<SpaceRefitPackageDraft>,
): JsonValue {
  const parsed = parseSpaceHullsDoc(doc);
  if (index < 0 || index >= parsed.refit_packages.length) {
    return serializeSpaceHullsDoc(parsed);
  }
  const current = parsed.refit_packages[index]!;
  parsed.refit_packages[index] = { ...current, ...patch };
  return serializeSpaceHullsDoc(parsed);
}

/**
 * Validate `space/hulls.json` invariants locally before Check & Sign:
 * - Non-empty unique hull IDs and short_ids
 * - Positive dry mass, fuel capacity, thrust, and delta-v
 * - Every refit package references a known `target_hull_id` with matching `refit_family`
 */
export function validateSpaceHullsDoc(doc: JsonValue | null): string[] {
  const parsed = parseSpaceHullsDoc(doc);
  const issues: string[] = [];
  const hullById = new Map<string, SpaceHullSpecDraft>();
  const shortIds = new Set<string>();

  for (const h of parsed.hulls) {
    if (!h.id.trim()) {
      issues.push("Hull has an empty canonical id.");
    } else if (hullById.has(h.id)) {
      issues.push(`Duplicate hull id '${h.id}'.`);
    } else {
      hullById.set(h.id, h);
    }
    if (!h.short_id.trim()) {
      issues.push(`Hull '${h.id}' has an empty short_id.`);
    } else if (shortIds.has(h.short_id)) {
      issues.push(`Duplicate hull short_id '${h.short_id}'.`);
    } else {
      shortIds.add(h.short_id);
    }
    if (h.dry_mass_t <= 0) {
      issues.push(`Hull '${h.short_id}' must have dry_mass_t > 0.`);
    }
    if (h.delta_v_mps <= 0) {
      issues.push(`Hull '${h.short_id}' must have delta_v_mps > 0.`);
    }
  }

  const pkgIds = new Set<string>();
  for (const p of parsed.refit_packages) {
    if (!p.id.trim()) {
      issues.push("Refit package has an empty id.");
    } else if (pkgIds.has(p.id)) {
      issues.push(`Duplicate refit package id '${p.id}'.`);
    } else {
      pkgIds.add(p.id);
    }
    const target = hullById.get(p.target_hull_id);
    if (!target) {
      issues.push(
        `Refit package '${p.id}' references unknown target_hull_id '${p.target_hull_id}'.`,
      );
    } else if (target.refit_family !== p.refit_family) {
      issues.push(
        `Refit package '${p.id}' family '${p.refit_family}' does not match target hull '${target.id}' family '${target.refit_family}'.`,
      );
    }
  }

  return issues;
}
