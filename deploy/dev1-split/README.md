# dev1 — the SPLIT (hot-restart) deployment

This is the **hot-restartable** way to run the dev1 instance (M16c/M16d). It runs
the forked pair instead of the all-in-one `mushd`, so a deploy can rebuild and
restart the **game server** while the **player connections stay up** — players
see a brief "restarting…" then "Reconnected" and a fresh room render, mid-scene,
never a disconnect. As of **M16d this holds for WEB (WebSocket) players too**,
not just SSH: the gateway now serves the web client (HTTPS :6790) itself. As of
**doc 33 GG.4 the gateway itself moves without a drop too**: it runs as one of
two *generations* (`dev1mush-gateway@a` / `@b`), and a gateway change is a
*swap* — the idle generation starts on the same ports, the serving one drains
its sessions with warnings and exits when they are gone (see *Gateway
generations* below).

It is a **drop-in alternative to `deploy/dev1`** for the *same* instance: same
Redis (6788), same `instance.env` and so the same rendered `mushd.toml`, same
SSH port (6789), same web port (6790), same data/TLS/env. You run **one or the
other**, not both (they share the SSH and web ports).

**The config is rendered, not hand-written** (16b cut F.5). `deploy/dev1` no
longer carries a `mushd.toml`; it carries
[`instance.env`](../dev1/instance.env), and both deploy scripts install a copy
of it at `/opt/rustmush/dev1/instance.env` with `EXTERNAL_IP` resolved, then
render `/opt/rustmush/dev1/mushd.toml` from
[`../instance/mushd.toml.tmpl`](../instance/mushd.toml.tmpl) with
[`../instance/render.sh`](../instance/render.sh). The render is preflighted with
`mush-gateway --check-config` **and** `mush-server --check-config` before this
script starts or restarts anything, and nothing is installed if either refuses —
so a bad config is a failed deploy, not a crash-looping unit.

## How it differs from the all-in-one

| | all-in-one (`deploy/dev1`) | split (this dir) |
|---|---|---|
| service(s) | `dev1mush.service` (mushd) | `dev1mush-server.service` (mush-server) + `dev1mush-gateway@a` / `@b` (mush-gateway, one template unit, ONE generation serving at a time) |
| processes | one (`mushd`) | two, each its own unit (doc 33 GG.1): the gateway dials the server over the `[ipc]` socket; during a swap, briefly three (doc 33 GG.4) |
| SSH :6789 | served by mushd | served by the gateway |
| **HTTPS :6790 (web client)** | **served by mushd** | **served by the gateway** (M16d) |
| deploy = restart | `systemctl restart dev1mush` — **drops connections** | server change: `systemctl restart dev1mush-server` — **holds connections (SSH + web)**; gateway change: `gateway-swap.sh dev1` — **drains, drops nobody** (doc 33 GG.4) |
| server crash | systemd restarts the whole process (players drop) | systemd restarts the server unit (`Restart=on-failure`); the gateway holds every connection and re-attaches (players held) |
| server stays down | — | past the unit's start limit (5 starts / 60 s) the gateway tells players the server is down and staff are paged, `/healthz` says `server_ok: false`; `systemctl reset-failed dev1mush-server && systemctl start dev1mush-server` heals it, nobody reconnects |

### Web (M16d): now served + hot-restart-surviving

The gateway serves the **web client (HTTPS :6790)** itself — the axum web ingress
is wired into `mush-gateway::serve` on the SAME session layer as SSH. A WS player
authenticates with a token, gets the same `hello` manifest the all-in-one
advertises (who / channels / mail / `<ruleset>.sheet`), and plays over the split
boundary. Crucially, a **WS connection survives a server restart exactly like an
SSH one**: it is never sent a Close frame — it receives "restarting" →
"Reconnected" → a fresh room render as ordinary envelopes, with the world intact
from Redis, and keeps playing (proven by
`crates/mush-gateway/tests/split_halves/web_restart_survival.rs`). So switching dev1 to the
split no longer takes the web client offline, and web players get the same
connection-holding hot-restart payoff SSH players do.

The gateway reads the SAME `[web]` block (listen + `[web.tls]`) from `mushd.toml`
that the all-in-one uses, so **the gateway must be built with `--features tls`**
to serve HTTPS from the on-disk cert/key (see the build steps below).

## First switch to the split (from the dev box, over SSH)

Assumes `deploy/dev1` has already been deployed once (redis, tree, env, cert
exist). Ship the current tree + build BOTH binaries + run this deploy:

