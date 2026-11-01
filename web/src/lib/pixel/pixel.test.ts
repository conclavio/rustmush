// Doc 21 cut G.3b; doc 28 P2: "the fixture renders pixel-equal in the browser
// and the TUI". The wasm module (built by `npm run wasm`, which `npm test`
// runs first) draws the cockpit fixture, the overhead fixture, and the
// bucket-B pictures (`ChartPicture`, `DollPicture`, `ReplayPicture`) at the
// goldens' sizes and looks, and each frame hashes what the native renderer
// pinned.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import golden from "../fixtures/pixel/cockpit_hashes.json";
import overheadGolden from "../../../../crates/mush-pixel/tests/golden/overhead_hashes.json";
import {
  FEED_CHART,
  FEED_COCKPIT,
  FEED_MAP,
  FEED_REPLAY,
  FEED_REPLAY_CURSOR,
  FEED_SPACE_COCKPIT,
  FEED_SPACE_DOCKING,
  FEED_SPACE_HOLOTANK,
  KIND_CHART,
  KIND_COCKPIT,
  KIND_DOCKING,
  KIND_DOLL,
  KIND_HOLOTANK,
  KIND_ISO25D,
  KIND_OVERHEAD,
  KIND_REPLAY,
  KIND_SPACE_COCKPIT,
  PIXEL_ABI,
  PixelModule,
} from "./wasm";

const WASM = new URL("./mush_pixel.wasm", import.meta.url);
const MODELS = new URL("../../../../crates/mush-pixel/models/", import.meta.url);
const fixture = (name: string) => readFileSync(new URL(`../fixtures/cockpit/${name}`, import.meta.url), "utf8");

async function load(): Promise<PixelModule> {
  if (!existsSync(WASM)) {
    throw new Error("web/src/lib/pixel/mush_pixel.wasm is missing: run `npm run wasm`");
  }
  const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
  const m = new PixelModule(instance);
  // Every model file the page fetches (doc 31 D-31-2), as the native goldens
  // draw with them embedded.
  const files = readdirSync(MODELS).filter((f) => f.endsWith(".mupk"));
  expect(files.length).toBeGreaterThan(0);
  for (const f of files) m.loadUnits(new Uint8Array(readFileSync(new URL(f, MODELS))));
  return m;
}

