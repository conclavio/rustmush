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

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { cockpitView, cockpitViewLines, composeWindow, NOTICE_SENTENCE_TAG } from "./cockpit";
import { GameApp, type ComposerHandle } from "./game.svelte";
import type { Envelope, OutputEvent, Query, ViewEnvelope } from "./proto";
import { MAP_ASK_MS } from "./stores/cockpit.svelte";
import { LIVE_STALE_MS, REPLAY_ASK_MS } from "./stores/replay.svelte";
import { UNIT_RESYNC_MS } from "./stores/unit.svelte";
import { CLOSE_TOKEN_COOLOFF, type WebSocketLike } from "./ws";

/** The feeds the fixture manifest advertises. */
const UNIT_FEED = "tactics.unit";
const CONTACTS_FEED = "tactics.contacts";
const SHEET_FEED = "wod.sheet";
const VITALS_FEED = "wod.vitals";
const VIEW_FEED = "foo.view";

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

  /** The connection died (an UNCLEAN close): the client reconnects after
   * its backoff. `close()` above is the clean one, which never does. */
  drop(code = 1006): void {
    this.readyState = 3; // CLOSED
    this.onclose?.({ code, wasClean: false });
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

/** A minimal I9 generic-view keyframe: enough for `GenericViewStore.apply`
 * to hold it, which is what makes `GameApp` register a screen for the feed. */
function viewKeyframe(): unknown {
  return {
    schema: { id: "foo", version: 1, shape: "list", columns: [{ key: "a" }] },
    rows: [{ a: "1" }],
  };
}

/** Every scrollback block as its first line's plain text. */
function lineTexts(app: GameApp): string[] {
  return app.scrollback.entries.map((b) => b.lines[0]?.map((s) => s.text).join("") ?? "");
}

/** A `.vitals` keyframe at `revision`, one gauge, one subject. */
function vitalsFrame(revision: number): unknown {
  return {
    subject: { id: "#7", name: "Atlas" },
    revision,
    vitals: [
      {
        id: "wod.health",
        label: "Health",
        role: "dominant",
        value: { kind: "bounded", current: revision, min: 0, max: 10 },
        state: "ok",
      },
    ],
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

  it("re-subscribes every panel feed after a reconnect, and replies still correlate", () => {
    // I4-C41: the R lens rested on Hello-before-Resumed ordering and nothing
    // drove it. The re-subscribe is the hello pass's own work — `resumed`
    // only marks the seam — so the panel state a viewer had before the drop
    // is asked for again on the NEW socket.
    const panels = [SHEET_FEED, VITALS_FEED, "scenes"];
    const { app, ws } = boot(panels);
    expect(ws.reqs().map(feedOf).filter((f) => f !== null)).toEqual([VITALS_FEED, "scenes"]);

    ws.drop();
    expect(app.session.status).toBe("reconnecting");
    vi.advanceTimersByTime(500); // the first backoff window, `random: () => 1`
    const next = openHello(panels);
    next.deliver(env(0, { t: "resumed", d: { replayed: false } }));

    expect(next).not.toBe(ws); // a genuinely new socket
    expect(app.session.status).toBe("connected");
    expect(next.reqs().map(feedOf).filter((f) => f !== null)).toEqual([VITALS_FEED, "scenes"]);

    // Correlation survives the seam: `#hydrate` clears the pending map for
    // the old socket's ids, and the new ones climb from where the old left
    // off — so a reply is only ever answered by the id read off THIS socket.
    const vitalsReq = next.reqs().filter((r) => feedOf(r) === VITALS_FEED)[0]!;
    expect(vitalsReq.id).toBeGreaterThan(ws.reqs()[ws.reqs().length - 1]!.id);
    next.deliver(reply(9, vitalsReq.id, panelEvent(VITALS_FEED, "keyframe", vitalsFrame(3))));
    expect(app.vitals.list[0]?.frame.revision).toBe(3);
  });
});

// Feed screens across a character switch (LB-2). `ScreensStore.clear()` keeps
// `feedScreens` on purpose — the bare-hello path needs the unit screen to
// survive a re-hello — but the full game-state reset must not: a `foo.view`
// screen registered against server A's manifest kept digit 6 on server B, so
// the tactical unit screen was refused and never appeared. Both registrars
// discarded their reason, so the refusal was silent as well.
describe("feed screens across a character switch", () => {
  it("a `.view` screen holding digit 6 refuses the unit screen, out loud", () => {
    const { app, ws } = boot([VIEW_FEED]);
    ws.deliver(panel(3, VIEW_FEED, "keyframe", viewKeyframe()));
    expect(app.screens.feedScreens.map((f) => f.def.digit)).toEqual([6]);
    expect(lineTexts(app)).toEqual([]);

    // The same session learns a `.unit` feed: `#hydrate`'s registration is
    // refused because digit 6 is spoken for.
    ws.deliver(hello([VIEW_FEED, UNIT_FEED]));

    expect(app.screens.feedScreens.map((f) => f.def.id)).toEqual([`view:${VIEW_FEED}`]);
    expect(lineTexts(app)).toEqual([
      "This client could not add the unit screen: screen unit: digit 6 used twice.",
    ]);
  });

  it("the other order refuses the view screen, also out loud", () => {
    const { app, ws } = boot([UNIT_FEED, VIEW_FEED]);
    expect(app.screens.feedScreens.map((f) => f.def.id)).toEqual(["unit"]);

    ws.deliver(panel(3, VIEW_FEED, "keyframe", viewKeyframe()));

    expect(app.screens.feedScreens).toHaveLength(1);
    // The view is still HELD — only its screen was refused, and the digitless
    // screen policy (LB-15) is what will eventually place it.
    expect(app.genericViews.get(VIEW_FEED)).not.toBeNull();
    expect(lineTexts(app)).toEqual([
      `This client could not add the ${VIEW_FEED} screen: no free hotkey digit for a view screen.`,
    ]);
  });

  it("a generic-view screen's `r` asks for its own feed, never WHO", () => {
    // LB-3: `localAction("refresh")` names every built-in screen and falls
    // through to `refreshWho()` for anything else — so a module's own view
    // screen refreshed the WHO list. Its feed IS its refresh.
    const { app, ws } = boot([VIEW_FEED]);
    ws.deliver(panel(3, VIEW_FEED, "keyframe", viewKeyframe()));
    expect(app.screens.switchTo(`view:${VIEW_FEED}`, app.gate)).toBe(true);
    const before = ws.reqs().length;

    expect(app.runBinding("r")).toBe(true);

    expect(ws.reqs().slice(before).map((r) => r.q)).toEqual([
      { t: "panel_subscribe", d: { feed: VIEW_FEED } },
    ]);
  });

  it("a character switch drops them, so the next server's screen registers", () => {
    vi.stubGlobal(
      "localStorage",
      memoryStorage({
        "rustmush.token": "tok",
        "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
      }),
    );
    const app = makeApp();
    app.client.connect("tok", "7");
    const ws = openHello([VIEW_FEED]);
    ws.deliver(panel(3, VIEW_FEED, "keyframe", viewKeyframe()));
    expect(app.screens.feedScreens.map((f) => f.def.digit)).toEqual([6]);

    // A different character on a tactics server.
    app.enterGame(7);
    expect(app.screens.feedScreens).toEqual([]);
    openHello([UNIT_FEED]);

    expect(app.screens.feedScreens.map((f) => f.def.id)).toEqual(["unit"]);
    expect(lineTexts(app)).toEqual([]); // nothing was refused, so nothing was said
  });
});

// The vitals watermark across a world restart (I1-C4). A web socket survives
// a world restart with no fresh Hello (`mush-gateway/tests/web_restart_
// survival.rs`) while `VitalsRevisions` restarts at zero, so a viewer holding
// revision N drops every gauge the restarted world publishes — silently, for
// the rest of the session. Every seam where the client may now be looking at
// a different run of the world resets the watermarks.
describe("the vitals watermark", () => {
  it("accepts a restarted world's revision 1 after a hello; without one it stays stale", () => {
    const { app, ws } = boot([VITALS_FEED]);
    ws.deliver(panel(3, VITALS_FEED, "keyframe", vitalsFrame(5)));
    expect(app.vitals.list[0]?.frame.revision).toBe(5);

    // No hello: the monotonic guard is doing its real job — a late frame
    // from the SAME run must never roll a gauge back.
    ws.deliver(panel(4, VITALS_FEED, "keyframe", vitalsFrame(4)));
    expect(app.vitals.list[0]?.frame.revision).toBe(5);

    // The world restarted and its counter with it. The hello marks every
    // held group accept-next; the frames stay on screen meanwhile.
    ws.deliver(hello([VITALS_FEED]));
    expect(app.vitals.list[0]?.frame.revision).toBe(5);
    ws.deliver(panel(5, VITALS_FEED, "keyframe", vitalsFrame(1)));
    expect(app.vitals.list[0]?.frame.revision).toBe(1);

    // The mark is spent: the new run's own numbers are guarded again.
    ws.deliver(panel(6, VITALS_FEED, "keyframe", vitalsFrame(3)));
    ws.deliver(panel(7, VITALS_FEED, "keyframe", vitalsFrame(2)));
    expect(app.vitals.list[0]?.frame.revision).toBe(3);
  });

  it("resets on the two reconnect seams: a replayed resume and a resume gap", () => {
    // `resumed { replayed: true }` replays the gap silently — no hello, no
    // rehydrate — so it is the ONLY seam a restart inside a short gap
    // crosses.
    const { app, ws } = boot([VITALS_FEED]);
    ws.deliver(panel(3, VITALS_FEED, "keyframe", vitalsFrame(5)));
    ws.deliver(env(0, { t: "resumed", d: { replayed: true } }));
    ws.deliver(panel(4, VITALS_FEED, "keyframe", vitalsFrame(1)));
    expect(app.vitals.list[0]?.frame.revision).toBe(1);

    // The gap outgrew the ring: `onFullRehydrate`. The strip is not blanked
    // — a stale gauge is a smaller lie than a missing one.
    ws.deliver(panel(5, VITALS_FEED, "keyframe", vitalsFrame(9)));
    ws.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    expect(app.vitals.list[0]?.frame.revision).toBe(9);
    ws.deliver(panel(6, VITALS_FEED, "keyframe", vitalsFrame(2)));
    expect(app.vitals.list[0]?.frame.revision).toBe(2);
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
    // A re-attach is a fresh VIEW, not a `/clear`: the reset clears the
    // scrollback (which draws the backlog floor, LB-1) and then begins a new
    // segment, which lifts it for the new session's ring.
    expect(app2.scrollback.moreBacklog).toBe(true);
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

// The narrative backlog floor (LB-1). `/clear` threw away every held `seq`,
// so `oldestNarrativeSeq` is `null` again and the next page is the NEWEST
// one — the very lines the viewer just cleared, repainted, with nothing to
// dedup them against. The TUI draws a floor instead (`docs/help/tui.md`
// "Scene history"); the web now agrees.
describe("the narrative backlog floor", () => {
  /** A room narrative line at `seq` — the backlog's cursor comes off these. */
  function said(seq: number): Envelope {
    return env(seq, {
      t: "narrative",
      d: { kind: "say", actor: { id: "#7", name: "Atlas" }, text: [{ text: "hello" }] },
    } as OutputEvent);
  }

  it("/clear stops older history from repainting the lines it cleared", () => {
    const { app, ws } = boot();
    ws.deliver(said(3));
    ws.deliver(said(4));
    const before = ws.reqs().length;

    // The affordance works: a page is asked for, from the oldest held seq.
    app.loadOlderNarrative();
    expect(ws.reqs()).toHaveLength(before + 1);
    expect(lastReq(ws).q).toEqual({
      t: "narrative_backlog",
      d: { before_seq: 3, limit: 200 },
    });

    app.submit("/clear");

    // The floor, not the in-flight latch: `clear()` releases that, so this
    // assertion is about `moreBacklog` alone.
    expect(app.scrollback.fetchingBacklog).toBe(false);
    expect(app.scrollback.oldestNarrativeSeq).toBeNull();
    app.loadOlderNarrative();
    expect(ws.reqs()).toHaveLength(before + 1);

    // A fresh view lifts it: the resume-gap rehydrate starts a new session
    // segment, whose ring the gateway primes from the scene.
    ws.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    app.loadOlderNarrative();
    expect(ws.reqs()).toHaveLength(before + 2);
    // A new session's ring: no seq survived the segment, so the cursor is
    // null and the server sends its newest page.
    expect(lastReq(ws).q).toEqual({
      t: "narrative_backlog",
      d: { before_seq: null, limit: 200 },
    });
  });

  it("/history is the same verb the TUI has, and it honours the floor", () => {
    // LB-20: the TUI and line mode have had `/history` since I12; the web
    // could only reach it by scrolling to the top of the pane.
    const { app, ws } = boot();
    ws.deliver(said(3));
    const before = ws.reqs().length;

    app.submit("/history");

    expect(ws.reqs()).toHaveLength(before + 1);
    expect(lastReq(ws).q).toEqual({
      t: "narrative_backlog",
      d: { before_seq: 3, limit: 200 },
    });
    expect(app.paletteEntries).toEqual([]);
    app.paletteQuery("/hist");
    expect(app.paletteEntries.map((e) => e.insert)).toContain("/history");

    // Below a `/clear` floor there is nothing to ask for, and saying so is
    // the difference between "nothing older" and "the command is broken".
    app.submit("/clear");
    app.submit("/history");
    expect(ws.reqs()).toHaveLength(before + 1);
    expect(lineTexts(app)).toEqual(["No older history in this view."]);
  });
});

// The modal shell's whole key model (I4-C45). `act` is where a `NavIntent`
// from `keys.ts` becomes a side effect, and it was the one part of the shell
// with no test at all — `composer` is a public, fake-able handle, so this
// needs no DOM.
describe("GameApp.act", () => {
  /** The composer the shell registers, recorded rather than rendered. */
  class FakeComposer implements ComposerHandle {
    /** Every `focus` argument, in order (`undefined` = keep the draft). */
    focuses: (string | undefined)[] = [];
    blurs = 0;
    cycles = 0;
    #text = "";

    focus(prefill?: string): void {
      this.focuses.push(prefill);
      if (prefill !== undefined) {
        this.#text = prefill;
      }
    }
    blur(): void {
      this.blurs += 1;
    }
    cycleMode(): void {
      this.cycles += 1;
    }
    text(): string {
      return this.#text;
    }
    setText(next: string): void {
      this.#text = next;
    }
  }

  /** `focusComposer` defers to a microtask so a render in flight settles
   * before the box is focused; `queueMicrotask` is not faked by
   * `vi.useFakeTimers()`, so one awaited tick is enough. */
  const tick = (): Promise<void> => Promise.resolve();

  function withComposer(panels?: string[]): { app: GameApp; ws: FakeSocket; c: FakeComposer } {
    const { app, ws } = boot(panels);
    const c = new FakeComposer();
    app.composer = c;
    return { app, ws, c };
  }

  it("compose focuses the box, keeping the draft unless a trigger typed one", async () => {
    const { app, c } = withComposer();

    // Enter: focus, no prefill — an empty prefill must reach `focus` as
    // `undefined`, or the half-typed draft is wiped.
    expect(app.act({ kind: "compose", prefill: "" })).toBe(true);
    await tick();
    expect(c.focuses).toEqual([undefined]);

    // A speech trigger types itself into the box.
    expect(app.act({ kind: "compose", prefill: '"' })).toBe(true);
    await tick();
    expect(c.focuses).toEqual([undefined, '"']);
    expect(c.text()).toBe('"');
  });

  it("from the channel screen, which has no shell box, a prefill goes to the scene screen", async () => {
    // 16b cut B.1: the channel screen keeps its own box (addressed to the
    // channel) and the shell's is not mounted there. A tray action's `Reply`,
    // the `Enter type` chip and `"` `:` `;` must still reach a box, so from
    // this one screen `focusComposer` switches — and the line is never put in
    // the channel's box.
    const { app, c } = withComposer(["comsys"]);
    expect(app.screens.switchTo("channels", app.gate)).toBe(true);
    app.composer = null; // what `Game.svelte`'s effect writes on this screen

    app.focusComposer("page Bob=");
    expect(app.screens.active).toBe("scene");
    app.composer = c; // …and what it writes once the switch has rendered
    await tick();
    expect(c.focuses).toEqual(["page Bob="]);

    // Enter and the palette key take the same road, and are consumed.
    expect(app.screens.switchTo("channels", app.gate)).toBe(true);
    app.composer = null;
    expect(app.act({ kind: "compose", prefill: "" })).toBe(true);
    expect(app.screens.active).toBe("scene");
    app.composer = c;
    await tick();
    expect(c.focuses).toEqual(["page Bob=", undefined]);

    expect(app.screens.switchTo("channels", app.gate)).toBe(true);
    app.composer = null;
    expect(app.act({ kind: "palette" })).toBe(true);
    expect(app.screens.active).toBe("scene");
    app.composer = c;
    await tick();
    await tick();
    expect(c.focuses).toEqual(["page Bob=", undefined, "/"]);
    expect(app.screens.paletteOpen).toBe(true);
  });

  it("a screen's prefill binding stays on that screen with the line", async () => {
    const { app, c } = withComposer([SHEET_FEED, "mail"]);
    expect(app.screens.switchTo("mail", app.gate)).toBe(true);

    expect(app.act({ kind: "binding", key: "s" })).toBe(true);
    await tick();

    // 16b cut B.1 (Q5): the box is the shell's, under every screen but the
    // channel screen, so a prefill from screen 3 stays on screen 3 — with the
    // line ready, unsent (D-I-18). Until B.1 it switched to the scene screen,
    // where the one box used to be mounted.
    expect(app.screens.active).toBe("mail");
    expect(c.focuses).toEqual(["@mail/send "]);
    // Where no shell box is mounted at all (the shell has not mounted yet),
    // the palette key is not consumed and opens nothing that would surface
    // later behind a box that never had focus.
    app.composer = null;
    expect(app.act({ kind: "palette" })).toBe(false);
    expect(app.screens.paletteOpen).toBe(false);
    app.composer = c;
    // An unbound key on this screen is not the shell's.
    expect(app.act({ kind: "binding", key: "z" })).toBe(false);
  });

  it("an exit key walks the room, on the wire", () => {
    const { app, ws, c } = withComposer();
    const before = ws.sentJson().length;

    expect(app.act({ kind: "exit", name: "North" })).toBe(true);

    expect(ws.sentJson().slice(before)).toEqual([{ t: "line", d: "North" }]);
    expect(c.focuses).toEqual([]); // an exit never opens the box
  });

  it("help toggles, screens switch or refuse, and the browser keeps its own keys", () => {
    const { app } = withComposer([SHEET_FEED]);

    expect(app.act({ kind: "help" })).toBe(true);
    expect(app.screens.helpOpen).toBe(true);
    expect(app.act({ kind: "help" })).toBe(true);
    expect(app.screens.helpOpen).toBe(false);

    expect(app.act({ kind: "screen", digit: 2 })).toBe(true);
    expect(app.screens.active).toBe("sheet");
    // Digit 6 is the unit screen's, and this manifest has no unit feed.
    expect(app.act({ kind: "screen", digit: 6 })).toBe(false);
    expect(app.screens.active).toBe("sheet");

    // The browser scrolls and tabs on its own here: not consumed.
    expect(app.act({ kind: "scroll", rows: 10 })).toBe(false);
    expect(app.act({ kind: "pane", dir: 1 })).toBe(false);
    expect(app.act({ kind: "jump", bottom: true })).toBe(false);
  });

  it("the palette opens on an empty box with a slash, and closes on a second press", async () => {
    const { app, c } = withComposer();

    expect(app.act({ kind: "palette" })).toBe(true);
    await tick();
    expect(c.focuses).toEqual(["/"]);
    expect(app.screens.paletteOpen).toBe(true);

    // Open again with text already typed: the box keeps it.
    app.screens.paletteOpen = false;
    c.setText("+roll ");
    expect(app.act({ kind: "palette" })).toBe(true);
    await tick();
    expect(c.focuses).toEqual(["/", undefined]);

    expect(app.act({ kind: "palette" })).toBe(true);
    expect(app.screens.paletteOpen).toBe(false);
  });

  it("`?` on a palette row asks the game for that command's help", () => {
    // I4-C23: the affordance was a comment in `CommandPalette.svelte` and
    // nothing else — no handler, no footer hint.
    const { app, ws, c } = withComposer();
    app.commands.apply({
      commands: [{ name: "look", usage: "look [<thing>]", summary: "Look around." }],
    });
    c.setText("/look");
    app.openPalette();
    // A command row inserts its USAGE, so the help verb is its first word.
    expect(app.paletteEntries[app.paletteSelected]?.insert).toBe("look [<thing>]");
    const before = ws.sentJson().length;

    app.paletteHelp();

    expect(app.screens.paletteOpen).toBe(false);
    expect(ws.sentJson().slice(before)).toEqual([{ t: "line", d: "help look" }]);

    // A client-local row has no server help page: `/help` is its card, and
    // it never reaches the wire.
    c.setText("/clear");
    app.openPalette();
    expect(app.paletteEntries[app.paletteSelected]?.local).toBe(true);
    const local = ws.sentJson().length;
    app.paletteHelp();
    expect(ws.sentJson()).toHaveLength(local);
    expect(lineTexts(app).some((t) => t.startsWith("Client commands"))).toBe(true);
  });
});

// I5's demotion rule (I5-C14): OOC, channel, system and roll lines step back
// — small mono, less contrast, still there. `c.on("sys")` said so; three
// other server-driven system pushes said `ev-sys` alone and rendered at full
// contrast in the scene's prose face.
describe("system-line demotion", () => {
  it("every system line the GAME sent is demoted; only client-local ones are not", () => {
    const { app, ws } = boot();

    // 1. an unsolicited Sys event
    ws.deliver(env(5, { t: "sys", d: [{ text: "The lights flicker." }] } as OutputEvent));
    // 2. a typed WHO — session output (seq > 0), so it also echoes classically
    ws.deliver(
      env(6, {
        t: "who",
        d: [{ name: "Atlas", idle_s: 0, conn_s: 60 }],
      } as OutputEvent),
    );
    // 3. a reply-borne Sys with nothing pending: the generic error line
    ws.deliver(reply(7, 9999, SYS_REFUSAL));

    const sys = app.scrollback.entries.filter((b) => b.cls.includes("ev-sys"));
    expect(sys).toHaveLength(3);
    for (const block of sys) {
      expect(block.cls).toBe("ev-sys ev-demoted");
    }

    // A client-local line is the one system family that is NOT demoted: it
    // is italic (`ev-local`) because it never went to the game at all.
    app.submit("/theme dark");
    const local = app.scrollback.entries.filter((b) => b.cls.includes("ev-local"));
    expect(local.map((b) => b.cls)).toEqual(["ev-sys ev-local"]);
  });
});

// The Request/Respond app seam (I10-C9). The stores were tested; that a
// `request` envelope reaches `RequestStore` at all, and that `respond` puts
// the documented frame on the wire, was pinned nowhere.
describe("a server-initiated request", () => {
  function request(seq: number, id: number): Envelope {
    return env(seq, {
      t: "request",
      d: {
        id,
        kind: "choice",
        prompt: [{ text: "Who takes point?" }],
        options: [
          { id: "mara", label: "Mara" },
          { id: "atlas", label: "Atlas" },
        ],
      },
    } as OutputEvent);
  }

  it("retains a failed answer for retry, and sends its original epoch", () => {
    const {app, ws} = boot();
    const event = request(5, 5);
    if (event.t === "request") {
      event.d.epoch = "current-world";
      event.d.kind = "text";
      event.d.options = [];
    }
    ws.deliver(event);
    ws.readyState = 3;
    expect(app.respond(5, {t:"text", d:"my answer"})).toBe(false);
    expect(app.requests.current?.id).toBe(5);
    ws.readyState = 1;
    expect(app.respond(5, {t:"text", d:"my answer"})).toBe(true);
    expect(ws.sentJson().at(-1)).toEqual({t:"respond",d:{id:5,epoch:"current-world",choice:{t:"text",d:"my answer"}}});
    expect(app.requests.current).toBeNull();
  });

  it("drops obsolete asks on Hello", () => {
    const {app, ws} = boot();
    ws.deliver(request(5, 5));
    ws.deliver({...hello(), seq:6});
    expect(app.requests.current).toBeNull();
  });

  it("reaches the store, answers by id on the wire, and is dropped locally", () => {
    const { app, ws } = boot();

    ws.deliver(request(5, 5));

    expect(app.requests.current?.id).toBe(5);
    expect(app.requests.current?.options.map((o) => o.id)).toEqual(["mara", "atlas"]);
    const before = ws.sentJson().length;

    app.respond(5, { t: "choose", d: "mara" });

    // The client reports the choice and nothing else — the server owns the
    // effect (D-I-21) — and drops it locally rather than waiting for an ack.
    expect(ws.sentJson().slice(before)).toEqual([
      { t: "respond", d: { id: 5, epoch: "", choice: { t: "choose", d: "mara" } } },
    ]);
    expect(app.requests.current).toBeNull();
  });

  it("is cleared at a game-entry seam: the registry is RAM-only server-side", () => {
    vi.stubGlobal(
      "localStorage",
      memoryStorage({
        "rustmush.token": "tok",
        "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
      }),
    );
    const app = makeApp();
    app.client.connect("tok", "7");
    const ws = openHello();
    ws.deliver(request(6, 6));
    expect(app.requests.current?.id).toBe(6);

    // A re-attach: the server's request registry is RAM-only (D-I-22) and
    // the id may be reminted for something else, so a stale answer must
    // never be sendable.
    app.enterGame(7);
    expect(app.requests.pending).toEqual([]);

    // Same rule at the resume-gap seam.
    const next = openHello();
    next.deliver(request(7, 7));
    expect(app.requests.current?.id).toBe(7);
    next.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    expect(app.requests.pending).toEqual([]);
  });
});

// LB-21: a world with two rulesets loaded publishes two `.roll` feeds. The
// web routed frames from `session.rollFeed` alone — the tie-break's winner —
// and silently dropped the other ruleset's rolls. The TUI has always
// rendered every `.roll` feed.
describe("roll feeds", () => {
  const ROLL_A = "sr.roll";
  const ROLL_B = "wod.roll";

  function rollFrame(summary: string): unknown {
    return { who: "#7", summary };
  }

  it("renders every `.roll` feed, not only the tie-break's winner", () => {
    const { app, ws } = boot([ROLL_A, ROLL_B]);
    expect(app.session.rollFeed).toBe(ROLL_A); // sorted-first

    ws.deliver(panel(3, ROLL_A, "keyframe", rollFrame("Sr5 roll: 3 hits.")));
    ws.deliver(panel(4, ROLL_B, "keyframe", rollFrame("oWoD roll: 2 successes.")));

    expect(app.rolls.entries.map((r) => r.summary)).toEqual([
      "oWoD roll: 2 successes.", // newest first
      "Sr5 roll: 3 hits.",
    ]);
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
    // The receipt line is a system line the game sent: demoted (I5-C14).
    expect(app.scrollback.entries[0]?.cls).toBe("ev-sys ev-demoted");
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

// The cockpit on the web (16b cut Q.5; doc 16 §5). Two `#routePanel` branches,
// two hydration queries, the once-per-match screen switch and the 2 s map
// self-heal — and the observer rule over the whole of what the screen prints.
describe("the cockpit feeds", () => {
  const COCKPIT_FEED = "arena.cockpit";
  const ARENA_MAP_FEED = "arena.map";

  const cockpitFrame = (): Record<string, unknown> =>
    JSON.parse(
      readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/cockpit_frame.json"), "utf8"),
    ) as Record<string, unknown>;
  const arenaMap = (): Record<string, unknown> =>
    JSON.parse(
      readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/arena_map.json"), "utf8"),
    ) as Record<string, unknown>;
  const viewFrame = (): Record<string, unknown> =>
    JSON.parse(
      readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/view_frame.json"), "utf8"),
    ) as Record<string, unknown>;

  const ARENA_PANELS = [COCKPIT_FEED, ARENA_MAP_FEED];

  /** Whether the strip carries the cockpit tab (16b cut B.3, `T-63`). */
  const hasTab = (app: GameApp): boolean =>
    app.screens.available(app.gate).some((d) => d.id === "cockpit");
  const clearedFrame = (): Record<string, unknown> => ({
    schema: { id: "arena.cockpit", version: 1, retain: "latest" },
    seated: false,
  });

  it("hydrates both feeds, and neither on a server that publishes neither", () => {
    const { app, ws } = boot(ARENA_PANELS);
    expect(ws.reqs().map(feedOf)).toEqual([null, null, COCKPIT_FEED, ARENA_MAP_FEED]);
    // 16b cut B.3 (`T-63`, Q21): the manifest alone earns no tab — a
    // bystander on an arena server has none until a frame seats them.
    expect(hasTab(app)).toBe(false);
    expect(app.gate.cockpitSeated).toBe(false);
    const bare = boot([SHEET_FEED]);
    expect(bare.ws.reqs().map(feedOf).filter((f) => f !== null)).toEqual([]);
    expect(hasTab(bare.app)).toBe(false);
  });

  it("routes a pushed frame and a pulled map, and opens the screen once a match", () => {
    const { app, ws } = boot(ARENA_PANELS);
    const before = ws.reqs().length;
    expect(app.screens.active).toBe("scene");

    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));

    expect(app.cockpit.seated).toBe(true);
    expect(app.cockpit.matchId).toBe(7);
    expect(app.screens.active).toBe("cockpit"); // the first frame opens it
    // …and asks for nothing: the hello pass already pulled the ground, and
    // that ask armed the latch.
    expect(ws.reqs().slice(before)).toHaveLength(0);

    ws.deliver(panel(4, ARENA_MAP_FEED, "keyframe", arenaMap()));
    expect(app.cockpit.holdsTheGround).toBe(true);

    // A pilot who walks away is not dragged back by the next pulse, and the
    // held ground is never asked for again.
    app.screens.switchTo("scene", app.gate);
    ws.deliver(panel(5, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.screens.active).toBe("scene");
    expect(ws.reqs().slice(before)).toHaveLength(0);

    // 16b cut B.3 (`T-63`, Q21): the tab came with the first seated frame and
    // goes with the cleared one; a pilot standing on it lands on the scene.
    expect(hasTab(app)).toBe(true);
    app.screens.switchTo("cockpit", app.gate);
    ws.deliver(panel(6, COCKPIT_FEED, "keyframe", clearedFrame()));
    expect(hasTab(app)).toBe(false);
    expect(app.screens.activeDef(app.gate).id).toBe("scene");
    expect(app.screens.switchDigit(0, app.gate)).toBe(false);
    // `active` itself still names the cockpit (only `activeDef` falls back),
    // so put her on the scene for real: the reopen below must be a switch.
    app.screens.switchTo("scene", app.gate);
    expect(app.screens.active).toBe("scene");
    // A new seat brings it back and opens it.
    ws.deliver(panel(7, COCKPIT_FEED, "keyframe", { ...cockpitFrame(), match: 8 }));
    expect(app.screens.active).toBe("cockpit");
    expect(hasTab(app)).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
  });

  it("takes a VIEW composite as one more status frame", () => {
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", viewFrame()));
    expect(app.cockpit.seated).toBe(true);
    expect(app.cockpit.frame?.callsign).toBe("RA");
    expect(app.cockpit.frame?.contacts).toHaveLength(4);
  });

  it("pulls the whole arena's terrain ONCE on an attach, not twice", () => {
    // The hello pass asks for the ground (it is pulled and never pushed, so a
    // re-attach holds none), and that ask ARMS the store's latch. Before the
    // fix the first cockpit frame found the latch unarmed and asked again, so
    // a pilot re-attaching mid-match was sent the whole terrain twice on a
    // socket that had only just reopened.
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000); // the hello pass's ask is stamped here
    const { ws } = boot(ARENA_PANELS);
    const asks = (): number => ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).length;
    expect(asks()).toBe(1);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(1);
    // The hello's ask is a wildcard, not a free pass: if it is never answered
    // the self-heal still takes over on its own cadence.
    at(102_000);
    ws.deliver(panel(4, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(2);
  });

  it("re-asks for a dropped map at most once every 2 s", () => {
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { ws } = boot(ARENA_PANELS);
    // The hello pass already asked once, stamped at 100 s; that ask covers the
    // first frame's match, so the SELF-HEAL's asks are counted from there and
    // begin only once it has aged out.
    const base = ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).length;
    expect(base).toBe(1);
    const asks = (): number =>
      ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).length - base;

    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(0); // the hello's ask still stands
    // Inside the latch: nothing, however many pulses arrive.
    at(101_000);
    ws.deliver(panel(4, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(0);
    at(102_000);
    ws.deliver(panel(5, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(1);
    // The unseated answer is dropped and does NOT buy a fresh ask.
    ws.deliver(
      panel(6, ARENA_MAP_FEED, "keyframe", {
        schema: { id: "arena.map", version: 1, shape: "terrain", retain: "latest" },
        seated: false,
      }),
    );
    at(102_500);
    ws.deliver(panel(7, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(1);
    // The real ground stops the asking for good.
    ws.deliver(panel(8, ARENA_MAP_FEED, "keyframe", arenaMap()));
    at(200_000);
    ws.deliver(panel(9, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(1);
  });

  it("heals a lost map on a timer, with no frame to ask on (B-59)", () => {
    // The arena pushes a frame only when one changes, so a pilot sitting still
    // whose map answer was dropped has no frame to ask on. The heal asks on a
    // `MAP_ASK_MS` interval through the same latch the frames use, and stops
    // the moment the ground lands.
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { app, ws } = boot(ARENA_PANELS);
    const mapReqs = (): ReqFrame[] => ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED);
    const asks = (): number => mapReqs().length;
    const idle = vi.getTimerCount();
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asks()).toBe(1); // the hello pass's, which the frame counts as its own
    expect(vi.getTimerCount()).toBe(idle + 1); // seated, dark: the heal runs
    // No frame arrives again. The hello's ask ages out and the timer asks.
    vi.advanceTimersByTime(MAP_ASK_MS); // 102_000
    expect(asks()).toBe(2);

    // Each tick asks through the LATCH, not on the interval's say-so: a pull
    // at 103_000 arms it, so the tick at 104_000 — 1 s after the pull — asks
    // nothing, and the next one, the latch aged, does.
    vi.advanceTimersByTime(1_000);
    expect(app.runBinding("p")).toBe(true);
    expect(asks()).toBe(3);
    vi.advanceTimersByTime(1_000); // 104_000
    expect(asks()).toBe(3);
    vi.advanceTimersByTime(MAP_ASK_MS); // 106_000
    expect(asks()).toBe(4);

    // A wrong answer — another match's map, a late reply — is dropped and
    // keeps the heal running: the next tick asks again.
    const wrong = { ...arenaMap(), match: 6 };
    ws.deliver(reply(4, mapReqs()[3]!.id, panelEvent(ARENA_MAP_FEED, "keyframe", wrong)));
    expect(app.cockpit.map).toBeNull();
    expect(vi.getTimerCount()).toBe(idle + 1);
    vi.advanceTimersByTime(MAP_ASK_MS); // 108_000
    expect(asks()).toBe(5);

    // The ground lands: the interval is cleared, and nothing more is asked.
    ws.deliver(panel(5, ARENA_MAP_FEED, "keyframe", arenaMap()));
    expect(vi.getTimerCount()).toBe(idle);
    vi.advanceTimersByTime(5 * MAP_ASK_MS);
    expect(asks()).toBe(5);
  });

  it("stops the heal on a cleared frame while dark, on a closed client and on a reset (B-59)", () => {
    vi.stubGlobal(
      "localStorage",
      memoryStorage({
        "rustmush.token": "tok",
        "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
      }),
    );
    const cleared = {
      schema: { id: "arena.cockpit", version: 1, retain: "latest" },
      seated: false,
    };
    const { app, ws } = boot(ARENA_PANELS);
    const asks = (s: FakeSocket): number => s.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).length;
    const idle = vi.getTimerCount();

    // The seat goes while the ground is still dark: the interval goes with it.
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(vi.getTimerCount()).toBe(idle + 1);
    ws.deliver(panel(4, COCKPIT_FEED, "keyframe", cleared));
    expect(vi.getTimerCount()).toBe(idle);
    const after = asks(ws);
    vi.advanceTimersByTime(5 * MAP_ASK_MS);
    expect(asks(ws)).toBe(after);

    // Seated and dark again, then the client is closed: nothing will answer.
    ws.deliver(panel(5, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(vi.getTimerCount()).toBe(idle + 1);
    app.client.close();
    expect(app.session.status).toBe("closed");
    expect(vi.getTimerCount()).toBe(idle);

    // Seated and dark on a fresh session, then a character switch resets it.
    app.enterGame(7);
    const next = openHello(ARENA_PANELS);
    next.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(vi.getTimerCount()).toBe(idle + 1);
    app.enterGame(7);
    expect(app.cockpit.seated).toBe(false);
    expect(vi.getTimerCount()).toBe(idle);
    const quiet = asks(next);
    vi.advanceTimersByTime(5 * MAP_ASK_MS);
    expect(asks(next)).toBe(quiet);
  });

  it("a `seated: false` map answer to a held seat asks for the cockpit once (world restart)", () => {
    // A socket that rides a world restart behind the gateway is handed no
    // hello and no cleared frame (`web_restart_survival.rs`), and the match
    // died with the world. The heal's map ask is answered `seated: false`:
    // that answer asks for the cockpit, whose cleared answer ends the seat
    // and the heal — instead of an ask every `MAP_ASK_MS` for the session.
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { app, ws } = boot(ARENA_PANELS);
    const idle = vi.getTimerCount();
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    const cockpitAsks = (): ReqFrame[] => ws.reqs().filter((r) => feedOf(r) === COCKPIT_FEED);
    const before = cockpitAsks().length;
    vi.advanceTimersByTime(MAP_ASK_MS);
    const ask = ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).at(-1)!;
    const unseated = { schema: { id: "arena.map", version: 1, retain: "latest" }, seated: false };
    ws.deliver(reply(4, ask.id, panelEvent(ARENA_MAP_FEED, "keyframe", unseated)));
    expect(cockpitAsks()).toHaveLength(before + 1);
    ws.deliver(
      reply(
        5,
        cockpitAsks().at(-1)!.id,
        panelEvent(COCKPIT_FEED, "keyframe", {
          schema: { id: "arena.cockpit", version: 1, retain: "latest" },
          seated: false,
        }),
      ),
    );
    expect(app.cockpit.seated).toBe(false);
    expect(vi.getTimerCount()).toBe(idle);
    // A bystander's own unseated answer — no seat held — asks for nothing.
    ws.deliver(panel(6, ARENA_MAP_FEED, "keyframe", unseated));
    expect(cockpitAsks()).toHaveLength(before + 1);
  });

  it("`p` pulls the cockpit again, and the map only while it is dark", () => {
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.screens.active).toBe("cockpit");
    const before = ws.reqs().length;
    const since = (): (string | null)[] => ws.reqs().slice(before).map(feedOf);

    at(105_000);
    expect(app.runBinding("p")).toBe(true);
    expect(since()).toEqual([COCKPIT_FEED, ARENA_MAP_FEED]);
    // The pull's ask armed the latch: the next frame does not double it.
    ws.deliver(panel(4, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(since()).toEqual([COCKPIT_FEED, ARENA_MAP_FEED]);

    // With the ground held it asks for the frame alone.
    ws.deliver(panel(5, ARENA_MAP_FEED, "keyframe", arenaMap()));
    app.runBinding("p");
    expect(since()).toEqual([COCKPIT_FEED, ARENA_MAP_FEED, COCKPIT_FEED]);
  });

  it("a full rehydrate mid-match empties the cockpit and does not drag the pilot back (T-42)", () => {
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(panel(4, ARENA_MAP_FEED, "keyframe", arenaMap()));
    expect(app.cockpit.holdsTheGround).toBe(true);
    expect(app.cockpit.notices).toHaveLength(2);
    app.screens.switchTo("scene", app.gate);

    // The gap outgrows the ring: a new socket, its hello, then the full
    // rehydrate — in that order on the wire (`mush-net-web`'s pump).
    ws.drop();
    vi.advanceTimersByTime(500); // the first backoff window, `random: () => 1`
    const next = openHello(ARENA_PANELS);
    next.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    expect(app.cockpit.seated).toBe(false);
    expect(app.cockpit.map).toBeNull();
    expect(app.cockpit.notices).toEqual([]);
    // 16b cut B.3: the tab survives the rehydrate — the seat very likely
    // still stands, and the re-asked frame's answer decides.
    expect(hasTab(app)).toBe(true);
    const asked = (feed: string): ReqFrame[] => next.reqs().filter((r) => feedOf(r) === feed);
    // The hello pass re-asked both feeds, once each.
    expect(asked(COCKPIT_FEED)).toHaveLength(1);
    expect(asked(ARENA_MAP_FEED)).toHaveLength(1);

    // The answer re-seats the pilot in the SAME match: the switch for match 7
    // was already spent, so the pilot stays on the scene screen…
    next.deliver(reply(5, asked(COCKPIT_FEED)[0]!.id, panelEvent(COCKPIT_FEED, "keyframe", cockpitFrame())));
    expect(app.cockpit.seated).toBe(true);
    expect(app.cockpit.matchId).toBe(7);
    expect(app.screens.active).toBe("scene");
    // …and the ground is not pulled twice: the hello's ask stands for it.
    expect(asked(ARENA_MAP_FEED)).toHaveLength(1);

    // The map answer lands; a frame past `MAP_ASK_MS` asks nothing more.
    next.deliver(reply(6, asked(ARENA_MAP_FEED)[0]!.id, panelEvent(ARENA_MAP_FEED, "keyframe", arenaMap())));
    expect(app.cockpit.holdsTheGround).toBe(true);
    vi.advanceTimersByTime(MAP_ASK_MS);
    next.deliver(panel(7, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(asked(ARENA_MAP_FEED)).toHaveLength(1);
    expect(app.screens.active).toBe("scene");
    expect(hasTab(app)).toBe(true);
  });

  it("a rehydrate whose seat went meanwhile loses the tab on the cleared answer (16b cut B.3)", () => {
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.screens.active).toBe("cockpit");
    ws.drop();
    vi.advanceTimersByTime(500);
    const next = openHello(ARENA_PANELS);
    next.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    // Still standing on the tab through the blip…
    expect(hasTab(app)).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
    // …until the hello pass's re-ask is answered with the cleared frame.
    const ask = next.reqs().filter((r) => feedOf(r) === COCKPIT_FEED)[0]!;
    next.deliver(reply(5, ask.id, panelEvent(COCKPIT_FEED, "keyframe", clearedFrame())));
    expect(hasTab(app)).toBe(false);
    expect(app.screens.activeDef(app.gate).id).toBe("scene");
  });

  it("a tab resumed into the next match shows only that match's notices (B-80, T-42)", () => {
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    const match7 = [...app.cockpit.notices];
    expect(match7).toEqual(["X4 new contact", "BA section destroyed: turret"]);
    app.screens.switchTo("scene", app.gate);

    // A short drop: the gap replays from the ring, and the ring holds match
    // 8's keyframe with NO cleared frame before it (a newer frame of the feed
    // supersedes the cleared one — `cockpit_cleared`'s doc).
    ws.drop();
    vi.advanceTimersByTime(500);
    const next = openHello(ARENA_PANELS);
    next.deliver(env(0, { t: "resumed", d: { replayed: true } }));
    next.deliver(panel(9, COCKPIT_FEED, "keyframe", { ...cockpitFrame(), match: 8 }));

    expect(app.cockpit.matchId).toBe(8);
    // Match 8's two lines, and none of match 7's above them.
    expect(app.cockpit.notices).toEqual(["X4 new contact", "BA section destroyed: turret"]);
    // A new match opens the screen, once.
    expect(app.screens.active).toBe("cockpit");
    app.screens.switchTo("scene", app.gate);
    next.deliver(panel(10, COCKPIT_FEED, "keyframe", { ...cockpitFrame(), match: 8, notices: [] }));
    expect(app.screens.active).toBe("scene");
    expect(app.cockpit.notices).toHaveLength(2);
  });

  it("the screen opens at once and the draft survives", () => {
    // The only server-driven screen switch in this client. Since 16b cut B.1
    // the box is the shell's (`Game.svelte`), so switching between screens
    // that carry it never unmounts it: the draft, the sticky mode, the history
    // buffer and the focus all survive, and the switch has nothing to wait for.
    const { app, ws } = boot(ARENA_PANELS);
    let blurs = 0;
    app.composer = {
      focus: () => {},
      blur: () => {
        blurs += 1;
      },
      cycleMode: () => {},
      text: () => "pose x",
      setText: () => {},
    };
    app.screens.inputFocused = true;
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.cockpit.seated).toBe(true);
    // On this very frame, mid-sentence.
    expect(app.screens.active).toBe("cockpit");
    // …and the half-typed line is untouched: still focused, never blurred.
    expect(app.screens.inputFocused).toBe(true);
    expect(app.composer.text()).toBe("pose x");
    expect(blurs).toBe(0);
    // Still once per match: a pilot who walks away is not dragged back.
    app.screens.switchTo("scene", app.gate);
    ws.deliver(panel(4, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.screens.active).toBe("scene");
  });

  it("a prefill leaves the pilot on the cockpit", async () => {
    // What `CockpitScreen`'s contact click and its five prefill keys do: the
    // box sits under the cockpit screen too (16b cut B.1, Q5), so the order is
    // put in it where the pilot is looking — the line ready, unsent.
    const { app, ws } = boot(ARENA_PANELS);
    const focuses: (string | undefined)[] = [];
    app.composer = {
      focus: (prefill?: string) => {
        focuses.push(prefill);
      },
      blur: () => {},
      cycleMode: () => {},
      text: () => "",
      setText: () => {},
    };
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.screens.active).toBe("cockpit");
    app.focusComposer("lock BA");
    await Promise.resolve();
    expect(app.screens.active).toBe("cockpit");
    expect(focuses).toEqual(["lock BA"]);
  });

  it("a reply lands in the cockpit ring while seated", () => {
    // 16b cut B.1: with the pilot kept on screen 0 the scrollback is not in
    // front of them, so every `Sys` line that arrives while seated is ALSO
    // appended to the notices ring, as plain text (Q20: two screens may word
    // one event). Before any frame, it is the scrollback's alone.
    const { app, ws } = boot(ARENA_PANELS);
    const sys = (seq: number, text: string): Envelope =>
      env(seq, { t: "sys", d: [{ text: "Locking " }, { text }] } as OutputEvent);

    ws.deliver(sys(2, "RB."));
    expect(lineTexts(app)).toContain("Locking RB.");
    expect(app.cockpit.notices).toEqual([]);

    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(sys(4, "BA."));
    expect(app.cockpit.notices.at(-1)).toBe("Locking BA.");
    expect(lineTexts(app)).toContain("Locking BA.");
  });

  it("a seated pilot is told of the token cool-off, and of a dropped order, on the ring", () => {
    // The 4429 close (cut L) waits a whole minute before reconnecting; its
    // client-local line reaches the cockpit ring as well as the scrollback,
    // because on screen 0 the ring is the only reply surface (16b cut B.1).
    // So does the line a typed order gets while the socket is down.
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    expect(app.cockpit.seated).toBe(true);

    ws.drop(CLOSE_TOKEN_COOLOFF);
    const cooloff = "Too many reconnects from this network; retrying in one minute.";
    expect(app.cockpit.notices.at(-1)).toBe(cooloff);
    expect(lineTexts(app)).toContain(cooloff);

    app.submit("lock BA");
    expect(app.cockpit.notices.at(-1)).toBe("Not connected — command dropped.");
    expect(lineTexts(app)).toContain("Not connected — command dropped.");
  });

  it("a notice is rung once: the bridge's copy of its sentence is not a reply", () => {
    // What the bridge really sends each pulse (`arena_bridge.rs`, `on_frames`):
    // every notice's sentence as a `Sys` line tagged `arena-notice`, THEN the
    // keyframe carrying the same notices. The ring composes the notice from the
    // frame; ringing the sentence too would say it twice (16b cut F.2).
    const { app, ws } = boot(ARENA_PANELS);
    const tagged = (seq: number, text: string, tag: string): Envelope =>
      env(seq, { t: "sys", d: [{ text, tag }] } as OutputEvent);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    const before = app.cockpit.notices.length;

    ws.deliver(tagged(4, "New contact X4: bearing 030°, far.", NOTICE_SENTENCE_TAG));
    ws.deliver(tagged(5, "BA's turret is destroyed.", NOTICE_SENTENCE_TAG));
    ws.deliver(panel(6, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(tagged(7, "Locking BA.", "arena"));

    // The frame's two notices, composed, once each; the reply after them.
    expect(app.cockpit.notices.slice(before)).toEqual([
      "X4 new contact",
      "BA section destroyed: turret",
      "Locking BA.",
    ]);
    expect(app.cockpit.notices).not.toContain("BA's turret is destroyed.");
    // The scrollback still owns every sentence, the notices' own included.
    expect(lineTexts(app)).toContain("BA's turret is destroyed.");
    expect(lineTexts(app)).toContain("Locking BA.");
  });

  it("a new seat clears the previous mind trace and closes its readout", () => {
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(panel(4, "arena.mind", "keyframe", { schema: { id: "arena.mind", version: 1 }, available: true, unit: 7, traces: [] }));
    app.mindReadoutOpen = true;
    expect(app.mindTrace?.unit).toBe(7);
    ws.deliver(panel(5, COCKPIT_FEED, "keyframe", { ...cockpitFrame(), callsign: "RB" }));
    expect(app.mindTrace).toBeNull();
    expect(app.mindReadoutOpen).toBe(false);
  });

  it("a cleared frame empties the panel; a Sys refusal stays out of scrollback", () => {
    const { app, ws } = boot(ARENA_PANELS);
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(panel(4, ARENA_MAP_FEED, "keyframe", arenaMap()));
    const lines = app.scrollback.entries.length;

    ws.deliver(
      panel(5, COCKPIT_FEED, "keyframe", {
        schema: { id: "arena.cockpit", version: 1, retain: "latest" },
        seated: false,
      }),
    );
    expect(app.cockpit.seated).toBe(false);
    expect(app.cockpit.map).toBeNull();
    expect(app.cockpit.notices).toEqual([]);

    // A refusal answering either arena query is the panel's business: the
    // screen prints its own empty state and nothing lands in scrollback.
    const mapReq = ws.reqs().filter((r) => feedOf(r) === ARENA_MAP_FEED).pop()!;
    ws.deliver(reply(6, mapReq.id, SYS_REFUSAL));
    expect(app.scrollback.entries).toHaveLength(lines);
  });

  it("the observer rule: the screen's whole text carries nothing outside the payload", () => {
    const { app, ws } = boot([...ARENA_PANELS, UNIT_FEED, CONTACTS_FEED, SHEET_FEED]);
    // Seed every other store the shell holds with a decoy identity. A client
    // that joined a contact to a name it happened to know would break doc 16
    // §1 without the server ever sending a wrong byte.
    app.who.replace([{ name: "Decoy", idle: 0, conn: 0, doing: null } as never], Date.now());
    app.presence.applyComposing("Decoy", true, Date.now());
    ws.deliver(
      panel(2, UNIT_FEED, "keyframe", {
        object: { id: "#909", name: "Decoy" },
        revision: 1,
        state: "ok",
        sections: [],
        cells: [],
        components: [],
        capabilities: [],
      }),
    );
    ws.deliver(panel(3, COCKPIT_FEED, "keyframe", cockpitFrame()));
    ws.deliver(panel(4, ARENA_MAP_FEED, "keyframe", arenaMap()));
    // And the next pulse exactly as the bridge sends it to a seated pilot: the
    // notices' own sentences as `Sys` lines first (16b cut B.1: tagged, so the
    // reply ring leaves them out), then the frame that carries the notices.
    for (const [seq, text] of [
      [5, "New contact X4: bearing 030°, far."],
      [6, "BA's turret is destroyed."],
    ] as const) {
      ws.deliver(env(seq, { t: "sys", d: [{ text, tag: NOTICE_SENTENCE_TAG }] } as OutputEvent));
    }
    ws.deliver(panel(7, COCKPIT_FEED, "keyframe", cockpitFrame()));

    const frame = app.cockpit.frame!;
    const win = composeWindow(app.cockpit.map!, frame, 78, 20);
    const text = [
      ...cockpitViewLines(cockpitView(frame, win, app.cockpit.notices)),
      ...win.grid,
    ].join("\n");

    expect(text).not.toContain("Decoy");
    expect(text).not.toContain("#909");
    expect(text).not.toContain("#101"); // the pilot's own object number either
    // No contact id, no lock's copy of one: a sixteen-digit word never prints.
    expect(text).not.toMatch(/[0-9a-f]{16}/);
    // And none of the arena's own sentences, which the scrollback already owns.
    expect(text).not.toContain("New contact X4: bearing 030");
    expect(text).not.toContain("BA's turret is destroyed.");
  });

  it("the shell mounts the one command box, and the screens' own markup follows 16b cut B.1", () => {
    // Probes in the whitelist's style, for the Svelte halves the pure tests
    // cannot reach (this file runs in node and mounts nothing; the one jsdom
    // mount is `components/CockpitScreen.test.ts`, 16b cut B.2). Each one
    // fails on the code before cut B.1.
    const read = (name: string): string =>
      readFileSync(resolve(process.cwd(), `src/components/${name}`), "utf8");

    // `B-08`: the box and its palette are the shell's, under `.screen` and
    // before the legend, on every screen but the channel screen.
    const game = read("Game.svelte");
    expect(game).toMatch(
      /\{#if active\.id !== "channels"\}\s*<div class="composer-wrap">\s*<CommandPalette \{app\} \/>\s*<Composer bind:this=\{composer\} \{app\} \/>/,
    );
    const box = game.indexOf("<Composer bind:this={composer}");
    expect(game.indexOf('<div class="screen">')).toBeLessThan(box);
    expect(game.indexOf('<footer class="legend"')).toBeGreaterThan(box);
    expect(game).toContain("app.composer = composer ?? null;");
    // …and the scene screen no longer mounts or registers one.
    const scene = read("SceneScreen.svelte");
    for (const gone of ["import Composer", "import CommandPalette", "<Composer", "app.composer"]) {
      expect(scene, `SceneScreen still has ${gone}`).not.toContain(gone);
    }

    // `B-60`: only the glyphs moved — `zoom in` shows `+` and still runs
    // `zoom(-1)` (fewer cells), `zoom out` shows `−` and runs `zoom(1)`.
    const cockpit = read("CockpitScreen.svelte");
    expect(cockpit).toMatch(/aria-label="zoom in" onclick=\{\(\) => zoom\(-1\)\}>\+<\/button>/);
    expect(cockpit).toMatch(/aria-label="zoom out" onclick=\{\(\) => zoom\(1\)\}>−<\/button>/);

    // `B-09`: the paint loop labels what `labelPlan` says, once per frame, and
    // no longer gates every label on the cell size.
    expect(cockpit).toContain("labelPlan(w.markers, f.lock_designator, px)");
    expect(cockpit).toContain("if (labels[i] === true)");
    expect(cockpit).not.toContain("if (px >= 6)");
  });

  it("CockpitScreen.svelte touches nothing on the app but the cockpit", () => {
    // The component-level half of the rule, as a probe: this file has no DOM
    // to mount into, the strings the screen renders are pinned above through
    // the same pure functions it calls, and `components/CockpitScreen.test.ts`
    // mounts it on an app that IS this whitelist (16b cut B.2). What a probe
    // adds is that it calls nothing else.
    //
    // It is a WHITELIST and not a list of forbidden names on purpose: a
    // blocklist passes for every member it forgets, and `GameApp` grows. The
    // two below are the whole of what the screen may reach for — the payload
    // it was handed, and the box it prefills. Anything else, an identity store
    // or a send path, fails here.
    const src = readFileSync(
      resolve(process.cwd(), "src/components/CockpitScreen.svelte"),
      "utf8",
    );
    const reached = new Set([...src.matchAll(/\bapp\.(\w+)/g)].map((m) => m[1]));
    expect([...reached].sort()).toEqual(["cockpit", "focusComposer", "mindReadoutOpen", "refreshMindTrace"]);
    // And nothing on this screen sends: the five ordering verbs prefill only.
    for (const send of ["app.submit", "app.runBinding", "app.client", "app.act("]) {
      expect(src, `CockpitScreen calls ${send}`).not.toContain(send);
    }
  });
});

// The 2026-09-20 incident's client half. A gateway restart leaves an open tab
// holding a token the server no longer knows; the server now refuses it with
// close 4401 (`mush-net-web/src/ws.rs`), and the tab must believe it. Retrying
// a dead token is what fed the address to `IpGuard` five times in fifteen
// seconds and locked the owner out of SSH.
describe("a refused session token", () => {
  it("is dropped, ends the reconnect loop, and returns to login with one line", () => {
    const store = memoryStorage({
      "rustmush.token": "tok",
      "rustmush.account": "nick",
      "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
    });
    vi.stubGlobal("localStorage", store);
    const app = makeApp();
    app.enterGame(7);
    const ws = openHello();
    expect(app.screen).toBe("game");

    ws.drop(4401); // the server: "your session has ended"

    expect(app.screen).toBe("login");
    expect(app.loginNotice).toBe("Your session expired — please sign in again.");
    // The credential is GONE, not merely unused: a reload must not resurrect
    // the reconnect loop the close just stopped.
    expect((store as { getItem(k: string): string | null }).getItem("rustmush.token")).toBeNull();
    expect((store as { getItem(k: string): string | null }).getItem("rustmush.chars")).toBeNull();

    // And nothing reconnects, however long we wait.
    const sockets = FakeSocket.instances.length;
    vi.advanceTimersByTime(300_000);
    expect(FakeSocket.instances).toHaveLength(sockets);
  });

  it("a network drop is NOT a refusal: the token is kept and the backoff runs", () => {
    const store = memoryStorage({
      "rustmush.token": "tok",
      "rustmush.account": "nick",
      "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
    });
    vi.stubGlobal("localStorage", store);
    const app = makeApp();
    app.enterGame(7);
    const ws = openHello();

    ws.drop(); // 1006: the blip a browser reports for anything that is not a close frame

    expect(app.screen).toBe("game");
    expect(app.loginNotice).toBeNull();
    expect((store as { getItem(k: string): string | null }).getItem("rustmush.token")).toBe("tok");
    // The retry carries the SAME token, in the same subprotocol slot.
    vi.advanceTimersByTime(500);
    expect(FakeSocket.instances).toHaveLength(2);
    expect(lastSocket().protocols).toEqual(["rustmush.v1.tok"]);
  });

  // The server closes 1012 when the game half is restarting and 1013 when
  // admission is full — an ordinary `reload`, or a reconnect landing under
  // load. Neither says anything about the credential, so neither may take
  // the 4401 branch: widening that branch would sign every web player out on
  // a routine deploy.
  it.each([
    [1012, "the game half is restarting"],
    [1013, "admission is full"],
  ])("close %i (%s) keeps the token and retries", (code) => {
    const store = memoryStorage({
      "rustmush.token": "tok",
      "rustmush.account": "nick",
      "rustmush.chars": JSON.stringify([{ key: 7, name: "Atlas" }]),
    });
    vi.stubGlobal("localStorage", store);
    const app = makeApp();
    app.enterGame(7);
    const ws = openHello();

    ws.drop(code);

    expect(app.screen).toBe("game");
    expect(app.loginNotice).toBeNull();
    expect((store as { getItem(k: string): string | null }).getItem("rustmush.token")).toBe("tok");
    vi.advanceTimersByTime(500);
    expect(FakeSocket.instances).toHaveLength(2);
    expect(lastSocket().protocols).toEqual(["rustmush.v1.tok"]);
  });
});

describe("the attach look (16b cut B.4, B-14)", () => {
  /** A room with one lettered exit, so the exit keys have something to say. */
  function room(name: string): OutputEvent {
    return {
      t: "room",
      d: {
        name,
        id: "#2",
        desc: [{ text: "A plain room." }],
        contents: [],
        exits: [{ name: "north", aliases: ["n"] }],
      },
    } as OutputEvent;
  }

  /** What one app shows of a room: the band, the exit keys, the scrollback. */
  function shown(app: GameApp): { name: string | null; keys: [string, string][]; lines: string[] } {
    return {
      name: app.roomView?.name ?? null,
      keys: [...app.screens.exits.entries()],
      lines: lineTexts(app),
    };
  }

  it("applies a room that arrives as the answer to the `room` query exactly as a bare one", () => {
    // The gateway's attach asks the core `room` query with the uncorrelated
    // id 0, so the room arrives as `reply { id: 0, event: room }` to this one
    // connection. Nothing is pending under id 0 (request ids start at 1).
    const bare = boot();
    bare.ws.deliver(env(10, room("Hall")));
    const replied = boot();
    replied.ws.deliver(reply(10, 0, room("Hall")));

    expect(shown(replied.app)).toEqual(shown(bare.app));
    expect(replied.app.roomView?.name).toBe("Hall");
    expect(replied.app.screens.exits.get("n")).toBe("north");
    expect(lineTexts(replied.app).some((line) => line.includes("Hall"))).toBe(true);
  });
});

// The replay on the web (16b cut I.2; 16c N9, Q37): the pushed grid opens it on
// screen 0, the document is asked ONCE, the ground under the latch, and the
// four keys step, focus and close it over the document the client holds.
describe("the replay on screen 0 (16b cut I.2)", () => {
  const COCKPIT_FEED = "arena.cockpit";
  const ARENA_MAP_FEED = "arena.map";
  const REPLAY_FEED = "arena.replay";
  const PANELS = [COCKPIT_FEED, ARENA_MAP_FEED, REPLAY_FEED];

  const fixture = (dir: string, name: string): Record<string, unknown> =>
    JSON.parse(readFileSync(resolve(process.cwd(), `src/lib/fixtures/${dir}`, name), "utf8")) as Record<
      string,
      unknown
    >;
  const grid = (): Record<string, unknown> => fixture("replay", "replay_window.json");
  const doc = (): Record<string, unknown> => ({
    schema: { id: REPLAY_FEED, version: 1, shape: "replay" },
    ...fixture("replay", "replay.json"),
  });
  const ground = (): Record<string, unknown> => fixture("cockpit", "arena_map.json");
  const cleared = (): Record<string, unknown> => ({
    schema: { id: COCKPIT_FEED, version: 1, retain: "latest" },
    seated: false,
  });
  const hasTab = (app: GameApp): boolean => app.screens.available(app.gate).some((d) => d.id === "cockpit");
  /** The feeds asked since `from`, in order. */
  const asked = (ws: FakeSocket, from: number): (string | null)[] => ws.reqs().slice(from).map(feedOf);
  const lastAsk = (ws: FakeSocket, feed: string): ReqFrame =>
    ws.reqs().filter((r) => feedOf(r) === feed).at(-1)!;

  /** A viewer with no seat, the replay of match 7 open, its document and its
   * ground answered. */
  function opened(): { app: GameApp; ws: FakeSocket } {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", grid()));
    ws.deliver(reply(6, lastAsk(ws, REPLAY_FEED).id, panelEvent(REPLAY_FEED, "keyframe", doc())));
    ws.deliver(reply(7, lastAsk(ws, ARENA_MAP_FEED).id, panelEvent(ARENA_MAP_FEED, "keyframe", ground())));
    return { app, ws };
  }

  it("a grid opens screen 0 with no seat, asks the document once and the ground, both as `{}`", () => {
    const { app, ws } = boot(PANELS);
    expect(hasTab(app)).toBe(false);
    const before = ws.reqs().length;
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", grid()));
    expect(app.replay.open).toBe(true);
    expect(app.pictureShown).toBe(true);
    expect(app.gate.pictureOpen).toBe(true);
    expect(hasTab(app)).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
    expect(asked(ws, before)).toEqual([REPLAY_FEED, ARENA_MAP_FEED]);
    for (const r of ws.reqs().slice(before)) {
      // No params: there is nowhere to put them.
      expect(r.q).toEqual({ t: "panel_subscribe", d: { feed: feedOf(r) } });
    }
    // Outstanding: a second grid of the match asks nothing again.
    ws.deliver(panel(6, REPLAY_FEED, "keyframe", grid()));
    expect(asked(ws, before)).toEqual([REPLAY_FEED, ARENA_MAP_FEED]);
    // The document lands on the keyframe the grid showed.
    ws.deliver(reply(7, lastAsk(ws, REPLAY_FEED).id, panelEvent(REPLAY_FEED, "keyframe", doc())));
    expect(app.replay.doc?.match).toBe(7);
    expect(app.replay.cursorMs).toBe(5000);
    // A walk away is not undone by the next grid of the same match.
    app.screens.switchTo("scene", app.gate);
    ws.deliver(panel(8, REPLAY_FEED, "keyframe", grid()));
    expect(app.screens.active).toBe("scene");
    expect(app.screens.switchDigit(0, app.gate)).toBe(true);
  });

  it("the four keys step, focus and close through the screen's second table, and send nothing", () => {
    const { app, ws } = opened();
    const before = ws.reqs().length;
    const sent = ws.sentJson().length;
    expect(app.screens.activeKeys(app.gate).map((b) => b.key)).toEqual([",", ".", "u", "x", "p"]);
    expect(app.runBinding(".")).toBe(true);
    expect(app.replay.cursorMs).toBe(10000);
    expect(app.runBinding(",")).toBe(true);
    expect(app.replay.cursorMs).toBe(5000);
    expect(app.runBinding("u")).toBe(true);
    expect(app.replay.focus).toBe(1);
    // The cockpit's own keys are not dispatched under the replay.
    expect(app.runBinding("k")).toBe(false);
    expect(app.runBinding("c")).toBe(false);
    expect(ws.sentJson().length).toBe(sent);
    // `p` asks the document again (the ground is held, so it alone).
    expect(app.runBinding("p")).toBe(true);
    expect(asked(ws, before)).toEqual([REPLAY_FEED]);
    // `x` closes: dismissed, the tab gone, the scene in front.
    expect(app.runBinding("x")).toBe(true);
    expect(app.replay.open).toBe(false);
    expect(app.replay.dismissed.has(7)).toBe(true);
    expect(hasTab(app)).toBe(false);
    expect(app.screens.activeDef(app.gate).id).toBe("scene");
  });

  it("a replayed grid of a dismissed match does not re-open it; a fresh one does", () => {
    const { app, ws } = opened();
    app.runBinding("x");
    // The resume ring replays the retained grid with the seq it first had.
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", grid()));
    expect(app.replay.open).toBe(false);
    // A fresh `SIM REPLAY 7` — a newer seq — opens it again and asks again.
    const before = ws.reqs().length;
    ws.deliver(panel(30, REPLAY_FEED, "keyframe", grid()));
    expect(app.replay.open).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
    expect(asked(ws, before)).toContain(REPLAY_FEED);
  });

  it("a fresh attach's low seqs still reopen a match closed before the seam", () => {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5000, REPLAY_FEED, "keyframe", grid()));
    app.runBinding("x");
    expect(app.replay.dismissed.has(7)).toBe(true);
    // The gap outgrows the ring: a new socket, its hello, then the full
    // rehydrate — whose seqs count from 1 again.
    ws.drop();
    vi.advanceTimersByTime(500); // the first backoff window, `random: () => 1`
    const next = openHello(PANELS);
    next.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    expect(app.replay.dismissed.size).toBe(0);
    // `SIM REPLAY 7` typed on the new attach: its grid at seq 40 opens it.
    const before = next.reqs().length;
    next.deliver(panel(40, REPLAY_FEED, "keyframe", grid()));
    expect(app.replay.open).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
    expect(asked(next, before)).toContain(REPLAY_FEED);
  });

  it("`p` does not re-ask the document while an ask is outstanding", () => {
    vi.setSystemTime(new Date(100_000));
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", grid()));
    const count = (): number => ws.reqs().filter((r) => feedOf(r) === REPLAY_FEED).length;
    expect(count()).toBe(1);
    for (let i = 0; i < 5; i += 1) {
      app.runBinding("p");
    }
    expect(count()).toBe(1);
    // No answer by the latch: presumed dropped, and `p` asks once more.
    vi.advanceTimersByTime(REPLAY_ASK_MS);
    app.runBinding("p");
    app.runBinding("p");
    expect(count()).toBe(2);
  });

  it("`available: false` closes the picture and dismisses the match, and asks nothing", () => {
    const { app, ws } = opened();
    const before = ws.reqs().length;
    app.runBinding("p");
    ws.deliver(
      reply(
        9,
        lastAsk(ws, REPLAY_FEED).id,
        panelEvent(REPLAY_FEED, "keyframe", {
          schema: { id: REPLAY_FEED, version: 1, shape: "replay" },
          available: false,
          reason: "You have no replay open; type SIM REPLAY <match>.",
        }),
      ),
    );
    expect(app.replay.open).toBe(false);
    expect(app.replay.dismissed.has(7)).toBe(true);
    expect(asked(ws, before)).toEqual([REPLAY_FEED]);
    expect(app.screens.activeDef(app.gate).id).toBe("scene");
  });

  it("a seated pilot keeps the cockpit, and a seat closes an open replay", () => {
    const seated = boot(PANELS);
    seated.ws.deliver(panel(3, COCKPIT_FEED, "keyframe", fixture("cockpit", "cockpit_frame.json")));
    const before = seated.ws.reqs().length;
    seated.ws.deliver(panel(4, REPLAY_FEED, "keyframe", grid()));
    expect(seated.app.replay.open).toBe(false);
    expect(asked(seated.ws, before)).toEqual([]);

    const { app, ws } = opened();
    ws.deliver(panel(20, COCKPIT_FEED, "keyframe", { ...fixture("cockpit", "cockpit_frame.json"), match: 8 }));
    expect(app.replay.open).toBe(false);
    expect(app.replay.dismissed.has(7)).toBe(true);
    expect(app.pictureShown).toBe(false);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
  });

  it("a cleared cockpit keeps an open replay and its ground", () => {
    const { app, ws } = opened();
    ws.deliver(panel(20, COCKPIT_FEED, "keyframe", cleared()));
    expect(app.replay.open).toBe(true);
    expect(app.replay.holdsTheGround).toBe(true);
    expect(app.pictureShown).toBe(true);
    expect(hasTab(app)).toBe(true);
  });

  it("the hello pass re-asks an open replay's document and ground, once", () => {
    const { app, ws } = opened();
    const before = ws.reqs().length;
    ws.deliver(hello(PANELS));
    // The hydrate list: the hello pass — whose ground ask is the replay's too,
    // so the terrain is pulled once — then the replay's document.
    expect(asked(ws, before)).toEqual([null, null, COCKPIT_FEED, ARENA_MAP_FEED, REPLAY_FEED]);
    expect(app.replay.open).toBe(true);
    expect(app.replay.doc).toBeNull();
    // Outstanding: a grid of the match asks the document nothing more.
    ws.deliver(panel(40, REPLAY_FEED, "keyframe", grid()));
    expect(ws.reqs().slice(before).filter((r) => feedOf(r) === REPLAY_FEED)).toHaveLength(1);
  });

  it("heals a dark ground on the timer, and never asks the document on it", () => {
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { ws } = boot(PANELS);
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", grid()));
    const count = (feed: string): number => ws.reqs().filter((r) => feedOf(r) === feed).length;
    const maps = count(ARENA_MAP_FEED);
    expect(count(REPLAY_FEED)).toBe(1);
    vi.advanceTimersByTime(MAP_ASK_MS);
    expect(count(ARENA_MAP_FEED)).toBe(maps + 1);
    vi.advanceTimersByTime(10 * MAP_ASK_MS);
    expect(count(REPLAY_FEED)).toBe(1);
    ws.deliver(reply(9, lastAsk(ws, ARENA_MAP_FEED).id, panelEvent(ARENA_MAP_FEED, "keyframe", ground())));
    const held = count(ARENA_MAP_FEED);
    vi.advanceTimersByTime(10 * MAP_ASK_MS);
    expect(count(ARENA_MAP_FEED)).toBe(held);
  });

  it("ReplayScreen.svelte touches nothing on the app but the replay", () => {
    const src = readFileSync(resolve(process.cwd(), "src/components/ReplayScreen.svelte"), "utf8");
    const reached = new Set([...src.matchAll(/\bapp\.(\w+)/g)].map((m) => m[1]));
    expect([...reached].sort()).toEqual(["replay"]);
    for (const send of ["app.submit", "app.runBinding", "app.client", "app.act("]) {
      expect(src, `ReplayScreen calls ${send}`).not.toContain(send);
    }
    // And the shell mounts it on screen 0 only while a replay is drawn there.
    const game = readFileSync(resolve(process.cwd(), "src/components/Game.svelte"), "utf8");
    expect(game).toContain('{:else if active.id === "cockpit" && app.pictureShown}');
    expect(game.indexOf("<ReplayScreen {app} />")).toBeLessThan(game.indexOf("<CockpitScreen {app} />"));
  });
});

// The live picture on the web (16b cut I.4; 16c N10): a staffer's `@arena/watch`
// pushes the field on `arena.observer`; the first picture opens it LIVE on
// screen 0, closing an open replay; nothing steps it and only the server closes
// it; the hello pass re-asks it.
describe("the live picture on screen 0 (16b cut I.4)", () => {
  const COCKPIT_FEED = "arena.cockpit";
  const ARENA_MAP_FEED = "arena.map";
  const REPLAY_FEED = "arena.replay";
  const OBSERVER_FEED = "arena.observer";
  const PANELS = [COCKPIT_FEED, ARENA_MAP_FEED, REPLAY_FEED, OBSERVER_FEED];

  const fixture = (dir: string, name: string): Record<string, unknown> =>
    JSON.parse(readFileSync(resolve(process.cwd(), `src/lib/fixtures/${dir}`, name), "utf8")) as Record<
      string,
      unknown
    >;
  const live = (match = 7): Record<string, unknown> => ({
    schema: { id: OBSERVER_FEED, version: 1, retain: "latest" },
    match,
    now_ms: 90_000,
    clock: "1:30",
    tempo: "engaged",
    floor_ms: null,
    cell_cm: 1_000,
    width: 96,
    height: 64,
    in_flight: 2,
    units: [
      ["RA", "R", 6, 4, 90, 301, "pm"],
      ["BA", "B", 20, 14, 270, 302, "ph"],
    ],
  });
  const cleared = (): Record<string, unknown> => ({
    schema: { id: OBSERVER_FEED, version: 1, retain: "latest" },
    watching: false,
  });
  const hasTab = (app: GameApp): boolean => app.screens.available(app.gate).some((d) => d.id === "cockpit");
  const asked = (ws: FakeSocket, from: number): (string | null)[] => ws.reqs().slice(from).map(feedOf);

  it("a picture opens screen 0 live, asks the ground and never a document", () => {
    const { app, ws } = boot(PANELS);
    const before = ws.reqs().length;
    ws.deliver(panel(5, OBSERVER_FEED, "keyframe", live()));
    expect(app.replay.open).toBe(true);
    expect(app.replay.live).toBe(true);
    expect(app.pictureShown).toBe(true);
    expect(app.gate.pictureLive).toBe(true);
    expect(hasTab(app)).toBe(true);
    expect(app.screens.activeDef(app.gate).id).toBe("cockpit");
    expect(asked(ws, before)).toEqual([ARENA_MAP_FEED]);
    // A later picture asks nothing.
    ws.deliver(panel(6, OBSERVER_FEED, "keyframe", live()));
    expect(asked(ws, before)).toEqual([ARENA_MAP_FEED]);
  });

  it("its table is `u` and `p` alone: `x`, `,` and `.` do nothing; `p` asks the picture", () => {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, OBSERVER_FEED, "keyframe", live()));
    expect(app.screens.activeKeys(app.gate).map((b) => b.key)).toEqual(["u", "p"]);
    for (const key of ["x", ",", "."]) {
      expect(app.runBinding(key)).toBe(false);
    }
    expect(app.replay.open).toBe(true);
    expect(app.runBinding("u")).toBe(true);
    expect(app.replay.focus).toBe(1);
    const before = ws.reqs().length;
    expect(app.runBinding("p")).toBe(true);
    expect(asked(ws, before)).toEqual([OBSERVER_FEED]);
  });

  it("`watching: false` closes it and dismisses the match; the scene is in front", () => {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, OBSERVER_FEED, "keyframe", live()));
    ws.deliver(panel(6, OBSERVER_FEED, "keyframe", cleared()));
    expect(app.replay.open).toBe(false);
    expect(app.replay.dismissed.has(7)).toBe(true);
    expect(hasTab(app)).toBe(false);
    expect(app.screens.activeDef(app.gate).id).toBe("scene");
  });

  it("a live picture closes an open replay", () => {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, REPLAY_FEED, "keyframe", fixture("replay", "replay_window.json")));
    expect(app.replay.live).toBe(false);
    ws.deliver(panel(6, OBSERVER_FEED, "keyframe", live(12)));
    expect(app.replay).toMatchObject({ open: true, live: true, match: 12 });
    expect(app.replay.dismissed.has(7)).toBe(true);
  });

  it("a picture gone quiet is re-asked on the heal's tick, with no hello; its `watching: false` stops the tick", () => {
    // A web socket rides a server-half restart with NO hello, so nothing but
    // the timer notices the pictures stopped (I4-R1). The heal runs while the
    // picture is open, ground held or not, and asks through the store's latch.
    const at = (ms: number) => vi.setSystemTime(new Date(ms));
    at(100_000);
    const { app, ws } = boot(PANELS);
    const idle = vi.getTimerCount();
    ws.deliver(panel(5, OBSERVER_FEED, "keyframe", live()));
    ws.deliver(panel(6, ARENA_MAP_FEED, "keyframe", fixture("cockpit", "arena_map.json")));
    expect(app.replay.holdsTheGround).toBe(true);
    expect(vi.getTimerCount()).toBe(idle + 1);
    const observerAsks = (): number => ws.reqs().filter((r) => feedOf(r) === OBSERVER_FEED).length;
    // A healthy `Quiet` watch: a picture every five seconds is never asked for.
    for (let n = 0; n < 4; n += 1) {
      vi.advanceTimersByTime(5_000);
      ws.deliver(panel(7 + n, OBSERVER_FEED, "keyframe", live()));
    }
    expect(observerAsks()).toBe(0);
    // Then nothing: the first tick past `LIVE_STALE_MS` asks, once.
    vi.advanceTimersByTime(LIVE_STALE_MS - 1);
    expect(observerAsks()).toBe(0);
    vi.advanceTimersByTime(MAP_ASK_MS);
    expect(observerAsks()).toBe(1);
    vi.advanceTimersByTime(LIVE_STALE_MS - MAP_ASK_MS);
    expect(observerAsks()).toBe(1);
    const ask = ws.reqs().filter((r) => feedOf(r) === OBSERVER_FEED).at(-1)!;
    ws.deliver(reply(12, ask.id, panelEvent(OBSERVER_FEED, "keyframe", cleared())));
    expect(app.replay.open).toBe(false);
    expect(vi.getTimerCount()).toBe(idle);
  });

  it("the hello pass re-asks an open live picture; its `watching: false` closes it", () => {
    const { app, ws } = boot(PANELS);
    ws.deliver(panel(5, OBSERVER_FEED, "keyframe", live()));
    const before = ws.reqs().length;
    ws.deliver(hello(PANELS));
    const feeds = asked(ws, before);
    expect(feeds.filter((f) => f === OBSERVER_FEED)).toHaveLength(1);
    expect(feeds).not.toContain(REPLAY_FEED);
    expect(app.replay.open).toBe(true);
    const ask = ws.reqs().filter((r) => feedOf(r) === OBSERVER_FEED).at(-1)!;
    ws.deliver(reply(9, ask.id, panelEvent(OBSERVER_FEED, "keyframe", cleared())));
    expect(app.replay.open).toBe(false);
  });
});

// Doc 20 cut SM.9: the quarry screen — its asks, its keys and its cue.
describe("the quarry (doc 20 cut SM.9)", () => {
  const QUARRY_PANELS = ["site.map", "body.ground", "tv.status"];
  const siteFixture = (): unknown =>
    JSON.parse(readFileSync(resolve(__dirname, "fixtures/site/site_map.json"), "utf8")) as unknown;
  const paramsOf = (r: ReqFrame): unknown =>
    r.q.t === "panel_subscribe" ? (r.q.d as { params?: unknown }).params : undefined;

  it("registers on site.map and asks for both frames with their params on entry", () => {
    const { app, ws } = boot(QUARRY_PANELS);
    expect(app.screens.feedScreens.some((f) => f.def.id === "quarry")).toBe(true);
    const before = ws.reqs().length;
    expect(app.act({ kind: "screenKey", key: "q" })).toBe(true);
    expect(app.screens.active).toBe("quarry");
    const asked = ws.reqs().slice(before);
    expect(asked.map(feedOf)).toEqual(["site.map", "body.ground"]);
    expect(paramsOf(asked[0]!)).toEqual({ view: "drive" });
    expect(paramsOf(asked[1]!)).toEqual({});
  });

  it("no key on the quarry sends a line; the view keys re-ask with their params", () => {
    const { app, ws } = boot(QUARRY_PANELS);
    app.act({ kind: "screenKey", key: "q" });
    const site = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    ws.deliver(reply(5, site.id, panelEvent("site.map", "keyframe", siteFixture())));
    expect(app.quarry.site?.site?.addressed).toBe(1);
    for (const key of ["s", "d", "x", "m", "t", "z", "l", "r"]) {
      expect(app.runBinding(key)).toBe(true);
    }
    expect(ws.sentJson().filter((f) => f.t === "line")).toEqual([]);
    const last = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    expect(paramsOf(last)).toEqual({ view: "site", rig: 1, family: "strategic" });
  });

  it("a tv.status keyframe re-pulls the site map on the quarry, and nowhere else", () => {
    const { app, ws } = boot(QUARRY_PANELS);
    const status = { schema: { id: "tv.status", shape: "status", version: 1 }, rig: {} };
    let before = ws.reqs().length;
    ws.deliver(panel(6, "tv.status", "keyframe", status));
    expect(ws.reqs().length).toBe(before);
    app.act({ kind: "screenKey", key: "q" });
    before = ws.reqs().length;
    ws.deliver(panel(7, "tv.status", "keyframe", status));
    expect(ws.reqs().slice(before).map(feedOf)).toEqual(["site.map"]);
  });

  it("never re-pulls on a timer, even with a leg in flight", () => {
    const { app, ws } = boot(QUARRY_PANELS);
    app.act({ kind: "screenKey", key: "q" });
    const moving = siteFixture() as { site: { rigs: Record<string, unknown>[]; clock: unknown } };
    moving.site.rigs[0]!.leg = {
      started_ic: 1_000,
      waypoints: [
        { x_cm: 0, y_cm: 0, at_s: 0 },
        { x_cm: 9_000, y_cm: 0, at_s: 90 },
      ],
    };
    moving.site.clock = { now_ic: 1_010, rate: 1 };
    ws.deliver(panel(8, "site.map", "keyframe", moving));
    expect(app.quarry.site?.site?.rigs[0]?.leg).not.toBeNull();
    const before = ws.reqs().length;
    vi.advanceTimersByTime(30_000);
    expect(ws.reqs().slice(before).filter((r) => feedOf(r) === "site.map")).toEqual([]);
  });

  it("asks no body.ground of a server without it", () => {
    const { app, ws } = boot(["site.map"]);
    app.act({ kind: "screenKey", key: "q" });
    expect(ws.reqs().map(feedOf)).not.toContain("body.ground");
    expect(ws.reqs().map(feedOf)).toContain("site.map");
  });

  // Lead ruling 6: s/d/x/m put the act's line for the quarry's rig in the box — the frame's
  // addressed rig, or the one `t` picked — and nothing more; with no frame, no rig is named.
  it("the prefill keys open the box with the rig's act line and send nothing", async () => {
    const { app, ws } = boot(QUARRY_PANELS);
    const focuses: (string | undefined)[] = [];
    app.composer = {
      focus: (prefill?: string) => {
        focuses.push(prefill);
      },
      blur: () => {},
      cycleMode: () => {},
      text: () => "",
      setText: () => {},
    };
    app.act({ kind: "screenKey", key: "q" });
    expect(app.runBinding("s")).toBe(true);
    await Promise.resolve();
    expect(focuses).toEqual(["+tv scan"]);
    const site = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    ws.deliver(reply(5, site.id, panelEvent("site.map", "keyframe", siteFixture())));
    focuses.length = 0;
    for (const key of ["s", "d", "x", "m"]) {
      expect(app.runBinding(key)).toBe(true);
    }
    await Promise.resolve();
    expect(focuses).toEqual(["+tv/1 scan", "+tv/1 drive ", "+tv/1 sound", "+tv/1 mine"]);
    // Two rigs listed: `t` picks rig 2 and the prefill follows it.
    const two = siteFixture() as { site: { rigs: Record<string, unknown>[] } };
    two.site.rigs.push({ ...two.site.rigs[0]!, slot: 2, line: "Rig 2: in the bay." });
    ws.deliver(panel(6, "site.map", "keyframe", two));
    expect(app.runBinding("t")).toBe(true);
    focuses.length = 0;
    expect(app.runBinding("d")).toBe(true);
    await Promise.resolve();
    expect(focuses).toEqual(["+tv/2 drive "]);
    expect(ws.sentJson().filter((f) => f.t === "line")).toEqual([]);
  });

  // The pre-commit round (the judge's correctness/F7): a frame that no longer lists the rig `t`
  // picked — returned, or the hull moved — clears the pick, so the next drive-view ask carries
  // no rig (the server addresses its own default); a reason frame lists none and clears it too.
  it("a frame that drops the picked rig clears the pick, and the next ask names no rig", () => {
    const { app, ws } = boot(QUARRY_PANELS);
    app.act({ kind: "screenKey", key: "q" });
    const two = siteFixture() as { site: { rigs: Record<string, unknown>[] } };
    two.site.rigs.push({ ...two.site.rigs[0]!, slot: 2, line: "Rig 2: in the bay." });
    ws.deliver(panel(6, "site.map", "keyframe", two));
    expect(app.runBinding("t")).toBe(true);
    expect(app.quarry.rig).toBe(2);
    ws.deliver(panel(7, "site.map", "keyframe", two));
    expect(app.quarry.rig).toBe(2);
    // Rig 2 returned: the frame lists rig 1 alone.
    ws.deliver(panel(8, "site.map", "keyframe", siteFixture()));
    expect(app.quarry.rig).toBeNull();
    expect(app.runBinding("r")).toBe(true);
    let last = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    expect(paramsOf(last)).toEqual({ view: "drive" });
    // A reason frame (the server's MAP_NO_RIG) lists no rigs: a pick goes with it.
    ws.deliver(panel(9, "site.map", "keyframe", two));
    expect(app.runBinding("t")).toBe(true);
    expect(app.quarry.rig).toBe(2);
    const reason = {
      schema: { id: "site.map", shape: "grid", version: 1 },
      window: 0,
      title: "Rig 2 is not on Ahuna Mons's ground or in the bay.",
      width: 0,
      height: 0,
      rows: [],
      legend: [],
      units: [],
      you: null,
    };
    ws.deliver(panel(10, "site.map", "keyframe", reason));
    expect(app.quarry.rig).toBeNull();
    expect(app.runBinding("r")).toBe(true);
    last = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    expect(paramsOf(last)).toEqual({ view: "drive" });
  });

  // §5, D16: the quarry's frames are the viewer's own knowledge; a character switch in the
  // same tab (or a reconnect) starts it empty, and the next ask carries none of its view.
  it("a character switch clears the quarry", () => {
    vi.stubGlobal(
      "localStorage",
      memoryStorage({
        "rustmush.token": "tok",
        "rustmush.chars": JSON.stringify([
          { key: 7, name: "Atlas" },
          { key: 8, name: "Bryn" },
        ]),
      }),
    );
    const app = makeApp();
    app.client.connect("tok", "7");
    const ws = openHello(QUARRY_PANELS);
    app.act({ kind: "screenKey", key: "q" });
    const site = ws.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    ws.deliver(reply(5, site.id, panelEvent("site.map", "keyframe", siteFixture())));
    app.runBinding("t");
    app.runBinding("l");
    app.runBinding("z");
    expect(app.quarry.site).not.toBeNull();
    expect(app.quarry.rig).toBe(1);

    app.enterGame(8);

    expect(app.quarry.site).toBeNull();
    expect(app.quarry.ground).toBeNull();
    expect(app.quarry.rig).toBeNull();
    expect(app.quarry.family).toBeNull();
    expect(app.quarry.mode).toBe("drive");
    const next = openHello(QUARRY_PANELS);
    app.act({ kind: "screenKey", key: "q" });
    const asked = next.reqs().filter((r) => feedOf(r) === "site.map").at(-1)!;
    expect(paramsOf(asked)).toEqual({ view: "drive" });
  });
});
