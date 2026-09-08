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
import type {
  ClientCaps,
  NarrativeKind,
  ObjRef,
  OutputEvent,
  Query,
  RespondChoice,
  RichText,
  RoomView,
  ViewEnvelope,
} from "./proto";
import {
  channelLine,
  fmtDuration,
  groupsBySpeaker,
  isRollNarration,
  narrativeBody,
  narrativeClasses,
  narrativeLine,
  plainText,
  renderRich,
  roomLines,
} from "./render";
import { isOwnRoll } from "./roll";
import { exitKeys, type NavIntent } from "./keys";
import { paletteEntries, type PaletteEntry } from "./palette";
import { storageDel, storageGet, storageGetJson, storageSet, storageSetJson } from "./storage";
import { ChannelsStore } from "./stores/channels.svelte";
import { CommandsStore } from "./stores/commands.svelte";
import { LegalActionsStore } from "./stores/actions.svelte";
import { GenericViewStore, isViewFeed } from "./stores/genericview.svelte";
import { RequestStore } from "./stores/requests.svelte";
import { NotificationStore } from "./stores/notifications.svelte";
import { LlmDashboardStore } from "./stores/llm.svelte";
import { MailStore } from "./stores/mail.svelte";
import { PresenceStore } from "./stores/presence.svelte";
import { RollsStore } from "./stores/rolls.svelte";
import { ScreensStore, UNIT_SCREEN, type GateInput } from "./stores/screens.svelte";
import { ScrollbackStore, type SbOlderBlock } from "./stores/scrollback.svelte";
import { SessionStore } from "./stores/session.svelte";
import { SheetStore } from "./stores/sheet.svelte";
import { CONTACTS_FEED, UNIT_RESYNC_MS, UnitStore } from "./stores/unit.svelte";
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
import { WsClient, type EnvelopeOf, type WebSocketLike } from "./ws";
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
  | { kind: "narrative_backlog" }
  | { kind: "sheet" }
  | { kind: "legal.actions" }
  | { kind: "llm_dashboard" }
  | { kind: "commands" }
  | { kind: "vitals"; feed: string }
  | { kind: "view"; feed: string }
  | { kind: "unit"; feed: string }
  | { kind: "contacts" }
  | { kind: "scenes" };

const HELP_LINES = [
  "Client commands (handled locally, never sent to the game):",
  "  /clear          — empty the scrollback (and draw a floor under it)",
  "  /history        — load the page of scene history above what you see",
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
  session: Pick<SessionStore, "hasPanel" | "hasFeature" | "sheetFeed" | "unitFeed" | "panels">,
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
    } else if (feed.endsWith(".view")) {
      out.push({ pending: { kind: "view", feed }, q: { t: "panel_subscribe", d: { feed } } });
    }
  }
  // Two discrete arms (interface cut I8), deliberately AFTER the suffix loop
  // and BEFORE `scenes`: the loop's order is pinned byte-for-byte, so a
  // `.unit` feed must never interleave into it.
  if (session.unitFeed !== null) {
    out.push({
      pending: { kind: "unit", feed: session.unitFeed },
      q: { t: "panel_subscribe", d: { feed: session.unitFeed } },
    });
  }
  if (session.hasPanel(CONTACTS_FEED)) {
    out.push({
      pending: { kind: "contacts" },
      q: { t: "panel_subscribe", d: { feed: CONTACTS_FEED } },
    });
  }
  if (session.hasPanel("scenes")) {
    out.push({ pending: { kind: "scenes" }, q: { t: "panel_subscribe", d: { feed: "scenes" } } });
  }
  return out;
}

/**
 * Construction seams. Production passes nothing (`App.svelte` builds a bare
 * `new GameApp()`); the vitest harness passes a fake socket factory so the
 * controller can be driven without a browser or a server — the transport's
 * own seams are `WsClientOptions`, and every field here is forwarded to it
 * unchanged, including `undefined` (WsClient reads all three with `??`).
 */
