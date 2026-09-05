// GameApp — the one controller wiring WsClient → stores → components.
//
// Owns: screen state (login → character select → game), credentials
// (token in localStorage, try/catch), event routing (every envelope type),
// Req correlation (reply ids → which store the payload hydrates), the
// hydration pass on `hello`, the resume UX, local /commands, toasts, and
// — since interface cut I4 — the modal shell's actions: the numbered
// screens, the composer's focus and prefill, the command palette, the
// composing signal, and the room's exit keys.

import { login as apiLogin, logout as apiLogout, register as apiRegister } from "./auth";
import type { CharSummary, LoginOk } from "./auth";
import type { ClientCaps, OutputEvent, Query, RoomView, ViewEnvelope } from "./proto";
import {
  channelLine,
  fmtDuration,
  narrativeLine,
  plainText,
  renderRich,
  roomLines,
} from "./render";
import { exitKeys, type NavIntent } from "./keys";
import { paletteEntries, type PaletteEntry } from "./palette";
import { storageDel, storageGet, storageGetJson, storageSet, storageSetJson } from "./storage";
import { ChannelsStore } from "./stores/channels.svelte";
import { CommandsStore } from "./stores/commands.svelte";
import { LegalActionsStore } from "./stores/actions.svelte";
import { LlmDashboardStore } from "./stores/llm.svelte";
import { MailStore } from "./stores/mail.svelte";
import { PresenceStore } from "./stores/presence.svelte";
import { RollsStore } from "./stores/rolls.svelte";
import { ScreensStore, type GateInput } from "./stores/screens.svelte";
import { ScrollbackStore } from "./stores/scrollback.svelte";
import { SessionStore } from "./stores/session.svelte";
import { SheetStore } from "./stores/sheet.svelte";
import { VitalsStore } from "./stores/vitals.svelte";
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
import { parseResolutionOutcome } from "./view";

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
  | { kind: "sheet" }
  | { kind: "legal.actions" }
  | { kind: "llm_dashboard" }
  | { kind: "commands" }
  | { kind: "vitals"; feed: string }
  | { kind: "scenes" };

const HELP_LINES = [
  "Client commands (handled locally, never sent to the game):",
  "  /clear          — empty the scrollback",
  "  /theme [dark|light|system] — switch theme (no argument: toggle)",
  "  /help           — this card",
  "Keys while nothing has focus: 1-9 screens, Enter opens the box,",
  '  " : ; \\ open it with that trigger, / the palette, ? the key help,',
  "  a letter runs the screen's own command (the chips at the bottom),",
  "  n/e/s/w/u/d/o walk the room's exits on the scene screen.",
  "Inside the box: Alt+key is the nav key, Esc leaves keeping the draft,",
  "  Ctrl+K the palette. Everything else is sent to the game as typed.",
];

/** What the composer offers the shell (registered by the scene screen). */
export interface ComposerHandle {
  /** Focus the box, optionally replacing its text. */
  focus(prefill?: string): void;
  blur(): void;
  cycleMode(): void;
  text(): string;
  setText(next: string): void;
}

/**
 * The hydration queries a hello (or a full rehydrate) issues, gated on the
 * manifest: WHO, channels, mail, the sheet, legal actions, the command
 * table, every `.vitals` feed, and the scenes feed (interface plan I4).
 * Pure, so the seam is pinned without a socket.
 */
export function hydrationQueries(
  session: Pick<SessionStore, "hasPanel" | "hasFeature" | "sheetFeed" | "panels">,
): Array<{ pending: PendingReq; q: Query }> {
  const out: Array<{ pending: PendingReq; q: Query }> = [{ pending: { kind: "who" }, q: { t: "who" } }];
  if (session.hasPanel("comsys") || session.hasFeature("channels")) {
    out.push({ pending: { kind: "comsys.state" }, q: { t: "panel_subscribe", d: { feed: "comsys" } } });
  }
  if (session.hasPanel("mail") || session.hasFeature("mail")) {
    out.push({ pending: { kind: "mail.list" }, q: { t: "mail_list", d: { folder: null } } });
  }
  if (session.sheetFeed !== null) {
    out.push({ pending: { kind: "sheet" }, q: { t: "sheet" } });
  }
  out.push({ pending: { kind: "commands" }, q: { t: "commands" } });
  for (const feed of session.panels) {
    if (feed.endsWith(".vitals")) {
      out.push({ pending: { kind: "vitals", feed }, q: { t: "panel_subscribe", d: { feed } } });
    }
  }
  if (session.hasPanel("scenes")) {
    out.push({ pending: { kind: "scenes" }, q: { t: "panel_subscribe", d: { feed: "scenes" } } });
  }
  return out;
}