```bash
VPS=root@conclav.io ; KEY=./hostinger
# ship the committed source (adds this dir + dev1/instance.env + the template)
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
# build the WEB CLIENT FIRST. `web/dist` is git-ignored, so the archive above
# never carries it, and `crates/mush-net-web/build.rs` embeds whatever is on
# disk — a tree with last deploy's bundle builds clean and serves the OLD
# client. This needs Node 22 on the box; if it is not there, build the bundle
# on the dev box and ship it instead (see "Building the SPA elsewhere" below).
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build/web && npm ci && npm run build'
# build the TWO split binaries (release). The gateway serves the web client
# (HTTPS :6790) now, so it needs --features tls to terminate TLS from the on-disk
# cert/key; the server is the game half (no web) and needs no tls.
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mush-gateway --features tls && ~/.cargo/bin/cargo build --release -p mush-server'
# switch dev1 to the split (stops the all-in-one, starts the server unit, then the gateway)
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/deploy/dev1-split/deploy.sh'
```

## Redeploy = HOT restart (the payoff)

```bash
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build/web && npm ci && npm run build'
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mush-gateway --features tls && ~/.cargo/bin/cargo build --release -p mush-server'
# restart the SERVER unit → the fresh binary comes up at the same socket and the
# gateway, untouched, re-attaches every SSH AND web connection. NOT
# `restart dev1mush-gateway` — that would drop them.
ssh -i $KEY $VPS 'systemctl restart dev1mush-server'
```

`deploy.sh` does the right restart on a re-run **by what changed** (doc 33
Q-33-6): it compares each freshly built binary with the one its unit is
running (`/proc/<MainPID>/exe`, by SHA-256; `deploy/drain/binary-changed.sh`)
and restarts the server unit only when the server binary differs, swaps the
gateway generation only when the gateway binary differs, and leaves alone the
half that did not move. `SERVER_RESTART=1` and `GATEWAY_SWAP=1` do each anyway.
So the plain re-run above is the deploy for every commit, whichever half it
touched. The gateway's `SIGHUP` is reserved (doc 33 §4.5): `systemctl reload`
is not a verb of this deployment, and the gateway unit has no `ExecReload=`.

### Which half moved: the server unit, or the gateway too

A server restart moves only the **server half**. The gateway process keeps
running the binary it was started with — so a commit that changed the GATEWAY
half (`web/`, `crates/mush-gateway`, `crates/mush-net-*`, `crates/mush-session`,
`crates/mush-ansi`, `crates/mush-proto`, `crates/mush-ipc`; `web/` because the
SPA is **embedded in the gateway binary**, not served from disk) needs the
gateway to move too. Since GG.4 that is a **swap**, not a restart, and it drops
nobody:

| the commit touched | what `deploy.sh` does |
|---|---|
| the gateway half (above) | `gateway-swap.sh dev1`: the idle generation starts on the same ports, the serving one drains — warnings at 12h/6h/2h/1h/15m/1m remaining, exit when empty or at 24 h — **connections held, then closed with notice** |
| the game (`mush-server`, `tacsim`, modules, content, config) | `systemctl restart dev1mush-server` — **connections held** |
| both (a wire change: `mush-proto`, `mush-ipc`) | both, server first; a change that bumps `IPC_VERSION` is **refused while a generation is still draining** (its players could not re-attach) unless `FORCE=1` finishes that generation first |

What still drops a connection is a `systemctl stop`/`restart` of the *serving*
generation by hand, which takes every in-progress session with it — the
attach, the scrollback ring, the resume point all live in the gateway's memory.
Neither that nor a swap signs anyone out: web tokens are Redis rows
(`token:{hash}`, 7-day sliding TTL), so an open tab reconnects by itself and is
back in the game without a password. Only the store losing the token asks for a
fresh sign-in: `scripts/reset-instance.sh` (which `FLUSHDB`s), `@newpassword`,
logout, or the TTL. Redis merely being slow or restarting is not one of those —
the gateway answers 503 ("cannot check right now"), which a tab retries with
its token kept — and neither is a server restart: a reconnect that lands while
the server half is down is answered 1012, not "session ended".

> **Each half stops alone** (doc 33 GG.1 + GG.2). `systemctl stop
> dev1mush-gateway@a` stops only that gateway generation: the server unit keeps
> running the world, that gateway's connections go link-dead in it, and the
> gateway that comes back dials in as a new link. `systemctl stop
> dev1mush-server` stops only the server: the gateway holds every connection
> through the gap.

