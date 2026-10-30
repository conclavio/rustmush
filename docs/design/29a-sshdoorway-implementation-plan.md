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
(10 s; the instance's own write timeout is 5 s). Request replies and channel opens use
10 s. A timeout is a
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

## 7. The adversarial review (2026-10-30) and what changed

Three independent reviewers read the built code cold, each against one attack surface:
the auth and trust boundary, the splice and denial of service, and drain and operations.
None found a way to log in without a password or a registered key. Every finding is listed
below with its disposition. Each fix has a test where one can be written on CI.

### Auth and trust

| # | Finding | Disposition |
|---|---|---|
| F1 | Authenticated sessions that are never spliced (menu, notice, `ssh -N`) never time out, so they can squat slots | **Fixed.** A second watchdog deadline cuts a session that opens no channel. The menu idle timer starts at channel open, not at the shell request, and a 15-minute hard cap applies. Tests: `an_authenticated_session_that_never_opens_a_channel_is_closed`, `a_menu_channel_that_never_asks_for_a_shell_times_out` |
| F2 | Anyone who can connect to the trusted socket logs into any account, because the attestation needs only a **public** key. That includes the gateway's own user, since the socket is 0660 owner `mush` | **Fixed.** `SO_PEERCRED`: the socket serves only a peer whose primary group is the socket's group, or root |
| F3 | A dead preamble or upstream is reused, so later passwords are rejected without reaching the instance | **Fixed.** Any upstream error drops `Up`, and the next attempt redials |
| F4 | `dev1+play+Kira` sent `none` as `play+Kira`, which the instance refuses | **Fixed.** A front door forwards the bare name. Unit test plus `a_front_door_with_a_character_part_still_reaches_the_welcome_screen` |
| F5 | Bidi and format characters in usernames | **Fixed.** `user_is_forwardable` refuses them |
| F5 | IPv6 tempban keyed per address at the instance | **Open, pre-existing.** The direct port has it too. Normalising the instance's ban keys to /64 is a separate change |
| F5 | An unlisted service's name is discoverable (it gets auth methods; a non-service name gets the menu) | **Accepted.** Unlisted means "not advertised", not "secret" |
| F5 | A version mismatch on a later frame closes without saying so | **Accepted** (cosmetic) |

### Splice and DoS

| # | Finding | Disposition |
|---|---|---|
| 1 (critical) | The menu or notice channel's unread queue of 100 wedges the session loop. russh pushes every channel message into it before calling the handler | **Fixed.** Every downstream channel's read half is dropped at open. Test: `hundreds_of_resizes_and_keystrokes_at_the_menu_do_not_wedge_it` |
| 2 | Menu and notice stages have no deadline | **Fixed** (as F1) |
| 3 | The global cap is cheap to exhaust. Over-cap connections lived about 60 s, and a /48 holds 65,536 /64 keys | **Fixed.** Over-cap connections get 5 s. A per-/48 aggregate cap (`per_v6_48_connections`, default 64). Unit test |
| 4 | A slow, healthy player who types while their own window is full can deadlock both loops (`data` awaited the upstream write) | **Fixed.** A per-channel **writer task** with a bounded queue. Callbacks only enqueue, so the session loop stays free to process window adjusts |
| 5 | Drain broadcast and close are serial at 5 s each. Wedged sessions survive the close, so the generation never exits | **Fixed.** Concurrent under one bound. Each connection's stream sits in a kill switch (`kill.rs`), cut at the deadline and on wedge, because russh's own session task cannot be aborted from outside |
| 6 | The reply FIFO desyncs on a timeout | **Fixed.** A reply that never comes closes the connection |
| 7 | Untimed control writes in the pump | **Fixed.** All timed |
| 8 | O(65k) sweep of a full rate table on every accept | **Fixed.** Swept at most once a second |
| 9 | Close and Eof before the shell reply; `biased` starving warnings; close churn; auth-chain length | **Fixed**: `wait_reply` returns on Close and Eof, `biased` removed, opens capped at 8 per connection. **Accepted**: menu type-ahead after `\r` is dropped, and a slow upstream can make an auth chain overrun the deadline (the watchdog closes it) |

### Drain and ops

| # | Finding | Disposition |
|---|---|---|
| H1 | Gateway units under `ProtectSystem=strict` cannot create `/run/rustmush-ssh/*.sock` | **Fixed.** `ReadWritePaths=-/run/rustmush-ssh` in dev1, dev1-split and dev2-split |
| H2 | `STOPPING=1` made the draining unit "deactivating", which broke swap.sh's guard | **Fixed.** The drain sends no `STOPPING`, and swap.sh reads `ActiveState` and the drain marker |
| H3 | `SIGTERM` = drain turned stop, restart, reboot or needrestart into a 24 h outage | **Fixed.** `SIGUSR1` drains (swap.sh only), `SIGTERM` finishes in about 30 s, `TimeoutStopSec=120` |
| M1 | A crash-restarted draining generation comes back into service | **Fixed.** A `draining-<gen>` marker makes the restart refuse. swap.sh clears it, waits 5 s and checks the new generation is still up |
| M2 | check-logging.sh checked only `a.log`, and its archive check proved nothing | **Fixed.** It checks every running generation, needs a *new* archive (`-newer` a stamp), checks reopen by the file's re-creation, uses `mktemp` and refuses unknown arguments |
| M3 | No hard exit; serial loops; a repeated finish restarted the grace | **Fixed.** A hard exit 60 s after the close, concurrent loops, and a repeated finish ignored |
| M4 | Signals arriving during startup kill the process | **Fixed.** Handlers are installed first |
| Low | Live socket stolen by a second instance; `[log]` changes ignored on reload; logrotate state dir before first start; unknown script arguments | **Fixed**: connect-probe before unlinking, a warning on reload, tmpfiles creates `/var/lib/sshdoorway`, argument checks. **Accepted**: warnings reach only the first spliced channel (the game channel) |

**Not covered by a test:**
- The peer-credential refusal itself: it needs a second group on the box. The acceptance
  path is tested on Linux.
- The systemd, logrotate and swap.sh behaviour. That is DW.4's box-level smoke.

## 8. The second review (2026-10-30, PR #193) and what changed

The DoS hardening commit (`68f42ae6`, F-1..F-11: the kill switch and `abort_after` for
half-closed clients, the menu-try, dial and password caps, the notice idle bound, the fast
key-offer rejection, the asynchronous socket probe, the menu timer stop before a splice) was
read against the code, and then three fresh reviewers took the same three surfaces as §7:
auth and trust (B), splice and denial of service (A), drain and operations (C). The
accounting invariants (every slot and drain guard released, every splice earned, the exact
attested binding, "a notice stays a notice") and the absence of any login without a password
or a registered key were re-verified. What follows is everything else.

### From the hardening commit itself

| # | Finding | Disposition |
|---|---|---|
| F-5 | The rate table refused new connections once it held 65,536 keys (fail closed) | **Reverted to fail open by eviction.** One IPv6 /48 is exactly 65,536 /64s: connect-and-close from one prefix filled the table in under a minute and locked every new player out. The table now evicts one entry and counts `rate-evict`; the global and /48 caps still run before the insert. Tests: `the_rate_table_is_bounded_and_evicts_rather_than_refuse`, `one_v6_48_cannot_fill_the_rate_table_against_everyone_else` |
| CI | Three rustdoc links to private items failed the documentation job under `-D warnings` | **Fixed** (plain text) |

### Auth and trust (B)

| # | Finding | Disposition |
|---|---|---|
| B-1 (critical) | The trusted ingress allocated connection ids from 1, the same range as the instance's public SSH port, and both feed one `SessionManager`; a collision silently handed one connection the other's session | **Fixed.** The trusted ingress allocates from `TRUSTED_CONN_ID_BASE` (2^31); public from 1, web from 2^32, Discord from 2^48. `attach` logs an error on a duplicate before its `debug_assert`. Test: `trusted_ids_never_meet_the_public_ports` |
| B-2 | An accept error ended the trusted accept loop for good (an `EMFILE` burst took the doorway's way in with it) | **Fixed.** The loop logs, waits 100 ms and goes on. `LimitNOFILE=65536` on the three gateway units |
| B-3 | A socket directory that was not setgid, or that left the socket in the gateway's own group, only warned; the peer check then trusted the wrong group | **Fixed, fail closed.** `bind` refuses unless the directory is setgid and the socket's group is not the process's own (`TrustedConfig::allow_own_group`, tests only). Test: `a_socket_in_the_processs_own_group_is_refused` |
| B-4 | No per-user (account) connection cap at the instance; the per-IP cap is the only one | **Open, pre-existing.** The direct port has the same shape; an account cap belongs with the instance's ban keys |
| B-5 | The instance's pre-auth deadline (120 s) is longer than the doorway's, and `Up` is kept open while a player sits at the menu | **Accepted.** The doorway's own deadlines bound the connection; the upstream is dropped on any error and redialled |
| B-6 | `user_is_forwardable` lets through some Unicode that renders confusingly (confusables, combining marks) | **Accepted.** Names are matched by the instance's account store; the doorway only has to keep control and format characters out of the log and the frame, which it does |
| B-7 | `KeyQuery` answered as fast as the instance looked the key up: a timing oracle on account existence | **Fixed.** Every reply waits at least `KEY_QUERY_FLOOR` (50 ms) from the question |

### Splice and DoS (A)

| # | Finding | Disposition |
|---|---|---|
| A-1 | `per_ip_connections = 8` is tight for a CGNAT or campus address | **Tunable, for the owner.** The instance's own per-IP default is the number to align with; both are config |
| A-2 | After three failed passwords the connection is aborted in 2 s even if a key (or the front door) lets it in meanwhile | **Fixed.** `abort_unless_authed` |
| A-3 | A channel the doorway closed (the instance ended it) that the client never acknowledges keeps the connection, and its slot, with nothing open | **Fixed.** The pump gives the client the auth deadline to answer with its own close, then disconnects (`PumpCtx::close_grace`) |
| A-4 | The instance's "no pending data before connect" gate dropped a healthy client whose window was exhausted during a rekey | **Fixed.** Three strikes (`PENDING_STRIKES`) before the disconnect |
| A-5 | `DISCONNECT_GRACE` (2 s) and `NOTICE_IDLE` (5 s) cut slow links before the message arrived | **Fixed.** 5 s and 10 s |

### Drain and ops (C)

| # | Finding | Disposition |
|---|---|---|
| C-1 (high) | The §7 M1 fix did not work: the restarted draining generation consumed its marker and exited 0 *before* `READY=1`, which under `Type=notify` is a protocol failure, so `Restart=on-failure` started it once more, now with no marker, serving beside its successor | **Fixed.** The doorway never removes the marker. A start that finds it logs, reports `READY=1` and exits 0 (a clean run). Only swap.sh clears it, right before starting that generation. The marker lives in `$STATE_DIRECTORY` (what swap.sh reads); unit tests on `marker_dir` |
| C-2 | Every authenticated connection wrote an `info` line at its end, so menu and notice sessions let an address fill the log | **Fixed.** `info` only for spliced sessions; menu and notice ends are counted (`menu-ended`, `notice-ended`) and logged at `debug` |
| C-3 | `channel_close` left the pump running; blocked on that channel's exhausted window it hit `WRITE_TIMEOUT` and ended the whole connection as wedged. The close itself was `try_send`, dropped when the queue was full | **Fixed.** The pump is aborted at close (its owed replies fail), and the close is sent with a bound |
| C-4 | swap.sh's "did not stay up" path left the failed generation to `Restart=on-failure`, which brought it up beside the old one | **Fixed.** Stop, `reset-failed`, disable |
| C-5 | Nothing was enabled, so a reboot left port 22 unserved | **Fixed.** `enable --now sshdoorway@a` at install; swap.sh enables the new generation and disables the old, so exactly one is enabled |
| C-6 | Listeners were bound before the readiness probe but accepted only after it: for up to 30 s the kernel queued half the new connections on a socket nobody read | **Fixed.** Accept from the moment of the bind |
| C-7 | Injected lines (farewell, drain warnings) carry the username and operator text | **Accepted.** The farewell's user has passed `user_is_forwardable` (no control or format characters); the drain message is operator config |
| C-8 | As B-2 | **Fixed** (as B-2) |
| C-9 | As A-5 | **Fixed** (as A-5) |
| C-10 | `UP_QUEUE` of 256 packets parked up to 8 MiB per channel at the doorway | **Fixed.** 32 (1 MiB per channel) |
| C-11 | The trusted stale-socket probe treats any connect failure as stale | **Accepted.** The probe is bounded at 2 s; a live socket whose backlog is full answers within it, and a refused connect is the only common failure. The unlink is of a socket file only, never another kind of file |
| C-12 | A race between the drain's "empty" and its "work done" arms in the signal loop | **Accepted.** Either arm ends the process cleanly |
| C-13 | `force_grace_secs = 0` was accepted | **Fixed.** Validated with the other limits |
| C-14 | As B-3 | **Fixed** (as B-3) |
| C-15 | logrotate ran as root in a directory the doorway owns; further unit hardening possible | **Partly.** `su sshdoorway sshdoorway` in the logrotate stanza. Further systemd hardening (`SystemCallFilter`, `RestrictNamespaces`) is DW.4's box-level work, where it can be tested |

**Not covered by a test**, as before: the systemd, logrotate and swap.sh behaviour (C-1,
C-4, C-5) is DW.4's box-level smoke, and the half-closed-channel grace (A-3) needs a
client that ignores a channel close, which russh's client does not offer.
