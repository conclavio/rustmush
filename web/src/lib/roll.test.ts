// wod.roll readout tests: outcome classification, die highlighting, and
// defensive parsing of the panel payload.

import { describe, expect, it } from "vitest";

import { classifyResult, dieClass, parseRollPanel, youOrId } from "./roll";

describe("classifyResult", () => {
  it("classifies a plain success", () => {
    expect(classifyResult({ Success: 1 })).toEqual({ kind: "success", label: "1 success" });
    expect(classifyResult({ Success: 3 })).toEqual({ kind: "success", label: "3 successes" });
  });

  it("classifies a failure (zero net successes, no botch)", () => {
    expect(classifyResult("Failure")).toEqual({ kind: "failure", label: "Failure" });
  });

  it("classifies a botch, singular vs plural", () => {
    expect(classifyResult({ Botch: 1 })).toEqual({ kind: "botch", label: "Botch" });
    expect(classifyResult({ Botch: 3 })).toEqual({ kind: "botch", label: "Botch (3)" });
  });
});

describe("dieClass", () => {
  it("highlights a success", () => {
    expect(dieClass({ face: 8, is_success: true, is_one: false, doubled: false })).toBe(
      "die-success",
    );
  });

  it("marks a doubled (specialty 10-again) success", () => {
    expect(dieClass({ face: 10, is_success: true, is_one: false, doubled: true })).toBe(
      "die-success die-doubled",
    );
  });

  it("highlights a 1", () => {
    expect(dieClass({ face: 1, is_success: false, is_one: true, doubled: false })).toBe("die-one");
  });

  it("plain die otherwise", () => {
    expect(dieClass({ face: 4, is_success: false, is_one: false, doubled: false })).toBe(
      "die-plain",
    );
  });
});

describe("youOrId", () => {
  it("resolves the viewer's own dbref to 'You'", () => {
    expect(youOrId("#42", "#42")).toBe("You");
  });

  it("leaves anyone else's dbref as-is", () => {
    expect(youOrId("#7", "#42")).toBe("#7");
  });

  it("no self id known -> always the raw dbref", () => {
    expect(youOrId("#42", null)).toBe("#42");
  });
});

describe("parseRollPanel", () => {
  const botchPayload = {
    who: "#42",
    summary: "Nick rolls (strength+brawl, 5 dice, diff 6) and rolled 3 2 1 -> botch",
    outcome: {
      difficulty: 6,
      dice: [
        { face: 3, is_success: false, is_one: false, doubled: false },
        { face: 2, is_success: false, is_one: false, doubled: false },
        { face: 1, is_success: false, is_one: true, doubled: false },
      ],
      raw_successes: 0,
      ones: 1,
      willpower_success: false,
      extra_successes: 0,
      net: -1,
      result: { Botch: 1 },
    },
  };

  it("parses a full payload", () => {
    const parsed = parseRollPanel(botchPayload);
    expect(parsed).not.toBeNull();
    expect(parsed?.who).toBe("#42");
    expect(parsed?.outcome?.dice).toHaveLength(3);
    expect(parsed?.outcome?.result).toEqual({ Botch: 1 });
  });

  it("a specialty (doubled 10) success round-trips", () => {
    const parsed = parseRollPanel({
      who: "#1",
      summary: "rolled 10 10 6 -> 5 successes",
      outcome: {
        difficulty: 6,
        dice: [
          { face: 10, is_success: true, is_one: false, doubled: true },
          { face: 10, is_success: true, is_one: false, doubled: true },
          { face: 6, is_success: true, is_one: false, doubled: false },
        ],
        raw_successes: 5,
        ones: 0,
        willpower_success: false,
        extra_successes: 0,
        net: 5,
        result: { Success: 5 },
      },
    });
    expect(parsed?.outcome?.dice.filter((d) => d.doubled)).toHaveLength(2);
    expect(classifyResult(parsed!.outcome!.result).label).toBe("5 successes");
  });

  it("missing/malformed payloads yield null rather than throwing", () => {
    expect(parseRollPanel(null)).toBeNull();
    expect(parseRollPanel({})).toBeNull();
    // No summary AND no usable outcome: nothing to render.
    expect(parseRollPanel({ who: "#1", outcome: "nope" })).toBeNull();
    expect(parseRollPanel({ who: "#1", outcome: { dice: "nope" } })).toBeNull(); // no difficulty
    expect(parseRollPanel({ who: "#1", summary: "" })).toBeNull(); // an empty one is none
    expect(parseRollPanel({ summary: "a roll with no roller" })).toBeNull();
  });

  // I6-C11: `Owod20Outcome` is ONE ruleset's shape. `difficulty` is oWoD's
  // own concept, and demanding it dropped every `sr.roll` frame — the web's
  // recent-rolls table and dice card were permanently empty on a Shadowrun
  // server while the TUI, which renders the summary, was fine.
  it("keeps another engine's roll on its summary, with a null outcome", () => {
    const parsed = parseRollPanel({
      who: "#42",
      summary: "Nick rolls Agility + Pistols: 3 hits.",
      outcome: { hits: 3, glitch: false, pool: 9 },
    });
    expect(parsed?.who).toBe("#42");
    expect(parsed?.summary).toBe("Nick rolls Agility + Pistols: 3 hits.");
    expect(parsed?.outcome).toBeNull();

    // …and a frame with no `outcome` key at all is the same case: an
    // Initiative push carries a score and a summary, nothing more.
    const init = parseRollPanel({ who: "#42", summary: "Nick rolls Initiative: = 14." });
    expect(init?.summary).toBe("Nick rolls Initiative: = 14.");
    expect(init?.outcome).toBeNull();
  });

  it("a malformed summary falls back to the classified result label", () => {
    const parsed = parseRollPanel({
      who: "#1",
      outcome: { difficulty: 6, dice: [], result: "Failure" },
    });
    expect(parsed?.summary).toBe("Failure");
  });

  it("malformed dice entries are dropped, not fatal", () => {
    const parsed = parseRollPanel({
      who: "#1",
      outcome: { difficulty: 6, dice: [{ face: 5 }, "garbage", null], result: "Failure" },
    });
    expect(parsed?.outcome?.dice).toHaveLength(1);
  });
});
