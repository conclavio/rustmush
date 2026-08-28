// Pure rendering helpers: mush-proto RichText/Span → renderable spans
// (CSS class + inline style + click-to-run command), plus the classic-style
// line formatting for narrative/channel/room events. These mirror
// crates/mush-ansi/src/render.rs — one semantic event, two renders (treaty
// ruling 1): the ANSI renderer formats for terminals, these format for DOM.
//
// Everything here is a pure function over wire types so the vitest suite
// covers it without a DOM.

import type {
  Color,
  NarrativeKind,
  ObjRef,
  RichText,
  RoomView,
  Span,
  Style,
} from "./proto";

/** One renderable span: what a scrollback component paints. */
export interface RSpan {
  text: string;
  /** Space-joined CSS classes, e.g. `"tag-player"`; `""` for plain prose. */
  cls: string;
  /** Inline CSS from an explicit `Style`; `""` when the span has none. */
  css: string;
  /** Click-to-run command (`CommandLink`), or `null` for plain text. */
  cmd: string | null;
}

/** One rendered line. */
export type RLine = RSpan[];

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/**
 * The 16-color ANSI palette, dark-theme values. Kept for `colorToCss`/
 * `idx256ToHex` (256-color cube math, and any caller that wants a raw hex
 * value); the span-rendering path below does NOT use this for `Color::Ansi`
 * or low `Color::Idx256` — those go through `ansi-<n>` / `ansi-bg-<n>` CSS
 * classes instead (defined for both themes in app.css next to the other
 * theme tokens), because a single fixed dark-optimized hex set emitted
 * inline made e.g. white(7)/yellow(3) nearly invisible on the light theme.
 * Explicit `Style.rgb` and the 256-cube/grayscale range stay inline (they
 * aren't part of the themed 16-color set).
 */
export const ANSI_PALETTE: readonly string[] = [
  "#3f4451", // 0 black
  "#e05561", // 1 red
  "#8cc265", // 2 green
  "#d1a458", // 3 yellow
  "#4aa5f0", // 4 blue
  "#c162de", // 5 magenta
  "#42b3c2", // 6 cyan
  "#d7dae0", // 7 white
  "#6c7484", // 8 bright black
  "#ff616e", // 9 bright red
  "#a5e075", // 10 bright green
  "#f0a45d", // 11 bright yellow
  "#4dc4ff", // 12 bright blue
  "#de73ff", // 13 bright magenta
  "#4cd1e0", // 14 bright cyan
  "#ffffff", // 15 bright white
];

/** xterm 256-color index → hex (16..231 color cube, 232..255 grayscale). */
export function idx256ToHex(idx: number): string {
  const i = Math.max(0, Math.min(255, Math.trunc(idx)));
  if (i < 16) {
    return ANSI_PALETTE[i]!;
  }
  const hex2 = (n: number) => n.toString(16).padStart(2, "0");
  if (i < 232) {
    const c = i - 16;
    const steps = [0, 95, 135, 175, 215, 255] as const;
    const r = steps[Math.trunc(c / 36) % 6]!;
    const g = steps[Math.trunc(c / 6) % 6]!;
    const b = steps[c % 6]!;
    return `#${hex2(r)}${hex2(g)}${hex2(b)}`;
  }
  const v = 8 + (i - 232) * 10;
  return `#${hex2(v)}${hex2(v)}${hex2(v)}`;
}

/** A wire `Color` (ansi index / 256 index / rgb triple) → CSS color. */
export function colorToCss(c: Color): string {
  if ("ansi" in c) {
    return ANSI_PALETTE[Math.max(0, Math.min(15, Math.trunc(c.ansi)))]!;
  }
  if ("idx256" in c) {
    return idx256ToHex(c.idx256);
  }
  const [r, g, b] = c.rgb;
  const clamp = (n: number) => Math.max(0, Math.min(255, Math.trunc(n)));
  return `rgb(${clamp(r)}, ${clamp(g)}, ${clamp(b)})`;
}

