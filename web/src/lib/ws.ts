// WsClient — the one WebSocket transport for the rustmush web client.
//
// Responsibilities (design 02-connectivity §6.3–§6.4, mush-net-web/src/ws.rs):
//   - connect with token + character key. The token travels in the
//     `Sec-WebSocket-Protocol` subprotocol `rustmush.v1.<token>` (the one
//     header browsers CAN set on a WebSocket; the server echoes it back and
//     never logs it) — never in the URL. `?char=<key>` selects the character.
//   - send `ClientCommand`s, receive `Envelope`s (`{seq, ts, t, d}` — the
//     mush-proto enum IS the wire format; types in ./proto are generated)
//   - typed event dispatch registry keyed on the envelope's `t`
//   - resume: track the last session-stamped `seq` (out-of-band frames —
//     hello/resumed/ping — carry seq 0 and are never resume points); on an
//     unclean close, reconnect with exponential backoff + jitter and put the
//     resume point in the URL (`?resume=<last_seq>`). Resume is
//     connection-setup only — a mid-connection `Resume` command is a server
//     protocol error (close 1008), so this client never sends one.
//   - the server answers a resume attempt with `resumed { replayed }`:
//     `true` → the gap replays in order (silent); `false` → the gap outgrew
//     the ring and the server fell back to a fresh attach — the
//     full-rehydrate callback fires so the app re-requests panel state
//   - keepalive: answer every `ping` with a `pong` echoing the nonce
//   - close 4401 (bad/expired token or session) is fatal: no reconnect,
//     `onSessionExpired` fires so the app returns to login — which drops the
//     stored token. This is the ONLY signal that distinguishes "your token is
//     dead" from "the network blipped": a browser cannot see the HTTP status
//     of a failed upgrade and reports every pre-upgrade refusal as 1006, so
//     the server upgrades and then closes 4401 rather than answering 401
//     (mush-net-web/src/ws.rs). Anything else — 1006 included — keeps the
//     token and the backoff.
//     Because 4401 destroys a credential the player cannot get back, the
//     server spends it ONLY on a refusal it has actually checked. Everything
//     transient gets a shape this loop retries: 503 on the upgrade (the token
//     store could not answer), 1012 (the game half is restarting), 1013
//     (admission full). A store outage or an ordinary server-half reload must
//     never log a web player out, so do not widen the fatal branch below.
//   - clean close: `close()` never triggers a reconnect
//
// The class takes injectable seams (WebSocket factory, RNG) so the vitest
// suite can drive it without a browser or a server.

import type { ClientCaps, ClientCommand, Envelope, OutputEvent, Query, InvokeRequest } from "./proto";

/** Discriminant of an [`OutputEvent`] — `"narrative"`, `"channel"`, … */
export type EventType = OutputEvent["t"];

/**
 * The envelope narrowed to one event type: `seq`/`ts` plus that event's
 * `t`/`d` (the event flattens into the envelope on the wire).
 */
export type EnvelopeOf<T extends EventType> = Envelope & Extract<OutputEvent, { t: T }>;

/** Handler for one event type. */
export type EventHandler<T extends EventType> = (env: EnvelopeOf<T>) => void;

/** Connection lifecycle, surfaced via `WsClientOptions.onStatus`. */
export type ConnStatus =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "closed";

/** The server's application close code for auth/session failures. */
export const CLOSE_BAD_SESSION = 4401;

/** Temporary network token cool-off. Keep credentials and retry after a minute. */
export const CLOSE_TOKEN_COOLOFF = 4429;

/** Subprotocol prefix carrying the auth token (mush-net-web/src/ws.rs). */
export const SUBPROTO_PREFIX = "rustmush.v1.";

/**
 * The subset of the DOM WebSocket surface the client uses — structural, so
 * tests inject a mock and non-browser environments can adapt.
 */
