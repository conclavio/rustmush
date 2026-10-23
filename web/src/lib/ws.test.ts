// WsClient framing tests: envelope parse + typed dispatch, seq tracking and
// the URL-based resume flow, ping→pong, backoff schedule, session-expiry
// close, clean close. Driven through the injectable WebSocket factory — no
// browser, no server.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ClientCaps, Envelope, OutputEvent } from "./proto";
import {
  backoffDelay,
  CLOSE_BAD_SESSION,
  CLOSE_TOKEN_COOLOFF,
  DEFAULT_BACKOFF,
  WsClient,
  type WebSocketLike,
  type WsClientOptions,
} from "./ws";

/** Scriptable stand-in for the DOM WebSocket. */
class MockWebSocket implements WebSocketLike {
  static instances: MockWebSocket[] = [];

  readyState = 0; // CONNECTING
  binaryType?: string;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;

  constructor(
    public url: string,
    public protocols?: string[],
  ) {
    MockWebSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(code = 1000): void {
    this.readyState = 3; // CLOSED
    this.onclose?.({ code, wasClean: true });
  }

  // -- test helpers --

  /** Simulates the server accepting the connection. */
  open(): void {
    this.readyState = 1; // OPEN
    this.onopen?.();
  }

  /** Delivers one server frame (an envelope, serialized). */
  deliver(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }

  /** Delivers a raw string frame (for malformed-input tests). */
  deliverRaw(data: unknown): void {
    this.onmessage?.({ data });
  }

  /** Simulates an unclean network drop. */
  drop(): void {
    this.readyState = 3;
    this.onclose?.({ code: 1006, wasClean: false });
  }

  /** Simulates a server-initiated close with an application code. */
  serverClose(code: number): void {
    this.readyState = 3;
    this.onclose?.({ code, wasClean: true });
  }

  /** Commands the client sent, parsed. */
  sentJson(): unknown[] {
    return this.sent.map((s) => JSON.parse(s));
  }
}

function lastSocket(): MockWebSocket {
  const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
  if (!ws) {
    throw new Error("no socket created");
  }
  return ws;
}

/** Builds a wire envelope: seq/ts with the event's t/d flattened in. */
function env(seq: number, event: OutputEvent): Envelope {
  return { seq, ts: 1756300000000 + seq, ...event } as Envelope;
}

const HELLO: OutputEvent = {
  t: "hello",
  d: { proto: 1, server: "rustmush test", features: [], panels: [] },
};

function makeClient(opts: WsClientOptions = {}): WsClient {
  return new WsClient({
    url: "ws://test/ws",
    webSocketFactory: (url, protocols) => new MockWebSocket(url, protocols),
    random: () => 1, // deterministic jitter: delay == full window
    ...opts,
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  MockWebSocket.instances = [];
});

afterEach(() => {
  vi.useRealTimers();
});

describe("backoffDelay", () => {
  it("doubles per attempt and caps at maxMs", () => {
    const opts = { baseMs: 500, maxMs: 30_000 };
    // random() = 1 → the full window.
    expect(backoffDelay(0, opts, () => 1)).toBe(500);
    expect(backoffDelay(1, opts, () => 1)).toBe(1000);
    expect(backoffDelay(2, opts, () => 1)).toBe(2000);
    expect(backoffDelay(6, opts, () => 1)).toBe(30_000);
    expect(backoffDelay(50, opts, () => 1)).toBe(30_000);
  });

  it("jitters within [window/2, window]", () => {
    const opts = { baseMs: 1000, maxMs: 30_000 };
    expect(backoffDelay(0, opts, () => 0)).toBe(500);
    expect(backoffDelay(0, opts, () => 0.5)).toBe(750);
    expect(backoffDelay(0, opts, () => 1)).toBe(1000);
  });

  it("has sane defaults", () => {
    expect(DEFAULT_BACKOFF.baseMs).toBeGreaterThan(0);
    expect(DEFAULT_BACKOFF.maxMs).toBeGreaterThan(DEFAULT_BACKOFF.baseMs);
  });
});

describe("WsClient", () => {
  it("connects with char in the URL and the token as a subprotocol", () => {
    const client = makeClient();
    client.connect("tok123", "12");
    const ws = lastSocket();
    // Token NEVER in the URL — it rides Sec-WebSocket-Protocol.
    expect(ws.url).toBe("ws://test/ws?char=12");
    expect(ws.protocols).toEqual(["rustmush.v1.tok123"]);
    expect(client.status).toBe("connecting");
    ws.open();
    expect(client.status).toBe("connected");
  });

  it("parses envelopes and dispatches by event type", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    const sys: unknown[] = [];
    const all: Envelope[] = [];
    client.on("sys", (e) => sys.push(e.d));
    client.onAny((e) => all.push(e));

    ws.deliver(env(1, { t: "sys", d: [{ text: "hello" }] }));
    ws.deliver(env(2, { t: "ping", d: { nonce: 7 } }));

    expect(sys).toEqual([[{ text: "hello" }]]);
    expect(all).toHaveLength(2);
    expect(client.lastSeq).toBe(2);
  });

  it("ignores seq 0 (out-of-band frames) for resume tracking", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    ws.deliver(env(0, HELLO));
    expect(client.lastSeq).toBeNull();
    ws.deliver(env(5, { t: "sys", d: [] }));
    expect(client.lastSeq).toBe(5);
    ws.deliver(env(0, { t: "ping", d: { nonce: 1 } }));
    expect(client.lastSeq).toBe(5);
  });

  it("unsubscribes handlers", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    let calls = 0;
    const off = client.on("sys", () => calls++);
    ws.deliver(env(1, { t: "sys", d: [] }));
    off();
    ws.deliver(env(2, { t: "sys", d: [] }));
    expect(calls).toBe(1);
  });

