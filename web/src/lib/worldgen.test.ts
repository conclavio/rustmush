// worldgen.draft panel: defensive payload parsing + the outbound @gen/*
// Line builders.

import { describe, expect, it } from "vitest";

import { genAcceptLine, genDiscardLine, genRetryLine, parseWorldgenDraft } from "./worldgen";

const fullPayload = {
  room: "#42",
  room_name: "The Pawn Shop",
  builder: "#7",
  model: "claude-opus-5",
  current: { desc: "A shop. Nothing special." },
  proposed: {
    desc: "Dusty glass counters line the walls, each one crowded with pawned trinkets.",
    short_desc: "a cramped pawn shop",
    details: [{ keyword: "counter", desc: "Scratched glass, worn smooth by elbows." }],
    exits: [
      {
        direction: "north",
        name: "Out",
        desc: "A door back to the street.",
        transit_msg: "slips out to the street",
      },
    ],
  },
};

describe("parseWorldgenDraft", () => {
  it("parses a full payload: desc, details, and exits all render", () => {
    const d = parseWorldgenDraft(fullPayload);
    expect(d).not.toBeNull();
    expect(d?.room).toBe("#42");
    expect(d?.roomName).toBe("The Pawn Shop");
    expect(d?.builder).toBe("#7");
    expect(d?.model).toBe("claude-opus-5");
    expect(d?.currentDesc).toBe("A shop. Nothing special.");
    expect(d?.proposed.desc).toContain("Dusty glass counters");
    expect(d?.proposed.shortDesc).toBe("a cramped pawn shop");
    expect(d?.proposed.details).toHaveLength(1);
    expect(d?.proposed.details[0]).toEqual({
      keyword: "counter",
      desc: "Scratched glass, worn smooth by elbows.",
    });
    expect(d?.proposed.exits).toHaveLength(1);
    expect(d?.proposed.exits[0]).toEqual({
      direction: "north",
      name: "Out",
      desc: "A door back to the street.",
      transitMsg: "slips out to the street",
    });
  });

  it("a sparse draft (only desc, the server's one required field) still parses", () => {
    const d = parseWorldgenDraft({ room: "#1", proposed: { desc: "Bare walls." } });
    expect(d).not.toBeNull();
    expect(d?.proposed.desc).toBe("Bare walls.");
    expect(d?.proposed.shortDesc).toBe("");
    expect(d?.proposed.details).toEqual([]);
    expect(d?.proposed.exits).toEqual([]);
    expect(d?.currentDesc).toBe(""); // no "current" object at all
    expect(d?.roomName).toBe("");
  });

  it("missing/malformed payloads yield null rather than throwing", () => {
    expect(parseWorldgenDraft(null)).toBeNull();
    expect(parseWorldgenDraft(undefined)).toBeNull();
    expect(parseWorldgenDraft("nope")).toBeNull();
    expect(parseWorldgenDraft(42)).toBeNull();
    expect(parseWorldgenDraft({})).toBeNull(); // no room reference at all
    expect(parseWorldgenDraft({ room: "" })).toBeNull();
  });

  it("malformed detail/exit entries are dropped, not fatal", () => {
    const d = parseWorldgenDraft({
      room: "#1",
      proposed: {
        desc: "x",
        details: [{ keyword: "ok", desc: "fine" }, "garbage", null, {}],
        exits: ["garbage", { direction: "n", desc: "ok" }, {}],
      },
    });
    expect(d?.proposed.details).toHaveLength(1);
    expect(d?.proposed.details[0]?.keyword).toBe("ok");
    expect(d?.proposed.exits).toHaveLength(1);
    expect(d?.proposed.exits[0]?.direction).toBe("n");
  });

  it("a non-array details/exits field degrades to empty rather than throwing", () => {
    const d = parseWorldgenDraft({ room: "#1", proposed: { desc: "x", details: "nope", exits: 5 } });
    expect(d?.proposed.details).toEqual([]);
    expect(d?.proposed.exits).toEqual([]);
  });
});

describe("outbound @gen/* Line builders", () => {
  it("genAcceptLine", () => {
    expect(genAcceptLine()).toBe("@gen/accept");
  });

  it("genDiscardLine", () => {
    expect(genDiscardLine()).toBe("@gen/discard");
  });

  it("genRetryLine with guidance appends it", () => {
    expect(genRetryLine("make it darker")).toBe("@gen/retry make it darker");
  });

  it("genRetryLine with empty/whitespace guidance omits the argument", () => {
    expect(genRetryLine("")).toBe("@gen/retry");
    expect(genRetryLine("   ")).toBe("@gen/retry");
  });

  it("genRetryLine collapses embedded newlines and trims", () => {
    expect(genRetryLine("  more rain\nand thunder  ")).toBe("@gen/retry more rain and thunder");
  });
});
