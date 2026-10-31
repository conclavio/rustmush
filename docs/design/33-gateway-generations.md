# 33. Gateway generations: restarting mush-gateway without dropping players

**Draft 0.2, 2026-10-30.** Based on `origin/main` at `2085d9f6`. Docs only: nothing in this
document is built. The doorway of doc 29 (PR #193) is on `main` as of `265629be`, so every
doorway file cited here is cited from `main`. The decisions in §7 come from the owner's design
conversation of 2026-10-30. Draft 0.2 records the owner's rulings of the same day: every
question in §8 was accepted at its default, so the cut train can start at GG.0 and GG.3.

## 1. Goal

Every restart of `mush-gateway` disconnects every player on the instance. The gateway holds
the SSH sessions (public port and the doorway's trusted ingress), the web sockets and the
Discord bridge, and it also carries the rendering: the line editor, the ratatui TUI and the
pixel compositor. Those change often, so the gateway restarts far more often than the
doorway, and each restart costs every player their session. The server half already restarts
without dropping anyone (M16–M18: the gateway holds the connections and re-attaches them).
This document gives the gateway the property the doorway has: **two generations**, a new one
that takes new connections and an old one that keeps its sessions, warns, and leaves when it
is empty or when its deadline passes.

The doorway built the mechanism (`SO_REUSEPORT`, `tcp_migrate_req`, two alternating units,
`SIGUSR1` drain, a drain marker, `swap.sh`). This design reuses it. What is new is on the
other side of the gateway: the IPC boundary must tolerate a version step while the old
generation drains, the server must accept more than one gateway at a time, and the server
must stop being the gateway's child.

### 1.1 Scope

- **In scope.** The IPC compatibility window, the server as its own unit, the multi-link
  server, listener handoff for the gateway's three kinds of listener, a shared drain crate,
  the gateway's drain presentation, the swap script and units, and the deploy order.
- **Out of scope.** Moving established connections between processes (impossible, doc 29
  §9.1). Changing what the gateway renders. Closing the direct ports (Q-29-4; §6).
- **Not changed.** The all-in-one `mushd` binary, which has no boundary and no generations.

## 2. What exists today

### 2.1 The deploy shape

`deploy/dev1-split/dev1mush-gateway.service` (dev2's differs only in names, ports and paths)
is `Type=simple`, `Restart=on-failure`, `KillMode=mixed`, `ExecReload=/bin/kill -HUP`. The
gateway **spawns** `mush-server` as a child (`spawn_server_child` and `SpawnLauncher` in
`crates/mush-gateway/src/lib.rs`), connects to the `[ipc].listen` endpoint the child binds,
and supervises it: on the child's exit it respawns under a crash-loop guard (`CrashGuard`,
`MAX_CONSECUTIVE_FAILURES`, a backoff), and on `SIGHUP` it restarts the child deliberately.
Across each restart the gateway enters a gap (input refused, a "restarting; hold on" `Sys`
line), reconnects, replays its full attachment roster as `ToServer::ReAnnounce`, crosses a
`ServiceReq::Barrier`, replays retained input custody, and resumes. That is the hot restart:
a deploy is `systemctl reload`, and players are never disconnected by a **server** change.

The gateway's own stop (`SIGTERM`) closes the boundary and waits up to 20 s
(`SHUTDOWN_EXIT_WAIT`) for the child to exit, so that a successor can take the child's locks:
the quarantine spool's lifetime lock (`.<spool>.rustmush-quarantine.lock`, exit class
`QuarantineLockHeld`) and the IPC socket. The launcher requires **positive exit
confirmation** (`LaunchedServer::exited`) before any respawn; a timeout never permits one.
Both roles, the lock handoff and the exit confirmation, must find a new home in §4.2.

The gateway unit also carries things the server needs: `EnvironmentFile=` with the LLM keys
(the child inherits them), `RuntimeDirectory=rustmush` for the IPC socket, and `Requires=` on
the instance's Redis.

### 2.2 The IPC boundary

Crate `mush-ipc` (`protocol.rs`, `io.rs`, `link.rs`). `IPC_VERSION` is 12. The server greets
first with `ToGateway::Hello { proto, server, features, panels }`; the gateway sends no hello
of its own and rejects `proto != IPC_VERSION` as `BoundaryFailure::VersionSkew` (logged with
the `IPC-VERSION-SKEW` marker). The version history in `protocol.rs` says "deploy both halves
together" at each recent bump (8, 10, 12), because nothing in the protocol lets the halves
differ. The encoding is self-describing rmp: defaulted fields are additive, and `panels` and
`SendTarget::Relay` were added without a bump. A **new variant** is not additive: an old peer
cannot decode it.

`[ipc].listen` is a TCP address or a Unix socket path (`IpcEndpoint`); production uses
`/run/rustmush/dev1.sock` and `/run/rustmush-dev2/dev2.sock`. The server binds it and accepts
**exactly one** gateway (`crates/mush-server/src/lib.rs`: "One gateway connection per
boundary (M16b)"), and when that link ends, for any reason, the server sends the world
`ControlMsg::Shutdown` and exits. Today the server's lifetime **is** the link.

The server keeps no session mirror: `ToServer::Attach` is accepted and ignored, `Detach` frees
that connection's retained input claims, and `ReAnnounce` goes to the world task as a
`ReAttachRec` list, applied record by record through `Engine::reattach` (additive: it does
not clear the connection map first). The world's connection state is `Engine.conns:
HashMap<ConnId, ConnInfo>` plus `World.connection_bindings` and the output brackets, all
keyed by the connection id alone. Output leaves the world as `ToGateway::Deliver { target,
event }` with `SendTarget::{Char, Conn, All, Relay}`; the gateway resolves `Char` to that
character's live connections on its own `SessionManager`, so a character with two SSH windows
already receives every line twice, once per connection.

### 2.3 Connection ids

A connection id is a `u64` the gateway allocates from disjoint ranges: public SSH from 1, the
doorway's trusted ingress from `TRUSTED_CONN_ID_BASE = 1 << 31`
(`crates/mush-net-ssh/src/trusted.rs`), web from `WEB_CONN_ID_BASE = 1 << 32`
(`crates/mush-net-web/src/hooks.rs`), Discord follows from `DISCORD_CONN_ID_BASE = 1 << 48`
(`crates/mush-gateway/src/discord_bridge.rs`). The session manager logs an error and
`debug_assert`s on a duplicate (doc 29a §7, round 2). The ranges are per process: two
gateways would allocate the same ids.

### 2.4 The doorway's drain, and what is reusable

All under `crates/mush-sshdoorway/src/` and `deploy/sshdoorway/`:

- `drain.rs`: `Drain` with `enter`/`live` (the live count and its guard), `register`/`track`
  (a session's inject queue and its kill switch), `abort`/`abort_all`, `phase`/`watch`/`begin`
  (`Serving` → `Draining`), `wait_empty`, `broadcast`, `close_all`, `run` (warnings on
  schedule, close at the deadline, early return when empty), `finish_now` (one warning, a
  grace, close), and `schedule` (the warning offsets from a deadline and a `Warning` list).
- `kill.rs`: `KillSwitch` and the `Killable<S>` stream wrapper, because a russh session task
  cannot be aborted from outside; the drain cuts the stream under it instead.
- `main.rs`: the signal lifecycle. `SIGUSR1` drain (swap.sh only), `SIGTERM`/`SIGINT` finish
  (one warning, 30 s, close), `SIGHUP` reload, `SIGUSR2` log reopen; the `draining-<gen>`
  marker in `$STATE_DIRECTORY` that the process never removes; `READY=1` through
  `notify.rs`; a hard exit 60 s after the close (`HARD_EXIT_AFTER`).
- `listen.rs`: `bind` with socket2 (`set_reuse_address`, `set_reuse_port`, `set_only_v6`) and
  `migrate_req_enabled` (reads `/proc/sys/net/ipv4/tcp_migrate_req`).
- `deploy/sshdoorway/swap.sh`: a flock, the serving generation found by `ActiveState` and
  the marker, `--force` to finish a still-draining generation, the marker cleared right
  before `systemctl start`, a 5 s stay-up check, and stop/`reset-failed`/disable of a
  successor that did not stay up, so `Restart=on-failure` cannot bring it up beside the old
  one; then enable the new and disable the old, so exactly one generation is enabled.
- `deploy/sshdoorway/99-sshdoorway.conf`: `net.ipv4.tcp_migrate_req = 1`, box-wide.
- `sshdoorway@.service`: `Type=notify`, `KillMode=mixed`, `TimeoutStopSec=120`,
  `Restart=on-failure`, `StateDirectory=sshdoorway`.

Two lessons from PR #193's reviews carry over verbatim (29a §7 H2/H3, §8 C-1): a
`Type=notify` process that exits **before** `READY=1` is a protocol failure that
`Restart=on-failure` restarts, so a "found my marker, refusing to serve" exit must send
`READY=1` first and exit 0; and a drain must send no `STOPPING=1`, because a unit that reports
itself stopping breaks the two-generation guard.

What the gateway does **not** have today: both `mush-net-ssh` (`server.rs`) and `mush-net-web`
(`app.rs`) bind with a plain `TcpListener::bind`, with no reuseport; the gateway has no
`--generation`, no marker, no `sd_notify`, and `SIGHUP` means "restart the server child".

## 3. The owner's three concerns

1. **The IPC must tolerate one version step.** While generation a (old binary) drains for up
   to 24 h, generation b (new binary) serves, and both talk to one server. §4.1 fixes the
   window at one step in one direction and makes the server the side that tolerates it.
2. **The gateway's size makes this worth more than it was for the doorway.** The doorway
   holds no game logic and rarely changes (doc 29 §9.5). The gateway carries three renderers
   and three transports; most player-visible work ships through it. Today that work either
   waits for an empty instance or drops everyone; `deploy.sh` makes a gateway restart an
   explicit opt-in (`GATEWAY_RESTART=1`) for that reason.
3. **A player may be connected to both generations.** A player who reconnects during a drain
   lands on the new generation while their old window is still on the old one. §4.6: the
   connection is the unit, never the player, and the server already serves one character on
   many connections.

## 4. Design

### 4.1 IPC compatibility: one step, one direction (D-33-1)

**Order.** The server upgrades first (it already restarts under held connections), then the
gateways swap. So at any moment the server is at version N and a gateway is at N-1 or N. The
server supports both; a gateway speaks only its own version.

**Hello in both directions.** `ToGateway::Hello` gains `min_proto: u32` (defaulted, so an
old decoder ignores it). The gateway's first frame becomes a new `ToServer::Hello { proto,
min_proto }`. The server accepts a gateway whose `proto` lies in `[server.min_proto,
server.proto]` and records the link's version; the gateway accepts a server whose range
contains its own `IPC_VERSION`, and otherwise fails with the existing `VersionSkew`. Normally
`min_proto = IPC_VERSION - 1`. Bootstrap rule for GG.0's own rollout: a link whose first frame
is not `ToServer::Hello` is a version 12 peer, the last version without one.

**Inside the window, additive only.** A `ToGateway` variant or field added at N is emitted
only to links that announced N; a link at N-1 gets the N-1 behaviour (usually: nothing, or
the older frame). New `ToServer` traffic needs no gate, because the server is never behind a
gateway. A removal waits one release: the server at N+1 still decodes everything an N
gateway sends. The window is never wider than one step, so each bump has exactly one
compatibility case to think about.

**Golden frames.** At each bump, a recording test writes one encoded sample of every variant
to `crates/mush-ipc/tests/golden/v<N>/`. A test beside `roundtrip.rs` decodes the previous
version's set with the current codec, and `mush-contract-tests` gains a lint that fails when
`IPC_VERSION` is newer than the newest golden set. The recorded frames are the contract; the
comment history in `protocol.rs` stops being the only record.

**Why one step is enough.** A drain lasts at most 24 h (or 30 s under `--force`). A server
upgrade to N+1 waits until no N-1 gateway generation is live; the marker makes that check a
one-liner in the deploy script, and the bound makes the wait at most a day. Rollback runs
the steps in reverse: swap the gateways back, then the server.

**This cut pays for itself alone.** Today every bump forces both halves to deploy together,
which is a full restart of the gateway, which drops every player. With the window, a server
bump is a hot restart and the gateway follows whenever it is convenient.

### 4.2 The server as its own unit (D-33-2)

`<inst>mush-server.service` runs `mush-server --config ... --ipc <socket>` directly, binds the
IPC Unix socket (it already does), and owns `RuntimeDirectory=rustmush`, the LLM
`EnvironmentFile=`, and `Requires=` on Redis. `Restart=on-failure` replaces the gateway's
respawn; systemd's start-limit replaces the crash-loop guard. The gateway units `Wants=` and
`After=` the server unit, never `Requires=`: a server stop must leave the gateway holding its
connections, which is the whole point of the gap.

On the gateway, `ServerSource::Spawn`, `SpawnLauncher`, `spawn_server_child`, `CrashGuard`
and the respawn arm of the supervisor retire. What stays is the gap and the heal, unchanged:
`BoundaryHandle::enter_gap`, the restarting notice, input refused not dropped, reconnect
with backoff, `ReAnnounce`, the barrier, custody replay, `BoundaryHandle::connect` to leave
the gap. The gateway's `SIGHUP` stops meaning
"restart the child"; a deploy restarts the server with `systemctl restart`, and the gateway
sees EOF and heals exactly as it does after a crash today.

The two roles of §2.1 move as follows. The **exit confirmation** becomes systemd's: a unit's
next start never begins until its main process is gone, which is the guarantee the launcher
built by hand. The **lock handoff** is then ordinary: the old server releases the quarantine
lock and the socket on exit, and the new one takes them; the server's existing 15 s wait for
the lock at boot (`QUARANTINE_LOCK_WAIT` in `mush-server/src/main.rs`) stays as the
backstop. The one hazard the gateway's comments record, that on conclav.io the control-group
sweep fails with `EINVAL` and systemd starts the next instance while an orphan still holds
the lock, applied to a **child** of the unit's main process. With the server as the main
process there is no orphan to sweep. Q-33-5 is ruled: `RestartSec=2`, five starts in 60 s,
the give-up notice after the same bound, the 15 s lock wait kept.

The gateway must also learn to boot with no server: today the first connect is boot-fatal
after `INITIAL_CONNECT_TIMEOUT`. It becomes a gap from the start: the gateway binds its
listeners, serves the restarting notice, and keeps reconnecting.

### 4.3 Many links, link-scoped connections (D-33-3, D-33-4)

The server's accept becomes a loop. Each accepted gateway is a **link** with its own reader,
writer, Hello, version and metrics. The connection map is keyed by `(link, conn)`: the world
keeps its `ConnId` keys, and the link layer keeps `link → set of conn` and `conn → link`.

- `Attach` and `ReAnnounce` are per link and additive (as `Engine::reattach` already is).
- A link's EOF detaches **that link's** connections, as if each had sent `ConnDown`, and
  nothing else. The world keeps running. "Boundary gone, shut the world down" goes away; an
  operator stops the server with `systemctl stop` or `@shutdown`.
- `Deliver` to `SendTarget::Conn` goes to the owning link only. `Char`, `All` and `Relay` go
  to every link, and each gateway resolves them against its own session table, which is what
  it does today. `SetOutputBrackets`, `SetFloodExempt`, `CloseSession`, `Detach` and
  `CloseWebSessions` follow the same rule by their target.
- The reply correlation id of `ToServer::Request` is per link.
- Anything that today reads "the gateway" as a singleton (`WorldHealth`, `DiscordControl`)
  is sent to every link. The siteban mirror needs nothing: it reads `ban:*` from Redis in
  each gateway, not from a frame.

**Ids must be unique across generations.** Two schemes:

- **A generation bit (recommended).** Generation b sets bit 62 on every id it allocates;
  generation a sets none; the per-transport bases stay. Values stay below 2^63, so a signed
  conversion anywhere stays positive. No round trip, no change to who allocates. The session
  manager's duplicate check stays as the wiring-bug guard, now also across links on the
  server.
- **Server-assigned ids at `Attach`.** Globally unique by construction, but the gateway keys
  its own session table before the server knows the connection exists, so it would need a
  provisional id and a rename, or a round trip before admission.

Q-33-1 is ruled: the bit. Connection ids live in process
memory on both halves; GG.2 confirms by grep that no store record carries one.

**Takeover rules.** Any rule of the form "a new connection to this character takes over" is
expressed as the server closing the old connection (`ToGateway::CloseSession`/`Detach` to the
owning link), so it works when the old connection is on the other generation. The gateway
never decides it locally.

### 4.4 Listener handoff (D-33-5)

- **TCP (SSH 6789/6792, web 6790/6793).** The doorway's recipe: `listen::bind` with
  `SO_REUSEPORT`, `tcp_migrate_req = 1` already installed box-wide by
  `99-sshdoorway.conf`, and a startup warning when the sysctl is off. `mush-net-ssh` and
  `mush-net-web` take a pre-bound `std::net::TcpListener` instead of an address, so the
  recipe lives once, in the shared crate (§4.5).
- **The trusted Unix socket.** `SO_REUSEPORT` does not apply. Each generation binds its own
  path, `<stem>.<gen>.sock` beside the configured `<stem>.sock` (for dev1:
  `/run/rustmush-ssh/dev1.a.sock`), and `<stem>.sock` becomes a **symlink** that the swap
  script re-points with `rename` of a freshly made temporary link, after the stay-up check,
  so a successor that died never receives doorway traffic. The doorway dials by path
  (`UnixConnector`, `UnixStream::connect`), which follows the symlink at connect time;
  established sessions keep their socket. The config value does not change: the gateway
  derives the per-generation path from `[ssh].trusted_socket` and `--generation`; without
  `--generation` (tests, `mushd`) it binds the configured path as today. The bind probe
  (`BoundTrustedServer::bind`) refuses anything that is not a socket, which is right: a
  generation must never be pointed at the symlink name, and `--check-config` refuses a
  `trusted_socket` that is a symlink when `--generation` is unset.
- **Socket activation** was considered and rejected: a socket unit hands its descriptors to
  one service unit, so there is no second generation to drain into (doc 29 §15 says the
  same).
- **The IPC socket** needs no handoff: the server binds it (§4.2) and both generations dial.

### 4.5 A shared drain crate (D-33-6)

`mush-drain` (working name) lifts `drain.rs`, `kill.rs`, `listen.rs`, `notify.rs`, the
signal lifecycle of `main.rs` (`SIGUSR1` drain, `SIGTERM`/`SIGINT` finish, `SIGUSR2` reopen,
the marker in `$STATE_DIRECTORY`, the hard exit), and a `swap.sh` parameterised by unit stem,
marker directory and, optionally, a symlink to re-point. The doorway is re-based on it with
no behaviour change (GG.3), so one implementation serves both daemons and the next review
finding is fixed once.

The crate is generic over what a "session" is: an inject sink and a close. The doorway's
entry is a russh `Handle` plus a channel; the gateway's is a `ConnectionHandle` on its
`SessionManager`. The doorway injects raw lines between upstream chunks because it cannot
know the frame; the gateway owns the frame, so its warnings are **`OutputEvent::Sys` events**
through the normal delivery path, rendered by the line editor, the TUI and the compositor as
the restarting notice is today, never a raw write into a kitty escape. Web connections get
the same `Sys` lines and, at the deadline, a close frame with the existing
`close_code::SERVICE_RESTART` and a reason the client can show. Q-33-4 is ruled: the client
reconnects by itself with its resume sequence once the warnings have been shown as `Sys`
lines, and shows the reason only when the reconnect fails. The Discord bridge is not connection-bound: exactly one generation runs the
bot, under an `flock` on `$STATE_DIRECTORY/discord.lock`; the draining generation stops its
bot at `SIGUSR1` and the new one, blocked on the lock since its start, connects. Its follows
are rebuilt by whichever generation holds the bot.

Gateway signals after this design:

| Signal | Effect |
|---|---|
| `SIGUSR1` | **Drain** (swap script only): stop accepting, re-point nothing (the script does), warn on the schedule, close at the deadline or when empty, exit 0. Leaves the marker. |
| `SIGTERM`, `SIGINT` | **Finish**: stop accepting, one warning, 30 s, close everything, exit 0. `systemctl stop`/`restart` and a reboot therefore never become a 24 h drain. |
| `SIGHUP` | Reserved for a config reload; logged and otherwise ignored. The deploy scripts stop sending it. |
| `SIGUSR2` | Reopen the log file, when the gateway's logging moves to the doorway's rotation scheme; until then unassigned. |

### 4.6 One player, two generations (D-33-7)

The unit of everything here is the **connection**. The drain warns and closes only the old
generation's connections. A player with a window on each generation is, to the server, a
character with two connections, which it already serves (§2.2): `Char` output reaches both,
each window's own input and replies stay on its connection, and when the old window closes,
the other plays on. Nothing identifies "the player" across generations, and nothing needs
to. Per-character decisions the gateway makes locally (the session's flood bucket, panel
keyframes, composing state) are per gateway and so are briefly made twice during a drain,
which is the same as two windows on two gateways would be; GG.2's test lists each and
confirms none is load-bearing.

### 4.7 The swap, step by step

1. `deploy.sh` builds. If the server binary changed, `systemctl restart <inst>mush-server`:
   the gateways hold their connections through the gap and heal (§4.2). This is a hot restart
   and needs no swap.
2. If the gateway binary changed, `gateway-swap.sh <inst>`: the flock; find the serving
   generation by `ActiveState` and marker; refuse if the other generation is still draining
   unless `--force`; clear the new generation's marker; `systemctl start <inst>mush-gateway@b`
   (returns at `READY=1`, which the gateway sends once its listeners are bound and its link
   to the server is up or its boot gap is serving); wait 5 s and confirm `active`, else stop,
   `reset-failed`, disable, and keep serving on a; re-point `<stem>.sock` to `b`'s socket;
   enable b and disable a; `systemctl kill --signal=USR1 <inst>mush-gateway@a`.
3. Generation a closes its TCP listeners (the kernel migrates their queues), keeps every
   session, sends the `Sys` warnings on the schedule, and exits when empty or at the
   deadline. A crash-restart of a finds its marker, reports ready and exits 0.
4. The next server upgrade with a version bump first checks that no gateway generation is
   draining (§4.1), and `--force`s the drain if the operator says so.

## 5. Alternatives considered and not chosen (D-33-8)

- **Move the rendering into the server.** The gateway would become a thin transport holder
  that rarely changes, and the server would carry the line editor, TUI and compositor. This
  reverses the M16 split, which put the rendering in the half that holds the connections so
  that a server restart never repaints a screen it does not own; it is a large refactor of
  three renderers; and every other gateway change (russh, axum, the transports, the trusted
  ingress) would still need a restart that drops everyone. Generations solve the whole
  class.
- **Make the doorway the connection holder, with IPC to the gateway.** The doorway would
  reproduce the SSH session semantics (PTY, env, exec, sftp, window changes) that the trusted
  relay was built to avoid reproducing (doc 29 §15), would discard the relay just built,
  would still leave the web and Discord surfaces with the old problem, and would become the
  one process that may never restart, which the doorway was designed not to be.
- **Pass established connections between processes.** The transport state is in process
  memory (doc 29 §9.1).
- **A symmetric window (server accepts N-1..N+1).** More cases per bump for no deploy the
  owner wants; a rollback runs the one-direction steps in reverse.

## 6. Closing the direct ports

Once the doorway is live, closing 6789/6792 (Q-29-4, ruled "keep open; revisit at launch")
would leave the gateway with one TCP listener (web) and one Unix socket. It changes nothing
in this design: the reuseport recipe is needed for the web port regardless, and the symlink
handoff for the trusted socket regardless. It is worth doing on its own and is independent
of every cut here. Q-33-3 re-asked it only because the swap script's listener list is shorter
if the answer is yes; the owner kept the earlier ruling.

## 7. Decisions

| Id | Decision |
|---|---|
| D-33-1 | The IPC window is one step, one direction: the server at N accepts gateways at N-1 and N; a gateway speaks only its own version. Hello both ways with `proto` and `min_proto`; additive-only inside the window; a removal waits one release; golden frames per version with a bump lint (§4.1). |
| D-33-2 | The server is its own systemd unit and binds the IPC socket; gateways are clients. The gateway's supervisor, respawn and crash-loop guard retire in favour of `Restart=on-failure` and the existing reconnect, gap and `ReAnnounce` heal. A gateway boots with no server as a gap (§4.2). |
| D-33-3 | The server accepts many links; its connection map is keyed by (link, connection); a link's EOF detaches that link's connections only and never stops the world; `Attach`/`ReAnnounce` are per link and additive; `Conn` output goes to the owning link, `Char`/`All`/`Relay` to every link (§4.3). |
| D-33-4 | Connection ids are unique across generations by a generation bit (bit 62 for generation b; Q-33-1 ruled). Takeover rules go through the server's per-connection close (§4.3). |
| D-33-5 | TCP listeners hand off with `SO_REUSEPORT` and the box-wide `tcp_migrate_req`; the trusted Unix socket is per generation (`<stem>.<gen>.sock`) behind a `<stem>.sock` symlink the swap script re-points with `rename`; socket activation rejected (§4.4). |
| D-33-6 | A shared drain crate (`mush-drain`): `drain.rs`, `kill.rs`, `listen.rs`, `notify.rs`, the signal lifecycle, the marker and a parameterised `swap.sh`; the doorway is re-based on it unchanged (§4.5). |
| D-33-7 | The connection is the unit. The gateway's drain warnings are `OutputEvent::Sys` events in its own frame; web connections close with `SERVICE_RESTART` and a reason; one generation runs the Discord bot under a lock (§4.5, §4.6). |
| D-33-8 | Deploy order: server first, then the gateway swap; a bumped server upgrade waits for a finished drain. Rendering into the server, the doorway as holder, fd passing and a symmetric window are rejected (§4.7, §5). |

## 8. Owner rulings (2026-10-30)

The owner accepted all six defaults on 2026-10-30. Each default is now a decision; the table
keeps the question for the record.

| Id | Question | Ruling (default accepted) |
|---|---|---|
| Q-33-1 | Id scheme: a generation bit (bit 62 on generation b's ids) or server-assigned ids at `Attach`? | **The bit.** No round trip and no change to who allocates; the duplicate guard stays. |
| Q-33-2 | The gateway's drain deadline and warning schedule: the doorway's 24 h with 12h/6h/2h/1h/15m/1m, or shorter, since gateway swaps will be frequent? | **Same as the doorway**, one config in the shared crate. The warnings mean a player is never surprised, and an empty generation exits at once. Revisit with data from dev2. |
| Q-33-3 | Close the direct ports (6789/6792) once the doorway is live? Re-asks Q-29-4 (§6). | **Keep the earlier ruling** (open until launch). Nothing here depends on it. |
| Q-33-4 | What does the web client do on a drain close? Reconnect by itself with its resume sequence (`?resume=<last_seq>`; it would land on the new generation through the same port) or show the reason and a button? | **Reconnect by itself**, after the warnings have been shown as `Sys` lines; a failed reconnect shows the reason. |
| Q-33-5 | Supervision values for the server unit: `Restart=on-failure`, `RestartSec=`, `StartLimitBurst=`/`StartLimitIntervalSec=` in place of the crash-loop guard; `Wants=`+`After=` from the gateways; what the gateway tells players when the server stays down past the old guard's give-up notice; and whether the quarantine lock's 15 s boot wait stays. | **`RestartSec=2`, five starts in 60 s**, the give-up notice after the same bound, the 15 s wait kept. |
| Q-33-6 | Should `deploy.sh` swap gateways only when the gateway binary changed (by hash against the running generation), or on every deploy? | **By hash.** A server-only change stays a hot restart. |

## 9. Cuts

| Cut | Content | Done when |
|---|---|---|
| GG.0 | The IPC contract: `min_proto` in `ToGateway::Hello`, the new `ToServer::Hello`, the version-12 bootstrap rule, the window check on both halves with the skew marker, the per-link version gate for additive `ToGateway` emission, the golden-frame recorder and decode test, the `mush-contract-tests` bump lint. | A two-process test serves a 12 gateway from a 13 server (held sessions, output, requests); a 13 gateway refuses a 12 server with `IPC-VERSION-SKEW`; the golden set for 12 decodes under 13; the lint fails on a bump with no golden set. Deployable alone, and worth deploying alone. |
| GG.1 | The server as its own unit: `<inst>mush-server.service` (socket, runtime dir, env, Redis), the gateway as client only (launcher, spawn, crash guard and respawn removed; boot-with-no-server as a gap; `SIGHUP` reserved), `deploy.sh` restarts the server unit. | `split_halves` passes with the server launched by the test harness and restarted externally; the restart-survival story (gap notice, refused input, replay, barrier, resumed play) is unchanged; a gateway started before the server serves the gap and heals when it appears. |
| GG.2 | The multi-link server: the accept loop, `(link, conn)` keying, link-scoped EOF, per-link `Attach`/`ReAnnounce`/replies, target routing by link, the generation bit and `--generation`, the duplicate-id guard across links, the per-character gateway-local state audit. | A test with two gateways on one server: connections on both; `Char` output reaches both, `Conn` output one; link a's EOF detaches a's connections only and b plays on; b's `ReAnnounce` leaves a untouched; a server takeover closes a connection on the other link; a duplicate id across links is refused loudly; the audit's list is in the cut note. |
| GG.3 | `mush-drain`: lift `drain.rs`, `kill.rs`, `listen.rs`, `notify.rs`, the signal lifecycle, the marker and the parameterised `swap.sh`; the doorway re-based on it. | No behaviour change: the doorway's `e2e` suite (including `draining_warns_on_schedule_and_closes_at_the_deadline`) and its unit tests stay green, and `deploy/sshdoorway/swap.sh` is a thin call into the shared script. |
| GG.4 | Gateway generations: `<inst>mush-gateway@.service` (`Type=notify`, `StateDirectory`), `--generation`, `READY=1`, the marker, reuseport listeners for SSH and web, the per-generation trusted socket and the symlink re-point in the swap script, `Sys` warnings, the web close, the Discord lock, `deploy.sh` swapping by hash. | A Linux two-generation test holds SSH (public and trusted) and web sessions on a while it drains (short deadline, scaled warnings), routes new connections to b, delivers every warning in order and in-frame, migrates a queued connection under `tcp_migrate_req = 1`, re-points the symlink and dials it, closes web with `SERVICE_RESTART`, and exits a at the deadline or at zero sessions; a refused third generation and `--force` both work. |
| GG.5 | The dev2 box smoke, on the owner's go-ahead like every deploy: install the server unit and both gateway units, swap while the owner plays on w11term through the doorway and the direct port, with the cockpit live, the web client open and the Discord bridge following; then a bumped deploy (server first, swap second). The runbook, `dev2-redeploy.sh` and the deploy memory are updated. | The owner's session survives the swap with the warnings shown in-frame; the doorway session survives it; the web client reconnects; Discord relays once, never twice; a server-only deploy is a hot restart with no swap. |

GG.0 and GG.3 touch disjoint files and can run in parallel. GG.1 depends on GG.0 only for the
Hello shape; GG.2 depends on GG.1; GG.4 depends on GG.2 and GG.3. GG.5 is a box day.

## 10. Risks and lessons carried over

- **Every lesson in 29a §7 and §8 under "Drain and ops"** applies to the gateway as written:
  `READY=1` before a marker exit; no `STOPPING=1`; `SIGTERM` finishes, never drains; stop,
  `reset-failed` and disable a successor that did not stay up; accept from the moment of the
  bind; a hard exit after the close; handlers installed before anything that can fail.
- **The server's lifetime is no longer the link's.** Every path that today ends the server on
  a gateway EOF, and every test that relies on it, is found by GG.1 and GG.2. An operator who
  expects `systemctl stop <inst>mush-gateway` to stop the server learns that it no longer
  does; the runbook says so.
- **Two bots.** A window in which both generations run the Discord bot relays every line
  twice. The lock in §4.5 closes it; GG.5 checks it on the box.
- **A server bump straddling a drain.** The deploy script's marker check is the only guard;
  `--force` is the operator's answer, and it costs the draining players a 30 s warning.
- **Tests are Linux-only** where they touch sockets and signals, as the doorway's are
  (D-29-17); the Windows build still compiles everything.