export interface GameAppOptions {
  /** WS endpoint override; defaults to same-origin `/ws` (see WsClient). */
  url?: string;
  /** Test seam: WebSocket constructor. Defaults to the DOM `WebSocket`. */
  webSocketFactory?: (url: string, protocols?: string[]) => WebSocketLike;
  /** Test seam: RNG for reconnect backoff jitter. */
  random?: () => number;
}

export class GameApp {
  session = new SessionStore();
  scrollback = new ScrollbackStore();
  channels = new ChannelsStore();
  actions = new LegalActionsStore();
  who = new WhoStore();
  mail = new MailStore();
  sheet = new SheetStore();
  unit = new UnitStore();
  rolls = new RollsStore();
  worldgen = new WorldgenStore();
  llm = new LlmDashboardStore();
  screens = new ScreensStore();
  commands = new CommandsStore();
  vitals = new VitalsStore();
  presence = new PresenceStore();
  genericViews = new GenericViewStore();
  requests = new RequestStore();
  notifications = new NotificationStore();

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
  /** The armed unit-resync coalescer (interface cut I8); see
   * `#scheduleUnitResync`. Cleared on every reset seam — a timer left armed
   * across a disconnect would latch `UnitStore.resyncPending` forever. */
  #unitResyncTimer: ReturnType<typeof setTimeout> | null = null;
  /** The in-flight unit request was armed by the coalescer, not by `r` — so
   * a refusal answering it earns ONE bounded retry. */
  #unitResyncFromDelta = false;
  #nextToast = 1;
  #seenActionCommits = new Set<string>();
  #actionCommitOrder: string[] = [];

