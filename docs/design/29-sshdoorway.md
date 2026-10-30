# 29. sshdoorway — one SSH front door on port 22

**Draft 0.2, 2026-10-30.** Based on `origin/main` at `ebeee17b`. Docs only: nothing in this
document is built. The decisions in §12 come from the owner's design conversation of
2026-10-30. The owner ruled every question in §13 the same day: five at their defaults, and
Q-29-2 with a longer warning schedule (§9.4). Draft 0.2 also adds the owner's logging
requirement: aggressive rotation and compression, checked before shipping (§8.1, D-29-18).

## 1. Goal

Today a player has to know a port number: `ssh -p 6789 faenix@conclav.io` reaches dev1 and
`-p 6792` reaches dev2. People who rarely use SSH find this confusing. The goal is that
nobody types a port:

```
ssh dev1@conclav.io             # dev1's welcome screen
ssh dev1+faenix@conclav.io      # dev1, account faenix, password or registered key
ssh dev1+faenix+Kira@conclav.io # dev1, account faenix, straight into character Kira
ssh conclav.io                  # (as $USER, e.g. nick) a menu: pick dev1 or dev2
```

**sshdoorway** is a new daemon that listens on port 22 and reads the SSH username. It routes
each connection to the right game instance on the same box. Everything after the first `+`
reaches the instance unchanged, so accounts, `+char` and `@sshkey` keep working.

