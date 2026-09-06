// The unified notification surface (interface plan I11): every server
// `Notice` — mail, a page, a mention, an escalation, a job update — held in
// one place and rendered the same. A transient toast for the newest unseen
// ones, a persistent tray for the history. A notice's action is a Prefill
// (D-I-18): a tap opens the composer with the line ready, never a bare
// mutation. Delivered, never polled; the client never derives severity, it
// reads it (rule 5).

import type { NoticeAction, NoticeSeverity, RichText } from "../proto";

/** One held notification. */
export interface Notification {
  /** Store-assigned render key. */
  rowId: number;
  /** The server's notice id, for de-duplication and dismissal. */
  serverId: number;
  severity: NoticeSeverity;
  category: string;
  text: RichText;
  action: NoticeAction | null;
  /** Unix ms. */
  ts: number;
  /** Whether the reader has seen it (a seen notice never toasts again). */
  seen: boolean;
}

/** The history cap — a live surface, not a log. */
export const NOTIFICATION_CAP = 100;
/** How long a toast shows before it auto-dismisses (ms). */
export const TOAST_TTL_MS = 6000;
/** The most toasts shown at once. */
export const MAX_TOASTS = 3;

const SEVERITIES: readonly NoticeSeverity[] = ["info", "warn", "urgent"];

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function parseAction(raw: unknown): NoticeAction | null {
  if (isObj(raw) && typeof raw.label === "string" && typeof raw.prefill === "string") {
    return { label: raw.label, prefill: raw.prefill };
  }
  return null;
}

/** Parse a `notice` envelope payload into a `Notification` (server id and ts
 * supplied), or `null` when malformed. */
export function parseNotice(d: unknown, ts: number): Omit<Notification, "rowId" | "seen"> | null {
  if (!isObj(d) || typeof d.id !== "number" || typeof d.category !== "string") {
    return null;
  }
  const severity = SEVERITIES.find((s) => s === d.severity) ?? "info";
  if (!Array.isArray(d.text)) {
    return null;
  }
  return {
    serverId: d.id,
    severity,
    category: d.category,
    text: d.text as RichText,
    action: parseAction(d.action),
    ts,
  };
}

export class NotificationStore {
  list = $state<Notification[]>([]);

  #nextRowId = 1;

  /** Apply one parsed notice; a duplicate `serverId` in the same category is
   * ignored (a resume replay racing a fresh delivery). Returns the new
   * notification, or `null`. */
  add(parsed: Omit<Notification, "rowId" | "seen"> | null): Notification | null {
    if (!parsed) {
      return null;
    }
    if (this.list.some((n) => n.serverId === parsed.serverId && n.category === parsed.category)) {
      return null;
    }
    const n: Notification = { ...parsed, rowId: this.#nextRowId++, seen: false };
    this.list = [n, ...this.list].slice(0, NOTIFICATION_CAP);
    return n;
  }

  /** Apply a raw `notice` wire payload. */
  push(d: unknown, ts: number): Notification | null {
    return this.add(parseNotice(d, ts));
  }

  /** The compatibility shim (I11): a legacy `MailNotice` becomes a `Notice`
   * so mail populates the one surface without a server change. */
  fromMail(from: string, subject: string, mailId: number, ts: number): Notification | null {
    return this.add({
      serverId: -mailId - 1, // a namespace disjoint from server notice ids
      severity: "info",
      category: "mail",
      text: [{ text: `New mail from ${from}: ${subject}`, tag: null }],
      action: { label: "Open", prefill: `@mail ${mailId}` },
      ts,
    });
  }

  /** The newest unseen notices, for transient toasts. */
  get toasts(): Notification[] {
    return this.list.filter((n) => !n.seen).slice(0, MAX_TOASTS);
  }

  /** Count of unseen notices. */
  get unseen(): number {
    return this.list.reduce((n, x) => n + (x.seen ? 0 : 1), 0);
  }

  /** Mark one seen (it leaves the toasts but stays in the tray). */
  markSeen(rowId: number): void {
    const n = this.list.find((x) => x.rowId === rowId);
    if (n && !n.seen) {
      n.seen = true;
      this.list = [...this.list];
    }
  }

  /** Mark every notice seen (opening the tray). */
  markAllSeen(): void {
    if (this.list.some((n) => !n.seen)) {
      this.list = this.list.map((n) => (n.seen ? n : { ...n, seen: true }));
    }
  }

  /** Remove one notice entirely. */
  dismiss(rowId: number): void {
    this.list = this.list.filter((n) => n.rowId !== rowId);
  }

  clear(): void {
    this.list = [];
  }
}
