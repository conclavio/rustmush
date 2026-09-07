// GameApp behavioural tests: the controller driven through a fake socket —
// the hello pass, the I8 unit-resync coalescer and its bounded retry, the
// two reset seams, the I13 sheet storm guard, and the reply-`Sys` arms. No
// browser, no server: `environment: "node"` is enough because every browser
// global GameApp touches at construction is optional-chained or try/catch'd
// ($lib/storage, $lib/theme, `#caps`), and the one bare `location` lives in
// `WsClient.#open` behind the `url` option this harness passes.
//
// These replace the `readFileSync` source probes that used to stand in for a
// harness in stores/unit.test.ts. The harness is inlined here rather than
// extracted, matching ws.test.ts's convention — a shared file is premature
// until a second consumer wants one.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GameApp } from "./game.svelte";
import type { Envelope, OutputEvent, Query, ViewEnvelope } from "./proto";
import { UNIT_RESYNC_MS } from "./stores/unit.svelte";
import type { WebSocketLike } from "./ws";

/** The feeds the fixture manifest advertises. */
const UNIT_FEED = "tactics.unit";
const CONTACTS_FEED = "tactics.contacts";
const SHEET_FEED = "wod.sheet";

/** One outbound client frame, parsed off the wire. */
interface ClientFrame {
  t: string;
  d?: unknown;
}

/** The payload of a correlated `{t:"req"}` frame. */
interface ReqFrame {
  id: number;
  q: Query;
}

/** Scriptable stand-in for the DOM WebSocket (ws.test.ts's convention). */
class FakeSocket implements WebSocketLike {
  static instances: FakeSocket[] = [];

  readyState = 0; // CONNECTING
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;

