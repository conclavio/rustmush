// Channels — tab metadata + per-channel message lists + unread math.
//
// Hydrated from the `comsys.state` panel keyframe (via
// `Req { q: panel_subscribe { feed: "comsys" } }` — the subscription ack IS
// the keyframe), backfilled per tab from `comsys.backlog` deltas, and fed
// live by `channel` envelopes.
//
// Unread model: the server's `last_read` cursor advances on live delivery
// (comsys/src/speak.rs), so for a connected session the *server* unread is
// almost always 0 — meaningful unread is client-side: `readSeq` records the
// channel seq last seen with the tab active; `unread = seq - readSeq`.
// There is no read-marking Line command (cursors are delivery-driven), so
// read state is client-local by design — documented in web/README.md.

import type { NarrativeKind, ObjRef, RichText } from "../proto";

/** One joined channel, from the comsys.state keyframe. */
export interface ChannelMeta {
  /** Channel display name (also the message-matching key). */
  name: string;
  /** The member's personal alias — the speak verb (`pub Hello`). */
  alias: string;
  /** Comsys title, when set. */
  title: string | null;
  gagged: boolean;
  /** Web color, e.g. `"#8cc265"`, when the channel defines one. */
  colorHex: string | null;
  /** Channel message seq (highest seen). */
  seq: number;
  /** Highest seq seen while this tab was active (client-side read cursor). */
  readSeq: number;
  loud: boolean;
  /** Whether the channel keeps history (gates backlog requests). */
  logged: boolean;
}

/** One channel message — live `channel` envelopes and backlog entries share
 * this shape (comsys serves the backlog with the live frame's field names). */
export interface ChanMsg {
  /**
   * Store-assigned monotonic id — ChannelView's `#each` key. `seq` is NOT
   * safe for that: it defaults to 0 on a malformed/missing wire value (see
   * `parseMsg`/`num`), and two such messages would give Svelte a duplicate
   * key and crash the whole app (`each_key_duplicate`). Mirrors
   * ScrollbackStore's `id`.
   */
  id: number;
  seq: number;
  actor: ObjRef;
  title: string | null;
  kind: NarrativeKind;
  text: RichText;
  ts: number;
}

/** Per-channel message cap (tab view only; scrollback has its own ring). */
export const CHANNEL_MSG_CAP = 500;

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

/** Defensive parse of one backlog/live message object (`id` is assigned by
 * the store on insert — see `ChannelsStore.#nextMsgId`). */
function parseMsg(v: unknown): Omit<ChanMsg, "id"> | null {
  if (typeof v !== "object" || v === null) {
    return null;
  }
  const o = v as Record<string, unknown>;
  const actor = o.actor as Record<string, unknown> | undefined;
  if (typeof actor !== "object" || actor === null) {
    return null;
  }
  const kind = str(o.kind) ?? "say";
  const text = Array.isArray(o.text) ? (o.text as RichText) : [];
  return {
    seq: num(o.seq),
    actor: { id: str(actor.id) ?? "#-1", name: str(actor.name) ?? "?" },
    title: str(o.title),
    kind: kind as NarrativeKind,
    text,
    ts: num(o.ts),
  };
}

export class ChannelsStore {
  list = $state<ChannelMeta[]>([]);
  /** Per-channel messages (key: lowercased channel name), oldest first. */
  messages = $state<Record<string, ChanMsg[]>>({});
  /** Active tab (lowercased channel name), or `null` for main scrollback. */
  active = $state<string | null>(null);
  /** Channels whose backlog was already requested (avoid re-fetch loops). */
  backlogFetched = $state<Record<string, boolean>>({});

  /** Next `ChanMsg.id` (Svelte `#each` key), across all channels. */
  #nextMsgId = 1;

