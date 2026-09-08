// Span rendering: tag → class, explicit Style → inline CSS (ansi/256/rgb
// color mapping), CommandLink plumbing, and the classic-style line builders
// mirrored from mush-ansi.

import { describe, expect, it } from "vitest";

import type { RoomView, Span } from "./proto";
import {
  ANSI_PALETTE,
  channelLine,
  colorToCss,
  fmtClock,
  fmtDuration,
  idx256ToHex,
  narrativeLine,
  plainText,
  renderRich,
  renderSpan,
  roomLines,
  styleToClasses,
  styleToCss,
  tagClass,
} from "./render";

describe("colors", () => {
  it("maps ansi indices through the palette (clamped)", () => {
    expect(colorToCss({ ansi: 1 })).toBe(ANSI_PALETTE[1]);
    expect(colorToCss({ ansi: 15 })).toBe(ANSI_PALETTE[15]);
    expect(colorToCss({ ansi: 99 })).toBe(ANSI_PALETTE[15]);
    expect(colorToCss({ ansi: -3 })).toBe(ANSI_PALETTE[0]);
  });

  it("computes the 256-color cube and grayscale ramp", () => {
    expect(idx256ToHex(16)).toBe("#000000"); // cube origin
    expect(idx256ToHex(231)).toBe("#ffffff"); // cube max
    expect(idx256ToHex(196)).toBe("#ff0000"); // pure red
    expect(idx256ToHex(232)).toBe("#080808"); // gray ramp start
    expect(idx256ToHex(255)).toBe("#eeeeee"); // gray ramp end
    expect(idx256ToHex(4)).toBe(ANSI_PALETTE[4]); // low indices reuse 16-color
  });

  it("passes rgb through (clamped)", () => {
    expect(colorToCss({ rgb: [255, 128, 0] })).toBe("rgb(255, 128, 0)");
    expect(colorToCss({ rgb: [999, -5, 12] })).toBe("rgb(255, 0, 12)");
  });
});

describe("styleToCss", () => {
  it("is empty for missing/plain styles", () => {
    expect(styleToCss(undefined)).toBe("");
    expect(styleToCss({})).toBe("");
  });

  it("renders rgb/high-idx256 fg/bg inline, plus bold/italic/underline", () => {
    expect(
      styleToCss({
        fg: { rgb: [1, 2, 3] },
        bg: { idx256: 196 }, // 256-cube red, not part of the 16-color set
        bold: true,
        italic: true,
        underline: true,
      }),
    ).toBe(
      "color: rgb(1, 2, 3); background-color: #ff0000; " +
        "font-weight: bold; font-style: italic; text-decoration: underline",
    );
  });

  it("omits ansi-indexed fg/bg (styleToClasses carries those instead)", () => {
    expect(styleToCss({ fg: { ansi: 7 }, bg: { idx256: 3 } })).toBe("");
    expect(styleToCss({ fg: { ansi: 7 }, bold: true })).toBe("font-weight: bold");
  });
});

describe("styleToClasses", () => {
  it("is empty for missing/plain styles and non-ansi colors", () => {
    expect(styleToClasses(undefined)).toBe("");
    expect(styleToClasses({})).toBe("");
    expect(styleToClasses({ fg: { rgb: [1, 2, 3] } })).toBe("");
    expect(styleToClasses({ bg: { idx256: 200 } })).toBe("");
  });

  it("maps Color::Ansi and low Color::Idx256 to ansi-<n>/ansi-bg-<n>", () => {
    expect(styleToClasses({ fg: { ansi: 3 } })).toBe("ansi-3");
    expect(styleToClasses({ bg: { ansi: 15 } })).toBe("ansi-bg-15");
    expect(styleToClasses({ fg: { idx256: 9 } })).toBe("ansi-9"); // low idx256 aliases ansi
    expect(styleToClasses({ fg: { ansi: 3 }, bg: { idx256: 9 } })).toBe("ansi-3 ansi-bg-9");
  });

  it("clamps out-of-range ansi indices", () => {
    expect(styleToClasses({ fg: { ansi: 99 } })).toBe("ansi-15");
    expect(styleToClasses({ fg: { ansi: -3 } })).toBe("ansi-0");
  });
});

describe("renderSpan", () => {
  it("maps tags to tag-* classes and drops unsafe tags", () => {
    expect(tagClass("player")).toBe("tag-player");
    expect(tagClass("Input-Echo")).toBe("tag-input-echo");
    expect(tagClass('x"><script>')).toBe("");
    expect(tagClass(null)).toBe("");
    expect(tagClass(undefined)).toBe("");
  });

  it("renders a full span (tag + style + link)", () => {
    const span: Span = {
      text: "north",
      tag: "exit",
      style: { bold: true },
      link: { cmd: "north" },
    };
    expect(renderSpan(span)).toEqual({
      text: "north",
      cls: "tag-exit cmdlink",
      css: "font-weight: bold",
      cmd: "north",
    });
  });

  it("renders a plain span with no class/style/cmd", () => {
    expect(renderSpan({ text: "hi" })).toEqual({ text: "hi", cls: "", css: "", cmd: null });
  });

  it("renders ansi-indexed colors as theme classes, not inline hex", () => {
    expect(renderSpan({ text: "x", style: { fg: { ansi: 3 } } })).toEqual({
      text: "x",
      cls: "ansi-3",
      css: "",
      cmd: null,
    });
    expect(renderSpan({ text: "y", tag: "say", style: { bg: { idx256: 9 } } })).toEqual({
      text: "y",
      cls: "tag-say ansi-bg-9",
      css: "",
      cmd: null,
    });
  });

  it("plainText flattens spans", () => {
    expect(plainText([{ text: "a" }, { text: "b", tag: "say" }])).toBe("ab");
  });
});

