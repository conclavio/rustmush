// Doc 21 cut G.3b: the picture drawn here feeds the module only when a
// keyframe changes, draws for the look, and says when a frame is new.
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";

import { LocalPicture, localCapable } from "./local";
import { PixelModule } from "./wasm";

async function module(): Promise<PixelModule> {
  const bytes = readFileSync(new URL("./mush_pixel.wasm", import.meta.url));
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return new PixelModule(instance);
}

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`../fixtures/cockpit/${name}`, import.meta.url), "utf8"));

describe("the picture drawn here", () => {
  it("feeds a keyframe once, draws for the look, and is quiet when nothing changed", async () => {
    const m = await module();
    const feed = vi.spyOn(m, "feed");
    const p = new LocalPicture(m);
    const cockpit = fixture("cockpit_frame.json");
    const map = fixture("arena_map.json");
    expect(p.draw(null, map, 0)).toBe(false);
    expect(p.draw(cockpit, map, 0)).toBe(true);
    const fed = feed.mock.calls.length;
    expect(p.draw(cockpit, map, 45_000)).toBe(true);
    expect(feed.mock.calls.length).toBe(fed);
    // A new cockpit object is a new keyframe, even with the same content.
    p.draw(structuredClone(cockpit), map, 45_000);
    expect(feed.mock.calls.length).toBe(fed + 1);
    // The seat went: nothing drawn.
    expect(p.draw({ seated: false }, map, 0)).toBe(false);
  });

  it("reports an unchanged frame as new again once told to forget it (doc 21a X-04)", async () => {
    const m = await module();
    const p = new LocalPicture(m);
    const cockpit = fixture("cockpit_frame.json");
    const map = fixture("arena_map.json");
    expect(p.draw(cockpit, map, 0)).toBe(true);
    expect(p.draw(cockpit, map, 0)).toBe(false);
    // A fresh canvas has nothing on it: the same frame must go on it.
    p.forget();
    expect(p.draw(cockpit, map, 0)).toBe(true);
    expect(p.draw(cockpit, map, 0)).toBe(false);
  });

  it("is not offered where the browser cannot draw it", () => {
    // Node has WebAssembly but no ImageData or animation frames.
    expect(localCapable()).toBe(false);
  });
});