  constructor(
    public url: string,
    public protocols?: string[],
  ) {
    FakeSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(code = 1000): void {
    this.readyState = 3; // CLOSED
    this.onclose?.({ code, wasClean: true });
  }

  // -- test helpers --

  /** The server accepted the connection. `readyState` MUST become OPEN, or
   * `WsClient.send` silently drops every frame and the guards under test
   * release themselves for the wrong reason. */
  open(): void {
    this.readyState = 1; // OPEN
    this.onopen?.();
  }

  /** Deliver one server envelope. */
  deliver(e: Envelope): void {
    this.onmessage?.({ data: JSON.stringify(e) });
  }

  /** Every frame the client sent, parsed. */
  sentJson(): ClientFrame[] {
    return this.sent.map((s) => JSON.parse(s) as ClientFrame);
  }

  /** Just the correlated requests, in order. */
  reqs(): ReqFrame[] {
    return this.sentJson().flatMap((f) => (f.t === "req" ? [f.d as ReqFrame] : []));
  }
}

function lastSocket(): FakeSocket {
  const ws = FakeSocket.instances[FakeSocket.instances.length - 1];
  if (!ws) {
    throw new Error("no socket created");
  }
  return ws;
}

/** The newest correlated request. Correlation ids climb across reconnects
 * (`WsClient.#nextReqId` is never reset), so a reply is always addressed to
 * the id read back off the wire, never to a hardcoded one. */
function lastReq(ws: FakeSocket): ReqFrame {
  const r = ws.reqs()[ws.reqs().length - 1];
  if (!r) {
    throw new Error("no request sent");
  }
  return r;
}

/** The feed a `panel_subscribe` names (`null` for any other query). */
function feedOf(r: ReqFrame): string | null {
  return r.q.t === "panel_subscribe" ? r.q.d.feed : null;
}

/** Builds a wire envelope: seq/ts with the event's t/d flattened in. */
function env(seq: number, event: OutputEvent): Envelope {
  return { seq, ts: 1756300000000 + seq, ...event } as Envelope;
}

/** The hello fixture: a manifest carrying a unit feed, its contacts list and
 * a sheet — the three `#routePanel` arms these tests exercise. `#routePanel`
 * is an if/else chain over suffix-derived feed names, so the manifest is
 * load-bearing: drop a feed and its arm becomes unreachable. */
function hello(panels: string[] = [UNIT_FEED, CONTACTS_FEED, SHEET_FEED]): Envelope {
  return env(0, {
    t: "hello",
    d: { proto: 1, server: "rustmush test", features: [], panels },
  });
}

/** A panel frame as an event, so it can ride either bare or inside a reply
 * (`#routeReply` → `#routePanel(d, pending)` is the real answer path for a
 * `panel_subscribe`; a bare frame is the server's unsolicited fan-out). */
function panelEvent(feed: string, kind: "keyframe" | "delta", data: unknown): OutputEvent {
  return { t: "panel", d: { feed, kind, data } } as OutputEvent;
}

function panel(
  seq: number,
  feed: string,
  kind: "keyframe" | "delta",
  data: unknown,
): Envelope {
  return env(seq, panelEvent(feed, kind, data));
}

function reply(seq: number, id: number, event: OutputEvent): Envelope {
  return env(seq, { t: "reply", d: { id, event } } as OutputEvent);
}

/** A refusal, as it rides inside a reply (`plainText` is what reaches the
 * store, so this must be a RichText array). */
const SYS_REFUSAL: OutputEvent = {
  t: "sys",
  d: [{ text: "You have no station on that unit." }],
};

/** A journal delta at `rev`, for the frozen contract's F1 subject. */
function delta(rev: number): unknown {
  return {
    object: { id: "#7", name: "Atlas" },
    from_revision: rev - 1,
    to_revision: rev,
    now: 1757000000,
    cause: null,
    entries: [{ kind: "cell", addr: "unit/heat", from: 1, to: 0 }],
  };
}

/** A sheet keyframe at `revision`. `parseSheetSchema` demands BOTH
 * `ruleset` and `version`; without them the keyframe is silently refused
 * and the storm-guard test measures nothing (hence the `loaded` pin). */
function sheetKeyframe(revision: number): unknown {
  return {
    target: "#7",
    revision,
    schema: { ruleset: "wod", version: 1, sections: [] },
    data: {},
  };
}

function makeApp(): GameApp {
  return new GameApp({
    url: "ws://test/ws",
    webSocketFactory: (url, protocols) => new FakeSocket(url, protocols),
    random: () => 1, // deterministic backoff jitter
  });
}

/** Accept the newest socket and answer it with a hello. */
function openHello(panels?: string[]): FakeSocket {
  const ws = lastSocket();
  ws.open();
  ws.deliver(hello(panels));
  return ws;
}

/** A connected app at the far side of a hello. `client.connect` is the whole
 * drive point: it needs no credentials and no storage. */
function boot(panels?: string[]): { app: GameApp; ws: FakeSocket } {
  const app = makeApp();
  app.client.connect("tok", "7");
  return { app, ws: openHello(panels) };
}

/** The subset of `localStorage` that `$lib/storage` touches — enough for the
 * credentialled reset seam (`enterGame`), unstubbed after every test. */
function memoryStorage(seed: Record<string, string>): unknown {
  const map = new Map<string, string>(Object.entries(seed));
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
  };
}