export class GameApp {
  session = new SessionStore();
  scrollback = new ScrollbackStore();
  channels = new ChannelsStore();
  actions = new LegalActionsStore();
  who = new WhoStore();
  mail = new MailStore();
  sheet = new SheetStore();
  rolls = new RollsStore();
  worldgen = new WorldgenStore();
  llm = new LlmDashboardStore();
  screens = new ScreensStore();
  commands = new CommandsStore();
  vitals = new VitalsStore();
  presence = new PresenceStore();

  screen = $state<Screen>("login");
  /** The latest `scenes` keyframe (the scene screen's glance rides in
   * `ambient.scene`; this is the full record for a future scene screen). */
  sceneFrame = $state<unknown>(null);
  /** The palette's current rows and highlight. */
  paletteEntries = $state<PaletteEntry[]>([]);
  paletteSelected = $state(0);
  /** The composer, while the scene screen is mounted. */
  composer: ComposerHandle | null = null;
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
  #seenActionCommits = new Set<string>();
  #actionCommitOrder: string[] = [];

  constructor() {
    this.client = new WsClient({
      caps: () => this.#caps(),
      onStatus: (s) => {
        this.session.status = s;
      },
      onFullRehydrate: () => {
        // Resume gap outgrew the ring: the server fell back to a fresh
        // attach; fresh Room output follows and the hello-pass re-requested
        // panel state (vitals and scenes included — the resume seam
        // re-subscribes them, interface plan I4). Keep old scrollback, mark
        // the seam; the mode resets to nav (D-I-17).
        this.scrollback.pushText("ev-sys ev-local", "— Reconnected (some history skipped) —");
        this.composer?.blur();
        this.screens.paletteOpen = false;
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
    this.actions.clear();
    this.who.clear();
    this.mail.clear();
    this.sheet.clear();
    this.rolls.clear();
    this.worldgen.clear();
    this.llm.clear();
    this.screens.clear();
    this.commands.clear();
    this.vitals.clear();
    this.presence.clear();
    this.sceneFrame = null;
    this.paletteEntries = [];
    this.paletteSelected = 0;
    this.session.clear();
    this.roomView = null;
    this.disconnectReason = null;
    this.#pending.clear();
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
      // The room's one-letter exits become nav keys on the scene screen
      // (D-I-19), rebuilt on every Room event.
      this.screens.exits = exitKeys(env.d);
      this.scrollback.push("ev-room", roomLines(env.d), env.ts);
    });
    c.on("composing", (env) => {
      this.presence.applyComposing(env.d.actor.name, env.d.active, Date.now());
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
    c.on("view", (env) => this.#routeView(env.d, null));
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

  /** Post-hello hydration (and the full-rehydrate seam): WHO, channel
   * tabs, mail, the sheet, the command table, every vitals feed and the
   * scenes feed — each gated on the hello's panel manifest — plus legal
   * actions through its own resync guard. */
  #hydrate(): void {
    this.#pending.clear(); // ids from a previous socket never resolve
    for (const { pending, q } of hydrationQueries(this.session)) {
      this.#req(pending, q);
    }
    if (this.session.hasPanel("legal.actions")) {
      this.#requestLegalActions();
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
      case "view":
        this.#routeView(event.d, pending);
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
        if (pending?.kind === "legal.actions") {
          this.actions.clear();
          return;
        }
        if (pending?.kind === "llm_dashboard") {
          // A Sys reply to this Req is always a refusal (staff-only gate)
          // or "no such panel" — never routed to scrollback (it would just
          // be clutter for the common "you're not staff" case); the panel
          // renders the message itself.
          this.llm.markRefused(plainText(event.d));
          return;
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
    } else if (d.feed === "llm.dashboard") {
      this.llm.applyKeyframe(d.data);
    } else if (d.feed === "commands.list") {
      this.commands.apply(d.data);
    } else if (d.feed.endsWith(".vitals")) {
      this.vitals.apply(d.feed, d.data, Date.now());
    } else if (d.feed === "scenes") {
      this.sceneFrame = d.data;
    } else if (d.feed === "npc.thinking") {
      this.presence.applyThinking(d.data, Date.now());
    }
    // Unknown feeds (future modules): ignored — additive evolution.
  }

  #routeView(view: ViewEnvelope, pending: PendingReq | null): void {
    if (view.view === "resolution.outcome") {
      const receipt = parseResolutionOutcome(view);
      if (receipt === null) return;
      if (this.#seenActionCommits.has(receipt.commit)) {
        this.client.ackCommit(receipt.commit);
        return;
      }
      this.#seenActionCommits.add(receipt.commit);
      this.#actionCommitOrder.push(receipt.commit);
      if (this.#actionCommitOrder.length > 1024) {
        const retired = this.#actionCommitOrder.shift();
        if (retired !== undefined) this.#seenActionCommits.delete(retired);
      }
      const summary = receipt.result?.summary ?? "Action committed.";
      this.scrollback.pushText("ev-sys", `${summary} Transaction: ${receipt.status}`);
      this.toast(summary);
      // ACK only after the semantic result has been validated, deduplicated,
      // and accepted into client state. A failed send leaves the gateway copy
      // live, so reconnect re-emits it and this same idempotent path retries.
      this.client.ackCommit(receipt.commit);
      return;
    }
    if (view.view !== "legal.actions") {
      return;
    }
    // A reply consumes the outstanding resync attempt before classification.
    // If the replacement is itself malformed/gapped, one new bounded request
    // may be issued rather than leaving the feed permanently wedged.
    if (pending?.kind === "legal.actions") {
      this.actions.resyncPending = false;
    }
    const alreadyPending = this.actions.resyncPending;
    if (this.actions.apply(view) === "resync" && !alreadyPending) {
      this.#req(
        { kind: "legal.actions" },
        { t: "panel_subscribe", d: { feed: "legal.actions" } },
      );
    }
  }

  #requestLegalActions(): void {
    if (!this.actions.requireResync()) {
      return;
    }
    this.#req(
      { kind: "legal.actions" },
      { t: "panel_subscribe", d: { feed: "legal.actions" } },
    );
  }

  /** Explicit refresh used by a future legal-actions control surface. */
  refreshLegalActions(): void {
    if (this.session.hasPanel("legal.actions")) {
      this.actions.resyncPending = false;
      this.#requestLegalActions();
    }
  }

  // -- the shell (interface plan I4) -----------------------------------------

  /** What the screen gates read. */
  get gate(): GateInput {
    return { panels: this.session.panels, features: this.session.features };
  }

  /** Act on a nav intent from the key model; `true` when it was consumed. */
  act(intent: NavIntent): boolean {
    switch (intent.kind) {
      case "screen":
        return this.screens.switchDigit(intent.digit, this.gate);
      case "compose":
        this.focusComposer(intent.prefill.length > 0 ? intent.prefill : undefined);
        return true;
      case "palette":
        if (this.screens.paletteOpen) {
          this.screens.paletteOpen = false;
        } else {
          const text = this.composer?.text() ?? "";
          this.focusComposer(text.length === 0 ? "/" : undefined);
          queueMicrotask(() => this.openPalette());
        }
        return true;
      case "help":
        this.screens.helpOpen = !this.screens.helpOpen;
        return true;
      case "exit":
        this.submit(intent.name);
        return true;
      case "binding":
        return this.runBinding(intent.key);
      case "pane":
      case "scroll":
      case "jump":
        // The browser scrolls and tabs on its own here.
        return false;
      default:
        return false;
    }
  }

  /** Focus the composer (switching to the scene screen when needed),
   * optionally replacing its text with `prefill`. */
  focusComposer(prefill?: string): void {
    if (this.screens.active !== "scene") {
      this.screens.switchTo("scene", this.gate);
    }
    queueMicrotask(() => this.composer?.focus(prefill));
  }

  /** Run the active screen's binding for `key` (D-I-18): a prefill opens
   * the box with the line ready, a run sends a read-only verb, a local
   * action refreshes or clears. */
  runBinding(key: string): boolean {
    const b = this.screens.binding(key, this.gate);
    if (!b) {
      return false;
    }
    switch (b.action.kind) {
      case "prefill":
        this.focusComposer(b.action.line);
        return true;
      case "run":
        this.submit(b.action.cmd);
        return true;
      case "local":
        this.localAction(b.action.action);
        return true;
      default:
        return false;
    }
  }

  /** A screen's local action. */
  localAction(action: "refresh" | "clear" | "who"): void {
    switch (action) {
      case "clear":
        this.scrollback.clear();
        return;
      case "who":
        this.refreshWho();
        return;
      case "refresh":
        switch (this.screens.active) {
          case "mail":
            this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: this.mail.folder } });
            return;
          case "sheet":
            this.refreshSheet();
            return;
          case "channels":
            this.#req({ kind: "comsys.state" }, { t: "panel_subscribe", d: { feed: "comsys" } });
            return;
          case "actions":
            this.refreshLegalActions();
            return;
          case "llm":
            this.refreshLlm();
            return;
          default:
            this.refreshWho();
        }
    }
  }

  /** Send a composing hint (D-I-11); dropped silently when offline. */
  sendComposing(active: boolean): void {
    this.client.send({ t: "composing", d: { active } });
  }

  /** Open the palette behind the composer's caret. */
  openPalette(): void {
    this.screens.paletteOpen = true;
    this.paletteQuery(this.composer?.text() ?? "");
  }

  /** Refilter the palette on the box's text (the part after the slash). */
  paletteQuery(text: string): void {
    const q = text.startsWith("/") ? text.slice(1) : text;
    this.paletteEntries = paletteEntries(q, {
      commands: this.commands.entries,
      aliases: this.channels.list.map((c) => c.alias).filter((a) => a.length > 0),
      exits: this.roomView?.exits.map((e) => e.name) ?? [],
    });
    this.paletteSelected = 0;
  }

  paletteMove(delta: number): void {
    const n = this.paletteEntries.length;
    if (n === 0) {
      return;
    }
    this.paletteSelected = Math.max(0, Math.min(n - 1, this.paletteSelected + delta));
  }

  /** Put the highlighted entry in the box (Enter), or run it (Ctrl+Enter);
   * a client-local command keeps its slash and always goes to the box. */
  palettePick(run = false): void {
    const entry = this.paletteEntries[this.paletteSelected];
    this.screens.paletteOpen = false;
    if (!entry) {
      return;
    }
    if (run && !entry.local) {
      this.composer?.setText("");
      this.submit(entry.insert);
      return;
    }
    this.composer?.focus(entry.insert);
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

  /** Re-request the viewer's sheet (the panel's Refresh button and the
   * sheet screen's `r`). Since interface cut I1 every command-side sheet
   * write republishes the keyframe, so this is a manual nudge, not a
   * liveness mechanism. No-op before a sheet feed is known. */
  refreshSheet(): void {
    if (this.session.sheetFeed === null) {
      return;
    }
    this.#req({ kind: "sheet" }, { t: "sheet" });
  }

  // -- llm dashboard ----------------------------------------------------------

  /** Re-request the staff LLM cost dashboard — the panel's mount effect,
   * its Refresh button, and its light visibility poll. No-op before the
   * `llm.dashboard` panel is known (nothing to ask for). A non-staff
   * viewer's request comes back as a polite refusal, not an error (see
   * `LlmDashboardStore.markRefused`). */
  refreshLlm(): void {
    if (!this.session.llmDashboardAvailable) {
      return;
    }
    this.llm.markRequested();
    this.#req({ kind: "llm_dashboard" }, { t: "panel_subscribe", d: { feed: "llm.dashboard" } });
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
