// The composer's line shaping (interface plan I4, D-I-8): the Say / Pose /
// Emit / OOC sticky mode is client-side; an explicit trigger (`"`, `:`,
// `;`, `\`) or a channel alias verb always wins; a Shift+Enter newline
// collapses to a space on send (the `@mail/quick` precedent).

import type { NarrativeKind, RichText } from "./proto";
import { narrativeLine, type RLine } from "./render";

/** The sticky modes, in `^P` order. */
export type ComposeMode = "none" | "say" | "pose" | "emit" | "ooc";

export const COMPOSE_MODES: readonly ComposeMode[] = ["none", "say", "pose", "emit", "ooc"];

/** The mode after `mode` in the cycle. */
export function nextMode(mode: ComposeMode): ComposeMode {
  const i = COMPOSE_MODES.indexOf(mode);
  return COMPOSE_MODES[(i + 1) % COMPOSE_MODES.length] ?? "none";
}

/** Newlines (and the whitespace around them) become one space. */
export function collapseNewlines(text: string): string {
  return text.replace(/[ \t]*[\r\n]+[ \t]*/g, " ").trim();
}

/** The trigger a line starts with, as a narrative kind, or `null`. */
export function explicitKind(text: string): NarrativeKind | null {
  switch (text.charAt(0)) {
    case '"':
      return "say";
    case ":":
      return "pose";
    case ";":
      return "semi_pose";
    case "\\":
      return "emit";
    default:
      return null;
  }
}

/**
 * The sticky mode once the shell has put `prefill` in the box (16b cut B.1).
 *
 * A prefill — a screen binding's `@mail/send `, a contact click's `lock BA`, a
 * palette pick, a tray action — is always a command (D-I-18), so it drops the
 * sticky mode back to `none`: left as it was, a pilot who had set Say in the
 * muster room would send `say lock BA`. The box is the shell's since cut B.1
 * and outlives a screen switch, so the mode set on the scene screen is still
 * set on the cockpit. A bare focus (`undefined`, the Enter key) keeps the mode,
 * and so does a speech trigger (`"`, `:`, `;`, `\`), which wins over it anyway.
 */
export function modeAfterPrefill(mode: ComposeMode, prefill: string | undefined): ComposeMode {
  return prefill === undefined || explicitKind(prefill) !== null ? mode : "none";
}

/** Whether the line's first word is one of the member's channel aliases. */
export function startsWithAlias(text: string, aliases: readonly string[]): boolean {
  const first = text.trim().split(/\s+/)[0] ?? "";
  return first.length > 0 && aliases.some((a) => a.toLowerCase() === first.toLowerCase());
}

/** The line to send for `text` under `mode` (newlines collapsed). */
export function composeLine(mode: ComposeMode, text: string, aliases: readonly string[]): string {
  const line = collapseNewlines(text);
  if (line.length === 0) {
    return "";
  }
  if (mode === "none" || explicitKind(line) !== null || startsWithAlias(line, aliases)) {
    return line;
  }
  switch (mode) {
    case "say":
      return `say ${line}`;
    case "pose":
      return `pose ${line}`;
    case "emit":
      return `@emit ${line}`;
    case "ooc":
      return `ooc ${line}`;
    default:
      return line;
  }
}

/**
 * The "renders as" preview: the narrative line the room would read, or
 * `null` when there is nothing to preview (an empty line, a channel alias,
 * a plain command in `none` mode).
 */
export function previewLine(
  mode: ComposeMode,
  text: string,
  actorName: string,
  aliases: readonly string[],
): RLine | null {
  const line = collapseNewlines(text);
  if (line.length === 0 || startsWithAlias(line, aliases)) {
    return null;
  }
  const explicit = explicitKind(line);
  const actor = { id: "", name: actorName };
  if (explicit !== null) {
    const body: RichText = [{ text: line.slice(1) }];
    return narrativeLine(explicit, actor, body);
  }
  if (mode === "none") {
    return null;
  }
  const body: RichText = [{ text: line }];
  return narrativeLine(mode, actor, body);
}
