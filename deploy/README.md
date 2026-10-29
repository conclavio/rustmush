# Deploying rustmush

Instances are named and isolated so several can run on one box (dev1, dev2, …).
Each instance `<name>` gets:

- systemd services `<name>mush.service` + `redis-<name>mush.service`
- working tree `/opt/rustmush/<name>` (build under `.../build`)
- its own Redis on its own localhost port (dev1 = **6788**)
- its own SSH + HTTPS ports (dev1 = **6789** / **6790**), bound to the box's
  **external IPv4 only** (not `0.0.0.0`, not the tailscale address)

The reference instance is **dev1** ([`dev1/`](dev1/)) — ports **6788/6789/6790**.
The unused dev2 deployment lane has been removed. A second testing instance can
be provisioned from the shared template with its own data directory, Redis,
ports, unit and IPC socket; no live instance is changed by this repository edit.

## A new instance: render the template
A new instance renders [`instance/mushd.toml.tmpl`](instance/mushd.toml.tmpl)
instead. It carries every section with a real value, and it is parameterised by
an `instance.env` of plain `KEY=value` lines — copy
[`instance/instance.env.example`](instance/instance.env.example), edit it, and
render:

```bash
cp deploy/instance/instance.env.example instance.env && $EDITOR instance.env
bash deploy/instance/render.sh instance.env deploy/instance/mushd.toml.tmpl mushd.toml \
  target/release/mushd            # or mush-gateway + mush-server, for the split
```

[`instance/render.sh`](instance/render.sh) is the ONE render, and both deploy
scripts and CI's provisioning job call it. It is a `sed` and nothing more:

```bash
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

Every argument after the output path names a binary whose own `--check-config`
is run over the render — `mushd` for the all-in-one, `mush-gateway` **and**
`mush-server` for the split. Nothing is installed until all of them pass: the
render lands on `<out>.new` and is moved into place only when it is complete
and preflighted, so a redeploy with a broken `instance.env` leaves the live
config alone and starts nothing. A binary that is not built yet is skipped with
a note (the first deploy of a box renders before it compiles).

Ship **both** files to the box and keep them together —
`/opt/rustmush/<name>/mushd.toml` beside `/opt/rustmush/<name>/instance.env`.
`git archive HEAD` carries the template, never your edited `instance.env`, so
the edited one is `scp`'d up alongside the config it produced; it is the input
a re-render reads, and the file `scripts/reset-instance.sh` below is pointed at.
(dev1 is the exception that proves the rule: its `instance.env` **is**
committed, because it holds no secret and its one box-specific value —
`EXTERNAL_IP=auto` — is resolved by its deploy script when it writes the
installed copy.)

`START_ROOM` / `START_ROOM_UNAPPROVED` name object ids that must EXIST in the
world you boot. A fresh store seeds only Limbo `#0` and God `#1`, so `#2`/`#3`
are the ids the instance's own seed — the rooms you dig before opening — will
take, not rooms you get for free. Check them after first boot (`look #2`,
`examine #3` as God) before letting anyone connect; WS25-S5 is where the seed
grows those rooms itself. A box that wants **no** gate at all renders `0` for
both instead, with `CHARGEN_REQUIRED=false`: that is
`mush_core::OnboardingConfig`'s documented all-default no-op, and it is the
right answer on a box with no approval staff whose starter area hangs straight
off Limbo (`#0` has the `ooc` exit into the OOC Area, and the Welcome Hall has
the door into the Simulator Lobby). dev1 is that box.

`crates/mush-config/tests/shipped_configs.rs` renders every shipped
`instance.env` on every build and asserts the sections are all there,
`[ipc].listen` cannot be the store's port, each box's `[log].format` is the one
it chose, each box's onboarding is the shape it chose, the rendered `[ssh]` and
`[web]` listeners parse as addresses, and `mushd.toml.example` declares the same
section set — so the shipped example and the shipped deployments cannot drift
apart.

Be precise about which gate holds dev1's `EXTERNAL_IP=auto`, because it is not
that test. The test renders the way the DEPLOY does, marker already resolved to
a documentation address, and then asserts no *other* value is the literal
marker; what keeps the real box honest is the deploy script's
`grep -q "^EXTERNAL_IP=$EXT_IP$"` over the file it just wrote (a committed env
with no `EXTERNAL_IP=` line is a FATAL there, not a silent pass-through) and
`render.sh`'s own refusal of a live `auto` in anything it rendered — which is
the one that catches a hand re-render from the committed file, where no deploy
script and no test is in the loop.

Pick fresh ports per instance, and rename the units (`<name>mush.service`,
`redis-<name>mush.service`) from `dev1/`'s.

Each instance can run one of **two ways**:

- **all-in-one `mushd`** (the default; what `dev1/`'s own
  `deploy.sh` still installs if you choose it) — one process serving SSH + the
  web client; a deploy = `systemctl restart`, which drops connections briefly.
- **split, hot-restartable** ([`dev1-split/`](dev1-split/), M16c/M16d — and what
  dev1 actually runs) — a
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

