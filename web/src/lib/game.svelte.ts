// GameApp — the one controller wiring WsClient → stores → components.
//
// Owns: screen state (login → character select → game), credentials
// (token in localStorage, try/catch), event routing (every envelope type),
// Req correlation (reply ids → which store the payload hydrates), the
// hydration pass on `hello`, the resume UX, local /commands, and toasts.

import { login as apiLogin, logout as apiLogout, register as apiRegister } from "./auth";
import type { CharSummary, LoginOk } from "./auth";
import type { ClientCaps, OutputEvent, RichText, RoomView } from "./proto";
import {
  channelLine,
  fmtDuration,
  narrativeLine,
  plainText,
  renderRich,
  roomLines,
} from "./render";
import { storageDel, storageGet, storageGetJson, storageSet, storageSetJson } from "./storage";
import { ChannelsStore } from "./stores/channels.svelte";
import { MailStore } from "./stores/mail.svelte";
import { RollsStore } from "./stores/rolls.svelte";
import { ScrollbackStore } from "./stores/scrollback.svelte";
import { SessionStore } from "./stores/session.svelte";
import { SheetStore } from "./stores/sheet.svelte";
import { WhoStore } from "./stores/who.svelte";
import { WorldgenStore } from "./stores/worldgen.svelte";
import {
  applyTheme,
  loadThemeChoice,
  systemPrefersLight,
  toggledTheme,
  type ThemeChoice,
} from "./theme";
import { genAcceptLine, genDiscardLine, genRetryLine } from "./worldgen";
import { WsClient, type EnvelopeOf } from "./ws";

const TOKEN_KEY = "rustmush.token";
const ACCOUNT_KEY = "rustmush.account";
const CHARS_KEY = "rustmush.chars";
const LAST_CHAR_KEY = "rustmush.char";

export type Screen = "login" | "chars" | "game";

export interface Toast {
  id: number;
  text: string;
}

/** What a pending Req id will hydrate when its reply lands. */
type PendingReq =
  | { kind: "who" }
  | { kind: "comsys.state" }
  | { kind: "mail.list" }
  | { kind: "backlog"; channel: string }
  | { kind: "sheet" };

const HELP_LINES = [
  "Client commands (handled locally, never sent to the game):",
  "  /clear          — empty the scrollback",
  "  /theme [dark|light|system] — switch theme (no argument: toggle)",
  "  /help           — this card",
  "Everything else is sent to the game as typed. Try: look, say <text>,",
  '":<pose>", WHO, +help, @mail, comlist.',
];

export class GameApp {
  session = new SessionStore();
  scrollback = new ScrollbackStore();
  channels = new ChannelsStore();
  who = new WhoStore();
  mail = new MailStore();
  sheet = new SheetStore();
  rolls = new RollsStore();
  worldgen = new WorldgenStore();

  screen = $state<Screen>("login");
  /** Login screen notice (e.g. "session expired"). */
  loginNotice = $state<string | null>(null);
  account = $state<string | null>(null);
  characters = $state<CharSummary[]>([]);
  activeCharKey = $state<number | null>(null);
  roomView = $state<RoomView | null>(null);
  toasts = $state<Toast[]>([]);
  /** Server-initiated disconnect (boot/QUIT/shutdown) → modal. */
  disconnectReason = $state<string | null>(null);
  theme = $state<ThemeChoice>("system");

