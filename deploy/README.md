# Deploying rustmush

Instances are named and isolated so several can run on one box (a WoD one, a
BattleTech one, …). Each instance `<name>` gets:

- systemd services `<name>mush.service` + `redis-<name>mush.service`
- working tree `/opt/rustmush/<name>` (build under `.../build`)
- its own Redis on its own localhost port (dev1 = **6788**)
- its own SSH + HTTPS ports (dev1 = **6789** / **6790**), bound to the box's
  **external IPv4 only** (not `0.0.0.0`, not the tailscale address)

The reference instance is **dev1** ([`dev1/`](dev1/)) — oWoD, ports **6788/6789/6790**.
A second instance **dev2** ([`dev2/`](dev2/)) runs **Shadowrun** (`[rulesets].enabled = ["shadowrun"]`)
on ports **6791** (redis) / **6792** (ssh) / **6793** (https), `/opt/rustmush/dev2`.
To add another, copy an instance directory, rename the units/config, and pick fresh ports.

Each instance can run one of **two ways**:

- **all-in-one `mushd`** (the default, `dev1/` + `dev2/`) — one process serving
  SSH + the web client; a deploy = `systemctl restart`, which drops connections
  briefly.
- **split, hot-restartable** ([`dev1-split/`](dev1-split/), M16c) — a
  `mush-gateway` owns the SSH connections and supervises a `mush-server` child;
  a deploy = `systemctl reload` → SIGHUP, which restarts only the server child
  while **holding every SSH connection** (players resume mid-scene). The gateway
  does **not** serve the web client yet, so instances whose players use the web
  UI stay on the all-in-one. See [`dev1-split/README.md`](dev1-split/README.md)
  and `docs/ops.md` → "Hot-restart (split) deployment".

Each new external port must be opened in **BOTH** ufw (`ufw allow <port>/tcp`, done by
this deploy) **and** the Hostinger network ACL (done in the Hostinger panel — ufw alone
is not enough for external reachability).

## First deploy of dev1 (from the dev box, over SSH)

The dev box is Windows with no git remote; ship the committed tree + the
locally-built web bundle, build the release binary on the VPS, then configure.

```bash
VPS=root@conclav.io ; KEY=./hostinger
# 1. Toolchain (Rust; git+openssl already present, Node NOT needed — we ship dist)
ssh -i $KEY $VPS 'command -v cargo || (curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . ~/.cargo/env)'
# 2. Ship the committed source (tracked files only — no target/, no .env)
ssh -i $KEY $VPS 'mkdir -p /opt/rustmush/dev1/build'
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
# 3. Ship the locally-built SPA (web/dist is gitignored; the release build embeds it)
npm --prefix web run build
tar -c -C web dist | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build/web'
# 4. Build the release binary WITH native TLS
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mushd --features tls'
# 5. Ship the LLM keys as the systemd EnvironmentFile (root:mush 640)
scp -i $KEY .env $VPS:/opt/rustmush/dev1/dev1.env
# 6. Configure + start (creates user/dirs/redis/cert, resolves external IP, wires systemd)
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/deploy/dev1/deploy.sh'
```

## Redeploy (new commit)

```bash
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
npm --prefix web run build && tar -c -C web dist | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build/web'
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mushd --features tls && systemctl restart dev1mush'
```

## Operate

```bash
systemctl status dev1mush redis-dev1mush
journalctl -u dev1mush -f
redis-cli -p 6788 info keyspace          # prod data (NOT the dev 6379)
```

Prod data lives in the **6788** instance; the box's **6379** Redis is the
port-forwarded development instance whose namespaces the gated test suite
wipes — never point a live server at 6379. See [`../docs/ops.md`](../docs/ops.md).