**dev1 runs the SPLIT shape**, so the walkthrough that matches the box as it
stands is [`dev1-split/README.md`](dev1-split/README.md) — two binaries, one
`dev1mush-gateway` unit, and a redeploy that holds connections. The all-in-one
recipe below is the same instance run the other way; both render the same
`mushd.toml` from the same `dev1/instance.env`, and you run one or the other,
never both.

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

**dev1 runs the split**, so this is the split's redeploy — two binaries, and
`deploy/dev1-split/deploy.sh` rather than a bare `systemctl`:

```bash
git archive HEAD | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build'
npm --prefix web run build && tar -c -C web dist | ssh -i $KEY $VPS 'tar -x -C /opt/rustmush/dev1/build/web'
ssh -i $KEY $VPS 'cd /opt/rustmush/dev1/build && ~/.cargo/bin/cargo build --release -p mush-gateway --features tls && ~/.cargo/bin/cargo build --release -p mush-server'
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/deploy/dev1-split/deploy.sh'
```

The SPA line is **not** optional: `web/dist` is git-ignored, so the archive
never carries it, and `crates/mush-net-web/build.rs` embeds whatever is on disk
and errors only when `index.html` is missing entirely. Since 16b cut Q.5
`deploy.sh` refuses to start or reload anything when the shipped bundle is
missing or its `web/dist/.commit` differs from the source commit, and names the npm command — the bundle can be
built on the box (`cd …/build/web && npm ci && npm run build`, Node 22) or
shipped as above.

Since doc 21 cut G.3b `npm run build` first builds the cockpit picture's
WebAssembly module (`web/scripts/build-wasm.mjs`, `crates/mush-pixel-wasm`),
which needs the Rust toolchain's `wasm32-unknown-unknown` target wherever the
bundle is built: `rustup target add wasm32-unknown-unknown` once (no
wasm-bindgen, nothing else). Shipping the bundle as above keeps the box free of
both Node and that target; building it on the box needs both.

A commit that moved the **gateway half** — `web/`, `mush-gateway`,
`mush-net-*`, `mush-session`, `mush-ansi`, `mush-proto`, `mush-ipc` — needs a
full restart, because SIGHUP restarts only the server child and the gateway
process would keep running the old binary. Set `GATEWAY_RESTART=1` on the
deploy line (`ssh … 'GATEWAY_RESTART=1 bash …/deploy/dev1-split/deploy.sh'`) and
connections drop once. Everything else stays a `reload`
([`dev1-split/README.md`](dev1-split/README.md) has the table). 16b cut AC.1 —
the split's world task holding the LLM service, so a squad's commander asks its
provider on the split as it did on the all-in-one (16c `B-82`) — is a
`mush-server` change and one of the `reload` cases; the one file it adds under
`crates/mush-gateway` is a test the gateway binary never compiles, so the table's
`crates/mush-gateway` row does not apply to it. It turns on every LLM-backed
effect the split used to drop or refuse, not only deliberation: the OOC area's
NPCs (Sharon in the Welcome Hall, where `ooc` lands everyone, and Aaron on the
bus) answer and summarise through the provider, and staff `@gen` and
`@weather/generate` call it too. From that deploy all of them spend inside the
template's one shared `[llm.budgets] global` cap, and only `worldgen_room` and
`npc_dialogue_major` have a per-feature cap of their own.

**What a full restart does to a web player.** Every SSH and web connection
drops once, and every in-progress *session* — the attach, the scrollback ring,
the resume point — is gone with the gateway's memory. Signing in is a different
thing and survives: web tokens are store rows (`token:{hash}`, 7-day sliding
TTL) in this instance's Redis, so an open tab reconnects on its own backoff and
lands back in the game without a password. Players are asked to sign in again
only when the **store** loses the token — `scripts/reset-instance.sh` (it
`FLUSHDB`s), `@newpassword`, an explicit logout, or the TTL running out. Redis
being *briefly unreachable* is not one of those: the gateway answers "cannot
check right now" (HTTP 503 on the upgrade), which a tab waits out with its
token intact, rather than "your session has ended", which would have made every
open tab throw away a valid credential over a few seconds of store trouble.

That combination is a trap worth knowing about on a box running a gateway
older than 2026-09-20: a tab reconnecting with a token the store no longer
holds was refused with an HTTP 401 that a browser cannot distinguish from a
dropped network, so it retried, and five refusals inside the backoff's first
fifteen seconds tempbanned the address — **out of SSH as well**, for fifteen
minutes, with `ssh -p 6789 play@…` answering a password prompt that could
never succeed. Since that date a refused token is answered with a WebSocket
close the client understands (it drops the token and shows the login form),
refused tokens are counted apart from passwords and can no longer ban
anything, and a blocked address is told in words how long is left. If you meet
the old behaviour on an un-updated box: wait fifteen minutes, or restart the
gateway (the tempban is gateway memory and does not survive it).

