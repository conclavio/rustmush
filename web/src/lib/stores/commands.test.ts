// CommandsStore: the commands.list keyframe, completion, and the absence
// of verbs the server did not list.

import { describe, expect, it } from "vitest";

import { CommandsStore } from "./commands.svelte";

describe("CommandsStore", () => {
  it("applies a keyframe defensively", () => {
    const s = new CommandsStore();
    expect(s.apply(null)).toBe(false);
    expect(s.apply({ commands: "nope" })).toBe(false);
    expect(
      s.apply({
        commands: [
          { name: "look", aliases: ["l"], switches: [], usage: "look [<thing>]", summary: "See." },
          { name: 7 },
          { name: "@mail", aliases: "x", usage: null },
        ],
        truncated: true,
      }),
    ).toBe(true);
    expect(s.entries.map((c) => c.name)).toEqual(["look", "@mail"]);
    expect(s.entries[1]).toEqual({ name: "@mail", aliases: [], switches: [], usage: "@mail", summary: "" });
    expect(s.truncated).toBe(true);
    expect(s.loaded).toBe(true);
  });

  it("finds by name or alias and completes prefixes", () => {
    const s = new CommandsStore();
    s.apply({
      commands: [
        { name: "look", aliases: ["l"], usage: "look", summary: "" },
        { name: "lock", aliases: [], usage: "lock", summary: "" },
        { name: "@mail", aliases: [], usage: "@mail", summary: "" },
      ],
    });
    expect(s.find("L")?.name).toBe("look");
    expect(s.find("lock")?.name).toBe("lock");
    expect(s.find("@shutdown")).toBeNull();
    expect(s.complete("lo")).toEqual(["lock", "look"]);
    expect(s.complete("")).toEqual([]);
    s.clear();
    expect(s.loaded).toBe(false);
  });
});