  it("tolerates an event variant it has no handler for (I1-C42)", () => {
    // Variant tolerance is a CLIENT property, not a wire one: the server's
    // round-trip tests cover unknown FIELDS, and nothing pinned that a whole
    // event type this client predates is survivable. A server adding one
    // (`Composing` was exactly that) must not stall the stream — the seq has
    // to advance, or the resume point freezes at the last known variant and
    // every reconnect replays from there.
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    const sys: unknown[] = [];
    const all: Envelope[] = [];
    client.on("sys", (e) => sys.push(e.d));
    client.onAny((e) => all.push(e));

    expect(() =>
      ws.deliver(env(3, { t: "composing", d: { actor: { id: "#7", name: "Atlas" }, active: true } })),
    ).not.toThrow();

    expect(sys).toEqual([]); // no handler for it
    expect(all).toHaveLength(1); // …but it IS an envelope
    expect(client.lastSeq).toBe(3); // the stream advanced past it
    ws.deliver(env(4, { t: "sys", d: [{ text: "still here" }] }));
    expect(sys).toEqual([[{ text: "still here" }]]);
    expect(client.lastSeq).toBe(4);

    // A variant this client does not know AT ALL is the same case: `t` is a
    // string, so it is an envelope, and it dispatches to nobody.
    ws.deliverRaw(JSON.stringify({ seq: 5, ts: 0, t: "from_the_future", d: { x: 1 } }));
    expect(all).toHaveLength(3);
    expect(client.lastSeq).toBe(5);
  });

