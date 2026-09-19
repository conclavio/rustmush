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

## The simulator centre

One key turns the simulator on: **`[tacsim].enabled`**. The rendered instance
takes it from `TACSIM_ENABLED` in `instance.env`; `deploy/dev1` and
`deploy/dev2` write it out as `true`; `mushd.toml.example` ships it `false`,
which is the documented default.

What the key decides is the *arena*, not the rooms. The centre's lobby, its two
team rooms and its eight pods come with the starter area (`[game].starter_area`)
and are in the world either way. The proving ground, its domain and the eight
stock tanks are the game's own boot packs and ride on this flag alone — so with
it off the pods have nothing behind them, `SIM` refuses out loud, and no match
can start. Turning it on drags in nothing else: no seed pack (`[tacsim].seeds`
is its own key), no corridor, no ephemeris change and no economy change; those
are read whether the layer is on or off (`crates/mush-server/src/boot.rs` hands
the flag to exactly one call, `content_module_with`'s `game_content`, in
`crates/tacsim/src/composition/mod.rs`).

The boot log says which it got, every time, and then whether the centre is ready:

```
tacsim enabled: the simulator's arena and its stock tanks apply at boot
simulator centre ready: the arena is applied and its tanks are standing roster_tanks=8
```

A `tacsim disabled (...)` line, or a `simulator centre NOT ready` warning, is the
thing to grep for before anyone connects. Note that `[tacsim]` is
`deny_unknown_fields` but a mistyped section **header** is not a key: `[tac_sim]`
parses clean and boots the layer off, and that negative log line is the only
symptom.

### A fresh store is the supported path

rustmush is pre-launch, so the supported way to get a working centre is a fresh
world: `scripts/reset-instance.sh` (above), then boot with the layer on. First
boot applies the starter packs and the game's two packs together and the centre
is whole.

This matters beyond the simulator: cut P deleted the legacy pose arm, so a store
written before 2026-09-16 carries placements the current build cannot read and
holds them to the recovery room.

### An existing world takes a newer OOC pack by hand

A shipped **boot** pack is applied only when the world holds no latch for it. A
world latched at an older version is not upgraded at boot; it logs one line
(`content: a newer version of a shipped pack is available; '@atlas/pack plan'
shows the diff`) and carries on. So a world seeded before the current
`neighborhood.ooc_area` needs staff to take the upgrade, **in this order**:

```
@atlas/pack plan neighborhood.ooc_area
@atlas/pack apply <fingerprint>
@atlas/pack plan neighborhood.staff_lobby
@atlas/pack apply <fingerprint>
```

`plan` prints the fingerprint `apply` takes. The order is not optional:
`neighborhood.staff_lobby` declares `requires neighborhood.ooc_area@<version>`
and is refused while the OOC pack is behind. Entries the live world does not
have are created; entries whose text moved are updated in place, and a room, an
item or a character keeps its object id and where it is standing. (An *exit*
entry is remade rather than edited — its live state is nothing — so its object
id changes.)

Do **not** reach for `@atlas/pack retire` to "start again": a retired latch is
marked retired and the boot pass then skips that pack for ever.

### The arena pack is not on the shelf

`tacsim.pack.proving_ground` and `tacsim.proving_ground` are boot packs and are
deliberately **not** on the `@atlas/pack` shelf: doc 16 §3.3's tanks are created
once, by the centre's content, and the latch is what makes a second boot make no
ninth tank. `@atlas/pack list` shows them under "Latched, not on the shelf" and
no verb applies them. If a world has the centre's rooms but an empty roster, the
repair is the config plus a restart, or a reset instance — not a staff command.

A standing tank is adopted by **name** when the arena entry is applied, so a
re-apply never makes a second Red Tank 1; it re-places the tank it finds on the
cell the pack authors. And since 16b cut F.1 the boot pass resets every roster
tank to its spec and to its authored cell on **every** boot, reading the slot out
of the compiled definition rather than out of the latch — so a shipped change to
the muster (16b cut F.4 moved it onto the ford road) reaches an existing world at
its next restart with no pack upgrade at all.

### The first staff login

There is no scripted one. A fresh world seeds Limbo `#0` and God `#1`, and `#1`
is linked to no account; `god` is a reserved account name, so it cannot be
registered at the welcome screen. The one route that exists is the legacy
one-time import: write `<DATA_DIR>/accounts.json` **before** the first boot,

```json
{"version": 1,
 "accounts": [{"name": "boss",
               "hash": "<argon2id PHC string>",
               "characters": [{"id": 1, "name": "God"}]}]}
```

and the boot imports it, logs `M1 accounts.json imported`, and renames the file
to `accounts.json.imported`. Nothing shipped produces the hash: make it with any
argon2id tool that emits a PHC string (`$argon2id$v=19$m=...`). After that,
`connect boss <password>` at the welcome screen is God, and ordinary players
register themselves with `create <name> <password>`.