The deploy script is the redeploy because the config is rendered: it re-resolves
the external IP, rewrites `instance.env`, **re-renders `mushd.toml`** from the
shipped template and preflights it with both halves before it touches the unit,
then `reload`s the already-running gateway — the hot restart that holds every
SSH and web connection. `systemctl reload dev1mush-gateway` on its own is the
faster form when nothing in the config or the template moved
([`dev1-split/README.md`](dev1-split/README.md)). The IC clock is the instance
env's too (`IC_RATIO` and `IC_EPOCH` render `[game].ic_ratio` and `ic_epoch`,
16b cut AC.2), so a hand edit of the rendered `mushd.toml` does not survive a
deploy — change the env. On dev1 that is the committed
[`dev1/instance.env`](dev1/instance.env), not the installed
`/opt/rustmush/dev1/instance.env`: the deploy rewrites the installed copy from
the committed one every time, so an edit made on the box is reverted the same
way. The world timeline is seeded from those two only on an empty store, and
`scripts/reset-instance.sh` restarts on the rendered `mushd.toml` it finds
without re-rendering, so to change the clock of a world you are about to wipe:
edit the env, run the deploy (which re-renders), THEN wipe.

Do **not** reach for `systemctl restart dev1mush` here. That is the ALL-IN-ONE
unit, which the split deploy disabled but left installed: starting it puts a
second process on 6789/6790 that the gateway already holds, so it fails to bind
and `Restart=on-failure` loops it every five seconds — and, because no deploy
script ran, the config it would have run on was never re-rendered. On a box
running the all-in-one instead (the single-process shape), the equivalent line is
`cargo build --release -p mushd --features tls && bash …/deploy/dev1/deploy.sh`,
which likewise re-renders before it restarts.

## Operate

```bash
systemctl status dev1mush-gateway redis-dev1mush
journalctl -u dev1mush-gateway -f        # gateway + its server child share this journal
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
sudo scripts/reset-instance.sh /opt/rustmush/dev1/instance.env --yes
```

It stops the instance's unit (either shape), flushes that instance's Redis, and
removes world-bound data: the archive, quarantine spool, offer/resolution/decision
keys and logs. It **preserves `DATA_DIR/ssh_host_ed25519` in place**, so existing
SSH host-key pins remain valid, and keeps the rendered configuration. Startup
reapplies the configured boot packs; on dev1 both starting rooms are Limbo `#0`.
The script reads the installed `instance.env`; it never sources it.

Add `--staff keeper` to create the first staff account before the service starts.
The script prompts without echo on a terminal, or reads one password line from
stdin. It validates the binary and config before stopping anything; a bootstrap
failure after the wipe leaves the service stopped so the operator can retry the
CLI below. The account is linked to God `#1`; passwords and hashes never appear
in command arguments or output. The reset requires the current release binaries,
`BUILD_DIR` in `instance.env`, and `mushd.toml` beside that installed file.
**The first time**, the two steps are ordered: `deploy.sh` is what puts
`instance.env` on the box, so it runs first, and the reset runs after it —

```bash
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/deploy/dev1-split/deploy.sh'
ssh -i $KEY $VPS 'bash /opt/rustmush/dev1/build/scripts/reset-instance.sh \
                    /opt/rustmush/dev1/instance.env --yes'
```

— which is the right order anyway on a box whose world predates the current
build: the deploy comes up on the old store, the reset flushes it and restarts
onto a fresh seed, and that fresh boot is the one that applies the starter area
and the simulator's two packs together (see *A fresh store is the supported
path* below).

CI's `Instance provisioning acceptance` job renders every shipped instance with
`deploy/instance/render.sh` — the script the deploy itself calls — runs all
three binaries' `--check-config` over what it produced, boots the reference one
against a throwaway store, and proves a typo'd `receipt_policy` is refused — so
the templates cannot drift past the binaries that read them
(`crates/mush-server/tests/split_boundary/provisioning_smoke.rs`).

## The simulator centre

One key turns the simulator on: **`[tacsim].enabled`**. A rendered instance
takes it from `TACSIM_ENABLED` in `instance.env`, which both
[`instance/instance.env.example`](instance/instance.env.example) and
[`dev1/instance.env`](dev1/instance.env) set to `true`;
`mushd.toml.example` ships it `false`, which is the documented default.