beforeEach(() => {
  // NEVER `vi.runAllTimers()` in this file: toasts arm 6 s, the mail helpers
  // 400 ms and the reconnect backoff 500 ms+, so running everything mints
  // fresh sockets. Advance to exact boundaries instead.
  vi.useFakeTimers();
  FakeSocket.instances = [];
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the hello pass", () => {
  it("declares caps, hydrates the manifest, and registers the unit screen once", () => {
    const { app, ws } = boot();

    expect(ws.url).toBe("ws://test/ws?char=7");
    expect(ws.protocols).toEqual(["rustmush.v1.tok"]);
    expect(app.session.status).toBe("connected");
    // Caps go out first, before any hydration request: the server's hello is
    // answered by the transport itself.
    expect(ws.sentJson()[0]).toEqual({
      t: "caps",
      d: {
        kind: "web",
        color: "true_color",
        width: 128,
        height: 42,
        utf8: true,
        structured: true,
      },
    });
    expect(ws.reqs().map((r) => r.q.t)).toEqual([
      "who",
      "sheet",
      "commands",
      "panel_subscribe",
      "panel_subscribe",
    ]);
    expect(ws.reqs().map(feedOf)).toEqual([null, null, null, UNIT_FEED, CONTACTS_FEED]);
    // A fresh client's ids start at 1 — the hydration burst is the first
    // correlated traffic on the wire.
    expect(ws.reqs().map((r) => r.id)).toEqual([1, 2, 3, 4, 5]);
    expect(app.screens.feedScreens.map((f) => f.def.id)).toEqual(["unit"]);
  });

  it("re-hydrates on a second hello without re-registering the screen", () => {
    const { app, ws } = boot();
    // `ScreensStore.clear()` keeps `feedScreens`, so the screen registered
    // during `boot()` is still there on the second hello. The duplicate
    // refusal is SILENT — `registerFeedScreen` returns "screen unit declared
    // twice" and `#hydrate` discards the string, leaving `feedScreens`
    // untouched either way — so the length assertion below cannot see the
    // guard. The spy is what pins it: the guard must keep `#hydrate` from
    // calling the registrar a second time at all.
    const reg = vi.spyOn(app.screens, "registerFeedScreen");
    app.unit.resyncPending = true;
    app.scrollback.fetchingBacklog = true;
    const before = ws.reqs().length;

    ws.deliver(hello());

    expect(reg).not.toHaveBeenCalled(); // the one legitimate call was in boot()
    expect(app.screens.feedScreens).toHaveLength(1);
    // Both in-flight latches are released: a request armed on a previous
    // socket is never answered, and either latch would otherwise wedge.
    expect(app.unit.resyncPending).toBe(false);
    expect(app.scrollback.fetchingBacklog).toBe(false);
    // The whole manifest is re-requested, on fresh ids.
    expect(ws.reqs().slice(before).map((r) => r.q.t)).toEqual([
      "who",
      "sheet",
      "commands",
      "panel_subscribe",
      "panel_subscribe",
    ]);
    expect(ws.reqs().slice(before).map((r) => r.id)).toEqual([6, 7, 8, 9, 10]);
  });

  it("registers no unit screen when the manifest carries no unit feed", () => {
    const { app, ws } = boot([SHEET_FEED]);

    expect(app.screens.feedScreens).toEqual([]);
    expect(ws.reqs().map((r) => r.q.t)).toEqual(["who", "sheet", "commands"]);
  });
});

