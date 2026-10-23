// The site map's frames through the web's grid decoder (doc 20 cut SM.4b; lead ruling 7, V5's
// web half). The two `site.map` frames the Rust side blessed (`TACSIM_BLESS=1 cargo test -p
// tacsim --test driving sm4b_the_site_map_fixtures_are_blessed`) — the viewer's own and a second
// principal's on the same pad — decode through the generic grid decoder (`parseMapFrame` /
// `expandRows`, the units overlaid) to exactly the grid rows each frame's own `site.print` (the
// line-mode `+tv/map site`) prints, glyph for glyph. The second principal's rows hold no ore
// glyph; the legend line is doc 20's, verbatim. (The `FeedScreen` that will draw it is SM.9's.)

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { expandRows, parseMapFrame } from "./stores/map.svelte";

const LEGEND =
  "@ you  H hull  o rig  , regolith  ^ rock  = ice  O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim";

const ORE_GLYPHS = ["1", "2", "3", "+", "."];

type SiteFrame = Record<string, unknown> & {
  height: number;
  site: { print: string[]; legend_line: string };
};

function fixture(name: string): SiteFrame {
  const path = resolve(__dirname, "fixtures/site", name);
  return JSON.parse(readFileSync(path, "utf8")) as SiteFrame;
}

/** The print's grid rows, de-doubled, south first (the decoder's order). */
function printedRows(frame: SiteFrame): string[] {
  const rows = frame.site.print.slice(3, 3 + frame.height).map((line) => {
    const glyphs = [...line];
    for (let i = 0; i < glyphs.length; i += 2) {
      expect(glyphs[i]).toBe(glyphs[i + 1]);
    }
    return glyphs.filter((_, i) => i % 2 === 0).join("");
  });
  return rows.reverse();
}

describe("site.map", () => {
  for (const [name, mine] of [
    ["site_map.json", true],
    ["site_map_other.json", false],
  ] as const) {
    it(`${name} decodes to the rows its own print shows`, () => {
      const raw = fixture(name);
      const frame = parseMapFrame(raw);
      expect(frame).not.toBeNull();
      expect(frame?.schema.id).toBe("site.map");
      const decoded = expandRows(frame!).map((row) => row.join(""));
      expect(decoded).toEqual(printedRows(raw));
      expect(raw.site.legend_line).toBe(LEGEND);
      const hasOre = decoded.some((row) => [...row].some((c) => ORE_GLYPHS.includes(c)));
      expect(hasOre).toBe(mine);
    });
  }
});
