// The probe (interface plan I6, extended by I8): the board, the widgets and
// the unit screen carry no ruleset knowledge — no pool, track, trait, part
// or ruleset name appears in their source. Everything they show comes from
// the `.vitals` frame, the `.unit` frame and the sheet schema at render
// time; the only words these files interpret are the four gauge states.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const FILES = [
  "src/lib/widgets.ts",
  "src/lib/rollbuilder.ts",
  "src/lib/stores/unit.svelte.ts",
  "src/components/VitalsBoard.svelte",
  "src/components/RollBuilder.svelte",
  "src/components/UnitScreen.svelte",
  "src/components/UnitSectionMap.svelte",
  "src/components/widgets/Gauge.svelte",
  "src/components/widgets/TrackBoxes.svelte",
  "src/components/widgets/KeyValue.svelte",
  "src/components/widgets/Table.svelte",
];

/** Character-sheet ruleset vocabulary (I6). */
const FORBIDDEN = /\b(stun|edge|blood|willpower|shadowrun|wod|vampire|essence|initiative|limits?)\b/i;
/** Tactical ruleset vocabulary (I8): the client names no part and no kit. */
const FORBIDDEN_UNIT = /\b(mech|torso|armou?r|biped|gauss|heat|hardpoint)\b/i;

describe("the vitals board and the unit screen have no ruleset knowledge", () => {
  for (const file of FILES) {
    it(file, () => {
      const src = readFileSync(resolve(process.cwd(), file), "utf8");
      for (const re of [FORBIDDEN, FORBIDDEN_UNIT]) {
        const hit = re.exec(src);
        expect(hit, hit ? `${file} mentions ${hit[0]}` : undefined).toBeNull();
      }
    });
  }
});