## Gateway generations (doc 33 GG.4)

The gateway unit is a **template**, `dev1mush-gateway@.service`, with two
instances, `dev1mush-gateway@a` and `dev1mush-gateway@b` (`--generation %i`).
Exactly one is *enabled* and serving at any time; the other is idle, or
draining after a swap. Both share the template's `StateDirectory=`
(`/var/lib/dev1mush-gateway`): a draining generation leaves `draining-<gen>`
there, and the Discord bridge's lock (`discord.lock`) lives there so exactly one
generation runs the bot. The unit is `Type=notify`: a generation says `READY=1`
once its listeners are bound and its link to the server is up (or its boot gap
is serving). A generation that starts and finds its own `draining-<gen>` marker
— `Restart=on-failure` bringing a drained generation back beside the serving
one — says `READY=1`, logs that it is not serving, and exits 0; only the swap
script clears the marker.

**The ports.** Both generations bind the public SSH and web ports with
`SO_REUSEPORT`, so the new one can be listening before the old one stops. With
`net.ipv4.tcp_migrate_req = 1` (installed by `deploy.sh` as
`/etc/sysctl.d/99-rustmush-gateway.conf` when the doorway's own sysctl file is
not on the box) the kernel moves connections still queued on the closing
listener to the sibling instead of resetting them. The gateway warns at startup
when the sysctl is off.

**The trusted socket.** The doorway dials the configured
`[ssh].trusted_socket` (`/run/rustmush-ssh/dev1.sock`), which under the
generations is a **symlink** that no gateway binds: generation `a` binds
`/run/rustmush-ssh/dev1.a.sock` beside it, `b` binds `dev1.b.sock`, and the swap
script re-points the link (a fresh temporary link renamed over it, never a
moment without one) at the generation that just came up. The doorway's next
dial follows it. `mush-gateway --check-config` without `--generation` refuses a
`trusted_socket` that is a symlink, by design: the single-unit gateway must not
be started on this config.

**The signals** (`deploy/dev1-split/dev1mush-gateway@.service`):

| signal | sent by | what the generation does |
|---|---|---|
| `SIGUSR1` | `gateway-swap.sh` only | **drain**: close the TCP listeners (the sibling takes new connections), keep every session, warn each on the doorway's schedule (12h/6h/2h/1h/15m/1m remaining) as system lines on its own transport, close at the 24 h deadline or as soon as the last session is gone, exit 0, leave `draining-<gen>` |
| `SIGTERM` / `SIGINT` | `systemctl stop`, `FORCE=1` | **finish**: one warning, 30 s, close everything, exit 0; no marker |
| `SIGHUP` | — | reserved: logged, ignored |
| `SIGUSR2` | — | unassigned |

At the close, SSH sessions are told the reason and their channel ends; web
sessions get a `1012 SERVICE_RESTART` close frame with the reason, and the web
client reconnects by itself with its resume point through the same port, onto
the serving generation (a fresh attach, since the drained generation's ring is
gone), showing the reason only if that reconnect fails (Q-33-4). The Discord
bot stops in the draining generation at `SIGUSR1` and hands its follows over;
the new generation, blocked on the lock since it started, connects and
re-follows them.

**The swap** (`gateway-swap.sh dev1`, built on `deploy/drain/swap.sh`):

```bash
bash /opt/rustmush/dev1/build/deploy/dev1-split/gateway-swap.sh dev1          # what deploy.sh runs when the gateway binary changed
bash /opt/rustmush/dev1/build/deploy/dev1-split/gateway-swap.sh dev1 --force  # the idle generation is still draining: finish it first
```

Under one `flock`: find the serving generation (`ActiveState` plus marker);
refuse while the other generation is still draining, unless `--force` (which
finishes it: `SIGTERM`); run `mush-gateway --check-config --generation a` over
the installed config; clear the new generation's marker; start it and wait for
`READY=1`; confirm it stays `active` for 5 s — if not, stop it, `reset-failed`,
leave it disabled and keep serving on the old one; re-point the trusted link;
enable the new generation and disable the old; `systemctl kill --signal=USR1`
the old one. Exactly one generation is enabled when it is done.

**First install (cut GG.5, with the owner).** A box still serving from the
single `dev1mush-gateway.service` keeps doing so: `deploy.sh` installs the
template, says so, and starts no generation. The move is
`GATEWAY_MIGRATE=1 bash deploy.sh` — it stops the single unit (connections drop
this once), removes its unit file, starts `dev1mush-gateway@a`, enables it, and
creates the trusted link `dev1.sock -> dev1.a.sock`. It also installs the
sysctl. After that, every gateway change is a swap.

