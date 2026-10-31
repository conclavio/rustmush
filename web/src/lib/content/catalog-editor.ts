// Pure data models, ballistics curve preview, Dray / Dray-P stowage density
// calculator, and JSON patchers for Cut CS.3 — Components, Tools, Hardware,
// Goods & Resources Studio (doc 34 §5.1).
import type { JsonValue } from "../proto";

export type ComponentDocShape =
  | "rules_pack"
  | "goods_table"
  | "resource_table"
  | "tool_def"
  | "unknown";

export interface WeaponBandPreview {
  bandIndex: number;
  maxRangeM: number;
  dispersionMrad: number;
  dispersionRadiusM: number;
  flightTimeS: number;
}

export interface WeaponBallisticsSummary {
  maxRangeM: number;
  roundsPerMinute: number;
  bands: WeaponBandPreview[];
}

export interface HoldStowageSummary {
  drayMaxUnits: number;
  drayLimitingFactor: "mass (weighs out)" | "volume (cubes out)" | "incompatible hold" | "tank/locker";
  drayPMaxUnits: number;
  drayPLimitingFactor: "mass (weighs out)" | "volume (cubes out)" | "tank/locker";
}

export const DRAY_HOLD = {
  massKg: 100_000,
  volumeL: 120_000,
} as const;

export const DRAY_P_HOLD = {
  massKg: 86_000,
  volumeL: 103_200,
} as const;

function asRecord(val: unknown): Record<string, unknown> {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : {};
}

export function detectComponentDocShape(doc: JsonValue | null): ComponentDocShape {
  const root = asRecord(doc);
  if (root.manifest && root.entries) return "rules_pack";
  if (Array.isArray(root.goods) && Array.isArray(root.measures)) return "goods_table";
  if (Array.isArray(root.resources)) return "resource_table";
  if (typeof root.id === "string" && root.role) return "tool_def";
  return "unknown";
}

/** Compute weapon range bands, 1σ dispersion radius, flight time, and RPM. */
export function computeWeaponBallistics(weaponDef: JsonValue): WeaponBallisticsSummary {
  const def = asRecord(weaponDef);
  const cycleMs = Math.max(1000, typeof def.cycle_ms === "number" ? def.cycle_ms : 6000);
  const burst = Math.max(1, typeof def.burst_rounds === "number" ? def.burst_rounds : 1);
  const speedCms =
    typeof def.muzzle_speed_cms === "number" && def.muzzle_speed_cms > 0
      ? def.muzzle_speed_cms
      : 0;
  const rawBands = Array.isArray(def.bands) ? def.bands : [];

  const bands: WeaponBandPreview[] = rawBands.map((b, idx) => {
    const bo = asRecord(b);
    const maxRangeCm = typeof bo.max_range_cm === "number" ? bo.max_range_cm : 0;
    const dispersionMrad =
      typeof bo.dispersion_mrad === "number" ? bo.dispersion_mrad : 0;
    const maxRangeM = maxRangeCm / 100;
    const dispersionRadiusM = (maxRangeM * dispersionMrad) / 1000;
    const flightTimeS = speedCms > 0 ? maxRangeCm / speedCms : 0;
    return {
      bandIndex: idx + 1,
      maxRangeM,
      dispersionMrad,
      dispersionRadiusM: Number(dispersionRadiusM.toFixed(2)),
      flightTimeS: Number(flightTimeS.toFixed(2)),
    };
  });

  const maxRangeM = bands.length > 0 ? (bands[bands.length - 1]?.maxRangeM ?? 0) : 0;
  const roundsPerMinute = Number(((60_000 / cycleMs) * burst).toFixed(1));
  return { maxRangeM, roundsPerMinute, bands };
}

/**
 * Compute how many units of a good fit in the standard Dray (`100 t / 120 m³`, `stow.bulk`)
 * and the Dray-P (`86 t / 103.2 m³`, `stow.bulk` + `stow.ore`) and whether the good
 * weighs out or cubes out first.
 */
export function computeHoldStowage(
  massKg: number,
  volumeL: number,
  stowage: string,
): HoldStowageSummary {
  const safeMass = Math.max(1, massKg);
  const safeVol = Math.max(1, volumeL);

  if (stowage === "stow.propellant" || stowage === "stow.provisions") {
    return {
      drayMaxUnits: stowage === "stow.propellant" ? 80 : 120,
      drayLimitingFactor: "tank/locker",
      drayPMaxUnits: stowage === "stow.propellant" ? 80 : 120,
      drayPLimitingFactor: "tank/locker",
    };
  }

  const drayByMass = Math.floor(DRAY_HOLD.massKg / safeMass);
  const drayByVol = Math.floor(DRAY_HOLD.volumeL / safeVol);
  const drayCompatible = stowage === "stow.bulk";
  const drayMaxUnits = drayCompatible ? Math.min(drayByMass, drayByVol) : 0;
  const drayLimitingFactor = !drayCompatible
    ? "incompatible hold"
    : drayByMass <= drayByVol
      ? "mass (weighs out)"
      : "volume (cubes out)";

  const drayPByMass = Math.floor(DRAY_P_HOLD.massKg / safeMass);
  const drayPByVol = Math.floor(DRAY_P_HOLD.volumeL / safeVol);
  const drayPMaxUnits = Math.min(drayPByMass, drayPByVol);
  const drayPLimitingFactor =
    drayPByMass <= drayPByVol ? "mass (weighs out)" : "volume (cubes out)";

  return {
    drayMaxUnits,
    drayLimitingFactor,
    drayPMaxUnits,
    drayPLimitingFactor,
  };
}

/** Update a single entry's `def` inside a `rules_definition` pack document. */
export function updateRulesPackEntryDef(
  doc: JsonValue,
  entryId: string,
  nextDef: JsonValue,
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

/** Update a single good row inside `market/goods.json`. */
export function updateGoodRow(
  doc: JsonValue,
  goodId: string,
  patch: Record<string, unknown>,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const goods = Array.isArray(clone.goods) ? clone.goods : [];
  clone.goods = goods.map((item) => {
    const g = asRecord(item);
    if (g.id !== goodId) return item;
    return { ...g, ...patch };
  });
  return clone as JsonValue;
}