  it("ignores malformed and non-envelope frames", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    let calls = 0;
    client.onAny(() => calls++);
    ws.deliverRaw("{not json");
    ws.deliverRaw(JSON.stringify({ seq: 1 })); // no "t"
    ws.deliverRaw(12345); // binary-ish
    expect(calls).toBe(0);
    expect(client.lastSeq).toBeNull();
  });

  it("answers ping with a pong echoing the nonce", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    ws.deliver(env(0, { t: "ping", d: { nonce: 42 } }));
    expect(ws.sentJson()).toContainEqual({ t: "pong", d: { nonce: 42 } });
  });

  it("acknowledges an applied authoritative commit token", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    const commit = "0123456789abcdef0123456789abcdef";
    expect(client.ackCommit(commit)).toBe(true);
    expect(ws.sentJson()).toContainEqual({ t: "ack", d: { commit } });
  });

  it("declares caps after the server hello, and sends nothing else", () => {
    const caps: ClientCaps = {
      kind: "web",
      color: "true_color",
      width: 120,
      height: 40,
      utf8: true,
      structured: true,
    };
    const client = makeClient({ caps: () => caps });
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    ws.deliver(env(0, HELLO));
    // No in-band resume — resume is connection-setup only (?resume=).
    expect(ws.sentJson()).toEqual([{ t: "caps", d: caps }]);
  });

  it("reconnects with backoff and puts the resume point in the URL", () => {
    const client = makeClient();
    client.connect("tok", "7");
    const ws1 = lastSocket();
    ws1.open();
    ws1.deliver(env(0, HELLO));
    ws1.deliver(env(1, { t: "sys", d: [] }));
    ws1.deliver(env(2, { t: "sys", d: [] }));
    ws1.deliver(env(3, { t: "sys", d: [] }));
    expect(client.lastSeq).toBe(3);

    ws1.drop();
    expect(client.status).toBe("reconnecting");
    expect(MockWebSocket.instances).toHaveLength(1);

    // random()=1 → first retry after exactly baseMs (500ms default).
    vi.advanceTimersByTime(499);
    expect(MockWebSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(MockWebSocket.instances).toHaveLength(2);

    const ws2 = lastSocket();
    expect(ws2.url).toBe("ws://test/ws?char=7&resume=3");
    expect(ws2.protocols).toEqual(["rustmush.v1.tok"]);
    ws2.open();
    expect(client.status).toBe("connected");
    // No in-band resume command on the new socket either.
    ws2.deliver(env(0, HELLO));
    expect(ws2.sentJson()).toEqual([]);
  });

  it("does not put resume in the URL on a first connection", () => {
    const client = makeClient();
    client.connect("t", "1");
    expect(lastSocket().url).toBe("ws://test/ws?char=1");
  });

  it("a fresh connect() resets the resume point", () => {
    const client = makeClient();
    client.connect("t", "1");
    const ws1 = lastSocket();
    ws1.open();
    ws1.deliver(env(9, { t: "sys", d: [] }));
    expect(client.lastSeq).toBe(9);

    client.connect("t", "2"); // switching character: new session stream
    expect(client.lastSeq).toBeNull();
    expect(lastSocket().url).toBe("ws://test/ws?char=2");
  });

  it("follows the exponential backoff schedule across repeated failures", () => {
    const client = makeClient({ backoff: { baseMs: 100, maxMs: 800 } });
    client.connect("t", "1");
    lastSocket().open();

    // Expected full-window delays (random()=1): 100, 200, 400, 800, 800.
    for (const delay of [100, 200, 400, 800, 800]) {
      const before = MockWebSocket.instances.length;
      lastSocket().drop();
      vi.advanceTimersByTime(delay - 1);
      expect(MockWebSocket.instances).toHaveLength(before);
      vi.advanceTimersByTime(1);
      expect(MockWebSocket.instances).toHaveLength(before + 1);
    }

    // A successful open resets the attempt counter.
    lastSocket().open();
    lastSocket().drop();
    vi.advanceTimersByTime(100);
    expect(MockWebSocket.instances).toHaveLength(7);
  });

  it("invokes the full-rehydrate callback on resumed{replayed:false}", () => {
    const rehydrate = vi.fn();
    const client = makeClient({ onFullRehydrate: rehydrate });
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    ws.deliver(env(0, { t: "resumed", d: { replayed: true } }));
    expect(rehydrate).not.toHaveBeenCalled();

    ws.deliver(env(0, { t: "resumed", d: { replayed: false } }));
    expect(rehydrate).toHaveBeenCalledTimes(1);
  });

  it("close 4401 stops reconnecting and fires onSessionExpired", () => {
    const expired = vi.fn();
    const client = makeClient({ onSessionExpired: expired });
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    ws.serverClose(CLOSE_BAD_SESSION);
    expect(expired).toHaveBeenCalledTimes(1);
    expect(client.status).toBe("closed");
    vi.advanceTimersByTime(120_000);
    expect(MockWebSocket.instances).toHaveLength(1);
  });

  it("close 4429 explains cool-off, keeps its token and waits a minute", () => {
    const expired = vi.fn();
    const cooloff = vi.fn();
    const client = makeClient({ onSessionExpired: expired, onTokenCooloff: cooloff });
    client.connect("still-valid", "1");
    lastSocket().open();
    lastSocket().serverClose(CLOSE_TOKEN_COOLOFF);
    expect(cooloff).toHaveBeenCalledTimes(1);
    expect(expired).not.toHaveBeenCalled();
    expect(client.status).toBe("reconnecting");
    vi.advanceTimersByTime(59_999);
    expect(MockWebSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(MockWebSocket.instances).toHaveLength(2);
    expect(lastSocket().protocols).toEqual(["rustmush.v1.still-valid"]);
  });

  it("req() assigns increasing correlation ids and frames the query", () => {
    const client = makeClient();
    expect(client.req({ t: "who" })).toBeNull(); // not connected: dropped
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    const a = client.req({ t: "who" });
    const b = client.req({ t: "mail_list", d: { folder: null } });
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(b).toBeGreaterThan(a!);
    expect(ws.sentJson()).toEqual([
      { t: "req", d: { id: a, q: { t: "who" } } },
      { t: "req", d: { id: b, q: { t: "mail_list", d: { folder: null } } } },
    ]);
  });

  it("close() is clean: no reconnect, drops are ignored afterwards", () => {
    const statuses: string[] = [];
    const client = makeClient({ onStatus: (s) => statuses.push(s) });
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();

    client.close();
    expect(client.status).toBe("closed");
    vi.advanceTimersByTime(120_000);
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(statuses).toEqual(["connecting", "connected", "closed"]);
  });

  it("close() during a pending reconnect cancels it", () => {
    const client = makeClient();
    client.connect("t", "1");
    lastSocket().open();
    lastSocket().drop();
    client.close();
    vi.advanceTimersByTime(120_000);
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(client.status).toBe("closed");
  });

  it("send() reports failure while disconnected", () => {
    const client = makeClient();
    expect(client.sendLine("look")).toBe(false);
    client.connect("t", "1");
    expect(client.sendLine("look")).toBe(false); // still CONNECTING
    lastSocket().open();
    expect(client.sendLine("look")).toBe(true);
    expect(lastSocket().sentJson()).toEqual([{ t: "line", d: "look" }]);
  });
});

describe("binary frames (doc 21 cut G.3c)", () => {
  it("hands the picture's binary messages to onBinary and never dispatches them", () => {
    const got: unknown[] = [];
    const client = makeClient({ onBinary: (data) => got.push(data) });
    const seen: unknown[] = [];
    client.on("hello", (e) => seen.push(e));
    client.connect("t", "1");
    const ws = lastSocket();
    ws.open();
    expect(ws.binaryType).toBe("blob");
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer;
    ws.deliverRaw(png);
    ws.deliverRaw(new Blob([png], { type: "image/png" }));
    ws.deliverRaw(42); // neither text nor binary: ignored
    expect(got).toHaveLength(2);
    expect(got[0]).toBe(png);
    expect(got[1]).toBeInstanceOf(Blob);
    expect(seen).toHaveLength(0);
  });
});
