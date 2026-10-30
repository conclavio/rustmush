// Doc 21 cut G.3b; doc 28 P2: "the fixture renders pixel-equal in the browser
// and the TUI". The wasm module (built by `npm run wasm`, which `npm test`
// runs first) draws the cockpit fixture, the overhead fixture, and the
// bucket-B pictures (`ChartPicture`, `DollPicture`, `ReplayPicture`) at the
// goldens' sizes and looks, and each frame hashes what the native renderer
// pinned.
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import golden from "../fixtures/pixel/cockpit_hashes.json";
import overheadGolden from "../../../../crates/mush-pixel/tests/golden/overhead_hashes.json";
import {
  FEED_CHART,
  FEED_COCKPIT,
  FEED_MAP,
  FEED_REPLAY,
  FEED_REPLAY_CURSOR,
  KIND_CHART,
  KIND_COCKPIT,
  KIND_DOLL,
  KIND_ISO25D,
  KIND_OVERHEAD,
  KIND_REPLAY,
  PIXEL_ABI,
  PixelModule,
} from "./wasm";

const WASM = new URL("./mush_pixel.wasm", import.meta.url);
const UNITS = new URL("./units.bin", import.meta.url);
const fixture = (name: string) => readFileSync(new URL(`../fixtures/cockpit/${name}`, import.meta.url), "utf8");

async function load(): Promise<PixelModule> {
  if (!existsSync(WASM)) {
    throw new Error("web/src/lib/pixel/mush_pixel.wasm is missing: run `npm run wasm`");
  }
  const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
  const m = new PixelModule(instance);
  // The unit pack the page fetches (doc 22 D-22-10), as the native goldens
  // draw with it embedded.
  if (!existsSync(UNITS)) {
    throw new Error("web/src/lib/pixel/units.bin is missing: run `npm run wasm`");
  }
  expect(m.loadUnits(new Uint8Array(readFileSync(UNITS)))).toBe(true);
  return m;
}

describe("the picture module", () => {
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
