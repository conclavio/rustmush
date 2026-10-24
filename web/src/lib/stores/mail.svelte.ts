// Mail panel — folder summaries + one folder's entries, from the
// `mail.list` panel keyframe; the unread badge follows both keyframes and
// live `mail_notice` envelopes.
//
// Reading a message is NOT a query (it mutates: marks Read) — the panel
// submits the ordinary `Line("@mail <n>")`; the body renders in scrollback
// exactly as over SSH, and the panel re-requests its list afterwards.
// `index` is the `@mail <n>` ordinal within the player's CURRENT server-side
// folder, so switching folders sends `@mail/folder <name>` first.

export interface MailFolder {
  name: string;
  count: number;
  unread: number;
}

export type MailEntryStatus = "urgent" | "unread" | "read";

export interface MailEntry {
  /**
   * Store-assigned monotonic id — MailPanel's `#each` key. The server's
   * `id` below is NOT safe for that: `applyList` defaults a missing/
   * malformed value to 0 (see `num`), and two such entries in one keyframe
   * would give Svelte a duplicate key and crash the whole app
   * (`each_key_duplicate`). Mirrors ScrollbackStore's `id`.
   */
  rowId: number;
  /** 1-based `@mail <n>` ordinal within the listed folder. */
  index: number;
  id: number;
  from: { id: string; name: string };
  subject: string;
  status: MailEntryStatus;
  sentAt: number;
  replyTo: number | null;
  delivery: string | null;
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

export class MailStore {
  folders = $state<MailFolder[]>([]);
  entries = $state<MailEntry[]>([]);
  /** The folder the current `entries` list belongs to. */
  folder = $state("inbox");
  quota = $state<{ used: number; max: number } | null>(null);
  /** Inbox unread badge (authoritative from keyframes + mail_notice). */
  unread = $state(0);
  /** Whether any keyframe has arrived (gates the panel's empty state). */
  loaded = $state(false);

  /** Next `MailEntry.rowId` (Svelte `#each` key), across all `applyList` calls. */
  #nextRowId = 1;

  /** Apply a `mail.list` keyframe (see mush-mail/src/query.rs). */
  applyList(data: unknown): void {
    if (typeof data !== "object" || data === null) {
      return;
    }
    const o = data as Record<string, unknown>;
    if (Array.isArray(o.folders)) {
      this.folders = o.folders.flatMap((raw) => {
        if (typeof raw !== "object" || raw === null) {
          return [];
        }
        const f = raw as Record<string, unknown>;
        const name = str(f.name);
        return name
          ? [{ name, count: num(f.count), unread: num(f.unread) }]
          : [];
      });
    }
    if (Array.isArray(o.entries)) {
      this.entries = o.entries.flatMap((raw) => {
        if (typeof raw !== "object" || raw === null) {
          return [];
        }
        const e = raw as Record<string, unknown>;
        const from =
          typeof e.from === "object" && e.from !== null
            ? (e.from as Record<string, unknown>)
            : {};
        const status = str(e.status);
        return [
          {
            rowId: this.#nextRowId++,
            index: num(e.index),
            id: num(e.id),
            from: { id: str(from.id) ?? "#-1", name: str(from.name) ?? "?" },
            subject: str(e.subject) ?? "",
            status: (status === "urgent" || status === "unread"
              ? status
              : "read") as MailEntryStatus,
            sentAt: num(e.sent_at),
            replyTo: typeof e.reply_to === "number" ? e.reply_to : null,
            delivery: str(e.delivery),
          },
        ];
      });
    }
    this.folder = str(o.folder) ?? this.folder;
    const q =
      typeof o.quota === "object" && o.quota !== null
        ? (o.quota as Record<string, unknown>)
        : null;
    this.quota = q ? { used: num(q.used), max: num(q.max) } : null;
    const inbox = this.folders.find((f) => f.name === "inbox");
    if (inbox) {
      this.unread = inbox.unread;
    }
    this.loaded = true;
  }

  /** A live `mail_notice`: the server includes the fresh unread count. */
  notice(unread: number): void {
    this.unread = unread;
  }

  clear(): void {
    this.folders = [];
    this.entries = [];
    this.folder = "inbox";
    this.quota = null;
    this.unread = 0;
    this.loaded = false;
  }
}