  readonly client: WsClient;
  #token: string | null = null;
  #pending = new Map<number, PendingReq>();
  #nextToast = 1;
  #sheetRefreshTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    this.client = new WsClient({
      caps: () => this.#caps(),
      onStatus: (s) => {
        this.session.status = s;
      },
      onFullRehydrate: () => {
        // Resume gap outgrew the ring: the server fell back to a fresh
        // attach; fresh Room output follows and the hello-pass re-requested
        // panel state. Keep old scrollback, mark the seam.
        this.scrollback.pushText("ev-sys ev-local", "— Reconnected (some history skipped) —");
        // Channel tabs have the same seam: their cached backlog may now be
        // missing the same messages, but `backlogFetched` would otherwise
        // block ever re-requesting it. Clear it (applyBacklog dedups by seq,
        // so a refetch is safe) and, if a tab is open right now, refetch it
        // immediately — closed tabs refetch on their next openChannelTab.
        this.channels.backlogFetched = {};
        if (this.channels.active !== null) {
          const meta = this.channels.byName(this.channels.active);
          if (meta) {
            this.openChannelTab(meta.name);
          }
        }
      },
      onSessionExpired: () => {
        this.#clearCreds();
        this.loginNotice = "Your session expired — please sign in again.";
        this.screen = "login";
      },
    });
    this.#wire();

    // Boot: restore theme + credentials.
    this.theme = loadThemeChoice();
    applyTheme(this.theme);
    this.#token = storageGet(TOKEN_KEY);
    this.account = storageGet(ACCOUNT_KEY);
    this.characters = storageGetJson<CharSummary[]>(CHARS_KEY) ?? [];
    if (this.#token && this.characters.length > 0) {
      this.screen = "chars";
    }
  }

  // -- auth flow ----------------------------------------------------------

  async login(account: string, password: string): Promise<void> {
    this.#adoptLogin(await apiLogin(account, password));
  }

  async register(account: string, password: string, charName: string): Promise<void> {
    this.#adoptLogin(await apiRegister(account, password, charName));
  }

  #adoptLogin(ok: LoginOk): void {
    this.#token = ok.token;
    this.account = ok.account;
    this.characters = ok.characters;
    this.loginNotice = null;
    storageSet(TOKEN_KEY, ok.token);
    storageSet(ACCOUNT_KEY, ok.account);
    storageSetJson(CHARS_KEY, ok.characters);
    const only = ok.characters.length === 1 ? ok.characters[0] : undefined;
    if (only) {
      this.enterGame(only.key); // single character: straight in
    } else {
      this.screen = "chars";
    }
  }

  /** The last character this browser played, when it is still on the list. */
  get lastCharKey(): number | null {
    const raw = storageGet(LAST_CHAR_KEY);
    const key = raw === null ? NaN : Number(raw);
    return this.characters.some((c) => c.key === key) ? key : null;
  }

  enterGame(charKey: number): void {
    if (!this.#token) {
      this.screen = "login";
      return;
    }
    storageSet(LAST_CHAR_KEY, String(charKey));
    this.activeCharKey = charKey;
    this.#resetGameState();
    this.screen = "game";
    this.client.connect(this.#token, String(charKey));
  }

  /** Back to character select (drops the connection cleanly). */
  toCharSelect(): void {
    this.client.close();
    this.disconnectReason = null;
    this.screen = "chars";
  }

  async logout(): Promise<void> {
    this.client.close();
    if (this.#token) {
      await apiLogout(this.#token);
    }
    this.#clearCreds();
    this.screen = "login";
  }

  #clearCreds(): void {
    this.#token = null;
    this.account = null;
    this.characters = [];
    this.activeCharKey = null;
    storageDel(TOKEN_KEY);
    storageDel(ACCOUNT_KEY);
    storageDel(CHARS_KEY);
  }

  /** The disconnect modal's reconnect button. */
  reconnect(): void {
    if (this.#token !== null && this.activeCharKey !== null) {
      this.disconnectReason = null;
      this.#resetGameState();
      this.client.connect(this.#token, String(this.activeCharKey));
    } else {
      this.screen = "login";
    }
  }

  #resetGameState(): void {
    this.scrollback.clear();
    this.channels.clear();
    this.who.clear();
    this.mail.clear();
    this.sheet.clear();
    this.rolls.clear();
    this.worldgen.clear();
    this.session.clear();
    this.roomView = null;
    this.disconnectReason = null;
    this.#pending.clear();
    if (this.#sheetRefreshTimer !== null) {
      clearTimeout(this.#sheetRefreshTimer);
      this.#sheetRefreshTimer = null;
    }
  }

  // -- event routing ------------------------------------------------------

  #wire(): void {
    const c = this.client;
    c.on("hello", (env) => {
      this.session.applyHello(env.d);
      this.#hydrate();
    });
    c.on("room", (env) => {
      this.roomView = env.d;
      this.scrollback.push("ev-room", roomLines(env.d), env.ts);
    });
    c.on("narrative", (env) => {
      this.scrollback.push(
        `ev-narrative kind-${env.d.kind}`,
        [narrativeLine(env.d.kind, env.d.actor ?? null, env.d.text)],
        env.ts,
      );
    });
    c.on("sys", (env) => {
      this.scrollback.push("ev-sys", [renderRich(env.d)], env.ts);
      this.#maybeRefreshSheet(env.d);
    });
    c.on("channel", (env) => this.#onChannel(env));
    c.on("who", (env) => {
      this.who.replace(env.d, Date.now());
      if (env.seq > 0) {
        // A typed WHO (session output), not panel hydration: give classic
        // scrollback feedback too.
        const names = env.d.map((e) => e.name).join(", ");
        this.scrollback.pushText(
          "ev-sys",
          `${env.d.length} player${env.d.length === 1 ? "" : "s"} connected: ${names}`,
          env.ts,
        );
      }
    });
    c.on("mail_notice", (env) => {
      this.mail.notice(env.d.unread);
      this.scrollback.pushText(
        "ev-sys",
        `MAIL: New message from ${env.d.from.name}: ${env.d.subject} (${env.d.unread} unread)`,
        env.ts,
      );
      this.toast(`New mail from ${env.d.from.name}: ${env.d.subject}`);
      if (this.mail.loaded) {
        this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: this.mail.folder } });
      }
    });
    c.on("weather", (env) => {
      this.scrollback.push("ev-weather", [renderRich(env.d.text)], env.ts);
    });
    c.on("panel", (env) => this.#routePanel(env.d, null));
    c.on("reply", (env) => {
      const pending = this.#pending.get(env.d.id);
      this.#pending.delete(env.d.id);
      this.#routeReply(env.d.event, pending ?? null, env.ts);
    });
    c.on("session_info", (env) => {
      this.session.character = env.d.character;
      this.session.account = env.d.account;
      this.session.connections = env.d.connections;
    });
    c.on("prompt", (env) => {
      this.session.prompt = env.d;
    });
    c.on("disconnect", (env) => {
      this.disconnectReason = plainText(env.d.reason) || "Disconnected.";
      // Server-side goodbye: do not fight it with auto-reconnects.
      this.client.close();
    });
  }

  #onChannel(env: EnvelopeOf<"channel">): void {
    const fresh = this.channels.live({
      channel: env.d.channel,
      seq: env.d.seq,
      actor: env.d.actor,
      title: env.d.title ?? null,
      kind: env.d.kind,
      text: env.d.text,
      ts: env.d.ts,
    });
    if (fresh) {
      // Every channel message also lands in the main scrollback,
      // classic-style; tabs are the channel-only view.
      const meta = this.channels.byName(env.d.channel);
      this.scrollback.push(
        "ev-channel",
        [
          channelLine(
            {
              channel: env.d.channel,
              actor: env.d.actor,
              title: env.d.title ?? null,
              kind: env.d.kind,
              text: env.d.text,
            },
            meta?.colorHex,
          ),
        ],
        env.ts,
      );
    }
  }

  /** Post-hello hydration: WHO + channel tabs + mail list + sheet (each
   * feature-gated on the hello's panel manifest). */
  #hydrate(): void {
    this.#pending.clear(); // ids from a previous socket never resolve
    this.#req({ kind: "who" }, { t: "who" });
    if (this.session.hasPanel("comsys") || this.session.hasFeature("channels")) {
      this.#req({ kind: "comsys.state" }, { t: "panel_subscribe", d: { feed: "comsys" } });
    }
    if (this.session.hasPanel("mail") || this.session.hasFeature("mail")) {
      this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: null } });
    }
    if (this.session.sheetFeed !== null) {
      this.#req({ kind: "sheet" }, { t: "sheet" });
    }
  }

  #req(pending: PendingReq, q: Parameters<WsClient["req"]>[0]): void {
    const id = this.client.req(q);
    if (id !== null) {
      this.#pending.set(id, pending);
    }
  }

  #routeReply(event: OutputEvent, pending: PendingReq | null, ts: number): void {
    switch (event.t) {
      case "who":
        this.who.replace(event.d, Date.now());
        return;
      case "panel":
        this.#routePanel(event.d, pending);
        return;
      case "sys":
        // Errors and polite refusals ride Sys inside the reply. Backlog
        // errors mark the channel fetched so tab opens don't loop; a Sheet
        // reply falling back to Sys means the query isn't available yet
        // (see SheetStore.markUnavailable's docs).
        if (pending?.kind === "backlog") {
          this.channels.backlogFetched[pending.channel.toLowerCase()] = true;
        }
        if (pending?.kind === "sheet") {
          this.sheet.markUnavailable();
        }
        this.scrollback.push("ev-sys", [renderRich(event.d)], ts);
        return;
      default:
        // Additive evolution: ignore payloads this client predates.
        return;
    }
  }

  #routePanel(
    d: { feed: string; kind: string; data: unknown },
    pending: PendingReq | null,
  ): void {
    if (d.feed === "comsys") {
      if (d.kind === "keyframe") {
        this.channels.applyState(d.data);
      } else {
        const merged = this.channels.applyBacklog(d.data);
        if (pending?.kind === "backlog" && merged === 0) {
          this.channels.backlogFetched[pending.channel.toLowerCase()] = true;
        }
      }
    } else if (d.feed === "mail" && d.kind === "keyframe") {
      this.mail.applyList(d.data);
    } else if (d.feed === this.session.sheetFeed) {
      // The `<ruleset>.sheet` feed: full state today (the query handler
      // always emits Keyframe — see sheet_cmds.rs's SheetQuery); applyDelta
      // is exercised once the server gains a push-on-mutation delta.
      if (d.kind === "keyframe") {
        this.sheet.applyKeyframe(d.data);
      } else {
        this.sheet.applyDelta(d.data);
      }
    } else if (d.feed === this.session.rollFeed) {
      this.rolls.push(d.data);
    } else if (d.feed === "worldgen.draft") {
      this.worldgen.applyKeyframe(d.data);
    }
    // Unknown feeds (future modules): ignored — additive evolution.
  }

  // -- input --------------------------------------------------------------

  /** Submit a line from the main input. Handles client-local /commands. */
  submit(line: string): void {
    const trimmed = line.trim();
    if (trimmed.length === 0) {
      return;
    }
    if (trimmed.startsWith("/")) {
      this.#localCommand(trimmed);
      return;
    }
    if (!this.session.masked) {
      this.scrollback.pushText("ev-echo", `> ${line}`);
    }
    if (!this.client.sendLine(line)) {
      this.scrollback.pushText("ev-sys ev-local", "Not connected — command dropped.");
    }
  }

  #localCommand(cmd: string): void {
    const [name, ...rest] = cmd.slice(1).split(/\s+/);
    const arg = rest.join(" ");
    switch ((name ?? "").toLowerCase()) {
      case "clear":
        this.scrollback.clear();
        return;
      case "theme": {
        const choice =
          arg === "dark" || arg === "light" || arg === "system"
            ? arg
            : toggledTheme(this.theme, systemPrefersLight());
        this.setTheme(choice);
        this.scrollback.pushText("ev-sys ev-local", `Theme: ${choice}.`);
        return;
      }
      case "help":
        for (const l of HELP_LINES) {
          this.scrollback.pushText("ev-sys ev-local", l);
        }
        return;
      default:
        this.scrollback.pushText(
          "ev-sys ev-local",
          `Unknown client command ${cmd.split(/\s/)[0]} — /help lists them. ` +
            "(Game commands never start with /.)",
        );
    }
  }

  setTheme(choice: ThemeChoice): void {
    this.theme = choice;
    applyTheme(choice);
  }

  /** Speak on a channel tab: wraps the member's alias verb. */
  channelSend(channel: string, text: string): void {
    const trimmed = text.trim();
    if (trimmed.length === 0) {
      return;
    }
    const meta = this.channels.byName(channel);
    const verb = meta?.alias || channel;
    if (!this.client.sendLine(`${verb} ${trimmed}`)) {
      this.scrollback.pushText("ev-sys ev-local", "Not connected — message dropped.");
    }
    // No local echo: the server delivers the channel line back to us.
  }

  /** First open of a channel tab: fetch its backlog (LOGGED channels only). */
  openChannelTab(channel: string): void {
    this.channels.activate(channel);
    const meta = this.channels.byName(channel);
    const key = channel.toLowerCase();
    if (meta?.logged && !this.channels.backlogFetched[key]) {
      this.channels.backlogFetched[key] = true; // one request per session
      this.#req(
        { kind: "backlog", channel },
        { t: "channel_backlog", d: { channel, before_seq: null, limit: 100 } },
      );
    }
  }

  /** Page older messages in a channel tab. */
  loadOlderChannel(channel: string): void {
    const oldest = this.channels.messagesOf(channel)[0];
    this.#req(
      { kind: "backlog", channel },
      {
        t: "channel_backlog",
        d: { channel, before_seq: oldest ? oldest.seq : null, limit: 100 },
      },
    );
  }

  // -- mail ---------------------------------------------------------------

  /** Switch the panel (and the server-side current folder) to `name`. */
  mailSwitchFolder(name: string): void {
    // `@mail <n>` ordinals resolve against the server-side current folder,
    // so the panel switches it first (mush-mail/src/query.rs).
    this.client.sendLine(`@mail/folder ${name}`);
    this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: name } });
  }

  /** Open one message: the body renders in scrollback (classic), then the
   * panel refreshes so the read-marker shows. */
  mailOpen(index: number): void {
    this.scrollback.pushText("ev-echo", `> @mail ${index}`);
    this.client.sendLine(`@mail ${index}`);
    setTimeout(() => {
      this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: this.mail.folder } });
    }, 400);
  }

  /** One-shot send (`@mail/quick`); multi-line compose stays a terminal
   * flow (the input-context wizard) — documented in web/README.md. */
  mailQuickSend(to: string, subject: string, body: string): void {
    const clean = (s: string) => s.replace(/[\r\n]+/g, " ").trim();
    const line = `@mail/quick ${clean(to)}=${clean(subject)}/${clean(body)}`;
    this.scrollback.pushText("ev-echo", `> ${line}`);
    this.client.sendLine(line);
    setTimeout(() => {
      this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: this.mail.folder } });
    }, 400);
  }

  refreshWho(): void {
    this.#req({ kind: "who" }, { t: "who" });
  }

  // -- sheet ----------------------------------------------------------------

  /** Re-request the viewer's sheet (the panel's Refresh button, the
   * post-mutation heuristic below, and `SheetPanel`'s light visibility
   * poll — see web/README.md for why polling exists at all). No-op before
   * a sheet feed is known (nothing to ask for). */
  refreshSheet(): void {
    if (this.session.sheetFeed === null) {
      return;
    }
    this.#req({ kind: "sheet" }, { t: "sheet" });
  }

  /**
   * A `Sys` line tagged with the loaded sheet's own ruleset id (e.g. `"wod"`
   * — `+harm`/`+heal`, splat pool spends, XP approval all tag their
   * confirmations this way; see ruleset-wod's `sys_to_actor(ctx.actor,
   * "wod", …)` call sites) plausibly means the viewer's own sheet changed,
   * so quietly refresh it (debounced — several such lines often land
   * together). This is a best-effort nudge, not the real fix: a public
   * (non-`/priv`) `+roll/wp` spends Willpower but narrates to the room
   * untagged, so it won't trigger this — `SheetPanel`'s poll and the
   * Refresh button cover that gap until the server pushes a real delta on
   * sheet mutation (see web/README.md).
   */
  #maybeRefreshSheet(text: RichText): void {
    if (!this.sheet.loaded) {
      return;
    }
    const ruleset = this.sheet.rulesetId;
    if (ruleset === null || !text.some((span) => span.tag === ruleset)) {
      return;
    }
    if (this.#sheetRefreshTimer !== null) {
      clearTimeout(this.#sheetRefreshTimer);
    }
    this.#sheetRefreshTimer = setTimeout(() => {
      this.#sheetRefreshTimer = null;
      this.refreshSheet();
    }, 350);
  }

  // -- worldgen -------------------------------------------------------------

  /** `@gen/accept` — the panel's Accept button. Writes every field of the
   * current draft through the normal attribute path and clears the draft.
   * The server has no "cleared" push (see `WorldgenStore`'s docs), so the
   * panel clears optimistically the moment the Line is sent. */
  worldgenAccept(): void {
    this.#sendGenLine(genAcceptLine());
    this.worldgen.clear();
  }

  /** `@gen/discard` — the panel's Discard button. */
  worldgenDiscard(): void {
    this.#sendGenLine(genDiscardLine());
    this.worldgen.clear();
  }

  /** `@gen/retry [guidance]` — the panel's Retry button (with the small
   * guidance input, or none). Keeps the draft on screen, marked pending,
   * until the regenerated one arrives (an off-loop LLM call — can take a
   * few seconds). */
  worldgenRetry(guidance: string): void {
    this.#sendGenLine(genRetryLine(guidance));
    this.worldgen.markPending();
  }

  #sendGenLine(line: string): void {
    this.scrollback.pushText("ev-echo", `> ${line}`);
    if (!this.client.sendLine(line)) {
      this.scrollback.pushText("ev-sys ev-local", "Not connected — command dropped.");
    }
  }

  // -- misc ---------------------------------------------------------------

  toast(text: string): void {
    const id = this.#nextToast++;
    this.toasts.push({ id, text });
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== id);
    }, 6000);
  }

  #caps(): ClientCaps {
    const w = globalThis.innerWidth ?? 1024;
    const h = globalThis.innerHeight ?? 768;
    return {
      kind: "web",
      color: "true_color",
      // Approximate cols/rows for parity with SSH caps (the web renders
      // its own layout; the server currently ignores these).
      width: Math.max(20, Math.floor(w / 8)),
      height: Math.max(10, Math.floor(h / 18)),
      utf8: true,
      structured: true,
    };
  }

  /** Header helper: connected time formatting for the session pill. */
  fmtConn(seconds: number): string {
    return fmtDuration(seconds);
  }
}
