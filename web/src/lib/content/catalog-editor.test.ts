import { describe, expect, it } from "vitest";
import {
  computeHoldStowage,
  computeWeaponBallistics,
  detectComponentDocShape,
} from "./catalog-editor";

describe("catalog-editor (Cut CS.3)", () => {
  it("detects rules_pack, goods_table, resource_table, and tool_def shapes", () => {
    expect(
      detectComponentDocShape({
        manifest: { id: "tacsim.pack.cannon_105" },
        entries: {},
      }),
    ).toBe("rules_pack");
    expect(
      detectComponentDocShape({
        version: 3,
        measures: [],
        goods: [],
      }),
    ).toBe("goods_table");
    expect(
      detectComponentDocShape({
        version: 1,
        resources: [],
      }),
    ).toBe("resource_table");
    expect(
      detectComponentDocShape({
        version: 1,
        id: "tacsim.tool.drill_core_mk1",
        role: { drill: {} },
      }),
    ).toBe("tool_def");
  });

  it("computes weapon ballistics bands, 1σ dispersion radius, flight time, and RPM", () => {
    const summary = computeWeaponBallistics({
      bands: [
        { max_range_cm: 150000, dispersion_mrad: 3 },
        { max_range_cm: 300000, dispersion_mrad: 6 },
      ],
      burst_rounds: 1,
      cycle_ms: 6000,
      muzzle_speed_cms: 100000,
    });
    expect(summary.maxRangeM).toBe(3000);
    expect(summary.roundsPerMinute).toBe(10);
    expect(summary.bands[0]).toEqual({
      bandIndex: 1,
      maxRangeM: 1500,
      dispersionMrad: 3,
      dispersionRadiusM: 4.5,
      flightTimeS: 1.5,
    });
    expect(summary.bands[1]).toEqual({
      bandIndex: 2,
      maxRangeM: 3000,
      dispersionMrad: 6,
      dispersionRadiusM: 18,
      flightTimeS: 3,
    });
  });

  it("computes Dray (100 t / 120 m³) and Dray-P (86 t / 103.2 m³) stowage limits accurately", () => {
    // Metals: 1000 kg, 400 L (weighs out at 100 t on Dray, 86 t on Dray-P)
    const metals = computeHoldStowage(1000, 400, "stow.bulk");
    expect(metals.drayMaxUnits).toBe(100);
    expect(metals.drayLimitingFactor).toBe("mass (weighs out)");
    expect(metals.drayPMaxUnits).toBe(86);

    // Instrument boards: 1000 kg, 2550 L (cubes out at 47 t on Dray, 40 t on Dray-P)
    const boards = computeHoldStowage(1000, 2550, "stow.bulk");
    expect(boards.drayMaxUnits).toBe(47);
    expect(boards.drayLimitingFactor).toBe("volume (cubes out)");
    expect(boards.drayPMaxUnits).toBe(40);
    expect(boards.drayPLimitingFactor).toBe("volume (cubes out)");

    // Raw ore (`stow.ore`): incompatible with standard Dray, fits Dray-P
    const ore = computeHoldStowage(1000, 600, "stow.ore");
    expect(ore.drayMaxUnits).toBe(0);
    expect(ore.drayLimitingFactor).toBe("incompatible hold");
    expect(ore.drayPMaxUnits).toBe(86);
  });
});