> **On a gateway older than 2026-09-20**, an open tab holding a token the store
> no longer knew could **tempban its own address for fifteen minutes** — and
> the tempban is per address, not per transport, so `ssh -p 6789 play@…` then
> answered a password prompt no password could satisfy. The fix: the refusal is
> now a WebSocket close the client acts on (it drops the token and shows the
> login form), refused tokens are counted apart from passwords and never ban,
> and a blocked address is told how many minutes are left. Stuck on an
> un-updated box: wait it out, or `systemctl restart dev1mush-gateway` — the
> tempban is gateway memory.

### Building the SPA elsewhere

`deploy.sh` refuses to start or restart anything when `web/dist/index.html` is
missing, or its `web/dist/.commit` differs from the source commit — `web/dist` is
git-ignored, so `git archive` never ships it and a stale bundle would otherwise
embed in silence. It does **not** run npm itself, so if the box has no Node 22,
build the bundle on the dev box and ship it:

```bash
npm --prefix web ci && npm --prefix web run build
tar -c -C web dist | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build/web'
```

`npm run build` stamps the full source commit in `web/dist/.commit` after Vite
succeeds. `git archive` expands the tracked `.source-commit` marker, so builds on
the VPS work without `.git`; local checkouts use `git rev-parse HEAD`. The deploy
compares these identities before changing a service or config. A rollback with a
newer old bundle, or a change outside `web/src`, is refused just like a missing
bundle. Archive the source and build the SPA from the same commit.
## Operate

```bash
systemctl status dev1mush-server dev1mush-gateway@a dev1mush-gateway@b redis-dev1mush
ls /var/lib/dev1mush-gateway               # draining-<gen> = that generation is draining
journalctl -u dev1mush-server -f           # the world (one journal per half)
journalctl -u 'dev1mush-gateway@*' -f      # the front door, both generations
systemctl restart dev1mush-server          # hot-restart the server; connections held
bash /opt/rustmush/dev1/build/deploy/dev1-split/gateway-swap.sh dev1
                                           # move the gateway to its other generation; nobody dropped
systemctl reset-failed dev1mush-server && systemctl start dev1mush-server
                                           # after "staff have been paged": the unit
                                           # exhausted its start limit (5 in 60 s)
/opt/rustmush/dev1/build/target/release/mush-server --ipc-version
                                           # the IPC boundary version a binary speaks
```

`mush-server` is its own systemd unit (doc 33 GG.1): `Type=notify` (READY=1
once the socket is bound), `Restart=on-failure`, `RestartSec=2`, and a start
limit of five starts in sixty seconds. The gateway never starts, stops or
restarts it — it dials the socket, holds every connection while the server is
away, and re-attaches when it answers. Past the start-limit bound without a
server, the gateway tells the held players the server is down and staff have
been paged (in place of "restarting; hold on"), logs `SERVER-UNREACHABLE`,
and reports `server_ok: false` on `/healthz` — and keeps dialing, so the
`start` above heals it with nobody reconnecting. **`systemctl stop
dev1mush-gateway@a` does not stop the server.**

## Switch back to the all-in-one

```bash
systemctl disable --now dev1mush-gateway@a dev1mush-gateway@b dev1mush-server
rm -f /run/rustmush-ssh/dev1.sock         # the generations' symlink; mushd binds the name itself
systemctl enable --now dev1mush          # re-run deploy/dev1/deploy.sh if the unit is gone
```

## Adapting for another instance

The unused dev2 lane has been removed. Provision a new instance from
[`../instance/instance.env.example`](../instance/instance.env.example) and the
shared template. Copy the dev1 server unit, gateway template
(`dev1mush-gateway@.service`), `gateway-swap.sh` (its `INST=`) and `deploy.sh`
(`deploy/dev2-split/` is exactly that, derived by `sed`), then give the new
instance its own `ROOT`, Redis service and port, SSH/HTTPS ports, TLS paths,
environment file (the server unit's; the gateway has none), the server unit's
`RuntimeDirectory`, Unix `IPC_SOCKET` inside that directory, the gateway
template's `StateDirectory`, and its trusted socket under `/run/rustmush-ssh`.
Render and run both halves' `--check-config` (the gateway's with `--generation
a`) before starting it. See the
[provisioning and reset guide](../README.md) for the commands, including
`reset-instance.sh --staff <account>` for a clean testing world with a staff login.
