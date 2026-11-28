// The command palette's entries (interface plan I4): `commands.list`
// merged with the member's channel aliases and the room's exits, filtered
// on what follows the slash, exact-prefix matches first. Pure, so vitest
// pins the ranking.

import { exitCommand } from "./keys";

/** One command from the `commands.list` keyframe, as the palette needs it. */
export interface PaletteCommand {
  name: string;
  usage: string;
  summary: string;
}

/** One live active, available, route, next-step, cycle, or between-steps next-guided quest shortcut in the palette. */
export interface PaletteQuestAction {
  kind: "active" | "available" | "route" | "step" | "cycle" | "nextGuided";
  key: string;
  npcId?: string;
  command?: string;
  title: string;
  detail: string;
  ready?: boolean;
}

/** What the palette merges. */
export interface PaletteSources {
  commands: readonly PaletteCommand[];
  /** The member's channel aliases (speak verbs). */
  aliases: readonly string[];
  /** The current room's exit names. */
  exits: readonly string[];
  /** Optional live active / room-available quest dialog shortcuts. */
  quests?: readonly PaletteQuestAction[];
}

/** One palette row. */
export interface PaletteEntry {
  /** What Enter puts in the composer. */
  insert: string;
  /** The left column. */
  label: string;
  /** The right column. */
  detail: string;
  /** A client-local command (keeps its slash). */
  local: boolean;
  /** Optional client-local or command quest action opened/prefilled/run on pick. */
  questAction?: {
    kind: "active" | "available" | "route" | "step" | "cycle" | "nextGuided";
    key: string;
    npcId?: string;
    command?: string;
  };
}

/** The client's own commands; they keep their slash when picked. */
export const LOCAL_COMMANDS: ReadonlyArray<readonly [string, string]> = [
  ["/clear", "empty the scrollback"],
  // The same verb and the same wording as the TUI's own list
  // (`tui/app.rs`), which has had it since I12 while the web could only
  // reach it from the scroll-top button (LB-20).
  ["/history", "load older scene history"],
  ["/quests", "open the quest log"],
  ["/theme", "switch theme (dark, light, system)"],
  ["/help", "the client commands"],
];

/** The most rows the palette lists. */
export const MAX_PALETTE_ENTRIES = 40;

/**
 * The entries matching `query` (the text after the slash, any case):
 * every source's exact-prefix matches first, in source order — local
 * commands, quest actions, aliases, exits, commands — then substring matches.
 */
export function paletteEntries(query: string, sources: PaletteSources): PaletteEntry[] {
  const q = query.trim().toLowerCase();
  const prefix: PaletteEntry[] = [];
  const rest: PaletteEntry[] = [];
  const consider = (entry: PaletteEntry, key: string): void => {
    const k = key.toLowerCase();
    if (q.length === 0 || k.startsWith(q)) {
      prefix.push(entry);
    } else if (k.includes(q)) {
      rest.push(entry);
    }
  };
  for (const [cmd, desc] of LOCAL_COMMANDS) {
    consider({ insert: cmd, label: cmd, detail: desc, local: true }, cmd);
  }
  if (sources.quests) {
    for (const qa of sources.quests) {
      const tag =
        qa.kind === "available" || qa.kind === "nextGuided"
          ? "[!]"
          : qa.kind === "route"
            ? "[→]"
            : qa.kind === "step"
              ? qa.ready
                ? "[?]"
                : "[▸]"
              : qa.kind === "cycle"
                ? "[↻]"
                : qa.ready
                  ? "[?]"
                  : "quest";
      const label = `${tag} ${qa.title}`;
      const isCmd = qa.kind === "route" || qa.kind === "step";
      const insert = isCmd && qa.command ? qa.command : "/quests";
      const extraKw =
        qa.kind === "route"
          ? "route walk board ashore"
          : qa.kind === "step"
            ? "next step action sail"
            : qa.kind === "cycle"
              ? "cycle switch next tracked"
              : qa.kind === "nextGuided"
                ? "next guided tour offer"
                : "";
      consider(
        {
          insert,
          label,
          detail: qa.detail,
          local: !isCmd,
          questAction: { kind: qa.kind, key: qa.key, npcId: qa.npcId, command: qa.command },
        },
        `quest ${qa.title} ${qa.detail} ${qa.command ?? ""} ${extraKw}`,
      );
    }
  }
  for (const alias of sources.aliases) {
    if (alias.length > 0) {
      consider({ insert: `${alias} `, label: alias, detail: "channel", local: false }, alias);
    }
  }
  for (const exit of sources.exits) {
    const cmd = exitCommand(exit);
    if (cmd !== null) {
      consider({ insert: cmd, label: exit, detail: "exit", local: false }, exit);
    }
  }
  for (const c of sources.commands) {
    consider(
      { insert: c.usage.length > 0 ? c.usage : c.name, label: c.name, detail: c.summary, local: false },
      c.name,
    );
  }
  return [...prefix, ...rest].slice(0, MAX_PALETTE_ENTRIES);
}