What the key decides is the *arena*, not the rooms. The centre's lobby, its two
team rooms and its twelve pods come with the starter area (`[game].starter_area`)
and are in the world either way. The proving ground, its domain and the twelve
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
simulator centre ready: the arena is applied and its tanks are standing roster_tanks=12 of 12 pods=12 of 12
```

A `tacsim disabled (...)` line, or a `simulator centre NOT ready` warning, is the
thing to grep for before anyone connects. A `simulator centre partly ready`
warning is a world that has not taken the six-a-side packs yet (below): it plays
four a side until it does. Note that `[tacsim]` is
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

**`ooc_area` 5 → 6 and `staff_lobby` 3 → 4 (16b cut D.5).** This pair changes
text and exit names only: the alias `l` comes off five exits (the three
`Lounge <L>` exits and the two `Lobby <L>` ones), so `l` is `look` again in the
Tutorial: Building, the Welcome Hall, the Bus Stop, the staff Forecourt and the
Breakroom, and the Simulator Selector's console text stops claiming `SIM START`
needs a pilot on both sides. No entry is created or retired. The five exits are
**remade**, as every exit entry is, so their object ids change — nothing else
moves, and nobody is displaced. A world that skips the upgrade keeps the old `l`
exits and loses nothing else. The Welcome Hall's sign and every other
pack-authored room detail become readable with `look <keyword>` on an
un-upgraded world too, because that half is code and not content.

### The arena pack is on the shelf

`tacsim.pack.proving_ground` and `tacsim.proving_ground` are boot packs **and**,
since 16b cut Z.3, on the `@atlas/pack` shelf beside the starter packs. The boot
pass is unchanged: a first boot applies them, and the latch is what makes a
second boot make no thirteenth tank. The shelf is how a standing world takes a
newer version by hand, and Z.3 is the first to ship one.

When the arena entry is applied, each standing tank is adopted by the arena's
**roster** (the slot it was made for), not by its name, so a tank staff renamed
is still the one its pod flies and no second one is made; the name is only
consulted for a slot the roster has never held. Every tank is re-placed on the
cell the pack authors, which is why the apply **refuses while a match is
running** — `tacsim.arena.proving_ground: a match is running; apply with the
centre idle` — and writes nothing until the centre is idle. And since 16b cut
F.1 the boot pass resets every roster tank to its spec and to its authored cell
on **every** boot, reading the slot out of the compiled definition rather than
out of the latch — so a shipped change to the muster (16b cut F.4 moved it onto
the ford road) reaches an existing world at its next restart with no pack
upgrade at all.

**Six a side: `ooc_area` 6 → 7, `staff_lobby` 4 → 5 and the arena 2 → 3 (16b cut
Z.3).** The server half takes the new binary as any server-half deploy does; no
gateway restart is needed for this cut. Until the packs are taken the centre is
**four a side exactly as before**: the lobby's cap follows the pods and tanks
that exist on both sides, so `SIM TEAM` hands out no fifth slot, `SIM AI 5` is
refused, and the boot log reads

```
simulator centre partly ready: 8 of 12 pods, 8 of 12 tanks; the lobby fields the smaller count a side; take the packs by hand (deploy/README.md)
```

Then, **with the centre idle** (`@arena/status` reads it; the applier refuses
while a match runs, because it re-places every tank), staff take **two** packs
by hand, in this order:

```
@atlas/pack plan neighborhood.ooc_area
@atlas/pack apply <fingerprint>
@atlas/pack plan tacsim.pack.proving_ground
@atlas/pack apply <fingerprint>
```

The first creates four pods in the team rooms and updates the two rooms' text in
place; nobody is moved and no exit is remade. The second adopts the eight
standing tanks by the roster, re-places them on their authored cells, creates
Red Tank 5, Red Tank 6, Blue Tank 5 and Blue Tank 6 at theirs, and rewrites the
roster document. After it the lobby seats six a side — a side lead's `SIM AI 6`
is accepted where it was refused (`AI fill is 1 to 4 a side.`) — and the next
boot logs `simulator centre ready: ... roster_tanks=12 of 12 pods=12 of 12`.
`@atlas/pack plan neighborhood.staff_lobby` reads
` Nothing to do.` on a live world (its only change is the version a fresh boot's
`requires` needs), so its latch stays at 4 and every boot logs one info line
saying a newer version is available; that line is harmless. A world that skips
both keeps four pods a side and loses nothing else. `@atlas/pack retire` is
still not the way to start again.

The shelf takes a **newer** version and nothing else: on a world whose arena is
already at the shipped version, `@atlas/pack plan tacsim.pack.proving_ground`
reads ` Nothing to do.` and `@atlas/pack apply` refuses `is applied at this
version`. So it does not re-make a stock tank that was destroyed or
`@unit/remove`d on such a world, and the `partly ready` warning and the
`SIM START` refusal (`The simulator has no unit for ...`) that follow such a loss
point at a route that has nothing to take. The lobby then seats the slots before
the missing one a side; a reset instance is the repair until a later version
ships.

**The Hold mode: the arena 3 → 4 (16b cut O.1).** Version 4 only declares the
modes the ground supports (Hold at The Crossroads, six cells, 120 s); no tank,
cell or roster changes. The arena catalog is compiled in and rebuilt at every
boot, so `SIM MODE hold` works the moment the new server half is up, whatever
the latch says; taking the pack only moves the latch. With the centre idle, run
`@atlas/pack plan tacsim.pack.proving_ground` **first**: on a version 3 latch it
reads `0 create, 1 update, 0 retire`, and `@atlas/pack apply <fingerprint>`
moves the latch to 4, adopts the twelve standing tanks by roster and remakes
none. On a version 2 latch (a store never reset since Z.3) the six-a-side
paragraph above applies first — `neighborhood.ooc_area` before the arena — and
the arena step then lands at 4 in one apply. A world that skips it loses nothing
but the boot's info line saying a newer version is available. This cut also
moves the gateway half (the replay's outcome words and its head's `[hold]` tag
on the TUI and the web), so its deploy is `GATEWAY_RESTART=1` with `web/dist`
rebuilt after the commit.

**The Bluff: the atlas 1 → 2 and the arena 4 → 5 (16b cut AE.1).** The arena
catalog is compiled in, so the server-half restart brings the fifth key spot,
**F The Bluff**, at once, whatever the latch says — **and every replay recorded
before the restart loses its map at the restart** (`The ground of match N is
not the ground loaded on this server.`: the arena definition's digest folds the
key spots). But the ground under `F` is the atlas pack, so the atlas apply must
follow the restart at once, or `F` names flat grass. With the centre idle (read
`@arena/status` first: the **atlas** apply is not guarded by a running match,
only the arena apply is), take two packs by hand, in this order:

```
@atlas/pack plan tacsim.proving_ground
@atlas/pack apply <fingerprint>
@atlas/pack plan tacsim.pack.proving_ground
@atlas/pack apply <fingerprint>
```

The atlas plan reads `Plan for tacsim.proving_ground@2: 0 create, 1 update, 0
retire, 5 unchanged` and names ` update   field.q1_0 (heightfield)` — one
update, the quadrant the bluff stands in, and nothing else — and its apply
replaces that heightfield in place. The arena plan reads `0 create, 1 update, 0 retire`, and
its apply moves the latch to 5 and adopts the twelve standing tanks by roster,
remaking none. Run no `SIM START` between the two applies. The next `SIM START`
fights on the bluff, since the arena definition is rebuilt from the records at
every start. The three new chassis (`scout_mk1`, `heavy_mk1`, `biped_mk2`) and
the per-row ground prices are compiled in: they need the restart and no apply.

**The showcase relief: the atlas 2 → 3 (doc 21 cut G.7b).** The atlas pack
`tacsim.proving_ground` is version 3: the same classes, The Bluff and the
fought-over ground (the road's corridor, rows 86 to 108, and the north-south
road) at version 2's heights to the centimetre, and a landscape everywhere
else — the northern highlands and Signal Hill, The Bowl, the river's valley,
The Spine and The Shelf. The arena pack is unchanged (still 5). A world that
holds version 2 logs the newer-pack notice at boot and fights on the old ground
until staff take it, with the centre idle (the atlas apply is not guarded by a
running match):

```
@atlas/pack plan tacsim.proving_ground
@atlas/pack apply <fingerprint>
```

The plan reads `Plan for tacsim.proving_ground@3: 0 create, 4 update, 0
retire, 2 unchanged` — the four heightfield quadrants — and the next `SIM
START` fights on the new ground, since the arena definition is rebuilt from the
records at every start. **Replays recorded on version 2 lose their map once the
new ground is taken** (`The ground of match N is not the ground loaded on this
server.`), as at AE.1. A fresh store (the reset) boots version 3 directly. The
picture draws the relief only from a gateway at doc 21 cut G.7b or later: the
map feed carries the heights from then (`elev`), so deploy with
`GATEWAY_RESTART=1`.

**Replays (16b cut I.1).** Every match that ends writes one more document in its
result's own batch, `tacsim:replay:<match>` — both sides' positions every 5 s,
the shots and the wrecks, at most 256 KiB — and the result says `"replay": true`.
It is kept for as long as the centre remembers the match (its last twenty) and
deleted with the row, so the store holds at most twenty of them; a boot reads
them back with the result documents. No pack and no config key. Every result
row written before this commit answers `SIM REPLAY` with `recorded before
replays existed`, and a match voided at a reload is recorded `restart`, which
writes no replay. Web clients see the `arena.replay` feed in their hello
only after a gateway restart, and draw it only after 16b cut I.2. From 16b cut
I.2's commit screen 0 carries the replay on the TUI and the web, a
gateway-half change: the fresh `web/dist` and `GATEWAY_RESTART=1`. From 16b cut
I.4's commit a staffer's `@arena/watch` draws a running match live on that same
screen over the new `arena.observer` feed, again a gateway-half change (the
fresh `web/dist` and `GATEWAY_RESTART=1`, which is what puts the feed in the web
hello), with no pack and no config key.

**Snapshots (16b cut AF.2).** A running match now writes two documents to the
store every `[tacsim.arena].snapshot_ms` of arena time (default 5,000):
`tacsim:snapshot:<match>` and `tacsim:live:<match>`, about 215 KB together as
stored for a twelve-tank match a minute into a fight (a 169 KB snapshot and a
45 KB live record, measured). The live record grows with the debriefs and the
replay: twenty-four full debriefs and a full replay put it at an estimated
1.4 MB, reached only late in a long, large match. Both are overwritten in place
and deleted with the result; `snapshot_ms` is the lever (a longer cadence
writes less, `0` writes none).

**Resume (16b cut AF.3).** From the first reload whose running half is 16b cut
AF.2 or later, a server-half reload resumes a running match from its last
snapshot (every five arena seconds; the match rewinds to it, every tank flies
on its sealed mind until its pilot boards again and reads what they missed), and a
full gateway restart resumes it too once the players reconnect. Four things
void it instead, recorded `aborted(restart)` with the reason in the log: a
deploy that changes the snapshot format (a key added under `[tacsim.arena]`,
`[tacsim.match]` or `[tacsim.sensors]` is one), the state hash's folds, the
proving ground's digest or the locomotion table (most arena cuts do one of
these — read the cut's note); `snapshot_ms = 0` under `[tacsim.arena]`, which
turns snapshots off; a server that dies again before the resumed match's next
snapshot, which is not resumed a third time; and a store that lost the tail of
the tape or half of the two documents.

After the first deploy of AF.3, start a match, reload once and read
`@arena/status`'s `resumed:` line — the one check the in-memory rigs cannot
make.

**Unit minds M6 compatibility.** M6 uses snapshot format 13, live-match record
format 2, cockpit frame 6 and gateway IPC 7. Rebuild `web/dist` and deploy the
matching server and gateway together (`GATEWAY_RESTART=1`). Finish existing
sorties before upgrading: older live records and snapshots are refused, with no
in-place migration. Mind API 4 changes Standard's source identity even though its
executable body remains stock behavior version 2. The immutable source and Roll
records must remain available with retained seals and replay tapes. Recovery
refuses oversized or damaged source/tape documents; unresolved tape references
block revision pruning until a repaired boot can reconstruct their roots.

The default input tape budget is 64 MiB per match and the retained tape cache is
128 MiB. These are encoded-data bounds; the historical measurements above are
not predictions for authored minds or large matches. See
[the M6 plan](../docs/design/17d-unit-minds-through-m6-plan.md) for the configured
limits and final acceptance evidence.

### The first staff login

Stop the instance, then run the current `mush-server` (or all-in-one `mushd`)
as its service user with `--bootstrap-staff <account>`. It boots the configured
world without opening listeners, creates one durable account linked to God `#1`,
and exits. It refuses an in-memory backend, invalid credentials, a live world's
lifetime lock, or **any existing account**, including an imported legacy account.
The empty-store check and account insertion are one atomic store operation, so
concurrent attempts cannot create two first staff accounts.