  constructor(opts: GameAppOptions = {}) {
    this.client = new WsClient({
      url: opts.url,
      webSocketFactory: opts.webSocketFactory,
      random: opts.random,
      caps: () => this.#caps(),
      onStatus: (s) => {
        this.session.status = s;
      },
      onFullRehydrate: () => {
        // Resume gap outgrew the ring: the server fell back to a fresh
        // attach; fresh Room output follows and the hello-pass re-requested
        // panel state (vitals and scenes included — the resume seam
        // re-subscribes them, interface plan I4). Keep old scrollback, mark
        // the seam; the mode resets to nav (D-I-17). The fresh attach has a
        // fresh resume ring and seq namespace, so start a new session segment
        // first (interface plan I12): old blocks lose their seqs — never the
        // cursor or de-dup key again — and later pages land at the seam.
        this.scrollback.beginSegment();
        this.scrollback.pushText("ev-sys ev-local", "— Reconnected (some history skipped) —");
        // A revision only means anything within the run of the world that
        // minted it (I1-C4): the world may have restarted inside the gap,
        // so the held watermarks accept the next frame per feed. Frames stay
        // on screen — the strip is stale, not gone.
        this.vitals.resetWatermarks();
        this.composer?.blur();
        this.screens.paletteOpen = false;
        // The unit panel is revision state against a subject that may have
        // changed while we were away: drop the view, the journal ring and
        // any armed resync, and let the hello pass re-subscribe (I8).
        this.#clearUnitResync();
        this.unit.clear();
        // The server's request registry is RAM-only (I10, D-I-22), so a full
        // re-attach means any pending request is gone server-side and its id
        // may be reminted for something else — drop it so a stale answer is
        // never sent.
        this.requests.clear();
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
    this.#clearUnitResync();
    // A re-attach is a FRESH VIEW, not a `/clear`: `clear()` draws the
    // backlog floor (LB-1) and `beginSegment` lifts it again for the new
    // session's ring, which the gateway primes from the scene the way the
    // TUI's `@notui`/`@tui` does (`docs/help/tui.md` "Scene history").
    this.scrollback.clear();
    this.scrollback.beginSegment();
    this.channels.clear();
    this.actions.clear();
    this.who.clear();
    this.mail.clear();
    this.sheet.clear();
    this.unit.clear();
    this.rolls.clear();
    this.worldgen.clear();
    this.llm.clear();
    this.screens.clear();
    // …and the registered feed screens with it (LB-2): they belong to the
    // manifest of the server this session is leaving, and a `.view` screen
    // still holding digit 6 would silently refuse the next server's unit
    // screen. NOT part of `screens.clear()` — the bare-hello path relies on
    // a re-hello finding its screen already registered.
    this.screens.clearFeedScreens();
    this.commands.clear();
    this.vitals.clear();
    this.genericViews.clear();
    this.requests.clear();
    this.notifications.clear();
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
      // A hello is a new run of the world as far as this client can tell —
      // the gauges it holds were numbered by the previous one (I1-C4).
      // BEFORE `#hydrate`, so the keyframes it asks for are the frames the
      // marks buy.
      this.vitals.resetWatermarks();
      this.#hydrate();
    });
    c.on("resumed", (env) => {
      // The gateway's reconnected seam, replayed half: the gap came back
      // from the resume buffer, so there is no rehydrate and no fresh hello
      // — this is the ONLY notice the client gets that it was away while
      // the world may have restarted (I1-C4). The replayed frames arrive in
      // order, so the first re-seeds each feed and the rest compare
      // normally. The `replayed: false` half is `onFullRehydrate`, which
      // runs before this handler and resets there.
      if (env.d.replayed) {
        this.vitals.resetWatermarks();
      }
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
      // Speech and action group under a speaker header (I5); emits, OOC
      // and the one-line roll narration stand alone.
      const actor = env.d.actor ?? null;
      const roll = env.d.kind === "emit" && isRollNarration(env.d.text);
      const groupable = actor !== null && groupsBySpeaker(env.d.kind);
      this.scrollback.push(
        narrativeClasses(env.d.kind, roll),
        [narrativeLine(env.d.kind, actor, env.d.text)],
        env.ts,
        {
          speaker: actor ? { id: actor.id, name: actor.name } : null,
          body: groupable ? [narrativeBody(env.d.kind, env.d.text)] : null,
          groupable,
          seq: env.seq,
        },
      );
    });
    c.on("sys", (env) => {
      this.scrollback.push("ev-sys ev-demoted", [renderRich(env.d)], env.ts);
    });
    c.on("channel", (env) => this.#onChannel(env));
    c.on("who", (env) => {
      this.who.replace(env.d, Date.now());
      if (env.seq > 0) {
        // A typed WHO (session output), not panel hydration: give classic
        // scrollback feedback too.
        const names = env.d.map((e) => e.name).join(", ");
        this.scrollback.pushText(
          // A system line the GAME sent steps back (I5-C14): small mono, less
          // contrast, still there. The only undemoted system lines are the
          // client-local ones, which are italic instead.
          "ev-sys ev-demoted",
          `${env.d.length} player${env.d.length === 1 ? "" : "s"} connected: ${names}`,
          env.ts,
        );
      }
    });
    c.on("mail_notice", (env) => {
      this.mail.notice(env.d.unread);
      // The unified notification surface (I11) shows the arrival — the tray is
      // its home. The old scrollback line and ad-hoc toast are retired; the
      // MailNotice event still carries the unread count for the mail badge (a
      // client-side shim until mail emits a Notice server-side).
      this.notifications.fromMail(env.d.from.name, env.d.subject, env.d.id, env.ts);
      if (this.mail.loaded) {
        this.#req({ kind: "mail.list" }, { t: "mail_list", d: { folder: this.mail.folder } });
      }
    });
    c.on("notice", (env) => {
      // A unified notification (I11): mail, a page, a mention, an escalation,
      // a job update — one surface. Never idle-stamps (it is delivered).
      this.notifications.push(env.d, env.ts);
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
    c.on("request", (env) => {
      // A server-initiated interaction (I10): hold it; RequestModal shows the
      // oldest and the answer goes back by id.
      this.requests.push(env.d);
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
      // An IC channel (I7) reads as prose; OOC chatter is demoted.
      this.scrollback.push(
        meta?.ic ? "ev-channel ev-channel-ic ev-prose" : "ev-channel ev-demoted",
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
    // Likewise a backlog page requested on a previous socket never arrives
    // (I12): release the in-flight latch on every hello. Same rule for the
    // unit feed's coalesced resync (I8) — a request armed on a previous
    // socket is never answered, and its guard would drop every later delta.
    this.scrollback.fetchingBacklog = false;
    this.unit.resyncPending = false;
    // The tactical unit screen (I8) registers here, ONCE: `registerFeedScreen`
    // has no idempotency guard of its own and `ScreensStore.clear()` keeps
    // `feedScreens`, so a second hello would otherwise be refused with
    // "screen unit declared twice". Registering only where the manifest
    // carries a `.unit` feed keeps digit 6 free for I9's view screens on
    // every other server.
    if (
      this.session.unitFeed !== null &&
      !this.screens.feedScreens.some((f) => f.def.id === UNIT_SCREEN.def.id)
    ) {
      this.#screenRefused(UNIT_SCREEN.def.label, this.screens.registerFeedScreen(UNIT_SCREEN));
    }
    for (const { pending, q } of hydrationQueries(this.session)) {
      this.#req(pending, q);
    }
    if (this.session.hasPanel("legal.actions")) {
      this.#requestLegalActions();
    }
  }

  /**
   * Surface a refused screen registration (LB-2). Both registrars return a
   * reason and both call sites used to DISCARD it, so a collision — a
   * `.view` screen already holding digit 6 when the unit screen arrives, or
   * the reverse — was invisible: the screen simply never appeared and
   * nothing said why. A client-local line is the right home; the server
   * never sees this and there is nothing to retry.
   */
  #screenRefused(what: string, reason: string | null): void {
    if (reason === null) {
      return;
    }
    this.scrollback.pushText(
      "ev-sys ev-local",
      `This client could not add the ${what} screen: ${reason}.`,
    );
  }

  /** Send a correlated request. Returns whether the frame actually went out
   * (a closed socket sends nothing), so a caller never latches an in-flight
   * flag on a request that was never made. */
  #req(pending: PendingReq, q: Parameters<WsClient["req"]>[0]): boolean {
    const id = this.client.req(q);
    if (id !== null) {
      this.#pending.set(id, pending);
      return true;
    }
    return false;
  }

  #routeReply(event: OutputEvent, pending: PendingReq | null, ts: number): void {
    if (pending?.kind === "sheet") {
      // ANY reply to a sheet Req releases the I13 resync storm guard before
      // classification — a keyframe re-seeds, and a refusal/odd reply must
      // not leave every later gap delta dropped silently.
      this.sheet.resyncPending = false;
    }
    if (pending?.kind === "unit") {
      // Same rule for the unit feed's coalesced resync (I8): ANY reply
      // releases the guard before classification, so a refusal can never
      // leave every later journal delta dropped silently.
      this.unit.resyncPending = false;
    }
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
        if (pending?.kind === "sheet" && !this.sheet.loaded) {
          // Only blank the panel when nothing was ever loaded: a transient
          // Sys (a throttle, a commit-pending refusal) must not wipe a live
          // sheet. The resync storm guard was released above regardless.
          this.sheet.markUnavailable();
        }
        if (pending?.kind === "narrative_backlog") {
          // A throttle/refusal reply: clear the in-flight flag so a later
          // scroll-to-top can retry; no scrollback clutter.
          this.scrollback.fetchingBacklog = false;
          return;
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
        if (pending?.kind === "unit") {
          // The unit feed is in every viewer's manifest, so a viewer with no
          // station gets the refusal here — it belongs in the panel, not in
          // scrollback (the llm.dashboard precedent above). While a view IS
          // held, a transient Sys is ignored rather than blanking it.
          if (!this.unit.loaded) {
            this.unit.markUnavailable(plainText(event.d));
          }
          // A refusal that answered a COALESCED resync (a throttle at the
          // end of a burst) would otherwise leave the pane behind the
          // journal until the next delta: retry once, further out.
          this.#scheduleUnitRetry();
          return;
        }
        if (pending?.kind === "contacts") {
          // The embedded contacts table renders its own empty state; an
          // unwired propagation layer is not scrollback clutter.
          return;
        }
        // The generic reply-error line, demoted like every other
        // server-driven system line (I5-C14).
        this.scrollback.push("ev-sys ev-demoted", [renderRich(event.d)], ts);
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
    if (d.feed === "narrative") {
      // A narrative backlog page (interface plan I12): older room lines this
      // viewer already saw, from the server's per-viewer resume ring. Render
      // and prepend, deduped by seq against the live scrollback.
      this.#applyNarrativeBacklog(d.data);
    } else if (d.feed === "comsys") {
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
      // The `<ruleset>.sheet` feed (interface plan I13): a keyframe replaces
      // whole state and seeds the revision watermark; a delta applies only on
      // a matching base_revision, else self-heals with a fresh keyframe (rule
      // 3), storm-guarded so one gap fires one refresh.
      if (d.kind === "keyframe") {
        this.sheet.applyKeyframe(d.data);
      } else if (this.sheet.applyDelta(d.data) === "resync" && !this.sheet.resyncPending) {
        this.sheet.resyncPending = true;
        this.refreshSheet();
      }
    } else if (d.feed === this.session.unitFeed) {
      // The `<module>.unit` feed (interface cut I8): a keyframe replaces the
      // whole view and seeds the watermark; a journal delta always rings,
      // and asks for a fresh keyframe only when it runs ahead of the
      // watermark — coalesced at UNIT_RESYNC_MS so a damage burst costs ONE
      // request, and storm-guarded so an armed one is never doubled.
      if (d.kind === "keyframe") {
        this.unit.applyKeyframe(d.data);
        // The burst was answered with a frame; there is nothing to retry.
        this.#unitResyncFromDelta = false;
      } else if (this.unit.applyJournal(d.data) === "resync" && !this.unit.resyncPending) {
        this.unit.resyncPending = true;
        this.#scheduleUnitResync();
      }
    } else if (d.feed === CONTACTS_FEED) {
      // An I9-convention list embedded in the unit screen rather than owning
      // a `.view` feed — `GenericViewStore` keys on the feed name, not the
      // suffix, so holding it here claims no screen digit.
      this.genericViews.apply(d.feed, d.data);
    } else if (d.feed.endsWith(".roll")) {
      // The roller's own roll also lands inline as a dice card (I5); the
      // room's one-line narration is what everyone else reads.
      //
      // EVERY `.roll` feed, not just `session.rollFeed` (LB-21): a world
      // with two rulesets loaded publishes two, the tie-break picked one,
      // and the other ruleset's rolls were silently dropped. The TUI has
      // always rendered every `.roll` feed (`panels.rs`).
      const entry = this.rolls.push(d.data);
      if (entry && isOwnRoll(entry.who, this.session.character?.id ?? null)) {
        this.scrollback.pushRoll(entry);
      }
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
    } else if (isViewFeed(d.feed)) {
      // A `<name>.view` generic view (I9): hold it and make sure a screen
      // exists for it — a module ships a view and gets a screen with no
      // bespoke code.
      if (this.genericViews.apply(d.feed, d.data)) {
        this.#screenRefused(d.feed, this.screens.registerViewScreen(d.feed));
      }
    }
    // Unknown feeds (future modules): ignored — additive evolution.
  }

  /** Fetch the next older narrative backlog page (interface plan I12) — the
   * scroll-to-top affordance calls this. One page in flight at a time; stops
   * once the server reports no more history below the buffer. */
  loadOlderNarrative(): void {
    if (this.scrollback.fetchingBacklog || !this.scrollback.moreBacklog) {
      return;
    }
    // Latch only once the frame actually went out: a request never sent (a
    // closed socket) must not wedge every later scroll-to-top.
    this.scrollback.fetchingBacklog = this.#req(
      { kind: "narrative_backlog" },
      {
        t: "narrative_backlog",
        d: { before_seq: this.scrollback.oldestNarrativeSeq, limit: 200 },
      },
    );
  }

  #applyNarrativeBacklog(data: unknown): void {
    this.scrollback.fetchingBacklog = false;
    const bl = (
      data as { backlog?: { more?: boolean; evicted?: boolean; lines?: unknown[] } }
    )?.backlog;
    if (!bl || !Array.isArray(bl.lines)) {
      this.scrollback.moreBacklog = false;
      return;
    }
    // The wire is newest-first; `prependOlder` wants oldest-first at the seam.
    const older = bl.lines
      .slice()
      .reverse()
      .map((l) => this.#renderBacklogLine(l))
      .filter((b): b is SbOlderBlock => b !== null);
    this.scrollback.prependOlder(older, bl.more === true, bl.evicted === true);
  }

  #renderBacklogLine(raw: unknown): SbOlderBlock | null {
    const l = raw as {
      seq?: number;
      ts?: number;
      kind?: NarrativeKind;
      actor?: ObjRef | null;
      text?: RichText;
    };
    if (typeof l?.seq !== "number" || l.kind === undefined || !Array.isArray(l.text)) {
      return null;
    }
    const actor = l.actor ?? null;
    const roll = l.kind === "emit" && isRollNarration(l.text);
    const groupable = actor !== null && groupsBySpeaker(l.kind);
    return {
      seq: l.seq,
      ts: typeof l.ts === "number" ? l.ts : Date.now(),
      cls: narrativeClasses(l.kind, roll),
      lines: [narrativeLine(l.kind, actor, l.text)],
      speaker: actor ? { id: actor.id, name: actor.name } : null,
      body: groupable ? [narrativeBody(l.kind, l.text)] : null,
      groupable,
    };
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
      this.scrollback.pushText("ev-sys ev-demoted", `${summary} Transaction: ${receipt.status}`);
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
  localAction(action: "refresh" | "clear" | "who" | "toggle"): void {
    switch (action) {
      case "clear":
        this.scrollback.clear();
        return;
      case "who":
        this.refreshWho();
        return;
      case "toggle":
        // The unit screen hosts two layouts on one digit (D-I8-4). This
        // needs its own arm: the `refresh` switch below falls through to
        // `refreshWho()` for anything it does not name.
        this.unit.toggleLayout();
        return;
      case "refresh": {
        // A generic-view screen (I9) has no bespoke arm below, and the
        // `default:` is `refreshWho()` — so `r` on a module's own view used
        // to refresh WHO (LB-3). Its feed IS its refresh, and the screen
        // definition already carries it, so this stays data-driven the way
        // the TUI's `screen_queries()` is.
        const viewFeed = this.screens.activeDef(this.gate).viewFeed;
        if (viewFeed !== undefined) {
          this.#req(
            { kind: "view", feed: viewFeed },
            { t: "panel_subscribe", d: { feed: viewFeed } },
          );
          return;
        }
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
          case "rolls":
            this.refreshVitals();
            return;
          case "unit":
            this.refreshUnit();
            this.refreshContacts();
            return;
          default:
            this.refreshWho();
        }
      }
    }
  }

  /** Re-subscribe every `.vitals` feed (the rolls screen's board, I6). A
   * lower revision in the answer is ignored by the store. */
  refreshVitals(): void {
    for (const feed of this.session.panels) {
      if (feed.endsWith(".vitals")) {
        this.#req({ kind: "vitals", feed }, { t: "panel_subscribe", d: { feed } });
      }
    }
  }

  /** Send a composing hint (D-I-11); dropped silently when offline. */
  sendComposing(active: boolean): void {
    this.client.send({ t: "composing", d: { active } });
  }

  /** Answer a server-initiated request (I10, D-I-21): report the choice by
   * id and drop it locally. The server owns the effect. */
  respond(id: number, choice: RespondChoice): void {
    this.client.send({ t: "respond", d: { id, choice } });
    this.requests.remove(id);
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

  /**
   * `?` on the highlighted palette row: ask for that command's help page
   * (I4-C23). The palette closes and the answer lands in scrollback.
   *
   * `help <verb>` reads state and nothing else, so this is a bare key
   * sending a read-only verb exactly as D-I-18 allows — `help` is already on
   * `RUN_ALLOWLIST`. A client-local row has no server help page (`/help` is
   * its card), so it gets that instead.
   */
  paletteHelp(): void {
    const entry = this.paletteEntries[this.paletteSelected];
    this.screens.paletteOpen = false;
    if (!entry) {
      return;
    }
    if (entry.local) {
      this.submit("/help");
      return;
    }
    const verb = entry.insert.trim().split(/\s+/)[0] ?? "";
    if (verb.length > 0) {
      this.submit(`help ${verb}`);
    }
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
      case "history":
        // The same verb the TUI and line mode have had since I12 (LB-20).
        // Below a `/clear` floor, or once the server's ring is exhausted,
        // there is nothing to ask for — say so rather than doing nothing
        // visible, which reads as a broken command.
        if (this.scrollback.moreBacklog) {
          this.loadOlderNarrative();
        } else {
          this.scrollback.pushText(
            "ev-sys ev-local",
            this.scrollback.backlogEvicted
              ? "Older history has rolled off the server's buffer."
              : "No older history in this view.",
          );
        }
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

  /** Re-request the viewer's sheet (the panel's Refresh button, the sheet
   * screen's `r`, and the I13 self-heal after a delta base gap). Since
   * interface cut I1 every command-side sheet write republishes — as a
   * keyframe then, an incremental delta since I13 — so this is a manual
   * nudge and the rule-3 recovery path, not a liveness mechanism. No-op
   * before a sheet feed is known. */
  refreshSheet(): void {
    if (this.session.sheetFeed === null) {
      return;
    }
    this.#req({ kind: "sheet" }, { t: "sheet" });
  }

  // -- tactical unit (interface cut I8) ---------------------------------------

  /**
   * Coalesce a burst of journal deltas into ONE keyframe request. The
   * boolean guard on the store stops a second arming; this timer turns the
   * whole burst into a single `panel_subscribe` UNIT_RESYNC_MS after the
   * first delta of it. A `@unit/damage` plan can publish many deltas in a
   * row, and every one of them would otherwise cost a round trip.
   */
  #scheduleUnitResync(): void {
    if (this.#unitResyncTimer !== null) {
      return;
    }
    // The request the timer sends answers a BURST, so a refusal to it earns
    // one bounded retry (`#scheduleUnitRetry`) rather than leaving the pane
    // behind its own journal until the next delta.
    this.#unitResyncTimer = setTimeout(() => {
      this.#unitResyncTimer = null;
      this.#unitResyncFromDelta = true;
      this.refreshUnit();
    }, UNIT_RESYNC_MS);
  }

  /** Retry ONCE, further out, when a delta-driven resync came back a Sys
   * (a throttle). Bounded to a single retry per burst: the flag is set only
   * by the coalescer's own callback and cleared as soon as it is read. */
  #scheduleUnitRetry(): void {
    if (!this.#unitResyncFromDelta) {
      return;
    }
    this.#unitResyncFromDelta = false;
    if (this.#unitResyncTimer !== null) {
      return;
    }
    this.#unitResyncTimer = setTimeout(() => {
      this.#unitResyncTimer = null;
      this.refreshUnit();
    }, 4 * UNIT_RESYNC_MS);
  }

  /** Disarm the coalescer (every reset seam). */
  #clearUnitResync(): void {
    this.#unitResyncFromDelta = false;
    if (this.#unitResyncTimer !== null) {
      clearTimeout(this.#unitResyncTimer);
      this.#unitResyncTimer = null;
    }
  }

  /** Re-request the unit keyframe: the screen's `r`, and the self-heal
   * after a journal delta ran ahead of the watermark. `panel_subscribe`
   * carries only a feed, so this is always the viewer's DEFAULT subject —
   * a browser can never name one. No-op before a unit feed is known. */
  refreshUnit(): void {
    const feed = this.session.unitFeed;
    if (feed === null) {
      return;
    }
    if (!this.#req({ kind: "unit", feed }, { t: "panel_subscribe", d: { feed } })) {
      // Nothing went out (a closed socket), so nothing will answer and
      // release the guard — drop it here or every later delta is discarded
      // by the `!resyncPending` test with no request in flight.
      this.unit.resyncPending = false;
    }
  }

  /** Re-request the contacts list the unit screen embeds. No-op when the
   * manifest does not carry it (the pane shows its placeholder instead). */
  refreshContacts(): void {
    if (!this.session.hasPanel(CONTACTS_FEED)) {
      return;
    }
    this.#req({ kind: "contacts" }, { t: "panel_subscribe", d: { feed: CONTACTS_FEED } });
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