/**
 * `Color::Ansi(n)` and the low `Color::Idx256(n < 16)` alias (the 16-color
 * base palette, whichever wire shape carried it) → the theme-tokenized
 * `ansi-<n>` class index, or `null` for RGB / the 256-cube / grayscale range
 * (those stay inline hex — they aren't part of the fixed 16-color set).
 */
function ansiIndex(c: Color): number | null {
  if ("ansi" in c) {
    return Math.max(0, Math.min(15, Math.trunc(c.ansi)));
  }
  if ("idx256" in c) {
    const i = Math.trunc(c.idx256);
    return i >= 0 && i < 16 ? i : null;
  }
  return null;
}

/**
 * An explicit `Style` → inline CSS declaration string (`""` when plain).
 * Ansi-indexed fg/bg are handled by `styleToClasses` instead (theme classes,
 * not inline hex) — see `ansiIndex`.
 */
export function styleToCss(style?: Style): string {
  if (!style) {
    return "";
  }
  const parts: string[] = [];
  if (style.fg && ansiIndex(style.fg) === null) {
    parts.push(`color: ${colorToCss(style.fg)}`);
  }
  if (style.bg && ansiIndex(style.bg) === null) {
    parts.push(`background-color: ${colorToCss(style.bg)}`);
  }
  if (style.bold) {
    parts.push("font-weight: bold");
  }
  if (style.italic) {
    parts.push("font-style: italic");
  }
  if (style.underline) {
    parts.push("text-decoration: underline");
  }
  return parts.join("; ");
}

/**
 * An explicit `Style`'s ansi-indexed fg/bg → `ansi-<n>` / `ansi-bg-<n>`
 * classes (`""` when the style has none) — the theme-legible counterpart to
 * `styleToCss`'s inline declarations. See `ansiIndex`.
 */
export function styleToClasses(style?: Style): string {
  if (!style) {
    return "";
  }
  const cls: string[] = [];
  const fg = style.fg ? ansiIndex(style.fg) : null;
  if (fg !== null) {
    cls.push(`ansi-${fg}`);
  }
  const bg = style.bg ? ansiIndex(style.bg) : null;
  if (bg !== null) {
    cls.push(`ansi-bg-${bg}`);
  }
  return cls.join(" ");
}

// ---------------------------------------------------------------------------
// Spans
// ---------------------------------------------------------------------------

/** Tags become `tag-<name>` classes; anything unsafe is dropped. */
export function tagClass(tag?: string | null): string {
  if (!tag || !/^[a-z0-9_-]+$/i.test(tag)) {
    return "";
  }
  return `tag-${tag.toLowerCase()}`;
}

/** One wire span → one renderable span. */
export function renderSpan(span: Span): RSpan {
  const cls = [tagClass(span.tag), styleToClasses(span.style), span.link ? "cmdlink" : ""]
    .filter(Boolean)
    .join(" ");
  return {
    text: span.text,
    cls,
    css: styleToCss(span.style),
    cmd: span.link ? span.link.cmd : null,
  };
}

/** A RichText → renderable spans. */
export function renderRich(rt: RichText): RLine {
  return rt.map(renderSpan);
}

/** RichText → plain string (titles, tooltips, tests). */
export function plainText(rt: RichText): string {
  return rt.map((s) => s.text).join("");
}

/** Local helper: build a plain/tagged/linked span. */
function mk(text: string, tag?: string, cmd?: string): RSpan {
  return {
    text,
    cls: [tagClass(tag), cmd ? "cmdlink" : ""].filter(Boolean).join(" "),
    css: "",
    cmd: cmd ?? null,
  };
}

// ---------------------------------------------------------------------------
// Classic-style lines (mirrors mush-ansi's render_narrative/render_channel/
// render_room so SSH and web read the same)
// ---------------------------------------------------------------------------

