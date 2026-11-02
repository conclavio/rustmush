import { describe, expect, it } from "vitest";
import {
  parseSpaceHullsDoc,
  updateHullSpec,
  updateRefitPackage,
  validateSpaceHullsDoc,
} from "./space-editor";

const SAMPLE_DOC = {
  schema: {
    id: "tacsim.space.hulls",
    version: 1,
  },
  hulls: [
    {
      id: "tacsim.unit.dray",
      short_id: "dray",
      label: "Dray-class Bulk Hauler",
      refit_family: "dray_hull",
      pixel_model: "freighter",
      dry_mass_t: 240,
      max_accel_mps2: 25,
      propellant_kg: 80000,
      delta_v_mps: 4500,
      return_reserve_dv_mps: 900,
      cooling_rate_permille_s: 30,
      hit_radius_m: 60,
      decoys: 4,
      hull_armor: [120, 80, 80, 60, 60],
      hull_internals: 600,
      hold_internals: 400,
      drive_internals: 250,
      torpedo_tubes: 0,
      torpedo_magazine: 0,
      laser_turrets: 1,
      railgun_rounds: 0,
      pdc_mounts: 1,
      pdc_ammo: 2000,
    },
    {
      id: "tacsim.unit.dray_p",
      short_id: "dray_p",
      label: "Dray-P Orbital Prospector",
      refit_family: "dray_hull",
      pixel_model: "freighter",
      dry_mass_t: 260,
      max_accel_mps2: 24,
      propellant_kg: 85000,
      delta_v_mps: 4800,
      return_reserve_dv_mps: 900,
      cooling_rate_permille_s: 35,
      hit_radius_m: 62,
      decoys: 4,
      hull_armor: [130, 85, 85, 65, 65],
      hull_internals: 620,
      hold_internals: 450,
      drive_internals: 260,
      torpedo_tubes: 0,
      torpedo_magazine: 0,
      laser_turrets: 2,
      railgun_rounds: 0,
      pdc_mounts: 1,
      pdc_ammo: 2000,
    },
  ],
  refit_packages: [
    {
      id: "refit.dray.prospector",
      label: "Dray-P Orbital Prospector Conversion",
      refit_family: "dray_hull",
      target_hull_id: "tacsim.unit.dray_p",
      cost_cr: 18500,
      work_hours: 12,
      add_delta_v_mps: 300,
      add_propellant_kg: 5000,
      add_laser_turrets: 1,
      add_torpedo_magazine: 0,
      add_railgun_rounds: 0,
      add_pdc_ammo: 0,
      add_decoys: 0,
      add_cooling_permille_s: 5,
    },
  ],
};

describe("Content Studio Space & Hulls editor (SF.19)", () => {
  it("parses and updates canonical space hulls and 5-facet armor", () => {
    const parsed = parseSpaceHullsDoc(SAMPLE_DOC);
    expect(parsed.hulls).toHaveLength(2);
    expect(parsed.hulls[0]?.short_id).toBe("dray");
    expect(parsed.hulls[1]?.laser_turrets).toBe(2);

    const updated = updateHullSpec(SAMPLE_DOC, 0, {
      delta_v_mps: 5200,
      hull_armor: [140, 80, 80, 60, 60],
    });
    const reparsed = parseSpaceHullsDoc(updated);
    expect(reparsed.hulls[0]?.delta_v_mps).toBe(5200);
    expect(reparsed.hulls[0]?.hull_armor[0]).toBe(140);
    expect(validateSpaceHullsDoc(updated)).toEqual([]);
  });

  it("updates refit packages and flags slot-stable family mismatches", () => {
    const bad = updateRefitPackage(SAMPLE_DOC, 0, {
      refit_family: "frigate_hull",
    });
    const issues = validateSpaceHullsDoc(bad);
    expect(issues).toHaveLength(1);
    expect(issues[0]).toContain("does not match target hull");
  });
});