Only the account name goes on argv. The password is one redirected stdin line,
4–128 bytes; direct terminal stdin is refused to avoid an echoed secret. In Bash:

```bash
sudo systemctl stop dev1mush-gateway
sudo -v
set +x
IFS= read -rs -p 'Staff password: ' staff_password; printf '\n'
printf '%s\n' "$staff_password" | sudo -u mush \
  /opt/rustmush/dev1/build/target/release/mush-server \
  --config /opt/rustmush/dev1/mushd.toml --bootstrap-staff keeper
unset staff_password
sudo systemctl start dev1mush-gateway
```

Check that bootstrap succeeded before starting the service. A password manager
can supply stdin directly. The existing argon2id helper hashes the password; no
manual PHC-string generation or `accounts.json` edit is needed. At the welcome
screen, `connect keeper <password>` attaches God. Normal players register with
`create <name> <password>` afterward. The legacy import remains supported, but
bootstrap never replaces an existing account or password.

## Named launch and the player market (17g C1)

**Until `[tacsim.minds].launch_path` is live for an instance, that world
composes no mind commerce at all**: no House chain (17g C0's 365-day owner
term included) and no player market. The templates ship the key commented
(`instance/mushd.toml.tmpl`), because a live path on a fresh store refuses the
boot: the document names organizations, accounts, rooms and characters that only
a booted world can create, and a named identity that does not exist is a boot
refusal. Making the path live per instance is an owner decision this cut does
not make: either an empty value meaning unset, or a per-instance overlay.
`instance.env` cannot say "unset" today, because an empty path is the boot
refusal `launch deployment path is empty`.