  #key(name: string): string {
    return name.toLowerCase();
  }

  byName(name: string): ChannelMeta | undefined {
    const k = this.#key(name);
    return this.list.find((c) => this.#key(c.name) === k);
  }

  messagesOf(name: string): ChanMsg[] {
    return this.messages[this.#key(name)] ?? [];
  }

  unreadOf(name: string): number {
    const c = this.byName(name);
    return c ? Math.max(0, c.seq - c.readSeq) : 0;
  }

  get totalUnread(): number {
    return this.list.reduce((n, c) => n + Math.max(0, c.seq - c.readSeq), 0);
  }

  /**
   * Apply a `comsys.state` keyframe (`{ channels: [...] }`). Existing
   * client-side read cursors survive (a re-hydrate must not mark everything
   * read or unread); new channels start read up to the server cursor.
   */
  applyState(data: unknown): void {
    const channels =
      typeof data === "object" && data !== null
        ? (data as Record<string, unknown>).channels
        : null;
    if (!Array.isArray(channels)) {
      return;
    }
    const next: ChannelMeta[] = [];
    for (const raw of channels) {
      if (typeof raw !== "object" || raw === null) {
        continue;
      }
      const o = raw as Record<string, unknown>;
      const name = str(o.name);
      if (!name) {
        continue;
      }
      const color =
        typeof o.color === "object" && o.color !== null
          ? (o.color as Record<string, unknown>)
          : {};
      const seq = num(o.seq);
      const serverRead = num(o.last_read, seq);
      const existing = this.byName(name);
      next.push({
        name,
        alias: str(o.alias) ?? name,
        title: str(o.title),
        gagged: o.gagged === true,
        colorHex: str(color.web_hex),
        seq: Math.max(seq, existing?.seq ?? 0),
        // Keep the local cursor when we already track this channel; else
        // trust the server's (delivery-driven) cursor.
        readSeq: existing ? existing.readSeq : Math.min(serverRead, seq),
        loud: o.loud === true,
        logged: o.logged === true,
      });
    }
    next.sort((a, b) => a.name.localeCompare(b.name));
    this.list = next;
    // Drop message lists of channels we left.
    const keep = new Set(next.map((c) => this.#key(c.name)));
    for (const k of Object.keys(this.messages)) {
      if (!keep.has(k)) {
        delete this.messages[k];
        delete this.backlogFetched[k];
      }
    }
    if (this.active !== null && !keep.has(this.active)) {
      this.active = null;
    }
  }

  /**
   * One live `channel` envelope. Returns `false` when the message was a
   * duplicate (resume replay racing a fresh keyframe) and should not be
   * re-echoed to scrollback. `id` is assigned by the store on insert (see
   * `#nextMsgId`), not supplied by the caller.
   */
  live(msg: Omit<ChanMsg, "id"> & { channel: string }): boolean {
    const k = this.#key(msg.channel);
    let meta = this.byName(msg.channel);
    if (!meta) {
      // Joined mid-session (addcom) before the next keyframe: stub it.
      meta = {
        name: msg.channel,
        alias: msg.channel,
        title: null,
        gagged: false,
        colorHex: null,
        seq: 0,
        readSeq: msg.seq - 1,
        loud: false,
        logged: false,
      };
      this.list.push(meta);
      this.list.sort((a, b) => a.name.localeCompare(b.name));
    }
    const listRef = this.messages[k] ?? (this.messages[k] = []);
    if (msg.seq > 0 && listRef.some((m) => m.seq === msg.seq)) {
      return false; // replayed duplicate
    }
    listRef.push({
      id: this.#nextMsgId++,
      seq: msg.seq,
      actor: msg.actor,
      title: msg.title,
      kind: msg.kind,
      text: msg.text,
      ts: msg.ts,
    });
    if (listRef.length > CHANNEL_MSG_CAP) {
      listRef.splice(0, listRef.length - CHANNEL_MSG_CAP);
    }
    if (msg.seq > meta.seq) {
      meta.seq = msg.seq;
    }
    if (this.active === k) {
      meta.readSeq = meta.seq; // reading it right now
    }
    return true;
  }

  /**
   * Merge a `comsys.backlog` delta (`{ backlog: { channel, messages:
   * [newest-first] } }`) into the channel's list (dedup by seq, ascending).
   * Returns the number of new messages merged.
   */
  applyBacklog(data: unknown): number {
    const backlog =
      typeof data === "object" && data !== null
        ? (data as Record<string, unknown>).backlog
        : null;
    if (typeof backlog !== "object" || backlog === null) {
      return 0;
    }
    const b = backlog as Record<string, unknown>;
    const name = str(b.channel);
    if (!name || !Array.isArray(b.messages)) {
      return 0;
    }
    const k = this.#key(name);
    const existing = this.messages[k] ?? [];
    const bySeq = new Map<number, ChanMsg>();
    for (const m of existing) {
      bySeq.set(m.seq, m);
    }
    let merged = 0;
    for (const raw of b.messages) {
      const m = parseMsg(raw);
      if (m && !bySeq.has(m.seq)) {
        bySeq.set(m.seq, { ...m, id: this.#nextMsgId++ });
        merged += 1;
      }
    }
    const all = [...bySeq.values()].sort((a, z) => a.seq - z.seq);
    if (all.length > CHANNEL_MSG_CAP) {
      all.splice(0, all.length - CHANNEL_MSG_CAP);
    }
    this.messages[k] = all;
    this.backlogFetched[k] = true;
    return merged;
  }

  /** Switch tabs; the newly active channel is marked read. */
  activate(name: string | null): void {
    this.active = name === null ? null : this.#key(name);
    if (name !== null) {
      const meta = this.byName(name);
      if (meta) {
        meta.readSeq = meta.seq;
      }
    }
  }

  clear(): void {
    this.list = [];
    this.messages = {};
    this.backlogFetched = {};
    this.active = null;
  }
}
