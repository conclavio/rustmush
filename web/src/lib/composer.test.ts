// The composer's line shaping (interface plan I4, D-I-8).

import { describe, expect, it } from "vitest";

import { collapseNewlines, composeLine, nextMode, previewLine } from "./composer";
import { narrativeLine } from "./render";

const aliases = ["pub", "st"];

describe("composeLine", () => {
  it("the four modes produce the right line", () => {
    expect(composeLine("none", "hello", aliases)).toBe("hello");
    expect(composeLine("say", "hello", aliases)).toBe("say hello");
    expect(composeLine("pose", "waves", aliases)).toBe("pose waves");
    expect(composeLine("emit", "The rain.", aliases)).toBe("@emit The rain.");
    expect(composeLine("ooc", "brb", aliases)).toBe("ooc brb");
  });

  it("an explicit trigger or a channel alias verb always wins", () => {
    expect(composeLine("say", ":waves", aliases)).toBe(":waves");
    expect(composeLine("pose", '"hi', aliases)).toBe('"hi');
    expect(composeLine("emit", ";'s hat", aliases)).toBe(";'s hat");
    expect(composeLine("ooc", "\\the wind", aliases)).toBe("\\the wind");
    expect(composeLine("say", "pub anyone?", aliases)).toBe("pub anyone?");
    expect(composeLine("say", "PUB anyone?", aliases)).toBe("PUB anyone?");
    expect(composeLine("say", "public anyone?", aliases)).toBe("say public anyone?");
  });

  it("collapses newlines to one space and trims", () => {
    expect(collapseNewlines("one\n  two\r\nthree  ")).toBe("one two three");
    expect(composeLine("say", "  \n ", aliases)).toBe("");
    expect(composeLine("none", "a\nb", aliases)).toBe("a b");
  });

  it("cycles modes in order", () => {
    expect(nextMode("none")).toBe("say");
    expect(nextMode("say")).toBe("pose");
    expect(nextMode("ooc")).toBe("none");
  });
});

describe("previewLine", () => {
  it("matches narrativeLine for each mode", () => {
    const actor = { id: "", name: "Nick" };
    expect(previewLine("say", "hello", "Nick", aliases)).toEqual(
      narrativeLine("say", actor, [{ text: "hello" }]),
    );
    expect(previewLine("pose", "waves", "Nick", aliases)).toEqual(
      narrativeLine("pose", actor, [{ text: "waves" }]),
    );
    expect(previewLine("emit", "Rain.", "Nick", aliases)).toEqual(
      narrativeLine("emit", actor, [{ text: "Rain." }]),
    );
    expect(previewLine("ooc", "brb", "Nick", aliases)).toEqual(
      narrativeLine("ooc", actor, [{ text: "brb" }]),
    );
  });

  it("follows an explicit trigger and previews nothing for commands or aliases", () => {
    const actor = { id: "", name: "Nick" };
    expect(previewLine("none", ":waves", "Nick", aliases)).toEqual(
      narrativeLine("pose", actor, [{ text: "waves" }]),
    );
    expect(previewLine("say", "look", "Nick", aliases)).toEqual(
      narrativeLine("say", actor, [{ text: "look" }]),
    );
    expect(previewLine("none", "look", "Nick", aliases)).toBeNull();
    expect(previewLine("say", "pub hi", "Nick", aliases)).toBeNull();
    expect(previewLine("say", "", "Nick", aliases)).toBeNull();
  });
});