The model is [sshpiper](https://github.com/tg123/sshpiper). sshdoorway is narrower: it knows
the rustmush backends and the trusted ingress they expose (§4), and refuses anything that is
not a game session.

### 1.1 Scope

- **In scope.** The `sshdoorway` binary, the trusted ingress it uses on the gateway, the
  preamble protocol between the two, the menu, abuse handling at the doorway, the drain
  restart, and the systemd and config shape.
- **Out of scope: host networking.** sshdoorway binds port 22 on a **configured list of
  addresses** (IPv4 and IPv6) and never on a wildcard. The owner plans to move admin `sshd`
  to its own address, for example IPv6 only, through a second IPv4 address on the VPS. That
  change is the owner's, made outside this document. Nothing here assumes how admin SSH is
  reached.
- **Out of scope: the web client.** It already needs no port, because it lives behind
  HTTPS.
- **Not changed.** Each instance's public SSH port (6789, 6792) keeps working as it does
  today. Q-29-4 asks whether those ports stay open.

## 2. Why the doorway terminates SSH

The SSH username is sent in `SSH_MSG_USERAUTH_REQUEST`, after key exchange, inside the
encrypted transport. A proxy cannot read the username and then splice the raw TCP stream to
a backend, because the stream is already bound to session keys that only the proxy holds.
sshdoorway is therefore a full SSH server facing the player and a full SSH client facing the
instance. One player is two SSH sessions. Four constraints follow, and the rest of this
document exists to meet them.

**C-1. Public keys cannot be passed through.** A publickey signature covers the session
identifier, the exchange hash of the client↔doorway session. The doorway↔instance session
has a different identifier, so a replayed signature fails upstream. The doorway can
**verify** that the client holds the key, but only the instance knows whether that key is
registered to the account. Today that check is `SshHooks::auth_account_key` in
`mush-net-ssh/src/server.rs` (`auth_publickey`).

**C-2. The instance must see the player's real address.** Every abuse gate in
`mush-net-ssh` is per IP: the per-IP connection cap (`connection_opened`), `is_ip_blocked`,
the five-failure password tempban, and the `block_reason` notice. Behind a plain proxy
every player would be `127.0.0.1`. One player's five mistyped passwords would tempban the
whole player base, and the connection cap would count the whole player base as one address.

**C-3. Nobody but the doorway may claim an address or a key.** Whatever carries C-1 and
C-2 must be unforgeable by a player and by other local processes.

**C-4. The doorway must preserve backpressure.** The instance detects a wedged client (for
example `^S` flow control) by a write timeout (`WRITE_TIMEOUT`, design 02 §8.4). If the
doorway buffered without bound, the instance would never see the wedge, and the doorway
would hold the memory.

## 3. Username grammar and routing

The doorway splits the username at the **first** `+` only:

| Username | First segment | Routed to | Upstream username |
|---|---|---|---|
| `dev1` | service `dev1` | dev1 | `play` (the front door) |
| `dev1+faenix` | service `dev1` | dev1 | `faenix` |
| `dev1+faenix+Kira` | service `dev1` | dev1 | `faenix+Kira` |
| `dev1+play` | service `dev1` | dev1 | `play` |
| `DEV1+faenix` | service `dev1` (case-insensitive) | dev1 | `faenix` |
| `nick`, `root`, `ubuntu`, `faenix` | not a service | the menu (§7) | `play` on the chosen service |
| `nick+foo` | not a service | the menu (§7) | `play` on the chosen service |

- **Service names are matched ASCII case-insensitively.** The upstream remainder is
  forwarded byte for byte; the instance's own `split_account_user` parses it as today.
- **A bare service name is the front door.** It maps to the instance's existing `play`
  username, which lands on the welcome screen (`connect`/`create`). Q-29-1 asks whether a
  bare service name should instead try key lookup.
- **An unknown first segment goes to the menu**, because SSH authentication happens before
  any channel exists. Once the doorway has accepted a connection in order to show a menu,
  that connection cannot then authenticate as an account. So the menu always routes to a
  front door, and it tells the player the direct username for next time (§7). Q-29-3 asks
  whether a default service should replace the menu for bare names.
- **The routing table is config, reloaded on `SIGHUP`** (§9.5). Adding a dev3 never
  restarts the doorway.

## 4. The trusted ingress (instance side)

Each instance's gateway gains a second SSH listener: a **Unix socket**, the
**trusted ingress**. Only sshdoorway can reach it. It runs the same `ConnHandler`, with the
same hooks, auth rules, welcome screen and TUI. The one difference is that the peer address
and an attested key arrive in a **preamble** written before the SSH bytes. The public TCP
listener is unchanged and never accepts a preamble.

### 4.1 The socket and its permissions

- **Path.** `/run/rustmush-ssh/<instance>.sock`, set by a new optional key in `[ssh]`:
  `trusted_socket = "/run/rustmush-ssh/dev1.sock"`. If the key is unset, there is no
  trusted ingress. All-in-one `mushd` local testing leaves it unset, so nothing changes
  there.
- **A separate directory from the IPC socket.** The gateway↔server IPC socket lives in
  `/run/rustmush` (mode 0750, group `mush`). It is unauthenticated: anyone who can connect
  to it can impersonate the gateway (deploy/dev1/instance.env). The doorway must **not** be
  in group `mush`. A tmpfiles.d entry creates `d /run/rustmush-ssh 0750 mush sshdoorway`.
  The gateway creates each socket with mode 0660 and group `sshdoorway`. Group membership
  then grants the SSH ingress and nothing else (C-3).
- **The socket path is the trust.** The doorway does not verify the instance's host key on
  this hop. It logs the key's fingerprint once per connection. The listener is local and
  guarded by permissions, and pinning keys would couple doorway config to every instance's
  key rotation.

### 4.2 The preamble protocol

The preamble is a short request/reply exchange on the Unix stream before SSH begins. It uses
the framing `mush-ipc` already uses: length-prefixed, self-describing MessagePack. The types
live in a new crate, `mush-sshdoorway-proto` (serde and rmp only, no russh), which both
`mush-net-ssh` and `mush-sshdoorway` depend on. Every message carries `version: u16`. A
version mismatch is refused, and the doorway reports it to the player as "service
unavailable".

| # | Doorway → instance | Instance → doorway | Instance action |
|---|---|---|---|
| 1 | `Hello { peer: SocketAddr, local: SocketAddr, conn: u64 }` | `Open` / `Blocked { notice }` / `Refused` | `connection_opened(peer.ip())` claims the per-IP slot for the **real** address (C-2). `Blocked` carries `block_notice(block_reason(peer))`. `Refused` = over the per-IP cap. |
| 2 | `KeyQuery { user, key_openssh }` (zero or more) | `KeyReply { acceptable: bool }` | `auth_account_key(account, key)`. **No failure is counted**, matching today's `auth_publickey_offered` (the QUERY phase). |
| 3 | `Begin { user, attested_key: Option<String> }` | `Ready` / `BadCreds` / `Banned` | With a key: `auth_account_key` again (the key may have been deleted since step 2). `Ready` binds `(user, key)` to the connection. Without a key: binds `user` only. A rejected `Begin` leaves the preamble open for another `Begin`. |
| — | after `Ready`: SSH bytes | | `russh::server::run_stream` on the rest of the stream, with `ConnHandler.addr = peer.ip()` |

After `Ready`, the instance's handler behaves as it does today, with two differences:

- **`auth_none(user)` is accepted for an attested key.** When `Begin` bound an attested
  key, `auth_none` for exactly that `user` is accepted with `AuthMethod::PublicKey` and
  `Route::Account`, the same values `auth_publickey` sets today. The `mind push|pull|check`
  exec gate (`auth_method == PublicKey`) therefore keeps working through the doorway. Any
  other username is rejected.
- **Without an attested key, auth is unchanged.** `auth_password`, `auth_keyboard_interactive`
  and front-door `auth_none` run exactly as today, against the real address. Password
  failures are counted and tempban the real address (C-2).

The preamble is bounded. It must reach `Ready` within `PREAUTH_TIMEOUT`, allows at most 8
`KeyQuery` and 4 `Begin` messages, and caps each frame at 16 KiB. Breaking any limit closes
the stream.

### 4.3 Why a preamble and not something in-band

Other designs were considered and rejected (§15). The preamble keeps two promises. First,
**passwords never cross it**: they travel inside the upstream SSH auth, so the instance's
password path and its failure counting are untouched. Second, the **authorization decision
stays in the instance**: the doorway attests only what it verified, which is the client's
address and its possession of a key. It holds no account data, no Redis connection and no
game state.

## 5. Authentication through the doorway

The doorway opens the upstream connection and sends `Hello` when it first learns the
username, at the client's first `USERAUTH_REQUEST`.

| Client does | Doorway, downstream | Doorway, upstream |
|---|---|---|
| `none` as `dev1` or `dev1+play` | accept | `Begin { user: "play", None }`, then SSH `none` as `play` |
| `none` as `dev1+faenix` | reject, offering publickey, password and keyboard-interactive (as the instance does) | nothing yet |
| publickey **offer** (no signature) as `dev1+faenix` | `KeyQuery` → accept the offer if `acceptable`, else reject toward interactive | `KeyQuery { "faenix", K }` |
| publickey **signed**, verified by russh | `Begin { "faenix", Some(K) }` → `Ready`: accept; `BadCreds`: reject toward interactive; `Banned`: reject | then SSH `none` as `faenix` |
| `password` as `dev1+faenix` | relay the result of the upstream attempt | `Begin { "faenix", None }` on the first attempt, then SSH `password` per attempt |
| `keyboard-interactive` as `dev1+faenix` | the doorway presents the same single `Password:` prompt the instance uses | relayed upstream as SSH `password` (the instance treats both methods the same way, through `verify_account`) |
| anything as an unknown name | `none`: accept (menu); `password`/`publickey`: reject with no further methods | nothing until the menu choice |

Further rules:

- **Order.** OpenSSH tries publickey before password, so the sequence above fits it.
  Once a password `Begin` has switched the stream to SSH, the preamble is spent. A key
  offered after that is rejected toward interactive. This is an accepted edge case.
- **The doorway never stores, logs or retries a password.** It only forwards each attempt
  once.
- **A backend `Blocked` reply is accept-and-tell.** SSH's banner goes out at service accept,
  before the username, and so before the doorway knows which instance's block list applies.
  So on `Blocked`, the doorway **accepts** the downstream auth. It opens no upstream session.
  On the shell request it prints the instance's notice and closes. A blocked player reads
  why, instead of being met with a prompt that can never succeed (the same intent as today's
  banner). The doorway's own blocks (§8) are not per-instance, so they **do** use the banner.
- **Host key.** The doorway has its own ed25519 host key, at a fixed path, loaded through the
  same `load_or_generate` the instances use. Every doorway generation (§9) loads the same
  file, so restarting the doorway never changes what players see in `known_hosts`. Players
  who already connect to `conclav.io:6789` keep a separate `known_hosts` entry for that port.

## 6. The session splice

After auth, the doorway relays SSH's **session-channel vocabulary** verbatim and refuses
everything else at the doorway. The rule is to forward and let the instance decide, so a new
instance feature (another env variable, another exec verb) never needs a doorway release.

| Relayed downstream → upstream | Relayed upstream → downstream | Refused at the doorway |
|---|---|---|
| `channel_open_session` (at most 2 open: the game and `sftp` for the asset pack, doc 21 G.11; the instance accepts or rejects) | channel open confirmation / failure | `direct-tcpip`, `tcpip-forward`, `x11-req`, `auth-agent-req@openssh.com`, any other channel type |
| `pty-req` with term, cols, rows, **pixel width/height** and modes (the cockpit's cell size comes from these) | `want_reply` success / failure for every request | global requests other than keepalive |
| `env` verbatim (at most 32 variables, 4 KiB in total) | | |
| `window-change` including pixels | | |
| `shell`, `exec` (verbatim; the instance gates `mind …`), `subsystem` (verbatim; the instance serves `sftp`) | | |
| `data`, `extended_data`, `eof`, `close`, `signal`, `break` | `data`, `extended_data`, `eof`, `close`, `exit-status`, `exit-signal` | |
| | `disconnect` with its reason (for example "login timeout", `@boot`) | |

- **Backpressure (C-4).** Each direction is one bounded relay. The doorway reads the next
  upstream chunk only after the previous one was accepted by the downstream channel's window.
  If a player's client stops reading, the downstream window fills, the doorway stops reading
  upstream, the upstream window fills, and the instance's write times out as it does today.
  The doorway buffers at most one channel window per direction.
- **`want_reply` is honoured end to end.** The downstream reply to `pty-req`, `shell` and so
  on is the upstream reply. The doorway awaits it inside the handler callback.
- **Keepalive and liveness.** Downstream keepalive uses the instance's settings (30 s
  interval, 3 missed, no inactivity timeout: idlers are furniture, design 02 §5.6). When
  either side closes, the doorway closes the other at once, so the instance sees a dead link
  exactly when a direct connection would have.
- **When the upstream connection drops, the player is told.** If the gateway restarts, the
  player sees `"<service> closed the connection; reconnect with ssh <user>@<host>"` before
  the doorway disconnects. A mush-server hot restart is invisible, because the gateway holds
  the stream.
- **Cost.** Encryption runs twice per byte, the second time over a Unix socket. The heaviest
  stream, the kitty cockpit, measured about 27 KB/s per moving player (PR #154), which is
  negligible.

## 7. The menu

The menu is a short line-mode screen, rendered by the doorway for connections whose first
segment is not a service.

```
conclav.io — choose a game server

  1  dev1   Development server 1
  2  dev2   Development server 2

Type a number or a name (q to quit):
```

- **It needs a PTY and a shell request.** `exec`, `subsystem` and no-PTY shells get
  "sshdoorway needs an interactive terminal" and a close, worded like the instance's own
  message. Scanners that "log in" as `root` with `none` get a menu or nothing, never a
  command.
- **On a choice,** the doorway sends `Hello` and `Begin { user: "play" }`, splices into that
  service's welcome screen, and first prints the hint
  `Next time: ssh dev1+<account>@conclav.io`.
- **Only listed services appear.** Each service has a `listed` flag (default `true`). An
  unlisted service is reachable by name but not shown (Q-29-6).
- **It times out.** The menu closes after 120 s without input, before any upstream is
  opened.
- **Width.** The menu is a few short lines and needs no layout. The 120-column floor for
  client surfaces does not apply to it.

## 8. Abuse handling at the doorway

Port 22 on a public address receives every scanner on the internet. The doorway's gates are
**its own**, separate from each instance's gates, which run on the real address through
`Hello`.

- **Before any upstream work:** a per-IP concurrent connection cap (default 8), a per-IP
  new-connection rate (default 20 a minute), a global connection cap (default 2048), and an
  auth deadline (default 30 s to route or reach the menu).
- **Unknown names get `none` or nothing.** `password` and `publickey` attempts for names
  that are not services are rejected with no further methods. OpenSSH's `none` probe still
  reaches the menu. Password-spraying bots meet a door with no password prompt.
- **The doorway's own blocks** (cap or rate exceeded) use the SSH banner, since they do not
  depend on the username.
- **Logging is rate-limited.** One line per routed connection: peer, first segment,
  service, auth method, duration, bytes each way. Unrouted rejections are counted, not
  logged one by one; a summary line per minute gives the counts by reason. Passwords and
  upstream remainders are never logged.

### 8.1 Log rotation and compression (owner requirement)

Port 22 traffic can grow without limit, so the doorway's logs ship with aggressive rotation
and compression, and **checking them is a ship-time gate** (DW.4). The instances work
differently: their daily file logs are kept forever (`mush-config/src/logging.rs`), with
pruning left to ops. The doorway does not inherit that.

- **Two sinks.**
  - **File.** `info` and above goes to `/var/log/sshdoorway/<generation>.log`, for example
    `a.log`. The name is stable, so logrotate can manage it.
  - **journald.** Only `warn` and above goes to journald, so `systemctl status` shows
    trouble without the connection lines flooding the box's journal.
- **Writes never block.** The file sink uses the same lossy non-blocking writer the
  instances use: under extreme pressure it drops lines instead of stalling connections.
- **The file is reopened on `SIGUSR2`** (§9.3), so logrotate can rename it without
  `copytruncate` losing lines.
- **logrotate config.** `/etc/sshdoorway/logrotate.conf` uses its own state file, so it does
  not depend on the box's daily system run:

  ```
  /var/log/sshdoorway/*.log {
      daily
      maxsize 20M
      rotate 60
      maxage 14
      compress
      nodelaycompress
      compresscmd /usr/bin/zstd
      compressoptions -10 -T0
      compressext .zst
      uncompresscmd /usr/bin/unzstd
      missingok
      notifempty
      sharedscripts
      postrotate
          systemctl kill --signal=USR2 'sshdoorway@*' >/dev/null 2>&1 || true
      endscript
  }
  ```

  The ceiling is at most 60 compressed archives and 14 days, plus at most 20 MB live per
  generation.
- **It runs every 15 minutes.** `sshdoorway-logrotate.timer` runs
  `logrotate -s /var/lib/sshdoorway/logrotate.status /etc/sshdoorway/logrotate.conf` with
  `OnCalendar=*:0/15`, so the 20 MB cap holds within a quarter hour, not a day.
- **journald rate limit.** The unit sets `LogRateLimitIntervalSec=30s` and
  `LogRateLimitBurst=200`.
- **The ship-time check.** `deploy/sshdoorway/check-logging.sh` must pass before the
  doorway is enabled on a box, and again after every swap. It verifies that:
  - `logrotate -d` parses the config cleanly;
  - the timer is enabled and has fired;
  - a forced rotation (`logrotate -f`) leaves a `.zst` archive that `unzstd -t` accepts;
  - the live file was reopened, by checking that it is non-empty after one test connection;
  - `zstd` is installed;
  - `systemctl show` reports the journald rate limits;
  - the log directory is under its budget (default 300 MB).

  The check fails loudly; it never warns and continues.

## 9. Graceful restart: drain

### 9.1 Why drain

An established SSH connection cannot be handed to a new process by passing its file
descriptor. The transport's state lives in the old process's memory: session keys, cipher
and MAC state, sequence numbers, rekey progress, channel windows. Serializing that out of
russh is not supported and not worth building. The **listening** socket carries no SSH state,
so the restart moves only new connections to the new process. The old process keeps serving
the sessions it already has until they end.

### 9.2 Mechanism

- **Two alternating units,** `sshdoorway@a` and `sshdoorway@b`, both bind every configured
  address with `SO_REUSEPORT`. The kernel spreads new connections across the bound group.
  IPv6 listeners set `IPV6_V6ONLY`.
- **`net.ipv4.tcp_migrate_req = 1`** (Linux 5.14+; the sysctl covers IPv6 too). Without it,
  closing a listener in a reuseport group resets every connection still waiting in **that**
  listener's accept queue. With it, the kernel migrates them to another listener in the
  group. A `sysctl.d` file ships with the units. At startup the doorway reads
  `/proc/sys/net/ipv4/tcp_migrate_req` and logs a warning if it is 0.
- **Readiness.** The new instance signals `READY=1` (`Type=notify`) only after it has bound
  every address, parsed the routing table, and completed a `Hello` round trip with every
  service's trusted socket. The repository has no sd_notify helper yet, so this is a short
  write to `$NOTIFY_SOCKET`.

### 9.3 Signals

| Signal | Effect |
|---|---|
| `SIGHUP` | Reload the routing table and limits. New connections use the new table; live sessions keep their route. Listen addresses and the host key path do not reload, because changing them needs a swap. |
| `SIGTERM` | **Drain.** Close every listener (the kernel migrates its queue, §9.2), keep every session, and exit 0 when the last one ends or when the drain deadline passes. |
| `SIGUSR1` | **Finish now.** Tell every session (§9.4), wait 30 s, close everything, exit 0. |
| `SIGUSR2` | Reopen the log file (logrotate's `postrotate`, §8.1). |
| second `SIGTERM` | The same as `SIGUSR1`. |

### 9.4 The swap and its limits

`deploy/sshdoorway/swap.sh` does the swap:

1. It finds the active unit, say `a`. If `b` is also active, `b` is still draining, and the
   script refuses unless given `--force`, which sends `SIGUSR1` to `b` and waits for it to
   exit. **At most two generations ever run.**
2. `systemctl start sshdoorway@b` returns once `b` is `READY`.
3. `systemctl kill --signal=TERM sshdoorway@a`. This uses `kill`, not `stop`, so the script
   does not block for hours while `a` drains. The units use `Restart=on-failure`, so a
   drained exit 0 is not restarted.

- **Drain deadline and warnings (Q-29-2, ruled).** The deadline is 24 h after the drain
  begins. Warnings start early, so a player can reconnect at a convenient moment instead of
  being cut off mid-scene.
  - **Schedule.** A session still on the draining generation gets a warning when **12 h,
    6 h, 2 h, 1 h, 15 m and 1 m** remain, that is, 12 h, 18 h, 22 h, 23 h, 23 h 45 m and
    23 h 59 m after the drain began.
  - **Wording.** Each warning is a line injected into the session's downstream channel:

    ```
    ATTENTION: Game Doorway updated and requires a restart, please find a safe place and reconnect within the next 12h.
    ```

    The last words change with the time left: `12h`, `6h`, `2h`, `1h`, `15m`, `1m`.
  - **Config.** Both the schedule and the deadline are config:
    `drain_deadline_secs = 86400` and `drain_warnings = ["12h", "6h", "2h", "1h", "15m", "1m"]`
    (time remaining), with the message template
    `ATTENTION: Game Doorway updated and requires a restart, please find a safe place and reconnect within the next {remaining}.`
    Tests use a short deadline and short warnings.
  - **At the deadline,** the session closes.
  - **Forced swaps.** A security fix uses `--force`. It does not wait the deadline out, and
    sends one immediate warning with `{remaining}` = `30s`.

  Players idle for days are expected, and the 12 h warning reaches them on their next look
  at the screen.
- **Injected lines go between frames.** A doorway-injected line enters the downstream
  data stream only between upstream chunks, so it never lands inside a kitty graphics
  escape. In TUI or cockpit mode the line may briefly mark the screen; the next repaint
  clears it.
- **`TimeoutStopSec`** is the drain deadline plus 5 minutes, so a plain `systemctl stop`
  during a box shutdown cannot block forever.

### 9.5 Rarely needing a restart at all

The best defence is rarely restarting. sshdoorway holds no game logic, no Redis and no
account data, and its routing reloads on `SIGHUP`. Its splice relays a fixed SSH vocabulary
verbatim (§6), so instance features never need a doorway change. Its dependency surface is
russh, tokio, serde and the proto crate. A doorway release is expected for russh security
fixes and changes to the doorway itself, not for game work.

## 10. Configuration and deployment

`/etc/rustmush/sshdoorway.toml`:

```toml
[listen]
# Explicit addresses only; never 0.0.0.0 or [::]. Port 22 normally.
addrs = ["203.0.113.10:22", "[2001:db8::10]:22"]
host_key = "/var/lib/sshdoorway/host_ed25519"

[limits]
per_ip_connections = 8
per_ip_new_per_minute = 20
global_connections = 2048
auth_deadline_secs = 30
menu_idle_secs = 120
drain_deadline_secs = 86400
drain_warnings = ["12h", "6h", "2h", "1h", "15m", "1m"]   # time remaining

[log]
file_dir = "/var/log/sshdoorway"   # <generation>.log, reopened on SIGUSR2
file_level = "info"
journal_level = "warn"

[menu]
title = "conclav.io — choose a game server"
# default_service = "dev1"   # Q-29-3: unset means bare names get the menu

[[service]]
name = "dev1"
title = "Development server 1"
socket = "/run/rustmush-ssh/dev1.sock"
listed = true

[[service]]
name = "dev2"
title = "Development server 2"
socket = "/run/rustmush-ssh/dev2.sock"
listed = true
```

- **Service names** are lowercase ASCII `[a-z0-9-]`, unique, and must not be `play`,
  `guest` or `new`, the instances' front-door names. The config is rejected at load (and on
  `SIGHUP`, keeping the old table) if any rule fails.
- **The user.** sshdoorway runs as user `sshdoorway`, in group `sshdoorway` and **not** in
  group `mush` (§4.1).
- **Port 22 without root.** The unit grants `AmbientCapabilities=CAP_NET_BIND_SERVICE` and
  sets `NoNewPrivileges=yes`. Systemd socket activation is not used, because the two-unit
  reuseport swap needs each generation to bind its own listener.
- **Deploy files.** `deploy/sshdoorway/` holds `sshdoorway@.service`, `sshdoorway.toml`
  (example), `99-sshdoorway.conf` (sysctl), `sshdoorway.conf` (tmpfiles, which also creates
  `/var/log/sshdoorway` with mode 0750 owned by `sshdoorway`), `logrotate.conf`,
  `sshdoorway-logrotate.service` and `.timer`, `check-logging.sh`, `swap.sh` and a README. Each instance's gateway unit gains `SupplementaryGroups=sshdoorway`, so its socket
  can carry that group.
- **Linux only.** The doorway and the trusted ingress are Linux-only (`cfg(unix)`: Unix
  sockets, `SO_REUSEPORT`, sysctls). Their tests are skipped on Windows and run on Linux CI
  and the VPS gate.
- **Not deployed until the owner says so.** No instance is redeployed for this without the
  owner's go-ahead (standing rule). The address change on the VPS is the owner's step (§1.1).

## 11. Crates

- **`mush-sshdoorway-proto`.** The preamble types and framing (§4.2). It depends on serde,
  rmp-serde and the same codec idiom as `mush-ipc`. It has no russh dependency.
- **`mush-sshdoorway`.** The binary `sshdoorway`: listener, routing, auth relay, splice,
  menu, limits, drain. It depends on russh (client and server), tokio and the proto crate.
  It does not depend on `mush-net-ssh`, which carries the game's rendering stack.
- **`mush-net-ssh` changes.** `SshServer::bind_trusted(path, hooks)`, which runs the
  preamble and then `run_stream`, plus the attested `auth_none` route. It reuses
  `ConnHandler` unchanged otherwise. Its russh types stay quarantined in the crate (treaty
  risk 4).
- **`mush-config` and `mush-gateway`.** The `[ssh] trusted_socket` key, and binding it next
  to `listen`.

New crates join the contract tests' feature census and panic budget (run before pushing).

## 12. Decisions

| Id | Decision |
|---|---|
| D-29-1 | A separate daemon, binary `sshdoorway` (crate `mush-sshdoorway`), one per box, in front of every instance on that box. The name avoids confusion with the unrelated sshdoor project and with "backdoor". |
| D-29-2 | The doorway terminates SSH: one player is one downstream and one upstream session (§2). |
| D-29-3 | It binds port 22 on a configured list of IPv4/IPv6 addresses, never a wildcard. How admin SSH is reached is the owner's host-networking change, outside this design. |
| D-29-4 | Routing is on the first `+` segment. The remainder is forwarded byte for byte. A bare service name means the front door `play`. An unknown first segment gets the menu (§3). |
| D-29-5 | Upstream runs over a per-instance trusted Unix socket on the gateway. It lives in its own directory and group, separate from the IPC socket. The public TCP port is unchanged (§4.1). |
| D-29-6 | A versioned preamble (`Hello` / `KeyQuery` / `Begin`) precedes the SSH bytes. Its peer address is authoritative for every per-IP gate in the instance (§4.2). |
| D-29-7 | Public keys: the doorway proves possession and the instance authorizes. An attested key permits upstream `none` for exactly that user, with `AuthMethod::PublicKey` (§4.2, §5). |
| D-29-8 | Passwords and keyboard-interactive are relayed as upstream SSH `password`. The instance counts failures against the real address. The doorway never stores or logs a password (§5). |
| D-29-9 | The doorway has its own host key, shared by every generation. The trusted hop's host key is not verified; socket permissions are the trust (§4.1, §5). |
| D-29-10 | The session-channel vocabulary is relayed verbatim, at most 2 channels, and the instance decides. Forwarding, agent, X11 and other channel types are refused at the doorway (§6). |
| D-29-11 | The splice is bounded to at most one channel window per direction, so the instance's wedge detection still works (§6). |
| D-29-12 | The menu needs a PTY shell, routes to a front door, and prints the direct username for next time (§7). |
| D-29-13 | A per-instance block is accept-and-tell in the session. The doorway's own blocks use the SSH banner (§5, §8). |
| D-29-14 | The doorway has its own per-IP, rate, global and deadline gates. Non-service names get `none` only (§8). |
| D-29-15 | Graceful restart is a drain: `SO_REUSEPORT`, `tcp_migrate_req = 1`, two alternating units, at most two generations, `SIGTERM` drains, a deadline with in-session notices, `SIGHUP` reloads (§9). |
| D-29-16 | The doorway holds no game state, account data or Redis connection (§4.3, §9.5). |
| D-29-17 | The doorway and the trusted ingress are Linux-only; their tests are `cfg(unix)` (§10). |
| D-29-18 | The doorway's logs rotate and compress aggressively: a stable file name per generation reopened on `SIGUSR2`, logrotate every 15 minutes with `maxsize 20M`, zstd, 60 archives and 14 days; journald gets `warn` and above only, rate-limited. `check-logging.sh` is a ship-time gate (§8.1). |

## 13. Owner rulings (2026-10-30)

The owner ruled all six questions on 2026-10-30. Five were accepted at their defaults, and
Q-29-2 with an amended warning schedule.

| Id | Question | Ruling |
|---|---|---|
| Q-29-1 | Should a bare `dev1@` with a registered key log straight into the key's account, by key lookup, instead of reaching the welcome screen? It needs a key→account index, and a key may be registered to more than one account. | **Default accepted.** No. A bare name is the front door; use `dev1+account`. |
| Q-29-2 | How long should the drain deadline be, and when should the warnings go out? | **Amended.** 24 h deadline. Warnings at 12 h, 6 h, 2 h, 1 h, 15 m and 1 m remaining, with the owner's wording (§9.4). |
| Q-29-3 | Should a non-service name (`faenix@conclav.io`) route to a default service as an account name, instead of showing the menu? This only makes sense once one production game exists. | **Default accepted.** No default service while there is no production game; the config key exists but is unset. |
| Q-29-4 | Should the instances' direct public ports (6789, 6792) stay open once the doorway is live? Bots, scripts and w11term profiles use them today. | **Default accepted.** Keep them open; revisit at launch. |
| Q-29-5 | Should the doorway's host-key fingerprint be published (website, MOTD, docs) for players who verify? | **Default accepted.** Yes, in the deploy README and the welcome banner. |
| Q-29-6 | Should unlisted services exist (reachable by name, hidden from the menu), for example for staging? | **Default accepted.** Yes, as a per-service `listed` flag, default `true`. |

## 14. Cuts

| Cut | Content | Done when |
|---|---|---|
| DW.0 | `mush-sshdoorway-proto`; `mush-net-ssh` `bind_trusted` with the preamble, the real-address `ConnHandler` and the attested `auth_none`; `[ssh] trusted_socket` in config and gateway. | A test client (russh client over a `UnixStream`, preamble first) gets: per-IP cap and tempban on the **asserted** address; `KeyQuery` with no failure counted; attested `Begin`, then `none`, into the account with `mind check` exec allowed; password failures tempbanning the asserted address and not another; the public listener still rejecting a preamble. |
| DW.1 | `mush-sshdoorway`: listener, grammar, auth relay (§5), splice (§6), host key, config load. | An end-to-end test (gateway + doorway, both in-process on Linux) passes: `svc`, `svc+acct` by password and by key, `svc+acct+char`, keyboard-interactive, `mind push` exec, `sftp` asset subsystem, `window-change` with pixels reaching caps, `env COLORTERM`, a disconnect reason relayed, forwarding refused, and a client that stops reading wedging the instance's write timeout (backpressure). |
| DW.2 | Menu, accept-and-tell blocks, the doorway's own limits and banner, rate-limited logging with the two sinks (§8.1). | A test covers: menu choice to the front door with the hint printed; exec as `root` refused; no-PTY refused; menu idle timeout; a per-instance block told in-session; the per-IP cap and rate refused with a banner; `info` reaching only the file and `warn` reaching both sinks; `SIGUSR2` reopening a renamed file. |
| DW.3 | Drain: `SO_REUSEPORT`, `V6ONLY`, signals (§9.3), readiness via `NOTIFY_SOCKET`, `SIGHUP` reload, the deadline and the warning schedule (§9.4), `deploy/sshdoorway/*` including the logrotate config, timer and `check-logging.sh`, and `swap.sh`. | A Linux test with two generations holds sessions open on generation A while it drains (short deadline and scaled warnings), routes new connections to generation B, reconnects nothing, delivers every warning in order with the owner's wording and the right `{remaining}`, and exits A at the deadline or at zero sessions. A queue-migration check runs under `tcp_migrate_req = 1`. A refused third generation and `--force` (one 30 s warning) both work. `check-logging.sh` passes on the VPS gate box. |
| DW.4 | conclav.io rollout, once the owner has moved admin SSH and assigned the addresses. Install the units, sysctl, tmpfiles, logrotate config and timer, and config; set gateway `trusted_socket` for dev1 and dev2. | **Ship-time gate: `check-logging.sh` passes on conclav.io before the doorway is enabled** (§8.1). Then a smoke passes with OpenSSH and w11term: `dev1+acct` by key, `dev2+acct+char`, the menu as `$USER`, the cockpit, and a live `swap.sh` while a session plays on, with `check-logging.sh` passing again after the swap. The deploy plan memory and the runbooks are updated. Only on the owner's go-ahead. |

DW.0 and DW.1 can be built in parallel once the proto crate's types are fixed. Their files do
not overlap.

## 15. Rejected alternatives

- **A transparent TCP splice after reading the username.** Impossible: the username is
  encrypted (§2).
- **Passing established connection fds to a new process on restart.** The transport state is
  in process memory (§9.1).
- **PROXY protocol v2 on a localhost TCP port.** It would carry the address, but a localhost
  port is reachable by every local process, the same reason the IPC boundary is a Unix
  socket. It also has no request/reply step for `KeyQuery`.
- **Encoding the attested key in the upstream username** (for example `faenix#fp=...`). It
  is forgeable on any listener that forgets to check where it came from, and it would show up
  in every log line that records a username.
- **The doorway authenticates accounts itself through an auth RPC to the gateway.** This
  would move the authorization decision and its failure counting out of the instance, making
  a second auth surface to keep correct. The preamble asks the instance only what SSH's own
  query phase asks.
- **Terminating SSH and speaking a non-SSH protocol upstream.** This would mean
  reimplementing everything the instance's SSH session layer does (PTY caps, env, the TUI,
  exec, sftp). Running SSH over the trusted socket reuses all of it.
- **One process with systemd socket activation.** That gives no second generation to drain
  into; the nginx-style listener handoff it would need is more machinery than two reuseport
  units.