describe("the unit-resync coalescer", () => {
  it("turns a burst of journal deltas into ONE panel_subscribe", () => {
    const { app, ws } = boot();
    const before = ws.reqs().length;

    for (const rev of [4, 5, 6]) {
      ws.deliver(panel(rev, UNIT_FEED, "delta", delta(rev)));
    }

    expect(app.unit.resyncPending).toBe(true);
    expect(app.unit.journal).toHaveLength(3); // every delta still rings
    expect(ws.reqs()).toHaveLength(before); // nothing has gone out yet

    vi.advanceTimersByTime(UNIT_RESYNC_MS - 1);
    expect(ws.reqs()).toHaveLength(before);
    vi.advanceTimersByTime(1);
    expect(ws.reqs()).toHaveLength(before + 1);
    expect(lastReq(ws).q).toEqual({ t: "panel_subscribe", d: { feed: UNIT_FEED } });
  });

  it("never doubles the armed timer when a reply releases the guard mid-burst", () => {
    // The store's boolean guard is the first line of defence; the timer's own
    // re-entrancy check is the second, and this is the sequence that reaches
    // it — the hello pass's unit subscribe is still outstanding, so its reply
    // can release `resyncPending` while the coalescer is already armed.
    const { app, ws } = boot();
    const hydrationId = ws.reqs().filter((r) => feedOf(r) === UNIT_FEED)[0]!.id;
    ws.deliver(panel(4, UNIT_FEED, "delta", delta(4)));
    expect(app.unit.resyncPending).toBe(true);

    ws.deliver(reply(8, hydrationId, SYS_REFUSAL));
    expect(app.unit.resyncPending).toBe(false);
    ws.deliver(panel(5, UNIT_FEED, "delta", delta(5)));
    expect(app.unit.resyncPending).toBe(true);

    const before = ws.reqs().length;
    vi.advanceTimersByTime(UNIT_RESYNC_MS);
    expect(ws.reqs()).toHaveLength(before + 1);
  });

  it("a Sys refusal releases the guard, stays out of scrollback, and retries exactly once", () => {
    const { app, ws } = boot();
    ws.deliver(panel(4, UNIT_FEED, "delta", delta(4)));
    vi.advanceTimersByTime(UNIT_RESYNC_MS);

    ws.deliver(reply(9, lastReq(ws).id, SYS_REFUSAL));

    expect(app.unit.resyncPending).toBe(false);
    expect(app.unit.unavailable).toBe("You have no station on that unit.");
    // The panel owns the message; scrollback stays clean.
    expect(app.scrollback.entries).toHaveLength(0);

    const n = ws.reqs().length;
    vi.advanceTimersByTime(4 * UNIT_RESYNC_MS - 1);
    expect(ws.reqs()).toHaveLength(n);
    vi.advanceTimersByTime(1);
    expect(ws.reqs()).toHaveLength(n + 1); // the ONE bounded retry
    expect(feedOf(lastReq(ws))).toBe(UNIT_FEED);

    // A refusal to the retry arms nothing: the "came from a delta" flag is
    // read-once, cleared before the retry is armed.
    ws.deliver(reply(10, lastReq(ws).id, SYS_REFUSAL));
    vi.advanceTimersByTime(60_000);
    expect(ws.reqs()).toHaveLength(n + 1);
  });

  it("a keyframe answering the burst clears the retry flag, so a later refusal arms nothing", () => {
    const { app, ws } = boot();
    ws.deliver(panel(4, UNIT_FEED, "delta", delta(4)));
    vi.advanceTimersByTime(UNIT_RESYNC_MS);
    const n = ws.reqs().length;

    // The honest no-subject frame is still an answer, and it arrives the way
    // a `panel_subscribe` is really answered — inside a reply, so the frame
    // routes through `#routeReply` → `#routePanel(d, pending)`.
    ws.deliver(reply(5, lastReq(ws).id, panelEvent(UNIT_FEED, "keyframe", null)));

    expect(app.unit.answered).toBe(true);
    expect(app.unit.noSubject).toBe(true);
    expect(app.unit.resyncPending).toBe(false);
    vi.advanceTimersByTime(60_000);
    expect(ws.reqs()).toHaveLength(n);

    // The flag is only ever read by `#scheduleUnitRetry`, so nothing above
    // could tell a cleared flag from an unread one: make the keyframe's
    // clear observable through that one consumer. A refused MANUAL refresh
    // after a frame-answered burst must arm NOTHING — a retry is earned by
    // the coalescer's own request, and this burst was already answered.
    app.refreshUnit();
    const manual = ws.reqs().length;
    expect(manual).toBe(n + 1);

    ws.deliver(reply(6, lastReq(ws).id, SYS_REFUSAL));
    vi.advanceTimersByTime(60_000);
    expect(ws.reqs()).toHaveLength(manual);
  });

  it("both reset seams disarm the coalescer and clear the view", () => {
    // Seam 1 — the resume gap: `resumed { replayed: false }` fires
    // `onFullRehydrate`, which clears the timer and the store.
    const { app, ws } = boot();
    ws.deliver(panel(4, UNIT_FEED, "delta", delta(4)));
    expect(app.unit.resyncPending).toBe(true);
    expect(app.unit.journal).toHaveLength(1);

    ws.deliver(env(0, { t: "resumed", d: { replayed: false } }));

    expect(app.unit.answered).toBe(false);
    expect(app.unit.journal).toEqual([]);
    expect(app.unit.resyncPending).toBe(false);
    const n = ws.reqs().length;
    vi.advanceTimersByTime(60_000);
    expect(ws.reqs()).toHaveLength(n); // the timer is gone, not merely idle

    // Seam 2 — `#resetGameState`, reached through `enterGame`. It needs a
    // stored token, so this is the one test that stubs storage.
    vi.stubGlobal(
      "localStorage",
      memoryStorage({
        "rustmush.token": "tok",
        "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
      }),
    );
    const app2 = makeApp();
    expect(app2.screen).toBe("chars");
    app2.client.connect("tok", "7");
    const first = openHello();
    first.deliver(panel(4, UNIT_FEED, "delta", delta(4)));
    expect(app2.unit.resyncPending).toBe(true);

    app2.enterGame(7);

    expect(app2.screen).toBe("game");
    expect(app2.unit.journal).toEqual([]);
    const next = openHello();
    const m = next.reqs().length;
    vi.advanceTimersByTime(60_000);
    expect(next.reqs()).toHaveLength(m);
  });

  it("releases the guard when the request never went out", () => {
    const { app, ws } = boot();
    const before = ws.reqs().length;
    app.client.close();

    app.unit.resyncPending = true;
    app.refreshUnit();

    // Nothing was sent, so nothing will answer and release it — the caller
    // must drop the guard itself or every later delta is discarded.
    expect(ws.reqs()).toHaveLength(before);
    expect(app.unit.resyncPending).toBe(false);
  });

  it("the screen's local actions reach the store and the wire", () => {
    const { app, ws } = boot();

    // `toggle` needs its own arm: `refresh` falls through to `refreshWho()`
    // for anything it does not name.
    app.localAction("toggle");
    expect(app.unit.layout).toBe("damage");

    expect(app.screens.switchTo("unit", app.gate)).toBe(true);
    const before = ws.reqs().length;
    app.localAction("refresh");

    const after = ws.reqs().slice(before);
    expect(after.map((r) => r.q.t)).toEqual(["panel_subscribe", "panel_subscribe"]);
    expect(after.map(feedOf)).toEqual([UNIT_FEED, CONTACTS_FEED]);
  });
});

