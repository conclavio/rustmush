# Deploying rustmush

Instances are named and isolated so several can run on one box (dev1, dev2, …).
Each instance `<name>` gets:

- systemd services `<name>mush.service` + `redis-<name>mush.service`
- working tree `/opt/rustmush/<name>` (build under `.../build`)
- its own Redis on its own localhost port (dev1 = **6788**)
- its own SSH + HTTPS ports (dev1 = **6789** / **6790**), bound to the box's
  **external IPv4 only** (not `0.0.0.0`, not the tailscale address)

The reference instance is **dev1** ([`dev1/`](dev1/)) — ports **6788/6789/6790**.
A second instance **dev2** ([`dev2/`](dev2/)) runs the same game (a second box, not a second ruleset — 16b cut R-b)
on ports **6791** (redis) / **6792** (ssh) / **6793** (https), `/opt/rustmush/dev2`.

## A new instance: render the template, do not copy a box

`dev1/mushd.toml` and `dev2/mushd.toml` predate [`instance/`](instance/) and omit
**eleven** sections between them — `[log]`, `[store.archive]`, `[offers]`,
`[resolution]`, `[discord]`, `[economy]`, `[weather]`, `[worldgen]`,
`[onboarding]`, `[npc]`, `[tacsim]` — so each of those
runs on a compiled-in default nobody chose: no log files, no ledger archive
tier, key directories nobody provisioned, and every character starting in Limbo
`#0`. Copying one of them inherits all of that silently.

A new instance renders [`instance/mushd.toml.tmpl`](instance/mushd.toml.tmpl)
instead. It carries every section with a real value, and it is parameterised by
an `instance.env` of plain `KEY=value` lines — copy
[`instance/instance.env.example`](instance/instance.env.example), edit it, and
substitute:

```bash
cp deploy/instance/instance.env.example instance.env && $EDITOR instance.env
while IFS='=' read -r k v; do
  case "$k" in ''|'#'*) continue;; esac
  printf 's|__%s__|%s|g\n' "$k" "$v"
done < instance.env > render.sed
sed -f render.sed deploy/instance/mushd.toml.tmpl > mushd.toml
grep -n '^[^#]*__' mushd.toml && { echo 'INCOMPLETE instance.env'; exit 1; }
```

The loop READS `instance.env`; it does not `source` it. Values are sed
replacements rather than shell words, so `INSTANCE_TITLE=rustmush dev3 (oWoD)`
is a good line here and a syntax error to `.` — do not be tempted by
`set -a; . ./instance.env`, which aborts on that line, leaves every key after
it unset, and renders a config with empty ports and an empty start room. The
completeness `grep` skips comment lines (`^[^#]*`) because the template's own
header spells the mechanism out with `__KEY__` tokens; that is exactly the rule
the test applies, so the shell and the test cannot disagree about what a
complete render is.

Ship **both** files to the box and keep them together —
`/opt/rustmush/<name>/mushd.toml` beside `/opt/rustmush/<name>/instance.env`.
`git archive HEAD` carries the template, never your edited `instance.env`, so
the edited one is `scp`'d up alongside the config it produced; it is the input
a re-render reads, and the file `scripts/reset-instance.sh` below is pointed at.

`START_ROOM` / `START_ROOM_UNAPPROVED` name object ids that must EXIST in the
world you boot. A fresh store seeds only Limbo `#0` and God `#1`, so `#2`/`#3`
are the ids the instance's own seed — the rooms you dig before opening — will
take, not rooms you get for free. Check them after first boot (`look #2`,
`examine #3` as God) before letting anyone connect; WS25-S5 is where the seed
grows those rooms itself.

`crates/mush-config/tests/shipped_configs.rs` renders both shipped `instance.env`
files on every build and asserts the sections are all there, the start rooms are
not Limbo, `[ipc].listen` cannot be the store's port, and `mushd.toml.example`
declares the same section set — so the shipped example and the shipped
deployment cannot drift apart.

Pick fresh ports per instance, and rename the units (`<name>mush.service`,
`redis-<name>mush.service`) from `dev1/`'s.

Each instance can run one of **two ways**:

- **all-in-one `mushd`** (the default, `dev1/` + `dev2/`) — one process serving
  SSH + the web client; a deploy = `systemctl restart`, which drops connections
  briefly.
- **split, hot-restartable** ([`dev1-split/`](dev1-split/), M16c/M16d) — a
  `mush-gateway` owns the SSH **and web** connections and supervises a
  `mush-server` child; a deploy = `systemctl reload` → SIGHUP, which restarts
  only the server child while **holding every SSH AND web connection** (players
  resume mid-scene). As of M16d the gateway serves the web client (HTTPS) itself,
  and WS connections survive the hot restart just like SSH — so the split is now
  a full drop-in for the all-in-one, web included. Build the gateway with
  `--features tls` for HTTPS. See [`dev1-split/README.md`](dev1-split/README.md)
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

## Reset an instance (flush and rebuild)

rustmush is pre-launch: there are no running games and no players, the store is
disposable, and the deployment target is a fresh box. So flush-and-rebuild is
the STANDARD remedy for a wedged store, a bad migration or a world someone
tested into a corner — and a standard remedy that lives in somebody's shell
history gets half-performed at 2am (the unit left running while Redis is
flushed; the data dir kept, so the quarantine spool and the key rings outlive
the world they belonged to). It is scripted instead:

```bash
sudo scripts/reset-instance.sh /opt/rustmush/dev3/instance.env --yes
```

It stops the instance's unit (either shape — `<name>mush.service` or the split
`<name>mush-gateway.service`), `FLUSHDB`s that instance's OWN Redis on its
`REDIS_PORT`, deletes and recreates `DATA_DIR` owned by the unit's `User=`, and
starts the unit again. The rendered `mushd.toml` is not touched: the config is
the thing you keep. After a reset the world is a fresh seed — Limbo `#0` and God
`#1` — so re-dig `START_ROOM` / `START_ROOM_UNAPPROVED` before letting anyone
connect. The script READS `instance.env` with the same loop the render recipe
uses; it never sources it.

CI's `Instance provisioning acceptance` job renders the shipped instance with
the recipe above, runs all three binaries' `--check-config` over what it
produced, boots it against a throwaway store, and proves a typo'd
`receipt_policy` is refused — so the templates cannot drift past the binaries
that read them (`crates/mush-server/tests/provisioning_smoke.rs`).
