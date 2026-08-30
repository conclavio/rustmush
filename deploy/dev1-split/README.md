# dev1 — the SPLIT (hot-restart) deployment

This is the **hot-restartable** way to run the dev1 instance (M16c/M16d). It runs
the forked pair instead of the all-in-one `mushd`, so a deploy can rebuild and
restart the **game server** while the **player connections stay up** — players
see a brief "restarting…" then "Reconnected" and a fresh room render, mid-scene,
never a disconnect. As of **M16d this holds for WEB (WebSocket) players too**,
not just SSH: the gateway now serves the web client (HTTPS :6790) itself.

It is a **drop-in alternative to `deploy/dev1`** for the *same* instance: same
Redis (6788), same `mushd.toml`, same SSH port (6789), same web port (6790),
same data/TLS/env. You run **one or the other**, not both (they share the SSH
and web ports).

## How it differs from the all-in-one

| | all-in-one (`deploy/dev1`) | split (this dir) |
|---|---|---|
| service(s) | `dev1mush.service` (mushd) | `dev1mush-gateway.service` (mush-gateway) |
| processes | one (`mushd`) | two (`mush-gateway` + its `mush-server` child) |
| SSH :6789 | served by mushd | served by the gateway |
| **HTTPS :6790 (web client)** | **served by mushd** | **served by the gateway** (M16d) |
| deploy = restart | `systemctl restart dev1mush` — **drops connections** | `systemctl reload dev1mush-gateway` — **holds connections (SSH + web)** |
| server crash | systemd restarts the whole process (players drop) | gateway auto-restarts the child in-process + re-attaches (players held) |

### Web (M16d): now served + hot-restart-surviving

The gateway serves the **web client (HTTPS :6790)** itself — the axum web ingress
is wired into `mush-gateway::serve` on the SAME session layer as SSH. A WS player
authenticates with a token, gets the same `hello` manifest the all-in-one
advertises (who / channels / mail / `<ruleset>.sheet`), and plays over the split
boundary. Crucially, a **WS connection survives a server restart exactly like an
SSH one**: it is never sent a Close frame — it receives "restarting" →
"Reconnected" → a fresh room render as ordinary envelopes, with the world intact
from Redis, and keeps playing (proven by
`crates/mush-gateway/tests/web_restart_survival.rs`). So switching dev1 to the
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
# ship the committed source (adds this dir + the [ipc] port in mushd.toml)
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
# build the TWO split binaries (release). The gateway serves the web client
# (HTTPS :6790) now, so it needs --features tls to terminate TLS from the on-disk
# cert/key; the server is the game half (no web) and needs no tls.
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mush-gateway --features tls && ~/.cargo/bin/cargo build --release -p mush-server'
# switch dev1 to the split (stops the all-in-one, starts the gateway)
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/deploy/dev1-split/deploy.sh'
```

## Redeploy = HOT restart (the payoff)

```bash
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mush-gateway --features tls && ~/.cargo/bin/cargo build --release -p mush-server'
# SIGHUP the gateway → it restarts ONLY the server child from the fresh binary,
# holding every SSH AND web connection. NOT `restart` — that would drop them.
ssh -i $KEY $VPS 'systemctl reload dev1mush-gateway'
```

`deploy.sh` also does the hot restart on a re-run (it `reload`s when the gateway
is already up), so re-running it is equivalent.

## Operate

```bash
systemctl status dev1mush-gateway redis-dev1mush
journalctl -u dev1mush-gateway -f          # gateway + child share this journal
systemctl reload dev1mush-gateway          # hot-restart the server child
```

The child `mush-server` is not its own systemd unit — the gateway owns its
lifecycle (spawns it, restarts it on crash with a crash-loop guard, reaps it on
stop). Its logs interleave into the gateway's journal.

## Switch back to the all-in-one

```bash
systemctl disable --now dev1mush-gateway
systemctl enable --now dev1mush          # re-run deploy/dev1/deploy.sh if the unit is gone
```

## Adapting for another instance (e.g. a dev2-split)

Copy this dir, then in the copies:

- rename the unit `dev2mush-gateway.service`, repoint `WorkingDirectory` /
  `EnvironmentFile` / `ExecStart` / `ReadWritePaths` at `/opt/rustmush/dev2`,
  and its `Requires`/`After` at `redis-dev2mush.service`;
- in `deploy.sh` set `ROOT=/opt/rustmush/dev2`, `DEV1=$SELF/../dev2`, the
  redis port (6791), and the unit names;
- give the instance its **own** `[ipc].listen` localhost port in its
  `mushd.toml` (dev1 uses `127.0.0.1:6794`) so two split instances on one box
  do not collide on the boundary socket.
