# dev1 — the SPLIT (hot-restart) deployment

This is the **hot-restartable** way to run the dev1 instance (M16c/M16d). It runs
the forked pair instead of the all-in-one `mushd`, so a deploy can rebuild and
restart the **game server** while the **player connections stay up** — players
see a brief "restarting…" then "Reconnected" and a fresh room render, mid-scene,
never a disconnect. As of **M16d this holds for WEB (WebSocket) players too**,
not just SSH: the gateway now serves the web client (HTTPS :6790) itself.

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
| service(s) | `dev1mush.service` (mushd) | `dev1mush-server.service` (mush-server) + `dev1mush-gateway.service` (mush-gateway) |
| processes | one (`mushd`) | two, each its own unit (doc 33 GG.1): the gateway dials the server over the `[ipc]` socket |
| SSH :6789 | served by mushd | served by the gateway |
| **HTTPS :6790 (web client)** | **served by mushd** | **served by the gateway** (M16d) |
| deploy = restart | `systemctl restart dev1mush` — **drops connections** | `systemctl restart dev1mush-server` — **holds connections (SSH + web)**; the gateway is not touched |
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

`deploy.sh` also does the hot restart on a re-run (it restarts the server unit
when it is already up), so re-running it is equivalent. The gateway's `SIGHUP`
is reserved (doc 33 §4.5): `systemctl reload dev1mush-gateway` is no longer a
verb of this deployment, and the gateway unit has no `ExecReload=`.

### Which half moved: the server unit, or the gateway too (16b cut Q.5)

A server restart moves only the **server half**. The gateway process keeps
running the binary it was started with — so a commit that changed the GATEWAY
half is still live as old code after a server restart, in the one half nothing
says so. Pick by what the commit touched:

| the commit touched | what to run |
|---|---|
| `web/`, `crates/mush-gateway`, `crates/mush-net-*`, `crates/mush-session`, `crates/mush-ansi`, `crates/mush-proto`, `crates/mush-ipc` | `GATEWAY_RESTART=1 bash …/deploy/dev1-split/deploy.sh` (or `systemctl restart dev1mush-gateway`) — **connections drop once** |
| anything else (the game: `mush-server`, `tacsim`, modules, content, config) | `bash …/deploy/dev1-split/deploy.sh` — `systemctl restart dev1mush-server`, connections held |

`web/` is on the restart list because the SPA is **embedded in the gateway
binary** (`crates/mush-net-web`), not served from disk.

A full restart drops every connection once and takes every in-progress session
with it — the attach, the scrollback ring, the resume point all live in the
gateway's memory. It does **not** sign anyone out: web tokens are Redis rows
(`token:{hash}`, 7-day sliding TTL), so an open tab reconnects by itself and is
back in the game without a password. Only the store losing the token asks for a
fresh sign-in: `scripts/reset-instance.sh` (which `FLUSHDB`s), `@newpassword`,
logout, or the TTL. Redis merely being slow or restarting is not one of those —
the gateway answers 503 ("cannot check right now"), which a tab retries with
its token kept — and neither is a server restart: a reconnect that lands while
the server half is down is answered 1012, not "session ended".

> **Each half stops alone** (doc 33 GG.1 + GG.2). `systemctl stop
> dev1mush-gateway` stops only the gateway: the server unit keeps running the
> world, the gateway's connections go link-dead in it, and the gateway that
> comes back dials in as a new link. `systemctl stop dev1mush-server` stops
> only the server: the gateway holds every connection through the gap.

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
systemctl status dev1mush-server dev1mush-gateway redis-dev1mush
journalctl -u dev1mush-server -f           # the world (one journal per half)
journalctl -u dev1mush-gateway -f          # the front door
systemctl restart dev1mush-server          # hot-restart the server; connections held
systemctl reset-failed dev1mush-server && systemctl start dev1mush-server
                                           # after "staff have been paged": the unit
                                           # exhausted its start limit (5 in 60 s)
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
dev1mush-gateway` does not stop the server.**

## Switch back to the all-in-one

```bash
systemctl disable --now dev1mush-gateway dev1mush-server
systemctl enable --now dev1mush          # re-run deploy/dev1/deploy.sh if the unit is gone
```

## Adapting for another instance

The unused dev2 lane has been removed. Provision a new instance from
[`../instance/instance.env.example`](../instance/instance.env.example) and the
shared template. Copy the dev1 pair of unit files, then give the new instance
its own `ROOT`, Redis service and port, SSH/HTTPS ports, TLS paths, environment
file (the server unit's; the gateway has none), the server unit's
`RuntimeDirectory`, and Unix `IPC_SOCKET` inside that directory. Render and run
both halves' `--check-config` before starting it. See the
[provisioning and reset guide](../README.md) for the commands, including
`reset-instance.sh --staff <account>` for a clean testing world with a staff login.