describe("the sheet storm guard", () => {
  it("fires ONE refresh for a run of gapped deltas", () => {
    const { app, ws } = boot();
    ws.deliver(panel(3, SHEET_FEED, "keyframe", sheetKeyframe(2)));
    expect(app.sheet.loaded).toBe(true);
    expect(app.sheet.revision).toBe(2);
    const before = ws.reqs().length;

    for (const rev of [5, 6, 7]) {
      ws.deliver(
        panel(rev, SHEET_FEED, "delta", { revision: rev, base_revision: rev - 1, data: {} }),
      );
    }

    expect(ws.reqs()).toHaveLength(before + 1);
    expect(lastReq(ws).q).toEqual({ t: "sheet" });
    expect(app.sheet.resyncPending).toBe(true);

    // ANY reply releases it before classification — even a refusal — and a
    // live sheet is never blanked by one.
    ws.deliver(reply(20, lastReq(ws).id, SYS_REFUSAL));
    expect(app.sheet.resyncPending).toBe(false);
    expect(app.sheet.loaded).toBe(true);
    expect(app.sheet.unavailable).toBe(false);
    // Unlike the unit arm, a sheet refusal has no early return: it falls
    // through to the generic reply-error line. Deliberate — the sheet panel
    // renders its own text only while nothing is loaded.
    expect(app.scrollback.entries).toHaveLength(1);
  });
});

describe("an authoritative action commit", () => {
  const commit = "0123456789abcdef0123456789abcdef";
  const outcome: ViewEnvelope = {
    view: "resolution.outcome",
    schema_version: 1,
    generation: Number.parseInt(commit.slice(0, 13), 16),
    sequence: 1,
    commit,
    data: {
      schema_version: 1,
      status: "k5:toy:brace:0000000000000007:01010101010101010101010101010101",
      commit,
      result: { kind: "brace", summary: "Brace committed.", data: { braced: true } },
    },
  };

  it("acks once, reports once, and never re-requests the sheet", () => {
    const { app, ws } = boot();
    const before = ws.reqs().length;

    ws.deliver(env(11, { t: "view", d: outcome }));

    expect(ws.sentJson().filter((f) => f.t === "ack")).toEqual([{ t: "ack", d: { commit } }]);
    expect(app.toasts.map((t) => t.text)).toEqual(["Brace committed."]);
    expect(app.scrollback.entries).toHaveLength(1);
    // The I13 interim mitigation is retired: the server republishes the
    // sheet on an action-state commit, so the viewer's own commit costs no
    // extra round trip here.
    expect(ws.reqs()).toHaveLength(before);

    // A replayed receipt is idempotent: acked again, reported once.
    ws.deliver(env(12, { t: "view", d: outcome }));
    expect(ws.sentJson().filter((f) => f.t === "ack")).toHaveLength(2);
    expect(app.scrollback.entries).toHaveLength(1);
    expect(ws.reqs()).toHaveLength(before);
  });
});