**What format 2 adds.** A format-1 document (`"format": 1`) composes the House
chain alone, exactly as before C1. A format-2 document adds a `market` block
(`instance/launch.json.example`): the clerk, the live character that runs
pending-charter settlement and paid-resale terminal steps; the Registry's
`registry_authority` (its organization, its `review` scope and 1 to 32
operators, in ascending order); and the `terms`: the personal Pin's price and
renewal price, the quote window, the personal renewal offer window (IC
seconds), the paid-resale offer window (real seconds) and the pending-charter
settlement window (IC seconds). The Active boot then composes the player market
after the House chain: personal filing and listing (v2 and v3), forks,
dependent filing, entitlements and their renewals, line updates, charter
transfers and pending charters, paid resale, Registry recall and revocation,
withdrawal, owner attestation and parent re-qualification. The boot checks that
the Registry organization exists, that its scope belongs to it, and that the
clerk and every operator are characters; it does not check their status or
their grants.

**Bring-up, from a fresh store** (C0 already requires one at the next deploy).
Every id below is the one your world printed; replace every id in the example.

1. Boot with no `launch_path`.
2. As #1, create the Republic and three system accounts: the Registry's, the
   counter's, and a faucet that only funds the House treasuries in step 3:

   ```
   @faction/create Republic
   @account/system launch.registry=tacsim.credit
   @account/system launch.venue=tacsim.credit
   @account/system launch.faucet=tacsim.credit
   ```

