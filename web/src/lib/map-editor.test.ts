import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { FEED_COCKPIT, FEED_MAP, KIND_COCKPIT, PixelModule } from "./pixel/wasm";
import {
  applySculptBrush,
  autoMusterTanks,
  BASE_ROOM_PRESETS,
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
  generatePresetMap,
  getHoldObjective,
  type GroundDraftJson,
  importGroundDraftJson,
  inferEntranceSide,
  markStudioEdited,
  moveTankSlot,
  paintTerrainBrush,
  paintTerrainLine,
  relaxEditorSlopes,
  removeEditorRoom,
  routeEditorMap,
  setHoldObjective,
  setTankHeadingDeg,
  stampRoomOnGrid,
  TERRAIN_PALETTE,
  updateEditorRoom,
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

  it("supports 512x512 (5.12km) maps with 16 quadrants and automatic 256x256 camera-centered arena.map windowing", () => {
    const state = createEditorMap(512, 512, "mars", 8);
    const report = checkEditorMap(state);
    expect(report.passed).toBe(true);
    expect(report.quadrants).toBe(16);
    expect(report.map_bytes).toBeLessThanOrEqual(report.map_max_bytes);

    const pin = { x: 300, y: 400, headingDeg: 90 };
    const { arenaMap, arenaCockpit } = editorToPictureFeeds(state, pin);
    expect(arenaMap.width).toBe(256);
    expect(arenaMap.height).toBe(256);
    expect(arenaMap.origin_cell).toEqual([172, 256]);
    expect(arenaMap.extent).toEqual([512, 512]);
    expect(JSON.stringify(arenaMap).length).toBeLessThanOrEqual(report.map_max_bytes);
    expect(arenaCockpit.x_cm).toBe(300 * 1000 + 500);
    expect(arenaCockpit.y_cm).toBe(400 * 1000 + 500);
  });

  it("generates valid procedural biome presets (proving_ground, mars_canyon, lunar_crater, ice_shelf) that pass §5 check", () => {
    const presets = [
      { id: "proving_ground" as const, biome: "temperate", allowFlora: true },
      { id: "mars_canyon" as const, biome: "mars", allowFlora: false },
      { id: "lunar_crater" as const, biome: "lunar", allowFlora: false },
      { id: "ice_shelf" as const, biome: "ice", allowFlora: false },
    ];

    for (const preset of presets) {
      const state = generatePresetMap(preset.id, 4242);
      expect(state.width).toBe(256);
      expect(state.height).toBe(256);
      expect(state.biome).toBe(preset.biome);
      expect(state.keySpots.length).toBeGreaterThanOrEqual(5);
      expect(state.tanks).toHaveLength(12);

      if (!preset.allowFlora) {
        // Flora codes in mush_atlas: 3 (grass), 13 (woods_light), 14 (woods_heavy)
        const hasFlora = state.classes.some((c) => c === 3 || c === 13 || c === 14);
        expect(hasFlora).toBe(false);
      }

      const report = checkEditorMap(state);
      expect(report.passed).toBe(true);
    }
  });

  it("edits 12-slot tank roster (autoMusterTanks, moveTankSlot, setTankHeadingDeg) and Hold objective with Rust TankDef parity", () => {
    const state = createEditorMap(128, 128, "temperate", 3);
    expect(state.tanks).toHaveLength(12);
    expect(state.tanks[0]).toMatchObject({
      id: "red_1",
      team: "red",
      unit: "tacsim.unit.line_tank",
    });
    expect(typeof state.tanks[0]!.heading_mdeg).toBe("number");

    expect(moveTankSlot(state, 0, 10, 60)).toBe(true);
    expect(state.tanks[0]!.x).toBe(10);
    expect(state.tanks[0]!.y).toBe(60);

    setTankHeadingDeg(state, 0, 135);
    expect(state.tanks[0]!.heading_mdeg).toBe(135_000);

    autoMusterTanks(state);
    const redZone = state.entryZones.find((z) => z.team === "red")!;
    expect(state.tanks[0]!.x).toBeGreaterThanOrEqual(redZone.x0);
    expect(state.tanks[0]!.x).toBeLessThanOrEqual(redZone.x1);

    const hold0 = getHoldObjective(state);
    expect(hold0).not.toBeNull();
    expect(hold0?.spot).toBe("Crossroads");

    setHoldObjective(state, {
      enabled: true,
      spot: "North Bridge",
      radius_cells: 9,
      hold_ms: 180_000,
    });
    const hold1 = getHoldObjective(state);
    expect(hold1).toEqual({
      mode: "hold",
      spot: "North Bridge",
      radius_cells: 9,
      hold_ms: 180_000,
    });

    setHoldObjective(state, { enabled: false });
    expect(getHoldObjective(state)).toBeNull();
  });

  it("stamps MUSH rooms and base hangars onto the grid, checks wheeled doorway reachability, routes room doors, and round-trips ground.draft.v1 JSON", () => {
    const state = createEditorMap(128, 128, "lunar", 5);
    const preset = BASE_ROOM_PRESETS[0]!;

    // Stamp a base hangar with a north-facing vehicle bay linked to Malapert Exchange
    const hangar = stampRoomOnGrid(state, {
      x0: 54,
      y0: 20,
      x1: 68,
      y1: 30,
      role: "hangar",
      side: "north",
      id: "red_hangar",
      name: "Red Base Hangar",
      short_desc: "Armored vehicle bay.",
      desc: "Blast doors open north toward the crossroads.",
      connects_to: {
        pack: preset.pack,
        room: preset.room,
        pack_version: preset.pack_version,
        exit_name: preset.exit_name,
        back_name: preset.back_name,
      },
    });
    expect(hangar).not.toBeNull();
    expect(hangar?.id).toBe("red_hangar");
    expect(hangar?.door).toEqual([61, 30]);
    expect(inferEntranceSide(hangar!)).toBe("north");

    // Verify wall (10), interior floor (5), 3-cell wide hangar door (11), and outer paved apron (1)
    expect(state.classes[20 * 128 + 54]).toBe(10); // Corner wall
    expect(state.classes[25 * 128 + 61]).toBe(5); // Interior floor
    expect(state.classes[30 * 128 + 60]).toBe(11); // Hangar bay door (left)
    expect(state.classes[30 * 128 + 61]).toBe(11); // Hangar bay door (center)
    expect(state.classes[30 * 128 + 62]).toBe(11); // Hangar bay door (right)
    expect(state.classes[31 * 128 + 61]).toBe(1); // Outer paved apron

    // Reject overlapping room footprints
    const overlap = stampRoomOnGrid(state, {
      x0: 60,
      y0: 25,
      x1: 72,
      y1: 35,
      role: "bunker",
    });
    expect(overlap).toBeNull();

    // Stamp a non-overlapping building linked locally to red_hangar
    const barn = stampRoomOnGrid(state, {
      x0: 80,
      y0: 58,
      x1: 90,
      y1: 66,
      role: "building",
      side: "west",
      id: "dispatch_barn",
      name: "Dispatch Barn",
      connects_to: {
        room: "room.red_hangar",
        exit_name: "Red Base Hangar;hangar",
        back_name: "Dispatch Barn;barn",
      },
    });
    expect(barn).not.toBeNull();
    expect(state.rooms).toHaveLength(2);

    // Check report verifies both room doors are wheeled-reachable
    const report = checkEditorMap(state);
    expect(report.rooms_wheeled_reachable).toBe(true);
    expect(report.unreachable_rooms).toEqual([]);

    // Route includes waypoint anchors near both room doors
    const routeRes = routeEditorMap(state);
    expect(routeRes.nodes.length).toBeGreaterThanOrEqual(state.keySpots.length + 2);
    for (const r of state.rooms) {
      const hasDoorNode = state.waypoints.nodes.some(
        (n) => Math.abs(n.x - r.door[0]) <= 2 && Math.abs(n.y - r.door[1]) <= 2,
      );
      expect(hasDoorNode).toBe(true);
    }

    // Update room role & entrance side
    expect(
      updateEditorRoom(state, 1, {
        name: "Fortified Dispatch Bunker",
        role: "bunker",
        side: "south",
      }),
    ).not.toBeNull();
    expect(state.rooms[1]?.role).toBe("bunker");
    expect(state.rooms[1]?.door).toEqual([85, 58]);

    // Round-trip via ground.draft.v1 JSON
    const exported = exportGroundDraftJson(state);
    expect(exported.rooms).toHaveLength(2);
    expect(exported.rooms?.[0]?.connects_to?.pack).toBe("neighborhood.malapert_exchange");

    const reimported = importGroundDraftJson(JSON.parse(JSON.stringify(exported)) as GroundDraftJson);
    expect(reimported.rooms).toEqual(state.rooms);

    // Remove room with clearTerrain=true restores natural terrain over footprint
    expect(removeEditorRoom(state, 1, true)).toBe(true);
    expect(state.rooms).toHaveLength(1);
    expect(state.classes[58 * 128 + 85]).not.toBe(11);
  });
});
