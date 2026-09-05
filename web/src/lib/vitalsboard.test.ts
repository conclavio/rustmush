// The probe (interface plan I6): the board and the widgets carry no
// ruleset knowledge — no pool, track, trait or ruleset name appears in
// their source. Everything they show comes from the `.vitals` frame and
// the sheet schema at render time.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const FILES = [
  "src/lib/widgets.ts",
  "src/lib/rollbuilder.ts",
  "src/components/VitalsBoard.svelte",
  "src/components/RollBuilder.svelte",
  "src/components/widgets/Gauge.svelte",
  "src/components/widgets/TrackBoxes.svelte",
  "src/components/widgets/KeyValue.svelte",
  "src/components/widgets/Table.svelte",
];

const FORBIDDEN = /\b(stun|edge|blood|willpower|shadowrun|wod|vampire|essence|initiative|limits?)\b/i;

describe("the vitals board has no ruleset knowledge", () => {
  for (const file of FILES) {
    it(file, () => {
      const src = readFileSync(resolve(process.cwd(), file), "utf8");
      const hit = FORBIDDEN.exec(src);
      expect(hit, hit ? `${file} mentions ${hit[0]}` : undefined).toBeNull();
    });
  }
});
