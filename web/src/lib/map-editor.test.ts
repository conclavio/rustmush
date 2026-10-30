import { describe, expect, it } from "vitest";
import {
  applyBrushDab,
  applyFloodFill,
  applyLineStroke,
  applyRampStroke,
  applyRectStroke,
  brushFootprint,
  decodeTileCellsBase64,
  EDITOR_TERRAINS,
  editorToPictureFeeds,
  encodeTileCellsBase64,
  enforceLocalGradeLimit,
  exportArenaDefJson,
  generatePresetMap,
  importArenaDefJson,
  terrainByCode,
} from "./map-editor";

describe("map-editor core", () => {
  it("provides all 18 mush_atlas TerrainClass entries in order", () => {
    expect(EDITOR_TERRAINS).toHaveLength(18);
    for (let i = 0; i < 18; i++) {
      expect(EDITOR_TERRAINS[i]!.code).toBe(i);
      expect(terrainByCode(i).code).toBe(i);
    }
  });

  it("computes brush footprints for circle, square, and diamond shapes", () => {
    const single = brushFootprint(5, 5, 1, "circle", 16, 16);
    expect(single).toEqual([{ x: 5, y: 5, weight: 1 }]);

    const square = brushFootprint(4, 4, 2, "square", 16, 16);
    // radius 2 -> span = 1 -> 3x3 = 9 cells
    expect(square).toHaveLength(9);

    const diamond = brushFootprint(4, 4, 2, "diamond", 16, 16);
    // Manhattan distance <= 1 -> center + 4 cardinal neighbors = 5 cells
    expect(diamond).toHaveLength(5);

    const clipped = brushFootprint(0, 0, 2, "square", 16, 16);
    expect(clipped).toHaveLength(4);
  });

  it("paints terrain and sculpts elevation while clamping adjacent grade to <= 45 cm", () => {
    const state = generatePresetMap("blank", 16, 16, 42);
    // Paint rocks (code 9) with radius 2 square
    applyBrushDab(state, 8, 8, {
      tool: "paint",
      shape: "square",
      radius: 2,
      terrainCode: 9,
      densityPct: 100,
      elevStepCm: 35,
      targetElevCm: 0,
    });
    expect(state.classes[8 * 16 + 8]).toBe(9);
    expect(state.classes[7 * 16 + 7]).toBe(9);

    // Raise elevation repeatedly at (8, 8) and verify grade limit holds across neighbors
    for (let i = 0; i < 6; i++) {
      applyBrushDab(state, 8, 8, {
        tool: "raise",
        shape: "circle",
        radius: 3,
        terrainCode: 9,
        densityPct: 100,
        elevStepCm: 45,
        targetElevCm: 0,
      });
    }
    expect(state.elevCm[8 * 16 + 8]).toBeGreaterThan(80);

    // Verify all 4-connected neighbors across the map differ by at most 45 cm
    for (let y = 0; y < state.height; y++) {
      for (let x = 0; x < state.width; x++) {
        const here = state.elevCm[y * state.width + x]!;
        if (x + 1 < state.width) {
          expect(Math.abs(here - state.elevCm[y * state.width + (x + 1)]!)).toBeLessThanOrEqual(45);
        }
        if (y + 1 < state.height) {
          expect(Math.abs(here - state.elevCm[(y + 1) * state.width + x]!)).toBeLessThanOrEqual(45);
        }
      }
    }

    // Flatten back toward 0 cm
    applyBrushDab(state, 8, 8, {
      tool: "flatten",
      shape: "square",
      radius: 1,
      terrainCode: 9,
      densityPct: 100,
      elevStepCm: 45,
      targetElevCm: 0,
    });
    expect(state.elevCm[8 * 16 + 8]).toBeLessThanOrEqual(45);
  });

  it("supports line, ramp, rectangle, and flood-fill strokes", () => {
    const state = generatePresetMap("blank", 16, 16, 42);
    // Rectangle outline of walls (code 10) from (2,2) to (6,6)
    applyRectStroke(state, 2, 2, 6, 6, 10, true);
    expect(state.classes[2 * 16 + 2]).toBe(10);
    expect(state.classes[4 * 16 + 2]).toBe(10);
    expect(state.classes[4 * 16 + 4]).toBe(3); // inside remains grass

    // Flood fill inside the walled box with floor (code 5)
    applyFloodFill(state, 4, 4, 5);
    expect(state.classes[4 * 16 + 4]).toBe(5);
    expect(state.classes[3 * 16 + 3]).toBe(5);
    expect(state.classes[1 * 16 + 1]).toBe(3); // outside untouched

    // Line stroke of road (code 7)
    applyLineStroke(state, 0, 10, 15, 10, {
      tool: "line",
      shape: "circle",
      radius: 1,
      terrainCode: 7,
      densityPct: 100,
      elevStepCm: 35,
      targetElevCm: 0,
    });
    expect(state.classes[10 * 16 + 0]).toBe(7);
    expect(state.classes[10 * 16 + 8]).toBe(7);
    expect(state.classes[10 * 16 + 15]).toBe(7);

    // Ramp stroke between (2, 12) at 0 cm and (12, 12) raised first
    state.elevCm[12 * 16 + 12] = 180;
    enforceLocalGradeLimit(state, 0, 0, 15, 15);
    applyRampStroke(state, 2, 12, 12, 12, 2, 7);
    expect(state.elevCm[12 * 16 + 7]!).toBeGreaterThan(state.elevCm[12 * 16 + 2]!);
    expect(state.elevCm[12 * 16 + 12]!).toBeGreaterThan(state.elevCm[12 * 16 + 7]!);
  });

  it("round-trips mush_atlas Cells RLE base64 and ArenaDef JSON", () => {
    const state = generatePresetMap("mars_canyon", 32, 32, 42);
    const b64 = encodeTileCellsBase64(state);
    expect(b64.length).toBeGreaterThan(8);

    const decoded = decodeTileCellsBase64(b64);
    expect(decoded).not.toBeNull();
    expect(decoded!.width).toBe(32);
    expect(decoded!.height).toBe(32);
    expect(Array.from(decoded!.classes)).toEqual(Array.from(state.classes));
    expect(Array.from(decoded!.elevCm)).toEqual(Array.from(state.elevCm));

    const json = exportArenaDefJson(state);
    const reimported = importArenaDefJson(json);
    expect(reimported).not.toBeNull();
    expect(reimported!.label).toBe(state.label);
    expect(reimported!.biome).toBe("mars");
    expect(reimported!.width).toBe(32);
    expect(reimported!.height).toBe(32);
    expect(Array.from(reimported!.classes)).toEqual(Array.from(state.classes));
    expect(Array.from(reimported!.elevCm)).toEqual(Array.from(state.elevCm));
    expect(reimported!.spots.length).toBe(state.spots.length);
  });

  it("synthesizes arena.map and arena.cockpit feeds for the live 3D/2.5D preview", () => {
    const state = generatePresetMap("proving_ground", 32, 32, 42);
    const { map, cockpit } = editorToPictureFeeds(state);
    expect(map.biome).toBe(state.biome);
    expect(map.rows).toHaveLength(32);
    expect(map.elev_field).toHaveLength(32);
    expect((map.elev_field as string[])[0]).toHaveLength(32);
    expect(cockpit.cell).toEqual([state.camera.x, state.camera.y]);
    expect((cockpit.contacts as unknown[]).length).toBeGreaterThanOrEqual(2);
  });
});
