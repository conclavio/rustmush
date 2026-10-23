// Doc 21 cut G.3b: "the fixture renders pixel-equal in the browser and the
// TUI". The wasm module (built by `npm run wasm`, which `npm test` runs
// first) draws the cockpit fixture at the web golden's size and looks, and
// each frame hashes what the native renderer pinned in
// `fixtures/pixel/cockpit_hashes.json` (`crates/mush-pixel/tests/web_golden.rs`).
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import golden from "../fixtures/pixel/cockpit_hashes.json";
import { FEED_COCKPIT, FEED_MAP, PIXEL_ABI, PixelModule } from "./wasm";

const WASM = new URL("./mush_pixel.wasm", import.meta.url);
const fixture = (name: string) => readFileSync(new URL(`../fixtures/cockpit/${name}`, import.meta.url), "utf8");

async function load(): Promise<PixelModule> {
  if (!existsSync(WASM)) {
    throw new Error("web/src/lib/pixel/mush_pixel.wasm is missing: run `npm run wasm`");
  }
  const { instance } = await WebAssembly.instantiate(readFileSync(WASM), {});
  return new PixelModule(instance);
}

describe("the picture module", () => {
  it("draws the native renderer's frames, hash for hash", async () => {
    const m = await load();
    expect(m.feed(FEED_COCKPIT, fixture("cockpit_frame.json"))).toBe(true);
    expect(m.feed(FEED_MAP, fixture("arena_map.json"))).toBe(true);
    expect(golden.cases).toHaveLength(3);
    for (const c of golden.cases) {
      m.reset();
      expect(m.render(golden.width, golden.height, { offsetMdeg: c.look_mdeg })).toBe(true);
      expect(m.hash()).toBe(c.hash);
      const px = m.pixels();
      expect(px.length).toBe(golden.width * golden.height * 4);
      expect(px[3]).toBe(255);
    }
  });

  it("draws nothing without a cockpit, and forgets one that is unseated", async () => {
    const m = await load();
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