describe("the picture module", () => {
  it("asks for a published design's look once and draws from it (doc 31 cut UD.3b)", async () => {
    const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
    const m = new PixelModule(instance);
    const asked: string[][] = [];
    m.askLooksWith((types) => asked.push(types));
    const unit = "tacsim.unit.design.web_probe";
    expect(m.feed(FEED_COCKPIT, JSON.stringify({ seated: true, unit_type: unit, contacts: [] }))).toBe(true);
    expect(asked).toEqual([[unit]]);
    expect(m.feed(FEED_COCKPIT, JSON.stringify({ seated: true, unit_type: unit, contacts: [] }))).toBe(true);
    expect(asked).toHaveLength(1);
    const look = { units: [unit], label: "Probe", short: "PR", model: "mc.st.tank_lvl2", size_cm: [700, 250] };
    expect(m.installLooks({ looks: { [unit]: look } })).toBe(1);
    expect(m.wantedLooks()).toEqual([]);
  });

  it("checks a draft look and previews a model for the unit designer (doc 31 cut UD.2)", async () => {
    const m = await load();
    const look = JSON.parse(readFileSync(new URL("../../../../crates/mush-pixel/looks/tracked_tank.json", import.meta.url), "utf8"));
    const pack = JSON.parse(readFileSync(new URL("../../../../crates/tacsim/content/units/line_tank.json", import.meta.url), "utf8"));
    const rules = pack.entries["tacsim.unit.line_tank"].def;
    const found = m.checkLook(look, rules);
    expect(found.refusals).toEqual(["Tracked tank: weapon laser is fitted to none of its units"]);
    expect(m.checkLook(look, { sections: [], components: [] }).refusals.length).toBeGreaterThan(1);
    const img = m.preview({ model: "mc.st.tank_lvl2", view: "iso", size: 64, facing: "front_right", pose: 0, team: "red" });
    expect(img?.side).toBe(64);
    expect(img?.rgba.some((v, i) => i % 4 === 3 && v === 255)).toBe(true);
    expect(m.preview({ model: "mc.nobody", view: "fp", size: 64, facing: "front", pose: 0, team: "red" })).toBeNull();
  });

  it("names the model files a feed needs, and none once they are in (doc 31 D-31-11)", async () => {
    const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
    const m = new PixelModule(instance);
    expect(m.wantedModels()).toEqual([]);
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    const wanted = m.wantedModels();
    expect(wanted.length).toBeGreaterThan(0);
    const files = readdirSync(MODELS);
    for (const f of wanted) {
      expect(files).toContain(f);
      expect(m.loadUnits(new Uint8Array(readFileSync(new URL(f, MODELS))))).toBe(true);
    }
    expect(m.wantedModels()).toEqual([]);
  });

  it("draws the native renderer's frames, hash for hash", async () => {
    const m = await load();
    expect(m.setKind(KIND_COCKPIT)).toBe(true);
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    const relief = readFileSync(new URL("../fixtures/pixel/arena_map_relief.json", import.meta.url), "utf8");
    // The flat case is the raycaster's: the yard without its all-zero heights.
    const flatMap = JSON.parse(fixture("arena_map.json")) as Record<string, unknown>;
    delete flatMap.elev;
    const flat = JSON.stringify(flatMap);
    // Doc 21 cut G.7b: the flat map (the raycaster) and the relief (the
    // heightfield over its baked colour map), three looks each.
    expect(golden.cases).toHaveLength(6);
    for (const c of golden.cases) {
      expect(m.feed(FEED_MAP, c.map === "relief" ? relief : flat)).toBe(true);
      m.reset();
      expect(m.render(golden.width, golden.height, { offsetMdeg: c.look_mdeg })).toBe(true);
      expect(m.hash()).toBe(c.hash);
      const px = m.pixels();
      expect(px.length).toBe(golden.width * golden.height * 4);
      expect(px[3]).toBe(255);
    }
  });

  it("draws the overhead golden's eight frames, hash for hash (doc 28 P2)", async () => {
    const m = await load();
    expect(m.setKind(KIND_OVERHEAD)).toBe(true);
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    const relief = readFileSync(new URL("../fixtures/pixel/arena_map_relief.json", import.meta.url), "utf8");
    const flatMap = JSON.parse(fixture("arena_map.json")) as Record<string, unknown>;
    delete flatMap.elev;
    const flat = JSON.stringify(flatMap);
    expect(overheadGolden.cases).toHaveLength(8);
    for (const c of overheadGolden.cases) {
      expect(m.feed(FEED_MAP, c.map === "relief" ? relief : flat)).toBe(true);
      m.reset();
      const [w = 0, h = 0] = c.size;
      const look =
        c.look_mdeg === 0
          ? { offsetMdeg: 0 }
          : { offsetMdeg: c.look_mdeg, hullOffsetMdeg: 10_000, orderedMdeg: 90_000 };
      expect(m.render(w, h, look)).toBe(true);
      expect(m.hash()).toBe(c.hash);
      const px = m.pixels();
      expect(px.length).toBe(w * h * 4);
      expect(px[3]).toBe(255);
    }
  });

  it("draws the bucket-B pictures (chart, doll, replay) and refuses unknown kinds (doc 28 P2)", async () => {
    const m = await load();
    expect(m.setKind(0)).toBe(false);
    expect(m.setKind(99)).toBe(false);

    // DollPicture
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_DOLL)).toBe(true);
    const dollHash = m.hash();
    expect(dollHash).toMatch(/^[0-9a-f]{16}$/);

    // ChartPicture
    expect(m.feed(FEED_CHART, null)).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_CHART)).toBe(false);
    expect(m.feed(FEED_CHART, JSON.stringify({ refused: true }))).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_CHART)).toBe(true);
    const chartHash = m.hash();
    expect(chartHash).not.toBe(dollHash);

    // ReplayPicture
    expect(m.feed(FEED_REPLAY, null)).toBe(true);
    expect(m.feed(FEED_REPLAY_CURSOR, JSON.stringify({ match: 1, t_ms: 0, focus: 0 }))).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_REPLAY)).toBe(true);
    expect(m.hash()).not.toBe(chartHash);

    // Iso25DPicture (2.5D tactical camera)
    const relief = readFileSync(new URL("../fixtures/pixel/arena_map_relief.json", import.meta.url), "utf8");
    expect(m.feed(FEED_MAP, relief)).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_ISO25D)).toBe(true);
    const isoHash = m.hash();
    expect(isoHash).toMatch(/^[0-9a-f]{16}$/);
    expect(isoHash).not.toBe(dollHash);

    // Space views (HolotankPicture, SpaceCockpitPicture, DockingPicture — doc 35 cut SF.7)
    expect(
      m.feed(
        FEED_SPACE_HOLOTANK,
        JSON.stringify({
          title: "KIRKWOOD PICKET",
          range_km: 500,
          contacts: [{ id: "raider", label: "KESTREL", x_km: 120, y_km: -60, z_km: 45, relation: "hostile" }],
        }),
      ),
    ).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_HOLOTANK)).toBe(true);
    const holotankHash = m.hash();
    expect(holotankHash).toMatch(/^[0-9a-f]{16}$/);

    expect(
      m.feed(
        FEED_SPACE_COCKPIT,
        JSON.stringify({
          callsign: "UNS-HALCYON",
          yaw_deg: 45,
          pitch_deg: 5,
          roll_deg: 0,
          speed_mps: 4820,
          delta_v_mps: 18500,
          heat_permille: 240,
          hull_pct: 94,
          pdc_mode: "AUTO",
          emcon: "STD",
          bodies: [
            {
              id: "neptune",
              label: "NEPTUNE",
              bearing_deg: 42,
              pitch_deg: 6,
              distance_km: 85000,
              radius_km: 24622,
              spin_deg: 30,
              ring_tilt_deg: 22,
            },
          ],
          contacts: [],
        }),
      ),
    ).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_SPACE_COCKPIT)).toBe(true);
    const spaceCockpitHash = m.hash();
    expect(spaceCockpitHash).not.toBe(holotankHash);

    expect(
      m.feed(
        FEED_SPACE_DOCKING,
        JSON.stringify({
          target_label: "DERELICT-KESTREL",
          target_kind: "disabled_derelict",
          mandatory: true,
          paused_from_world_tick: true,
          dx_mm: 60,
          dy_mm: -40,
          dz_mm: 3200,
          pitch_mdeg: 300,
          yaw_mdeg: -400,
          roll_mdeg: 500,
          vx_mmps: 0,
          vy_mmps: 0,
          vz_mmps: 420,
          dampeners: true,
          status: "ALIGNED-READY",
        }),
      ),
    ).toBe(true);
    expect(m.render(640, 480, { offsetMdeg: 0 }, KIND_DOCKING)).toBe(true);
    expect(m.hash()).not.toBe(spaceCockpitHash);
  });

  it("draws nothing without a cockpit, and forgets one that is unseated", async () => {
    const m = await load();
    expect(m.setKind(KIND_COCKPIT)).toBe(true);
    expect(m.render(320, 220, { offsetMdeg: 0 })).toBe(false);
    expect(m.feed(FEED_COCKPIT, "{not json")).toBe(false);
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    expect(m.render(320, 220, { offsetMdeg: 0 })).toBe(true);
    expect(m.feed(FEED_COCKPIT, JSON.stringify({ seated: false }))).toBe(true);
    expect(m.render(320, 220, { offsetMdeg: 0 })).toBe(false);
    expect(m.render(0, 220, { offsetMdeg: 0 })).toBe(false);
  });

  it("refuses a module that is not its ABI", async () => {
    const fake = { exports: { mp_abi: () => PIXEL_ABI + 1 } } as unknown as WebAssembly.Instance;
    expect(() => new PixelModule(fake)).toThrow(/no export|ABI/);
  });
});