export interface WebSocketLike {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: (() => void) | null;
  onclose: ((ev: { code: number; wasClean: boolean }) => void) | null;
  onerror: (() => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
}

/** WebSocket OPEN readyState (mirrored to avoid the DOM global in tests). */
const WS_OPEN = 1;

/** Reconnect backoff shape. */
export interface BackoffOptions {
  /** Delay bound for the first retry, milliseconds. */
  baseMs: number;
  /** Upper bound for the exponential growth, milliseconds. */
  maxMs: number;
}

export const DEFAULT_BACKOFF: BackoffOptions = { baseMs: 500, maxMs: 30_000 };

/**
 * Reconnect delay for the given attempt (0-based): exponential growth capped
 * at `maxMs`, with equal jitter — half the window fixed, half randomized —
 * so delays grow predictably but reconnect stampedes desynchronize.
 *
 * attempt 0 → 250..500ms, 1 → 500..1000ms, … capped at maxMs/2..maxMs.
 */
export function backoffDelay(
  attempt: number,
  opts: BackoffOptions = DEFAULT_BACKOFF,
  random: () => number = Math.random,
): number {
  const window = Math.min(opts.maxMs, opts.baseMs * 2 ** attempt);
  return window / 2 + random() * (window / 2);
}

export interface WsClientOptions {
  /**
   * WS endpoint. Defaults to `ws(s)://<location.host>/ws` (same-origin; the
   * vite dev server proxies `/ws` to the Rust backend).
   */
  url?: string;
  /**
   * Called when a resume attempt came back `resumed { replayed: false }`:
   * the gap exceeded the resume buffer and the server fell back to a fresh
   * attach (fresh `Room` output follows from the game). The app should
   * re-request panel state and mark the gap in scrollback.
   */
  onFullRehydrate?: () => void;
  /**
   * Called when the server closes with 4401: the token expired or the
   * session is gone for good. The client stops reconnecting; the app
   * returns to login.
   */
  onSessionExpired?: () => void;
  /** A network token cool-off; the client retains its token and waits a minute. */
  onTokenCooloff?: () => void;
  /** Connection lifecycle notifications (drive a status indicator). */
  onStatus?: (status: ConnStatus) => void;
  /**
   * Capability provider; declared to the server right after its `hello`.
   * Return `null` to skip declaring caps.
   */
  caps?: () => ClientCaps | null;
  /** Backoff tuning; see [`DEFAULT_BACKOFF`]. */
  backoff?: Partial<BackoffOptions>;
  /**
   * Test seam: WebSocket constructor. Defaults to the DOM `WebSocket`.
   * `protocols` carries the auth subprotocol (`rustmush.v1.<token>`).
   */
  webSocketFactory?: (url: string, protocols?: string[]) => WebSocketLike;
  /** Test seam: RNG for backoff jitter. Defaults to `Math.random`. */
  random?: () => number;
}

/** Internal: an untyped handler as stored in the registry. */
type AnyHandler = (env: Envelope) => void;

export class WsClient {
  #opts: WsClientOptions;
  #backoff: BackoffOptions;
  #random: () => number;
  #makeSocket: (url: string, protocols?: string[]) => WebSocketLike;

  #ws: WebSocketLike | null = null;
  #token: string | null = null;
  #char: string | null = null;

  #handlers = new Map<EventType, Set<AnyHandler>>();
  #anyHandlers = new Set<AnyHandler>();

  #lastSeq: number | null = null;
  #attempt = 0;
  #nextReqId = 1;
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #intentionalClose = false;
  #status: ConnStatus = "idle";

  constructor(opts: WsClientOptions = {}) {
    this.#opts = opts;
    this.#backoff = { ...DEFAULT_BACKOFF, ...opts.backoff };
    this.#random = opts.random ?? Math.random;
    this.#makeSocket =
      opts.webSocketFactory ??
      ((url: string, protocols?: string[]) =>
        new WebSocket(url, protocols) as unknown as WebSocketLike);
  }

  /** Current connection status. */
  get status(): ConnStatus {
    return this.#status;
  }

  /** Last session-stamped envelope `seq`, or `null` before any. */
  get lastSeq(): number | null {
    return this.#lastSeq;
  }

  /**
   * Opens the connection as character `char` (the numeric key from the
   * login payload, as a string) using the opaque auth `token` (from
   * `POST /api/auth/login`). Reconnects transparently until [`close`] is
   * called or the server answers 4401.
   */
  connect(token: string, char: string): void {
    this.#token = token;
    this.#char = char;
    this.#intentionalClose = false;
    this.#attempt = 0;
    this.#lastSeq = null; // a fresh connect is a fresh stream — no resume
    this.#clearTimer();
    this.#dropSocket();
    this.#open();
  }

  /**
   * Registers a handler for one event type; returns its unsubscribe
   * function. Multiple handlers per type are supported.
   */
  on<T extends EventType>(t: T, fn: EventHandler<T>): () => void {
    let set = this.#handlers.get(t);
    if (!set) {
      set = new Set();
      this.#handlers.set(t, set);
    }
    const untyped = fn as AnyHandler;
    set.add(untyped);
    return () => set.delete(untyped);
  }

  /** Registers a handler for every envelope; returns its unsubscribe fn. */
  onAny(fn: AnyHandler): () => void {
    this.#anyHandlers.add(fn);
    return () => this.#anyHandlers.delete(fn);
  }

  /**
   * Sends one command. Returns `false` (dropping the command) when the
   * socket is not open — callers that care queue on top of this.
   */
  send(cmd: ClientCommand): boolean {
    if (!this.#ws || this.#ws.readyState !== WS_OPEN) {
      return false;
    }
    this.#ws.send(JSON.stringify(cmd));
    return true;
  }

  /** Convenience: submit a command line, exactly as if typed. */
  sendLine(line: string): boolean {
    return this.send({ t: "line", d: line });
  }

  /** Acknowledge one validated authoritative result after applying it. */
  ackCommit(commit: string): boolean {
    return this.send({ t: "ack", d: { commit } });
  }