3. Create the three Houses (`Meridiani`, `Coprates`, `Tycho`). For each, with
   the publisher's character as `#<publisher>` (the tests use #1):

   ```
   @org/create Meridiani
   @org/scope create Meridiani=publication
   @account/open org:Meridiani/<publication scope>=tacsim.credit
   @org/grant #<publisher>=Meridiani roll.mind.publish scope=#<publication scope>
   @org/grant #<publisher>=Meridiani asset.title.transfer scope=#<publication scope>
   @org/grant #<publisher>=Meridiani economy.account.post scope=#<publication scope>
   ```

   `roll.mind.publish` is the House publication capability
   (`mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY`). Each
   treasury pays its House's filing fee (100cr) when the House files; the tests
   mint exactly one fee into each from the faucet of step 2, naming both by
   the account numbers the world printed
   (`@econ/mint <faucet account>=<treasury account> 10000`).
4. Create the Basin importer the hardware coordinates name, and the counter
   room, as `crates/tacsim/src/hardware/flow/launch.rs` (`setup_with`) and
   `crates/mush-server/src/world_task/submission_disposition/tests/player_market.rs`
   (`launch_world`) do:

   ```
   @dig Named launch counter
   @create Basin fitting shop
   @create Basin completed work
   @create Basin die depot
   @create Basin agent stock one
   @create Basin agent stock two
   @org/create Basin
   @org/scope create Basin=die-stock
   @org/scope create Basin=factory
   @vault/create HardwareStock0=container:#<die depot> owner=org:Basin scope=<die-stock scope>
   @vault/create HardwareStock1=container:#<agent stock one> owner=org:Basin scope=<die-stock scope>
   @vault/create HardwareStock2=container:#<agent stock two> owner=org:Basin scope=<die-stock scope>
   @account/open org:Basin/<die-stock scope>=tacsim.credit
   @account/open org:Basin/<factory scope>=tacsim.credit
   @facility/open <fitting shop>=2 core_integration core_refit
   @facility/owner <fitting shop>=org:Basin
   @asset/title #<fitting shop>=org:Basin scope=<factory scope>
   ```

   The second origin agent is the first character of a second login, which
   is not #1's (`keeper`): at the welcome screen,
   `create LaunchAgent <password>` makes that login and its character
   `LaunchAgent`. Then, as #1, for each of the two origin agents (#1 and
   `LaunchAgent`):
   `industry.core_die.issue`, `vault.deposit` and `vault.withdraw` with
   `scope=#<die-stock scope>`, and `industry.core.integrate` and
   `industry.core.refit` with `scope=#<factory scope>`, each by
   `@org/grant #<agent>=Basin <capability> scope=#<scope>`.
5. `@org/create Registry` and `@org/scope create Registry=review`.
6. Connected as `LaunchAgent` on that second login, create the clerk and the
   operator with `@newchar MarketClerk` and `@newchar RegistryOperator`
   (`@newchar` adds a character to the caller's own login, and a character
   name is letters, digits, `_` and `-` only, so no spaces). This keeps the
   clerk and the operator off #1's login, exactly as the tests place them
   (`launch_world`); the boot checks only that each is a character.
