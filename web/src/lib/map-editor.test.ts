import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { FEED_COCKPIT, FEED_MAP, KIND_COCKPIT, PixelModule } from "./pixel/wasm";
import {
  applySculptBrush,
  checkEditorMap,
  computePassabilityOverlay,
  computeViewshed,
  createEditorMap,
  decodeClassesBase64,
  decodeElevLe16Base64,
  despeckleEditorMap,
  EditorUndoStack,
  editorToPictureFeeds,
  encodeClassesBase64,
  encodeElevLe16Base64,
  encodeElevZigzagDelta,
  exportGroundDraftJson,
  floodFillTerrain,
  type GroundDraftJson,
  importGroundDraftJson,
  markStudioEdited,
  paintTerrainBrush,
  paintTerrainLine,
  relaxEditorSlopes,
  routeEditorMap,
  TERRAIN_PALETTE,
} from "./map-editor";
import pgParityFixture from "./fixtures/proving_ground_wg4_parity.json";

function loadShippedProvingGroundDraft(): GroundDraftJson {
  const draftPath = resolve(
    __dirname,
    "../../../crates/tacsim/content/ground/proving_ground.draft.json",
  );
  return JSON.parse(readFileSync(draftPath, "utf8")) as GroundDraftJson;
}

describe("map-editor WG.4 (ground.draft.v1 & parity with tacsim::ground)", () => {
  it("provides all 17 paintable mush_atlas TerrainClass codes (1..=17, omitting Void 0) and rejects Void cells in checkEditorMap", () => {
    expect(TERRAIN_PALETTE).toHaveLength(17);
    const codes = TERRAIN_PALETTE.map((s) => s.code).sort((a, b) => a - b);
    expect(codes).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);

    const state = createEditorMap(64, 64, "temperate", 3);
    state.classes[10] = 0;
    const report = checkEditorMap(state);
    expect(report.passed).toBe(false);
    expect(report.failures.some((f) => f.includes("Void cell"))).toBe(true);
  });

  it("updates provenance to studio v1.0 and preserves prior generator/pack provenance in inputs on edit", () => {
    const rawDraft = loadShippedProvingGroundDraft();
    const state = importGroundDraftJson(rawDraft);
    expect(state.provenance.source).toBe("generator");
    expect(state.provenance.seed).toBe(2350);

    paintTerrainBrush(state, 10, 10, 0, "square", 7);
    expect(state.provenance.source).toBe("studio");
    expect(state.provenance.version).toBe("1.0");
    expect(state.provenance.seed).toBeUndefined();
    expect(state.provenance.inputs).toContain("generator:v3:seed=2350");

    const packDraft: GroundDraftJson = {
      ...rawDraft,
      provenance: {
        source: "pack",
        version: "v3",
        seed: 1,
        inputs: [
          "tacsim.proving_ground:region.proving_ground",
          "tacsim.proving_ground:tacsim.arena.proving_ground",
        ],
      },
    };
    const packState = importGroundDraftJson(packDraft);
    expect(packState.provenance.source).toBe("pack");
    markStudioEdited(packState);
    expect(packState.provenance.source).toBe("studio");
    expect(packState.provenance.version).toBe("1.0");
    expect(packState.provenance.seed).toBeUndefined();
    expect(packState.provenance.inputs).toEqual([
      "tacsim.proving_ground:region.proving_ground",
      "tacsim.proving_ground:tacsim.arena.proving_ground",
      "pack:v3:seed=1",
    ]);
  });

  it("round-trips 3-byte class RLE and le16 elevation base64 codecs", () => {
    const state = createEditorMap(64, 64, "mars", 8);
    paintTerrainBrush(state, 16, 16, 4, "circle", 9);
    paintTerrainLine(state, 0, 32, 63, 32, 1, "square", 7);
    applySculptBrush(state, 20, 20, {
      mode: "raise",
      shape: "circle",
      radius: 5,
      strengthCm: 350,
      targetElevCm: 0,
      falloff: 0.5,
      relaxToPassable: true,
    });

    const classesB64 = encodeClassesBase64(64, 64, state.classes);
    const decodedClasses = decodeClassesBase64(classesB64);
    expect(decodedClasses.width).toBe(64);
    expect(decodedClasses.height).toBe(64);
    expect(Array.from(decodedClasses.classes)).toEqual(Array.from(state.classes));

    const elevB64 = encodeElevLe16Base64(state.elevCm);
    const decodedElev = decodeElevLe16Base64(elevB64, 64 * 64);
    expect(Array.from(decodedElev)).toEqual(Array.from(state.elevCm));
  });

  it("round-trips the shipped proving_ground.draft.json byte-for-byte on classes and elev", () => {
    const rawDraft = loadShippedProvingGroundDraft();
    const state = importGroundDraftJson(rawDraft);
    expect(state.width).toBe(256);
    expect(state.height).toBe(256);
    expect(state.biome).toBe("temperate");
    expect(state.keySpots).toHaveLength(5);
    expect(state.keySpots[0]).toEqual({ name: "The Ford", x: 64, y: 97 });
    expect(state.keySpots[4]).toEqual({ name: "The Bluff", x: 144, y: 111 });

    const reexported = exportGroundDraftJson(state);
    expect(reexported.schema).toBe("ground.draft.v1");
    expect(reexported.classes).toBe(rawDraft.classes);
    expect(reexported.elev).toBe(rawDraft.elev);
    expect(reexported.key_spots).toEqual(rawDraft.key_spots);
    expect(reexported.waypoints).toEqual(rawDraft.waypoints);
    expect(reexported.entry_zones).toEqual(rawDraft.entry_zones);
  });

  it("matches ground-pack check, route, and viewshed on the shipped proving_ground.draft.json", () => {
    const rawDraft = loadShippedProvingGroundDraft();
    const state = importGroundDraftJson(rawDraft);

    // 1. Check report parity with `ground-pack check`
    const report = checkEditorMap(state);
    expect(report.passed).toBe(true);
    expect(report.failures).toEqual([]);
    expect(report.quadrants).toBe(4);
    expect(report.tracked_passable_pct).toBeGreaterThanOrEqual(90);
    expect(report.legged_passable_pct).toBeGreaterThanOrEqual(95);
    expect(report.zones_reachable_tracked).toBe(true);
    expect(report.zones_reachable_wheeled).toBe(true);
    expect(report.zones_reachable_legged).toBe(true);
    expect(report.key_spots_wheeled_reachable).toBe(true);
    expect(report.waypoints_reachable).toBe(true);
    expect(report.max_road_grade_pct).toBeLessThanOrEqual(20);
    expect(report.max_entry_zone_step_cm).toBeLessThanOrEqual(100);

    // 2. Viewshed from The Bluff (144, 111) with 300 cm target height — cell-for-cell parity with tacsim::ground::viewshed
    const vs = computeViewshed(state, 144, 111, 300);
    expect(vs.length).toBe(256 * 256);
    expect(vs[111 * 256 + 144]).toBe(1); // Origin is seen
    const expectedVs = decodeClassesBase64(pgParityFixture.bluff_viewshed_144_111_300_rle_b64);
    expect(Array.from(vs)).toEqual(Array.from(expectedVs.classes));

    // 3. Route synthesis over the proving ground with its waypoint graph emptied — exact parity with tacsim::ground::route
    state.waypoints = { nodes: [], edges: [] };
    const routed = routeEditorMap(state);
    expect(routed).toEqual(pgParityFixture.routed_waypoints);
    const afterRouteCheck = checkEditorMap(state);
    expect(afterRouteCheck.passed).toBe(true);
    expect(afterRouteCheck.waypoints_reachable).toBe(true);
  });

  it("matches ground-pack check on a 45% ridge (passes) and a 50% ridge (fails until relaxed)", () => {
    const state = createEditorMap(64, 64, "temperate", 2);
    for (let x = 0; x < 64; x++) {
      state.classes[32 * 64 + x] = 7; // road on row 32
    }
    // Build a vertical ridge at x=32 with a 450 cm step (45% grade over 10m cells)
    for (let y = 0; y < 64; y++) {
      for (let x = 32; x < 64; x++) {
        state.elevCm[y * 64 + x] = 450;
      }
    }
    // Cut a <= 20% road ramp on row 32 (0 -> 150 -> 300 -> 450)
    state.elevCm[32 * 64 + 32] = 150;
    state.elevCm[32 * 64 + 33] = 300;
    state.keySpots = [];
    state.waypoints = { nodes: [], edges: [] };

    const report45 = checkEditorMap(state);
    expect(report45.max_grade_pct).toBe(45);
    expect(report45.zones_reachable_tracked).toBe(true);
    expect(report45.zones_reachable_wheeled).toBe(true);
    expect(report45.passed).toBe(true);

    // Raise ridge to 500 cm with no ramp (50% grade > 45% tracked limit and > 20% road limit)
    for (let y = 0; y < 64; y++) {
      for (let x = 32; x < 64; x++) {
        state.elevCm[y * 64 + x] = 500;
      }
    }
    const report50 = checkEditorMap(state);
    expect(report50.max_grade_pct).toBe(50);
    expect(report50.zones_reachable_tracked).toBe(false);
    expect(report50.passed).toBe(false);

    // Relax slopes and verify it passes check again
    const undo = new EditorUndoStack();
    undo.push(state);
    const changed = relaxEditorSlopes(state);
    expect(changed.length).toBeGreaterThan(0);
    const reportRelaxed = checkEditorMap(state);
    expect(reportRelaxed.max_grade_pct).toBeLessThanOrEqual(45);
    expect(reportRelaxed.max_road_grade_pct).toBeLessThanOrEqual(20);
    expect(reportRelaxed.zones_reachable_tracked).toBe(true);
    expect(reportRelaxed.zones_reachable_wheeled).toBe(true);
    expect(reportRelaxed.passed).toBe(true);

    // Undo restores the 50% ridge
    const restored = undo.undo(state);
    expect(restored).not.toBeNull();
    expect(checkEditorMap(restored!).max_grade_pct).toBe(50);
  });

  it("despeckles small natural islands while preserving 1-cell walls and key spots", () => {
    const state = createEditorMap(64, 64, "temperate", 3);
    // 2-cell woods_light (13) island at (20, 20) and (21, 20)
    state.classes[20 * 64 + 20] = 13;
    state.classes[20 * 64 + 21] = 13;
    // 1-cell wall (10) at (25, 25) — built class, must survive
    state.classes[25 * 64 + 25] = 10;
    // 1-cell rock (9) under a key spot at (30, 30) — must survive
    state.classes[30 * 64 + 30] = 9;
    state.keySpots.push({ name: "Lone Crag", x: 30, y: 30 });

    const changed = despeckleEditorMap(state, 3);
    expect(changed).toContain(20 * 64 + 20);
    expect(changed).toContain(20 * 64 + 21);
    expect(state.classes[20 * 64 + 20]).toBe(3);
    expect(state.classes[20 * 64 + 21]).toBe(3);
    expect(state.classes[25 * 64 + 25]).toBe(10);
    expect(state.classes[30 * 64 + 30]).toBe(9);
  });

  it("emits real arena.map (elev zigzag-leb128-delta) and arena.cockpit (x_cm, y_cm, z_cm, heading_mdeg) feeds", () => {
    const rawDraft = loadShippedProvingGroundDraft();
    const state = importGroundDraftJson(rawDraft);
    const bluffPin = { x: 144, y: 111, headingDeg: 270 };
    const { arenaMap, arenaCockpit } = editorToPictureFeeds(state, bluffPin);

    expect(arenaMap.surface).toBe("tacsim.arena.proving_ground");
    expect(arenaMap.biome).toBe("temperate");
    expect(arenaMap.origin_cell).toEqual([0, 0]);
    const elev = arenaMap.elev as { encoding: string; unit_cm: number; data: string };
    expect(elev.encoding).toBe("zigzag-leb128-delta");
    expect(elev.unit_cm).toBe(1);
    expect(elev.data.length).toBeGreaterThan(100);
    expect("elev_field" in arenaMap).toBe(false);
    expect(arenaMap.rows).toHaveLength(256);

    expect(arenaCockpit.x_cm).toBe(144 * 1000 + 500);
    expect(arenaCockpit.y_cm).toBe(111 * 1000 + 500);
    expect(arenaCockpit.z_cm).toBe(state.elevCm[111 * 256 + 144]);
    expect(arenaCockpit.heading_mdeg).toBe(270_000);
    expect("x_m" in arenaCockpit).toBe(false);

    // Also verify passability overlay classification (1 = reachable from entry zones)
    const overlay = computePassabilityOverlay(state, "tracked");
    expect(overlay.length).toBe(256 * 256);
    expect(overlay[97 * 256 + 64]).toBe(1); // The Ford is reachable

    // Verify first-person WASM frame hash at The Bluff matches the server's frame for the same pose
    const wasmBytes = readFileSync(resolve(__dirname, "./pixel/mush_pixel.wasm"));
    const modStudio = new PixelModule(
      new WebAssembly.Instance(new WebAssembly.Module(wasmBytes), {}),
    );
    modStudio.feed(FEED_MAP, JSON.stringify(arenaMap));
    modStudio.feed(FEED_COCKPIT, JSON.stringify(arenaCockpit));
    expect(modStudio.render(640, 480, { offsetMdeg: 0 }, KIND_COCKPIT)).toBe(true);
    const studioBluffHash = modStudio.hash();
    // Pinned across Rust (`crates/mush-contract-tests/tests/pixel_relief.rs`) and Web
    expect(studioBluffHash).toBe("bdce88aa2e6d6ece");

    const modServer = new PixelModule(
      new WebAssembly.Instance(new WebAssembly.Module(wasmBytes), {}),
    );
    const serverMap = {
      ...arenaMap,
      match: 1,
    };
    const serverCockpit = {
      ...arenaCockpit,
      match: 1,
    };
    modServer.feed(FEED_MAP, JSON.stringify(serverMap));
    modServer.feed(FEED_COCKPIT, JSON.stringify(serverCockpit));
    expect(modServer.render(640, 480, { offsetMdeg: 0 }, KIND_COCKPIT)).toBe(true);
    expect(studioBluffHash).toBe(modServer.hash());
  });
});