  /**
   * Sends a panel-hydration request with an auto-assigned correlation id.
   * Returns the id (match it against `reply` envelopes), or `null` when the
   * socket is not open.
   */
  req(q: Query): number | null {
    const id = this.#nextReqId++;
    return this.send({ t: "req", d: { id, q } }) ? id : null;
  }

  /** Connection-local mutation service; shares the query correlation namespace. */
  invoke(request: InvokeRequest): number | null {
    const encoded = JSON.stringify({ t: "invoke", d: { id: this.#nextReqId, request } });
    if (new TextEncoder().encode(encoded).length > 272 * 1024) return null;
    const id = this.#nextReqId++;
    return this.send({ t: "invoke", d: { id, request } }) ? id : null;
  }

  /**
   * Closes cleanly: cancels any pending reconnect and never schedules a new
   * one. The client can be reused via [`connect`].
   */
  close(): void {
    this.#intentionalClose = true;
    this.#clearTimer();
    const ws = this.#ws;
    this.#ws = null;
    if (ws) {
      ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
      ws.close(1000, "client close");
    }
    this.#setStatus("closed");
  }

  #dropSocket(): void {
    const ws = this.#ws;
    this.#ws = null;
    if (ws) {
      ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
      ws.close(1000, "superseded");
    }
  }

  #open(): void {
    const base =
      this.#opts.url ??
      `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;
    const sep = base.includes("?") ? "&" : "?";
    let url = `${base}${sep}char=${encodeURIComponent(this.#char ?? "")}`;
    // Resume is connection-setup only (server closes 1008 on an in-band
    // Resume command): the resume point rides the URL.
    if (this.#lastSeq !== null && this.#lastSeq > 0) {
      url += `&resume=${this.#lastSeq}`;
    }
    const protocols = this.#token ? [`${SUBPROTO_PREFIX}${this.#token}`] : undefined;

    this.#setStatus(this.#attempt > 0 ? "reconnecting" : "connecting");
    const ws = this.#makeSocket(url, protocols);
    this.#ws = ws;
    ws.onopen = () => {
      this.#attempt = 0;
      this.#setStatus("connected");
    };
    ws.onmessage = (ev) => this.#onMessage(ev.data);
    ws.onerror = () => {
      // A close event always follows an error; reconnect is handled there.
    };
    ws.onclose = (ev) => {
      if (this.#ws !== ws) {
        return; // superseded socket
      }
      this.#ws = null;
      if (this.#intentionalClose) {
        this.#setStatus("closed");
        return;
      }
      if (ev.code === CLOSE_BAD_SESSION) {
        // Token expired / session gone: retrying would loop forever — and a
        // loop against a dead token is what tempbanned an operator's own
        // address on 2026-09-20.
        this.#intentionalClose = true;
        this.#setStatus("closed");
        this.#opts.onSessionExpired?.();
        return;
      }
      if (ev.code === CLOSE_TOKEN_COOLOFF) {
        this.#opts.onTokenCooloff?.();
        this.#scheduleReconnect(60_000);
        return;
      }
      this.#scheduleReconnect();
    };
  }

  #onMessage(data: unknown): void {
    if (typeof data !== "string") {
      return; // binary frames are not part of the protocol
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(data);
    } catch {
      return; // hostile/garbled frame; ignore (fuzz-safety mirror of the server)
    }
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      typeof (parsed as { t?: unknown }).t !== "string"
    ) {
      return;
    }
    const env = parsed as Envelope;
    const event = env as OutputEvent; // t/d are flattened into the envelope

    // Only session-stamped envelopes advance the resume point; frames the
    // transport generates itself (hello, resumed, ping) carry seq 0 and are
    // not part of the resumable stream.
    if (typeof env.seq === "number" && env.seq > 0) {
      this.#lastSeq = env.seq;
    }

    // Protocol-internal behavior, before app handlers see the event.
    switch (event.t) {
      case "ping":
        this.send({ t: "pong", d: { nonce: event.d.nonce } });
        break;
      case "hello": {
        const caps = this.#opts.caps?.();
        if (caps) {
          this.send({ t: "caps", d: caps });
        }
        break;
      }
      case "resumed":
        if (!event.d.replayed) {
          this.#opts.onFullRehydrate?.();
        }
        break;
    }

    for (const fn of this.#anyHandlers) {
      fn(env);
    }
    const set = this.#handlers.get(event.t);
    if (set) {
      for (const fn of set) {
        fn(env);
      }
    }
  }

  #scheduleReconnect(minDelay = 0): void {
    this.#setStatus("reconnecting");
    const delay = Math.max(minDelay, backoffDelay(this.#attempt, this.#backoff, this.#random));
    this.#attempt += 1;
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = null;
      this.#open();
    }, delay);
  }

  #clearTimer(): void {
    if (this.#reconnectTimer !== null) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = null;
    }
  }

  #setStatus(status: ConnStatus): void {
    if (this.#status !== status) {
      this.#status = status;
      this.#opts.onStatus?.(status);
    }
  }
}