describe("narrativeLine", () => {
  const actor = { id: "#5", name: "Wizard" };

  it("formats say with quotes and a player-tagged name", () => {
    const line = narrativeLine("say", actor, [{ text: "hello" }]);
    expect(line.map((s) => s.text).join("")).toBe('Wizard says, "hello"');
    expect(line[0]).toMatchObject({ text: "Wizard", cls: "tag-player" });
  });

  it("formats pose/semi_pose/emit/page/whisper", () => {
    const t = (k: Parameters<typeof narrativeLine>[0]) =>
      narrativeLine(k, actor, [{ text: "x" }])
        .map((s) => s.text)
        .join("");
    expect(t("pose")).toBe("Wizard x");
    expect(t("semi_pose")).toBe("Wizardx");
    expect(t("emit")).toBe("x");
    expect(t("spoof")).toBe("x");
    expect(t("page")).toBe("Wizard pages: x");
    expect(t("whisper")).toBe('Wizard whispers, "x"');
    expect(t("ooc")).toBe('<OOC> Wizard says, "x"');
  });

  it("uses Someone for anonymous lines and bare ooc without actor", () => {
    expect(
      narrativeLine("say", null, [{ text: "boo" }])
        .map((s) => s.text)
        .join(""),
    ).toBe('Someone says, "boo"');
    expect(
      narrativeLine("ooc", null, [{ text: "boo" }])
        .map((s) => s.text)
        .join(""),
    ).toBe("<OOC> boo");
  });
});

describe("channelLine", () => {
  const msg = {
    channel: "Public",
    actor: { id: "#5", name: "Wizard" },
    title: null,
    kind: "say" as const,
    text: [{ text: "hi" }],
  };

  it("formats the classic [Channel] Name: text", () => {
    const line = channelLine(msg);
    expect(line.map((s) => s.text).join("")).toBe("[Public] Wizard: hi");
    expect(line[0]!.cls).toBe("tag-channel");
  });

  it("includes the comtitle and tints the prefix with a valid web_hex", () => {
    const line = channelLine({ ...msg, title: "the Grey" }, "#8cc265");
    expect(line.map((s) => s.text).join("")).toBe("[Public] the Grey Wizard: hi");
    expect(line[0]!.css).toBe("color: #8cc265");
    // Garbage hex is refused (style injection guard).
    expect(channelLine(msg, "red; background:url(x)")[0]!.css).toBe("");
  });

  it("formats pose and emit variants", () => {
    expect(
      channelLine({ ...msg, kind: "pose" })
        .map((s) => s.text)
        .join(""),
    ).toBe("[Public] Wizard hi");
    expect(
      channelLine({ ...msg, kind: "emit" })
        .map((s) => s.text)
        .join(""),
    ).toBe("[Public] hi");
  });
});

describe("roomLines", () => {
  const view: RoomView = {
    name: "The Hall",
    id: "#2",
    desc: [{ text: "A big hall." }],
    contents: [
      { id: "#5", name: "Wizard" },
      { id: "#9", name: "lamp" },
    ],
    exits: [
      { name: "north", aliases: ["n"] },
      { name: "out", aliases: [] },
    ],
  };

  it("renders name, desc, contents and clickable exits", () => {
    const lines = roomLines(view);
    expect(lines).toHaveLength(4);
    expect(lines[0]![0]).toMatchObject({ text: "The Hall", cls: "tag-room" });
    expect(lines[1]!.map((s) => s.text).join("")).toBe("A big hall.");
    expect(lines[2]!.map((s) => s.text).join("")).toBe("Contents: Wizard, lamp");
    const exits = lines[3]!.filter((s) => s.cmd !== null);
    expect(exits.map((s) => s.cmd)).toEqual(["north", "out"]);
    expect(exits[0]!.cls).toContain("tag-exit");
  });

  it("omits empty sections", () => {
    const bare = roomLines({ ...view, desc: [], contents: [], exits: [] });
    expect(bare).toHaveLength(1);
  });

  it("leaves the weather fact to the room band (I4-C20)", () => {
    // I3 put the weather module's prose in a `Weather:` line here explicitly
    // "until I4's room band". The band shipped and the line stayed, so the
    // fact was rendered twice. The room block is now the same four lines
    // whatever `ambient` carries.
    const wet = roomLines({
      ...view,
      ambient: { weather: { state: "rain", line: "Overhead: steady rain." } },
    });
    expect(wet).toHaveLength(4);
    expect(
      wet
        .flat()
        .map((s) => s.text)
        .join(" "),
    ).not.toContain("Weather");
    for (const ambient of [
      undefined,
      {},
      { weather: "rain" },
      { weather: { line: 7 } },
      { weather: { line: "" } },
      { weather: null },
    ]) {
      expect(roomLines({ ...view, ambient: ambient as RoomView["ambient"] })).toHaveLength(4);
    }
  });
});

describe("formatters", () => {
  it("fmtDuration", () => {
    expect(fmtDuration(0)).toBe("0s");
    expect(fmtDuration(59)).toBe("59s");
    expect(fmtDuration(60)).toBe("1m");
    expect(fmtDuration(3599)).toBe("59m");
    expect(fmtDuration(3600)).toBe("1h");
    expect(fmtDuration(3600 * 2 + 8 * 60)).toBe("2h08m");
    expect(fmtDuration(86400 * 3)).toBe("3d");
    expect(fmtDuration(-5)).toBe("0s");
  });

  it("fmtClock pads and tolerates garbage", () => {
    expect(fmtClock(Number.NaN)).toBe("");
    expect(fmtClock(new Date(2026, 7, 27, 9, 5).getTime())).toBe("09:05");
  });
});
