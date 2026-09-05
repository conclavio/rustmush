// The command palette's ranking (interface plan I4).

import { describe, expect, it } from "vitest";

import { LOCAL_COMMANDS, MAX_PALETTE_ENTRIES, paletteEntries, type PaletteSources } from "./palette";

const sources: PaletteSources = {
  commands: [
    { name: "look", usage: "look [<thing>]", summary: "See the room." },
    { name: "@mail", usage: "@mail [<n>]", summary: "Read mail." },
    { name: "+roll", usage: "+roll <pool>", summary: "Roll dice." },
  ],
  aliases: ["pub", "st"],
  exits: ["North", "quai"],
};

describe("paletteEntries", () => {
  it("merges local commands, aliases, exits and commands", () => {
    const all = paletteEntries("", sources);
    const labels = all.map((e) => e.label);
    expect(labels).toEqual([
      ...LOCAL_COMMANDS.map(([c]) => c),
      "pub",
      "st",
      "North",
      "quai",
      "look",
      "@mail",
      "+roll",
    ]);
    expect(all.find((e) => e.label === "pub")?.insert).toBe("pub ");
    expect(all.find((e) => e.label === "look")?.insert).toBe("look [<thing>]");
    expect(all.find((e) => e.label === "/clear")?.local).toBe(true);
    expect(all.find((e) => e.label === "look")?.local).toBe(false);
  });

  it("ranks exact prefix matches first, then substrings", () => {
    const labels = paletteEntries("ai", sources).map((e) => e.label);
    expect(labels).toEqual(["quai", "@mail"]);
    const l = paletteEntries("L", sources).map((e) => e.label);
    expect(l[0]).toBe("look");
    expect(l).toContain("/clear");
    expect(l).toContain("+roll");
    expect(l.indexOf("look")).toBeLessThan(l.indexOf("/clear"));
  });

  it("a verb absent from commands.list never appears", () => {
    const labels = paletteEntries("@shutdown", sources).map((e) => e.label);
    expect(labels).toEqual([]);
  });

  it("is bounded", () => {
    const many: PaletteSources = {
      commands: Array.from({ length: 100 }, (_, i) => ({
        name: `cmd${i}`,
        usage: `cmd${i}`,
        summary: "",
      })),
      aliases: [],
      exits: [],
    };
    expect(paletteEntries("cmd", many).length).toBe(MAX_PALETTE_ENTRIES);
  });
});
