// Scene typography (interface plan I5): the narrative body without its
// speaker, the prose / demoted classes, speaker grouping with its window
// and its breaks, and the roller-only dice card.

import { describe, expect, it } from "vitest";

import {
  groupsBySpeaker,
  isRollNarration,
  narrativeBody,
  narrativeClasses,
  plainText,
} from "./render";
import { isOwnRoll } from "./roll";
import type { RichText } from "./proto";
import {
  GROUP_WINDOW_S,
  groupBlocks,
  ScrollbackStore,
  type SbBlock,
} from "./stores/scrollback.svelte";
import type { RollEntry } from "./stores/rolls.svelte";

const text = (s: string): RichText => [{ text: s, tag: null }];

describe("narrative typography", () => {
  it("renders the body without the speaker per kind", () => {
    expect(plainText(narrativeBody("say", text("hello")))).toBe('"hello"');
    expect(plainText(narrativeBody("pose", text("nods.")))).toBe("nods.");
    expect(plainText(narrativeBody("semi_pose", text("'s hat")))).toBe("'s hat");
    expect(plainText(narrativeBody("page", text("psst")))).toBe("pages: psst");
    expect(plainText(narrativeBody("whisper", text("quiet")))).toBe('whispers, "quiet"');
    expect(plainText(narrativeBody("emit", text("Rain.")))).toBe("Rain.");
    expect(plainText(narrativeBody("ooc", text("brb")))).toBe('<OOC> "brb"');
  });

  it("groups speech and action only", () => {
    expect(groupsBySpeaker("say")).toBe(true);
    expect(groupsBySpeaker("pose")).toBe(true);
    expect(groupsBySpeaker("whisper")).toBe(true);
    expect(groupsBySpeaker("emit")).toBe(false);
    expect(groupsBySpeaker("spoof")).toBe(false);
    expect(groupsBySpeaker("ooc")).toBe(false);
  });

  it("prose is serif; OOC and roll lines are demoted", () => {
    expect(narrativeClasses("say", false)).toBe("ev-narrative kind-say ev-prose");
    expect(narrativeClasses("emit", false)).toBe("ev-narrative kind-emit ev-prose");
    expect(narrativeClasses("ooc", false)).toBe("ev-narrative kind-ooc ev-demoted");
    expect(narrativeClasses("emit", true)).toBe("ev-narrative ev-roll ev-demoted");
  });

  it("recognises the roll narration by its leading roll span", () => {
    expect(
      isRollNarration([
        { text: "roll", tag: "roll" },
        { text: "  ", tag: null },
        { text: "Razor", tag: "player" },
      ]),
    ).toBe(true);
    expect(isRollNarration(text("roll  Razor"))).toBe(false);
    expect(isRollNarration([])).toBe(false);
  });

  it("the dice card is the roller's alone", () => {
    expect(isOwnRoll("#5", "#5")).toBe(true);
    expect(isOwnRoll("#5", "#6")).toBe(false);
    expect(isOwnRoll("#5", null)).toBe(false);
  });
});

let nextId = 1;
const block = (
  ts: number,
  speaker: { id: string; name: string } | null,
  groupable = speaker !== null,
): SbBlock => ({
  id: nextId++,
  ts,
  cls: speaker ? "ev-narrative kind-say ev-prose" : "ev-sys ev-demoted",
  lines: [],
  speaker,
  body: speaker ? [[{ text: "x", cls: "", css: "", cmd: null }]] : null,
  groupable,
  roll: null,
  seq: null,
});

const nick = { id: "#5", name: "Nick" };
const mara = { id: "#7", name: "Mara" };

describe("speaker grouping", () => {
  it("merges consecutive lines by one speaker within the window", () => {
    const g = groupBlocks([block(0, nick), block(1000, nick), block(2000, nick)]);
    expect(g).toHaveLength(1);
    expect(g[0]?.speaker).toEqual(nick);
    expect(g[0]?.blocks).toHaveLength(3);
  });

  it("a different speaker starts a new group", () => {
    const g = groupBlocks([block(0, nick), block(1000, mara), block(2000, mara)]);
    expect(g.map((x) => [x.speaker?.name, x.blocks.length])).toEqual([
      ["Nick", 1],
      ["Mara", 2],
    ]);
  });

  it("an intervening event breaks the run", () => {
    const g = groupBlocks([block(0, nick), block(500, null), block(1000, nick)]);
    expect(g.map((x) => [x.speaker?.name ?? null, x.blocks.length])).toEqual([
      ["Nick", 1],
      [null, 1],
      ["Nick", 1],
    ]);
  });

  it("an emit or an OOC line from the same actor never joins", () => {
    const emit = block(500, nick, false);
    const g = groupBlocks([block(0, nick), emit, block(1000, nick)]);
    expect(g).toHaveLength(3);
    expect(g[1]?.speaker).toBeNull();
  });

  it("the window is five minutes", () => {
    expect(GROUP_WINDOW_S).toBe(300);
    const w = GROUP_WINDOW_S * 1000;
    expect(groupBlocks([block(0, nick), block(w, nick)])).toHaveLength(1);
    expect(groupBlocks([block(0, nick), block(w + 1, nick)])).toHaveLength(2);
    // The window runs from the previous line, not the group's first.
    const chain = groupBlocks([block(0, nick), block(w - 1, nick), block(2 * w - 2, nick)]);
    expect(chain).toHaveLength(1);
  });

  it("the store exposes the visible window as groups and carries the roll card", () => {
    const sb = new ScrollbackStore();
    sb.push("ev-narrative kind-say ev-prose", [], 10, {
      speaker: nick,
      body: [[{ text: '"hi"', cls: "", css: "", cmd: null }]],
      groupable: true,
    });
    sb.push("ev-narrative kind-pose ev-prose", [], 20, {
      speaker: nick,
      body: [[{ text: "nods.", cls: "", css: "", cmd: null }]],
      groupable: true,
    });
    // A speaker without `groupable` (an emit) is never grouped.
    sb.push("ev-narrative kind-emit ev-prose", [], 30, { speaker: nick });
    const entry: RollEntry = {
      rowId: 1,
      ts: 40,
      who: "#5",
      summary: "Dex+Brawl",
      outcome: {
        dice: [{ face: 8, is_success: true, is_one: false, doubled: false }],
        difficulty: 6,
        raw_successes: 1,
        ones: 0,
        willpower_success: false,
        extra_successes: 0,
        net: 1,
        result: { Success: 1 },
      },
    };
    sb.pushRoll(entry, 40);
    const groups = sb.visibleGroups;
    expect(groups.map((g) => [g.speaker?.name ?? null, g.blocks.length])).toEqual([
      ["Nick", 2],
      [null, 1],
      [null, 1],
    ]);
    expect(groups[2]?.blocks[0]?.roll).toBe(entry);
    expect(groups[2]?.blocks[0]?.cls).toBe("ev-roll-card");
    expect(sb.entries[0]?.groupable).toBe(true);
    expect(sb.entries[2]?.groupable).toBe(false);
  });
});
