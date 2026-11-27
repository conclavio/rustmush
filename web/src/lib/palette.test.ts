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
    expect(all.find((e) => e.label === "North")?.insert).toBe("goto North");
    expect(all.find((e) => e.label === "quai")?.insert).toBe("goto quai");
    expect(all.find((e) => e.label === "look")?.insert).toBe("look [<thing>]");
    expect(all.find((e) => e.label === "/clear")?.local).toBe(true);
    expect(all.find((e) => e.label === "look")?.local).toBe(false);
  });

  it("P2-CLIENT-1: prefixes authored exits with goto and drops control-char or blank exits", () => {
    const hostile: PaletteSources = {
      commands: [],
      aliases: [],
      exits: ["QUIT", "@destroy me", "/clear", '"spoof', "   ", "Bad\nQUIT", "Null\x00Exit", "Del\x7fExit"],
    };
    const entries = paletteEntries("", hostile).filter((e) => e.detail === "exit");
    expect(entries.map((e) => ({ label: e.label, insert: e.insert, detail: e.detail }))).toEqual([
      { label: "QUIT", insert: "goto QUIT", detail: "exit" },
      { label: "@destroy me", insert: "goto @destroy me", detail: "exit" },
      { label: "/clear", insert: "goto /clear", detail: "exit" },
      { label: '"spoof', insert: 'goto "spoof', detail: "exit" },
    ]);
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

  it("includes live active and available quest dialog shortcuts when supplied", () => {
    const entries = paletteEntries("quest", {
      ...sources,
      quests: [
        {
          kind: "active",
          key: "q.metals",
          title: "Metals Down the Corridor",
          detail: "Buy 5t metals",
          ready: false,
        },
        {
          kind: "available",
          key: "q.enroll",
          npcId: "#42",
          title: "First Papers at the Registry",
          detail: "Offered by Maren Halvorsen",
        },
      ],
    });
    expect(entries.some((e) => e.label === "/quests")).toBe(true);
    const activeRow = entries.find((e) => e.questAction?.key === "q.metals");
    expect(activeRow?.label).toBe("quest Metals Down the Corridor");
    expect(activeRow?.questAction?.kind).toBe("active");
    const availRow = entries.find((e) => e.questAction?.key === "q.enroll");
    expect(availRow?.label).toBe("[!] First Papers at the Registry");
    expect(availRow?.questAction?.npcId).toBe("#42");
  });
});