/** `Narrative { kind, actor, text }` → one formatted line. */
export function narrativeLine(
  kind: NarrativeKind,
  actor: ObjRef | null | undefined,
  text: RichText,
): RLine {
  const name = actor ? actor.name : "Someone";
  const rich = renderRich(text);
  switch (kind) {
    case "say":
      return [mk(name, "player"), mk(' says, "'), ...rich, mk('"')];
    case "pose":
      return [mk(name, "player"), mk(" "), ...rich];
    case "semi_pose":
      return [mk(name, "player"), ...rich];
    case "emit":
    case "spoof":
      return rich;
    case "page":
      return [mk(name, "player"), mk(" pages: "), ...rich];
    case "whisper":
      return [mk(name, "player"), mk(' whispers, "'), ...rich, mk('"')];
    case "ooc":
      return actor
        ? [mk("<OOC> "), mk(name, "player"), mk(' says, "'), ...rich, mk('"')]
        : [mk("<OOC> "), ...rich];
  }
}

/** The fields of a live `Channel` event / a backlog message this needs. */
export interface ChannelLineInput {
  channel: string;
  actor: ObjRef;
  title?: string | null;
  kind: NarrativeKind;
  text: RichText;
}

/**
 * `[Public] Title Name: text` — the classic channel render. `colorHex`
 * (comsys.state's `color.web_hex`) tints the `[channel]` prefix.
 */
export function channelLine(msg: ChannelLineInput, colorHex?: string | null): RLine {
  const prefix: RSpan = {
    text: `[${msg.channel}]`,
    cls: "tag-channel",
    css: colorHex && /^#[0-9a-f]{3,8}$/i.test(colorHex) ? `color: ${colorHex}` : "",
    cmd: null,
  };
  const rich = renderRich(msg.text);
  const namePrefix: RSpan[] = [];
  if (msg.title) {
    namePrefix.push(mk(`${msg.title} `));
  }
  namePrefix.push(mk(msg.actor.name, "player"));
  switch (msg.kind) {
    case "pose":
      return [prefix, mk(" "), ...namePrefix, mk(" "), ...rich];
    case "semi_pose":
      return [prefix, mk(" "), ...namePrefix, ...rich];
    case "emit":
    case "spoof":
      return [prefix, mk(" "), ...rich];
    default:
      return [prefix, mk(" "), ...namePrefix, mk(": "), ...rich];
  }
}

/** A structured `RoomView` → classic look output (multiple lines). */
export function roomLines(view: RoomView): RLine[] {
  const lines: RLine[] = [];
  lines.push([mk(view.name, "room")]);
  if (view.desc.length > 0) {
    lines.push(renderRich(view.desc));
  }
  if (view.contents.length > 0) {
    const line: RLine = [mk("Contents: ")];
    view.contents.forEach((obj, i) => {
      if (i > 0) {
        line.push(mk(", "));
      }
      line.push(mk(obj.name, "player", `look ${obj.name}`));
    });
    lines.push(line);
  }
  if (view.exits.length > 0) {
    const line: RLine = [mk("Obvious exits: ")];
    view.exits.forEach((exit, i) => {
      if (i > 0) {
        line.push(mk("  "));
      }
      line.push(mk(exit.name, "exit", exit.name));
    });
    lines.push(line);
  }
  return lines;
}

// ---------------------------------------------------------------------------
// Small formatters
// ---------------------------------------------------------------------------

/** Seconds → compact idle column: `12s`, `4m`, `2h08m`, `3d`. */
export function fmtDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  if (s < 60) {
    return `${s}s`;
  }
  const m = Math.floor(s / 60);
  if (m < 60) {
    return `${m}m`;
  }
  const h = Math.floor(m / 60);
  if (h < 24) {
    const rem = m % 60;
    return rem > 0 ? `${h}h${String(rem).padStart(2, "0")}m` : `${h}h`;
  }
  return `${Math.floor(h / 24)}d`;
}

/** Unix-ms timestamp → `HH:MM` in the viewer's locale (scrollback gutter). */
export function fmtClock(tsMs: number): string {
  const d = new Date(tsMs);
  if (Number.isNaN(d.getTime())) {
    return "";
  }
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