7. Write `launch.json` from `instance/launch.json.example` with
   `"mode": "initialize"` and the real ids. `registry_issuer` names the system
   principal the launch issues under (the example's `hardware.factory`); choose
   it once per world.
8. Make `launch_path` live for that instance (the owner's choice, above) and
   boot. It boots Initialize.
9. For each operator, as #1:

   ```
   @org/grant #<operator>=Registry roll.registry.recall scope=#<review scope>
   @org/grant #<operator>=Registry roll.registry.revoke scope=#<review scope>
   @org/grant #<operator>=Registry roll.registry.inspect scope=#<review scope>
   @org/grant #<operator>=Registry roll.registry.fine scope=#<review scope>
   @org/grant #<operator>=Registry roll.registry.claims scope=#<review scope>
   ```

   The third grant lets the operator read a hull with `@registry/inspect #<hull>`. The fourth
   lets them fine the holder of a hull an inspection found ungoverned with
   `@registry/fine #<hull> <amount>` (the amount in credits, up to
   `[tacsim.minds].fine_cap_minor`); the fine is one obligation on the holder's account, due
   at once, paid to the Registry's levy account. The fifth lets them read the quirk claims
   authors have filed with `@registry/claims`; a claim is testimony, and only an inspection
   establishes anything.

10. As #1, `+act roll-initialize-commerce with {}`, then the three industry
    initialize actions the same way: `initialize-core-die-stock`,
    `initialize-core-factory` and `initialize-core-refit`.
11. Set `"mode": "active"` and restart.
12. As #1, read `@mind/launch`. After the launch help (several lines), one line
    beginning `Player market:` states the player market's prices and windows.
    The last line names every operator in ascending order, in one line:
    `Registry operators: #<op> ready.`, or with two operators
    `Registry operators: #<a> ready, #<b> ready.`. An operator shown with
    `lacks` and one or more of `recall`, `revocation`, `inspection` and `fine` (for
    example `lacks fine`, or `lacks recall, revocation, inspection and fine`)
    is missing those step-9 grants (re-run each `@org/grant` it names), or the
    Registry is suspended or its scope retired.

**Also:**

- The operators default to one, the owner's staff character: step 6's
  `RegistryOperator`, or any other character the owner names except #0. The
  clerk is a character of its own and never #1 (the document refuses #0 and
  #1 as the clerk).
- Retiring the Registry's `review` scope (`@org/scope retire Registry=#<scope>`)
  or suspending the Registry organization is the kill switch for recall and
  revocation: both stop being offered at once, and the world still boots.
- Destroying an operator or the clerk object refuses the next boot, as a
  destroyed House publisher does, so edit `launch.json` first.
- Every price and window in the example is a provisional default (17g §1
  ruling 6). Personal term and Grace (90 and 3 IC days), the Registry levy (5%)
  and the filing fee (100cr) are doc 17's and are not in the document.
- A terms change reboots: with the market still empty, an initialized world
  whose `launch.json` changes `price_minor` and `renewal_offer_ic_seconds`
  boots Active on the new terms, `@mind/launch` prints them, and every offer
  is listed as before (`named_launch_market_terms_change_across_a_reboot`).
  A change after filings, listings, quotes or licences exist is untested:
  those records pin the exact policies they were made under, so treat a
  terms change on a world in play as needing a fresh store until it is
  measured.
- A world initialized under format 1 is not promised to boot under format 2:
  start the market on a fresh store. A reset removes the world the ids name, so
  take `launch_path` out of the live config before a reset and bring the
  launch up again from step 1.

**House procurement (doc 17h cut M8f), optional.** A named House buys mind licences from the
player market's listings once per IC calendar month through a procurement agent. For each House
that should buy, before booting with the policy:

1. Create the agent as a character on an operator-held login account of its own (never #1's, the
   clerk's or an operator's: the Quote refuses two characters of one account, so its account must
   differ from every seller's). The login is NEVER played: the agent only ever sees the
   `tacsim_procurement` offer the tick queues for it and that offer's refusals.
2. Stand it in the venue room (the counter): the market's Quote, Fund and Grant need it there.
3. As #1: `@org/grant #<agent>=<House org> roll.holder.org limit=<monthly budget in minor>` on the
   House's root scope (no `scope=`), so the grant itself caps any one buy at the month's budget.
4. With a backing faction, make the agent a member of it (the standing rules read membership).
5. Add a `procurement` entry to `launch.json`: `{"org", "scope" (the House org's ROOT scope),
   "agent"}`, and optionally `budget_minor_per_ic_month`, `cap_permille`, `standing` and `roles`
   (`sealed`, `open`, `open_resale`). The boot refuses an agent that is not a live character, a
   scope that is not the House's root and a faction that does not exist. `[tacsim.procurement]`
   holds the budget's and the cap's defaults.

A purchase whose Grant and Refund both keep failing (for example a revoked agent) pages
`escrow held` in the server log; staff settle that escrow by hand.

**House owner attestation (doc 17h cut M8h).** A House filing's free owner licence runs for the
House term (365 IC days) and lapses unless it is attested within Grace after the term ends. The
`tacsim.house_attestation` tick attests each due House filing unattended, as the House's
attestation agent, so the term renews every 365 days. Without an agent, a House's filings lapse
at the end of their first term. For each House:

1. The agent is the House's procurement agent when the House has a `procurement` entry; nothing
   more is needed for it but step 3. Otherwise create one as for procurement step 1 (a character
   on an operator-held login account of its own, never played), and name it in that House's
   `launch.json` entry as `"attestation_agent": <dbref number>`. Naming a different character
   beside a procurement agent refuses the boot, as does an agent that is not a live character.
2. It need not stand anywhere: the attestation is not a market step.
3. As #1: `@org/grant #<agent>=<House org> roll.holder.org scope=#<House scope>` on the House's
   scope (the `scope` of its `launch.json` entry, where its filings were made). The attestation
   moves no money, so a procurement agent's `limit=` grant also attests, but only when that grant
   covers the House's scope.

An agent without the grant pages `grant` in the server log
(`tacsim.house_attestation.refused`) and attests nothing. A House filing that flew on an
ungoverned core during its term is never attested: it logs
`tacsim.house_attestation.unattestable` once and lapses, which is the consequence.

Registry operators recall a filed revision with `@registry/recall <filed>` and revoke a licence
term with `@registry/revoke <licence>` (17g C2d). Anyone the deployment does not name, or whom staff
have not granted both capabilities, gets the same refusal as for a number that does not exist; check
readiness with `@mind/launch` as #1. `@mindrenewal` and `@housemindrenewal` remain the renewal
service's own commands; players use `+minds/renew`.
