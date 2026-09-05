// The command table (interface plan I4): the `commands.list` keyframe —
// name, aliases, switches, usage, summary per command the viewer may use,
// filtered server-side by role and power, bounded (`truncated` marks a
// cut). Feeds the palette and Tab completion. A verb absent from the list
// never appears client-side.

export interface CommandEntry {
  name: string;
  aliases: string[];
  switches: string[];
  usage: string;
  summary: string;
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function strList(v: unknown): string[] {
  return Array.isArray(v) ? v.flatMap((x) => (typeof x === "string" ? [x] : [])) : [];
}

export class CommandsStore {
  entries = $state<CommandEntry[]>([]);
  truncated = $state(false);
  loaded = $state(false);

  /** Apply a `commands.list` keyframe; `false` when it is not one. */
  apply(raw: unknown): boolean {
    if (typeof raw !== "object" || raw === null) {
      return false;
    }
    const o = raw as Record<string, unknown>;
    if (!Array.isArray(o.commands)) {
      return false;
    }
    const entries: CommandEntry[] = [];
    for (const c of o.commands) {
      if (typeof c !== "object" || c === null) {
        continue;
      }
      const cmd = c as Record<string, unknown>;
      const name = str(cmd.name);
      if (name === null || name.length === 0) {
        continue;
      }
      entries.push({
        name,
        aliases: strList(cmd.aliases),
        switches: strList(cmd.switches),
        usage: str(cmd.usage) ?? name,
        summary: str(cmd.summary) ?? "",
      });
    }
    this.entries = entries;
    this.truncated = o.truncated === true;
    this.loaded = true;
    return true;
  }

  /** The command named (or aliased) `verb`, case-insensitively. */
  find(verb: string): CommandEntry | null {
    const v = verb.toLowerCase();
    return (
      this.entries.find(
        (c) => c.name.toLowerCase() === v || c.aliases.some((a) => a.toLowerCase() === v),
      ) ?? null
    );
  }

  /** Names starting with `prefix` (case-insensitive), sorted. */
  complete(prefix: string): string[] {
    const p = prefix.toLowerCase();
    if (p.length === 0) {
      return [];
    }
    return this.entries
      .map((c) => c.name)
      .filter((n) => n.toLowerCase().startsWith(p))
      .sort();
  }

  clear(): void {
    this.entries = [];
    this.truncated = false;
    this.loaded = false;
  }
}
