# 29a. sshdoorway — implementation plan

**2026-10-30.** This is the build plan for [doc 29](29-sshdoorway.md) (Draft 0.2, all
questions ruled), covering cuts DW.0–DW.3. It is based on `origin/main` at `ebeee17b`,
with russh pinned at 0.63.3. DW.4 (the conclav.io rollout) is out of scope: it waits for the
owner's address change and go-ahead.

## 1. What the russh source decides

Three facts from russh 0.63.3 shape the splice. Each is a hazard the code has to work around.

**R-1. A request's reply can't be deferred.** russh's server keeps **one** `wants_reply`
flag per channel (`server/session.rs`, `channel_success`), and each incoming request
overwrites it. OpenSSH pipelines its requests: `pty-req` (reply wanted), then `env`
(no reply), then `shell` (reply wanted), all without waiting. So if the doorway answered
`pty-req` later, from another task, the `env` that followed would already have cleared the
flag, the answer would be dropped, and the client would hang. **Every downstream reply is
therefore sent inside its handler callback.** For `pty-req`, `shell`, `exec` and
`subsystem`, the callback waits for the upstream reply; for `env`, `window-change` and
`signal`, it forwards the request and answers locally.

**R-2. A blocked callback blocks the whole connection.** While a handler callback is
waiting, russh processes nothing else on that connection, window adjusts included. Both
session loops also wait on bounded channel queues: data is pushed with `chan.send(..).await`
into a queue of `channel_buffer_size` messages. That allows a cycle:

1. The client stops reading (`^S`), so the doorway's upstream→downstream pump waits for
   downstream window.
2. The pump stops draining the upstream queue, the queue fills, and the upstream client loop
   blocks.
3. A downstream callback that is itself waiting for something from upstream (window space
   for input, or a reply) never gets it.

**Every cross-session wait has a timeout.** Writes in either direction use `WRITE_TIMEOUT`
(30 s, the instance's value). Request replies and channel opens use 10 s. A timeout is a
wedge: the doorway closes both sessions. The instance's own wedge detection does the same
thing to a direct connection, so a wedged player is cut off at about the same time either
way.

**R-3. The buffering bound is a queue of packets, not one window.** The upstream channel's
receive queue holds at most `channel_buffer_size` (100) messages, each at most one packet
(32 KiB). Past that, the client loop stops reading the Unix socket and the instance's writes
stall. That stall is the backpressure doc 29's C-4 asks for. The bound is about 3 MiB per
direction per session, not "one window", and this plan corrects the wording in doc 29 §6.

## 2. Plan-time amendments to doc 29

These are recorded here and folded into doc 29 in the same PR. None of them changes a
decision's intent.

| # | Doc 29 says | The plan does | Why |
|---|---|---|---|
| A-1 | Gateway gets `SupplementaryGroups=sshdoorway`; the gateway chowns the socket | `/run/rustmush-ssh` is **setgid** (tmpfiles `d /run/rustmush-ssh 2750 mush sshdoorway`). The gateway only `chmod 0660`s its socket | Files created in a setgid directory inherit its group, so the gateway needs no gid lookup and no extra group |
| A-2 | Buffer "at most one channel window per direction" | Bounded by R-3, with every wait timed out (R-2) | What russh actually does |
| A-3 | Readiness completes a `Hello` round trip with each service | A `Ping` → `Pong` frame | `Hello` would claim a per-IP slot for the doorway's own address |
| A-4 | Per-IP limits key on the peer address | IPv4: the address. IPv6: the **/64** | One IPv6 host commonly holds a whole /64 and can rotate through it freely |
| A-5 | Every refusal gets a banner | Over the **per-IP concurrency cap**: an SSH banner, then reject (a household behind one NAT should be told). Over the **rate** or the **global** cap: the TCP connection is dropped before any SSH work | A handshake costs CPU; the cheapest answer to a flood is to do no work at all |

## 3. Crates and modules

### 3.1 `mush-sshdoorway-proto` (new, lib)

- **`lib.rs`.** The two message enums. `DoorwayMsg` (doorway → instance): `Ping`,
  `Hello { peer, local, conn }`, `KeyQuery { user, key }`, `Begin { user, attested_key }`.
  `IngressMsg` (instance → doorway): `Pong`, `Open`, `Blocked { notice }`, `Refused`,
  `KeyReply { acceptable }`, `Ready`, `BadCreds`, `Banned`, `VersionMismatch`. Every frame
  is a `Frame { version: u16, msg }`.
- **Framing.** A `u32` big-endian length, then self-describing MessagePack, the same shape
  as `mush-ipc/src/codec.rs`. `MAX_FRAME = 16 KiB`. `read_frame` uses `read_exact` only,
  never a buffered reader, so **not one byte past the frame** is consumed and the SSH
  stream that follows is intact.
- **Limits** shared by both sides: `MAX_KEY_QUERIES = 8`, `MAX_BEGINS = 4`,
  `MAX_USER_LEN = 256`, `MAX_KEY_LEN = 16 KiB`.
- **Dependencies:** serde, rmp-serde, tokio (io-util), thiserror. No russh.

### 3.2 `mush-net-ssh` (changed)

- **`trusted.rs` (new).** `serve_trusted_stream(stream, shared)` runs the instance side of
  the preamble (doc 29 §4.2), then `russh::server::run_stream` on the same stream. It is
  generic over `AsyncRead + AsyncWrite`, so the preamble logic is tested on every platform
  with `tokio::io::duplex`. `SshServer::bind_trusted(TrustedConfig, hooks)` is the
  `cfg(unix)` wrapper. It removes a stale socket **only if the path is a socket**, binds
  it, `chmod`s it 0660, and runs an accept loop.
- **`server.rs`.**
  - Creating a `ConnHandler` moves into `Ingress::handler_for(addr, counted)`, which both
    the TCP and the trusted paths call.
  - `ConnHandler` gains `attested: Option<Attested { user, account, characters }>`.
  - `auth_none` accepts exactly the attested user, setting `Route::Account` and
    `AuthMethod::PublicKey`. Every other path is unchanged.
- **The per-IP slot.** On the trusted path the slot is claimed at `Hello`
  (`connection_opened(peer.ip())`). It is released by the same `Drop`, exactly once, also
  when the preamble fails part-way.

### 3.3 `mush-config` / `mush-gateway` / `mushd` (changed)

- `[ssh] trusted_socket = ""`. Empty means off.
- `start_listener` takes an optional trusted path. `SshListener::stop_accepting` stops both
  listeners. On non-Unix platforms, a configured path is a boot error that names the reason.

### 3.4 `mush-sshdoorway` (new, lib + bin `sshdoorway`)

| Module | Content |
|---|---|
| `config.rs` | The TOML of doc 29 §10, `deny_unknown_fields`, and validation: service names, reserved names, explicit addresses, warnings shorter than the deadline, durations like `12h`/`15m`/`1m`/`30s` |
| `route.rs` | The username grammar (§3): `Route::Service { service, upstream_user, front_door }` or `Route::Menu` |
| `upstream.rs` | The `Connector` trait (prod: a Unix socket; tests: an in-process stream), the preamble client, and the russh client handler (logs the host-key fingerprint, relays the disconnect reason) |
| `auth.rs` | The downstream auth table (§5) as a state machine over the upstream phase (`None` → `Preamble` → `Ssh`). Changing the username to another service re-dials |
| `splice.rs` | The channel pair: the U→D pump task (a `select!` over upstream messages and injected lines), the D→U forwarders, the FIFO of pending replies, R-1/R-2 timeouts |
| `menu.rs` | The line menu, its tiny line editor, and stored pty/env/window state replayed upstream after the choice |
| `limits.rs` | Per-key concurrency, the per-key rate (a fixed window, bounded map, stale entries swept), the global cap, the counters behind the summary line |
| `drain.rs` | Generation state, the warning schedule, the session registry (an inject channel per session), deadline and force |
| `listen.rs` | socket2 listeners: `SO_REUSEPORT` (Unix), `IPV6_V6ONLY`, explicit addresses; the accept loop, which applies the limits before any SSH work |
| `logging.rs` | A reopenable file writer behind tracing-appender's lossy non-blocking writer, plus a stderr layer at `warn` and above (journald) |
| `notify.rs` | `sd_notify` `READY=1` / `STOPPING=1` (Unix datagram, abstract addresses supported) |
| `main.rs` | `sshdoorway --config <path> --generation <a\|b>`, signals (`HUP`, `TERM`, `USR1`, `USR2`) |

Every new crate root denies all six panic-shaped restriction lints, and both new crates
join `restriction_lints.rs` `MEASURED` at zero. Both new crates are network-facing, so they
go in the panic budget's **ratchet** tier at zero.

## 4. Tests

- **Windows and Linux.**
  - The proto codec: round trips, oversize frames, truncation, a frame followed by trailing
    bytes left in the stream.
  - Route and config validation.
  - Limits: the /64 key, the rate window, map bounds.
  - The drain schedule and duration parsing.
  - The trusted-ingress preamble over `duplex`.
  - The end-to-end tests: doorway + instance with an in-process `Connector`, over TCP
    loopback on the downstream side.
- **Linux only (`cfg(unix)`).**
  - The Unix-socket bind: mode 0660, stale-socket replacement, refusing to delete a
    non-socket.
  - Two `SO_REUSEPORT` generations on one port, with drain.
  - `SIGUSR2` log reopen.
  - `sd_notify`.

  These run on CI (Linux) and by hand on a VPS.
- **The adversarial pass (§6)** comes after DW.3.

## 5. Order

1. DW.0: proto, the trusted ingress, config and wiring. Then the `mush-net-ssh` tests and
   the contract tests.
2. DW.1: config, route, upstream, auth, splice, and the end-to-end tests.
3. DW.2: menu, limits, logging.
4. DW.3: drain, listen, notify, signals, `deploy/sshdoorway/*`, `check-logging.sh`.
5. The adversarial pass, then fixes, then re-test what changed.

Builds run one at a time with `-j 8` (Windows). fmt, clippy `-D warnings`, rustdoc
`-D warnings` and the contract tests run before pushing.

## 6. Adversarial test plan

Each attack runs against the built doorway and states its expected outcome.

| # | Attack | Expected |
|---|---|---|
| X-1 | Forged preamble on the instance's **public** TCP port | No preamble is parsed there; it is an SSH protocol error |
| X-2 | Malformed, oversized, truncated or out-of-order preamble frames on the trusted socket | Closed within limits; the slot is released exactly once (counter back to 0) |
| X-3 | `Begin` with an unregistered key, then `none` as that user | `BadCreds`; `none` rejected |
| X-4 | Attested user `faenix`, then SSH `none` as `admin` | Rejected (the attestation binds exactly one user) |
| X-5 | Password spray through the doorway from two addresses | Tempban hits the spraying address only |
| X-6 | Username tricks: `dev1++x`, `+dev1`, `dev1+`, a 10 KB username, NUL bytes, non-UTF-8, `DEV1` | Defined routes; no panic; bounded logs |
| X-7 | Switching username between attempts (`dev1+a` → `dev2+b`) | Re-dials the right service; no leaked upstream |
| X-8 | Forwarding, agent, X11, a third session channel, global requests | Refused at the doorway |
| X-9 | Client stops reading during heavy output (wedge) | Both sessions close within the write timeout; memory stays bounded |
| X-10 | Connection flood from one /64 and from many addresses | The rate limit drops TCP; the global cap holds; a legitimate client from another /64 still connects |
| X-11 | Scanner as `root` with `password` and with `exec` | No password prompt; exec refused |
| X-12 | Menu abuse: garbage input, a huge line, idling | Bounded line; idle timeout |
| X-13 | Instance down or restarting mid-session | Player told; doorway healthy; no task leak |
| X-14 | Drain under live sessions | Warnings in order with the owner's wording; exit at the deadline; new connections reach the new generation |
| X-15 | Log injection (CR/LF/escape sequences in the username) | Escaped in log lines |
