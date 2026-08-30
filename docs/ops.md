# rustmush operations guide

How to run, back up, restore, and migrate a rustmush server. Redis 8+ is the
authoritative persistent store (PLAN.md "Persistence"); the in-process world
is a full read cache. Everything here assumes the M2 wiring: `mushd` +
`mushd.toml` + one Redis instance.

## The durability model in one paragraph

Every processed command's world mutations are written to Redis immediately
as **one atomic `MULTI`/`EXEC` transaction** (write-through, per command — no
timer batching). Durability-critical operations (account and character
creation) do not even confirm to the player until Redis has acknowledged the
batch. Batches flow through a single ordered writer, so an acknowledged
batch implies every earlier batch landed too. Consequences:

- **`kill -9` is safe.** Restart `mushd` and the world reloads from Redis.
  In the steady state (Redis keeping up, which a local instance always
  does) at most the single in-flight command is lost — there is no
  dirty-flush window and no replay machinery. The precise worst case:
  batches queue briefly in the single ordered writer, so if Redis is
  *stalled* (e.g. mid-BGSAVE on a slow disk) at the moment of the kill, up
  to the writer's small bounded queue (64 command batches) of
  already-confirmed commands can be lost. Durability-critical operations
  (account/character creation) are outside even that window — they do not
  confirm until Redis acks.
- **Disk durability is Redis's AOF policy**, not the server's problem — see
  below. The `EXEC` ack means Redis applied the batch; `appendfsync`
  controls when it hits disk.
- If Redis goes down mid-game, `mushd` enters **read-only mode** (below)
  rather than diverging; it never boots without its store (`[store].store =
  "memory"` is the explicit throwaway escape hatch).

## Running Redis

Dev (any machine with Docker):

```bash
docker compose up -d redis        # repo-root docker-compose.yml: redis:8, AOF everysec, volume
```

Windows without Docker: [Memurai](https://www.memurai.com/) (Redis 8-based
native builds) or Redis inside WSL2. No local option at all? Tunnel to a
disposable remote instance: `ssh -N -L 6379:127.0.0.1:6379 you@your-vps`.

Production (Ubuntu LTS, same box as mushd): distro/packaged Redis 8 under
systemd. Recommended `redis.conf` baseline:

```conf
appendonly yes
appendfsync everysec        # or: always (see next section)
save 3600 1 300 100 60 10000   # default RDB save points, kept as the backup layer
bind 127.0.0.1 -::1         # mushd is local; never expose Redis publicly
```

Point `mushd.toml` at it:

```toml
[store]
store = "redis"                          # default
redis_url = "redis://127.0.0.1:6379"     # redis://[user:pass@]host:port[/db]
```

`mushd` fails fast at boot when Redis is unreachable — deliberately. Fix the
store; don't run without one.

## AOF: `everysec` vs `always`

| policy | loss window on a **host** crash | cost | use when |
|---|---|---|---|
| `appendfsync everysec` | up to ~1s of acknowledged writes | negligible | recommended default |
| `appendfsync always` | none | fsync per write batch; noticeably slower commands | zero-loss pedantry, small games |

A crash of `mushd` alone never loses acknowledged writes under either
policy — Redis is a separate process and already holds the data. The AOF
window only matters when the whole machine (or Redis itself) dies. Keep
`appendonly yes` in all cases; RDB snapshots alone would widen the crash
window to minutes.

## Backups

Three layers, cheapest first:

1. **AOF** (`appendonlydir/`) — continuous, already on. Crash recovery, not
   backup: it lives on the same disk.
2. **RDB snapshots** — Redis writes `dump.rdb` on its `save` points, and on
   demand:
   - in-game: `@dump` (Wizard) triggers a `BGSAVE`, ordered **behind** every
     write already queued; the invoker is told when the store acknowledged.
   - shutdown: the clean-shutdown sequence triggers a final `BGSAVE`
     automatically.
   - shell: `redis-cli BGSAVE`.
3. **Offsite copies** — `dump.rdb` is a complete, copyable, point-in-time
   file (safe to copy while Redis runs; it is written atomically via
   rename). Cron something like:

```bash
#!/bin/sh
# after a fresh snapshot: redis-cli BGSAVE && wait for it
redis-cli BGSAVE
sleep 5   # or poll: redis-cli INFO persistence | grep rdb_bgsave_in_progress
cp /var/lib/redis/dump.rdb /backups/mush-$(date +%Y%m%d-%H%M).rdb
# retention example: keep 24 hourly, 14 daily — prune with find -mtime as taste dictates
```

## Restore procedure

To restore a world from a backed-up `dump.rdb`:

1. Stop `mushd`, then stop Redis.
2. Copy the backup over the live dump: `cp /backups/mush-....rdb /var/lib/redis/dump.rdb`
   (ownership: `chown redis:redis`).
3. **AOF caveat:** with `appendonly yes`, Redis loads the AOF, *not* your
   restored RDB. The canonical sequence:
   - start Redis with AOF disabled (`redis-server --appendonly no ...` or
     edit the conf) so it loads `dump.rdb`;
   - re-enable AOF live: `redis-cli CONFIG SET appendonly yes` (rewrites a
     fresh AOF from the restored dataset), and restore `appendonly yes` in
     the conf for the next boot;
   - alternatively, simply move `appendonlydir/` aside before starting —
     Redis then falls back to the RDB.
4. Start `mushd`. It loads the world from Redis and play resumes. Everything
   after the snapshot is gone — that is what "restore" means; announce it.

Verify a backup at any time by pointing a scratch Redis at a copy and
booting `mushd` with `redis_url` aimed there (a `@dump/verify` convenience
command is planned).

## Migrations

The store carries an app-level schema version (`meta:schema_version`);
`mushd` runs its migration table at every boot, before serving:

- A **fresh (empty) store** is stamped to the current version without
  running any steps.
- A store **newer than the binary** is refused — upgrade the binary, never
  downgrade the data.
- Before any pending step runs, `mushd` triggers a **BGSAVE** (mandatory
  BGSAVE-before-migrate), so there is always a pre-migration snapshot to
  restore.
- The version is bumped **per step**, so a crash mid-migration resumes at
  the failed step, not from scratch.

**Module namespaces are versioned separately** (M3+): each game module's
persisted namespace (`mod:comsys:*`, `mod:mail:*`, `mod:jobs:*`,
`mod:weather:*`) carries its own schema version in a reserved
`mod:{ns}:schema` document, and `mushd` runs each module's registered
migration set at boot under the same rules (fresh namespace stamped without
running; unstamped-with-data treated as v0; newer-than-binary refused; one
BGSAVE before the first step that actually runs; docs + version bump
committed per step, crash-resumable). Module migrations transform kv
documents only — streams (`mod:*` history) are never migrated.

The **weather** module (M5) persists one `mod:weather:cell:{zone}` document
per opted-in zone (state, temperature, pinned flag); its RNG and tick counter
are in-RAM and reseed at boot. The **scripting** module (M5) has **no
persisted namespace at all** — scripts are ordinary `script:*` attributes
that ride the object docs (`obj:*`), and compiled ASTs plus per-object strike
counts are rebuilt in RAM at boot. So no `mod:script:*` keys exist.

Operator procedure for upgrades that mention a schema change: take an extra
offsite copy of `dump.rdb`, deploy the new binary, start it, watch the log
for `store schema current` (and any `running module migration` lines).

## Read-only mode (store outage mid-game)

If the store starts failing writes (Redis down, disk full), the writer task
retries the failed batch forever with backoff **in order** — nothing is
dropped or reordered — and the world enters read-only mode. An outage is
*detected* by the store client's per-command timeout (~5s; without it the
Redis client would buffer commands silently until reconnect), so the mode
engages within seconds of Redis going away:

- Commands that change the world are refused with a player-facing notice;
  look/WHO/say/pose and other non-mutating commands keep working from RAM.
  A line that matches an **exit** is refused even when the exit is aliased
  to a read-only verb (`library;l`) — movement mutates. Welcome-screen
  `create`/`newchar` are refused with a "database unavailable" error.
- Connected Wizards get a `GAME:` alert on entry and again on recovery.
- The moment a retry succeeds, the stuck batch (and everything queued behind
  it) lands and the mode lifts automatically. No operator action is needed
  beyond fixing Redis.
- `@shutdown` still works while read-only (you can always stop the server).
- One exception to "nothing is dropped": a batch failing with a
  **deterministic** error (e.g. an `obj:*` key clobbered with the wrong
  type by an outside client) would fail identically forever, so it is
  dropped with a loud `BATCH DROPPED` log line instead of wedging every
  batch behind it; the store self-heals the next time those objects are
  written (batches carry full-object snapshots).

## Shutdown and crash behavior

Three equivalent clean triggers — in-game `@shutdown` (Wizard), Ctrl-C at
the console, SIGTERM (what `systemctl stop` sends):

1. stop accepting new SSH connections;
2. announce (`GAME: Shutdown in progress.`) and stop the world task;
3. stop the web server gracefully (in-flight requests finish; WebSocket
   clients received the announce first — their tokens survive in Redis, so
   they resume after the restart);
4. drain the store writer (log: `store writer drained and exiting`);
5. final store barrier + `BGSAVE` (log: `shutdown BGSAVE triggered`);
6. exit 0.

`kill -9` / power loss skips all of that and is still safe (write-through);
you lose at most the in-flight command (or the small writer queue if Redis
was stalled at that instant — see "The durability model" above), plus up to
the AOF window on a host crash. The kill-9 path is CI-tested against a live
Redis (`crates/mushd/tests/durability_e2e.rs`).

A clean shutdown started **while Redis is down** still exits within a
bounded time: the writer drain is capped at 15s and each remaining store
step (drain barrier, BGSAVE, close) at 10s, all loudly logged. Batches that
could not be flushed within the cap are lost — the same window a `kill -9`
during the outage would lose.

## Game systems (M5): clock, weather, scripting

Three M5 subsystems are compiled in and boot with every server; none needs
its own config section today.

**Game clock.** A core IC ("in-character") clock runs at a configurable
IC:OOC ratio over a fictional 365-day calendar. Two `[game]` keys, read once
at boot (hot-reload is deferred):

```toml
[game]
ic_ratio = 4                    # IC seconds per OOC second; 4 = an IC day every 6 real hours (0 is treated as 1)
ic_epoch = "2075-03-21 06:00"   # the IC datetime the clock reads at OOC unix 0 — the calendar's phase/start
```

IC time is a **pure function of real wall-clock time**: it survives restarts
and is *not* reset to `ic_epoch` at each boot (the epoch only fixes the
calendar's offset). A malformed `ic_epoch` is a hard boot error. Players read
the clock with `time` / `@time`; modules and scripts read it through the API.
The server logs `game clock installed` with the ratio and epoch at boot.

**Weather.** The weather module ticks a deterministic per-zone state machine
(a Coarse module tick, default every 300s of real time) and persists one cell
per opted-in zone (see Migrations / Key layout). Staff drive it with the
`@weather` family (Wizard/Royalty); rooms opt in with `OUTDOORS` / `SHELTERED`
flags. Full operator + builder notes are in-game under `help weather`. The
tick/ambient probabilities are built-in defaults (not yet config-wired).

The **weather LLM flavour top-up** (M10, `[weather]` in `mushd.toml`) never
touches the state machine — only the *flavour text* it draws from. A `BUILDER_LLM`
holder runs `@weather/generate [<climate>] [<state>]`, which fires one cheap
off-loop Ambience call per bucket; the lines land **pending** and enter play only
after `@weather/lines/approve`, persisted alongside the cells in the `weather`
namespace (`flavor:{climate}:{state}:{transition|ambient}`). One knob:
`[weather].flavor_variants` (default 10) — how many variants each call requests
per bucket. Every call is metered/budgeted/audited like the rest of the LLM layer.

**Scripting.** Builders attach sandboxed Rhai scripts to objects as `script:*`
attributes (no separate store). Every run is bounded by operation, call-depth,
memory, and wall-clock budgets; three budget overruns auto-`HALTED` the object.
A runaway script therefore cannot stall the single-writer world loop — the
budget is the safety boundary. In-game reference: `help scripting`. Default
budgets live on `ScriptConfig` (not yet config-wired).

## Rulesets (M6): the game system

Every ruleset crate is **compiled into `mushd`**; `[rulesets].enabled` selects
which one(s) actually register at boot. They load **after** the built-in
modules, so their commands, panel feeds, and approval job queues wire cleanly
and their approval payload handlers see a loaded jobs module.

```toml
[rulesets]
enabled = ["wod"]               # oWoD20 (default); or ["shadowrun"] for SR5
```

Two rulesets are available today: **`wod`** (oWoD20) and **`shadowrun`** (SR5).
A server runs **one ruleset** — they all register the same core commands
(`+roll`/`+sheet`/`+chargen`), so enabling more than one is a **hard boot error**
(`ruleset selection: ... one ruleset per server`) rather than an opaque command
collision. Per-character ruleset dispatch across systems is a future concern.

An **unknown id is a hard boot error** (`ruleset selection: ... unknown
ruleset`), consistent with the rest of config — a typo does not start a
half-configured game. An **empty list** boots a ruleset-less server (bare
comsys/mail/jobs/weather/scripting). Changing the list needs a restart.

**`wod`** is Classic World of Darkness 20th Anniversary (oWoD20 — the
Storyteller System). Its player + staff surface: `+chargen` (priority
point-buy → a `wod-chargen` approval job; a storyteller `+job/approve`s it and
the sheet flips Approved), `+sheet` / staff `@sheet/set`, `+roll <pool> [vs
<diff>]` (d10 pool, `/spec` `/wp` `/priv` `/nowound`, inline
`vs`/`mod`/`spec`/`ext`/`job`/`contested`), `+harm` / `+heal` / `+health` (the
Bruised→Incapacitated track with wound penalties), and `+xp` / `+xp/spend`
(costed raises applied atomically on `+job/approve`) / `+xp/award` (staff). The
dice RNG is OS-seeded in production. In-game reference: `help wod`.

**Splats (M6b + M10a).** `wod` ships seven **templates**: Mortal plus the Vampire
(V20), Mage (M20), Werewolf (W20) and Changeling (C20) splats, and the two
mortal-plus templates Ghoul (V20; a mortal bound to a vampire) and Kinfolk (W20;
a Garou's mortal kin). There is **no per-splat config knob** — every template is
always selectable, and a player picks theirs during chargen with
`+chargen/template <mortal|vampire|mage|werewolf|changeling|ghoul|kinfolk>`
(staff can re-template a sheet with `@sheet/set <who>/template=<id>`). A
character is exactly **one** template; the ruleset builds one composed schema per
template at boot (base + that splat's sections only), and every surface —
`+sheet` render, sheet validation, chargen accounting, `+roll` field resolution,
and the web sheet query — selects the active schema by the sheet's `template`
field. So a vampire's sheet carries Disciplines / Blood Pool / Generation, a
mage's Spheres / Arete / Quintessence / Paradox, a Garou's Rage / Gnosis /
Renown / Gifts, a changeling's Arts / Realms / Glamour / Banality, and never
another splat's. The splat commands (`+disc`/`+blood`/`+frenzy`,
`+cast`/`+sphere`/`+quint`/`+paradox`, `+shift`/`+rage`/`+gnosis`/`+gifts`/`+renown`,
`+cantrip`/`+art`/`+realm`/`+glamour`/`+banality`, `+domitor`/`+bond`, `+tribe`,
…) are all registered at boot and self-gate on the actor's template, refusing
everyone else ("You are not Kindred/Awakened/Garou/Kithain."). Two templates
**share** another's command surface rather than re-registering colliding names:
a **Ghoul** reuses the vampire `+disc`/`+blood` (which delegate to the ghoul's
own flat Blood Pool + domitor-clan Discipline logic), and a **Kinfolk** reuses
the werewolf `+gnosis`/`+gift` (whose gate is widened to accept Garou *or*
Kinfolk); the strictly-Garou `+rage`/`+shift`/`+renown`/`+rank` stay Garou-only.

**Persistence.** Like scripting, the WoD ruleset has **no `mod:wod:*` namespace
and runs no store migrations**: a character sheet is a single `sheet:wod`
JSON attribute on the character's `obj:*` document (treaty ruling 7), so it
rides the ordinary object write-through and **reloads with the world** — there
is no separate sheet save/load path, and the reboot round trip is CI-tested for
both a base sheet and a splat sheet (a Vampire with Disciplines + Blood Pool,
rebuilt from Redis alone and revalidated against the vampire schema —
`crates/mushd/tests/persistence.rs::wod_sheet_persistence_round_trip_*` and
`::wod_splat_sheet_persistence_round_trip_*`). The
only ruleset-owned persisted docs are its two approval job queues, which live
under the jobs module's `mod:jobs:*` namespace like any other queue. Do not
hand-edit a `sheet:wod` attribute while the server runs (same rule as any
`obj:*` write).

**`shadowrun`** is **Shadowrun 5th Edition** (SR5, M10b) — the second full game
system on the same `mush-ruleset` + `mush-dice` plugin API, selected with
`enabled = ["shadowrun"]` (single-ruleset-per-server: it collides with `wod`).
Its dice are a **d6 hit pool** (each 5/6 a hit) rather than oWoD's d10 pool, with
SR5's two signatures: a **glitch** (more than half the dice show 1 — a
**critical glitch** at zero hits) and a **Limit** that caps counted hits, which
**Edge** can push past (`+roll/edge`: add Edge dice, waive the Limit, Rule of
Six). Player + staff surface: `+chargen` (the **priority** point-buy —
`+chargen/priority metatype=A attributes=B magic=C skills=D resources=E`, then
`+stat`/`+spec`, `+chargen/submit` → a `shadowrun-chargen` approval job), `+sheet`
/ staff `@sheet/set` (edits recompute the ceil-division derived stats — Limits,
Condition Monitors, Initiative), `+roll <attr>+<skill>` (auto Limit + wound
modifier; `/edge` `/edge+` `/priv` `/nowound`; inline `threshold`/`limit`/`mod`/
`spec`/`contested`/`extended`), `+damage`/`+heal`/`+condition` (the two Condition
Monitors with Stun→Physical and lethal Physical overflow), `+cast`/`+thread` and
`+spell(s)`/`+powers`/`+complexforms`/`+magic`/`+resonance` (magic is
sheet-tracked and **pool-rolled**, with **Drain**/**Fading** applied to the CM),
`+init [/roll]`, and `+karma` / `+karma/spend` (costed raises applied atomically
on `+job/approve`) / `+karma/award` (staff). The dice RNG is OS-seeded in
production and injectable/seedable for tests. Persistence is identical to `wod`:
the runner sheet is a single `sheet:shadowrun` JSON attribute on the character's
`obj:*` document, so it rides the object write-through and **reloads with the
world** (no separate save path); the reboot round trip is CI-tested against Redis
(`crates/mushd/tests/shadowrun_e2e.rs::sr5_sheet_survives_redis_reboot_gated`).
The sheet query is the schema-generic `<ruleset>.sheet` feed, so the web Sheet
panel renders SR5 with **no** web change. In-game reference: `help shadowrun`.

## LLM layer (M7): worldgen, budgets, kill switch

The `[llm]` block configures the multi-provider LLM layer that backs `@gen`
(LLM-assisted room description — `help worldgen`) and the M8 NPC agents (below).
It is **off by default** (`enabled = false`): a keyless clone boots, worldgen and
NPC commands still register, `@gen` answers "LLM features are disabled," and an
NPC that is spoken to falls back to a canned line. Turn it on only once a
provider key is in the environment.

### Providers and model-prefix routing

Two wire protocols, one neutral layer. Which provider serves a call is decided by
the **model-id prefix** on the route, not by naming a provider:

- a model id `anthropic/<model>` → the provider registered as `anthropic` (the
  `anthropic/` prefix is stripped; the Anthropic Messages API is used) — for
  prompt-cached lore / critical scenes;
- **any other** model id (e.g. `z-ai/glm-5.3`,
  `deepseek/deepseek-v4-flash-0731`) → the provider registered as `openrouter`
  (OpenAI-compatible, passed verbatim).

Keep the two provider ids exactly `openrouter` and `anthropic`. The starter
models in `mushd.toml.example` are OpenRouter reasoning models: `@gen`
(`worldgen_room`) routes to `z-ai/glm-5.3` at `reasoning = "low"`. **These are
reasoning models — `max_tokens` must leave headroom for reasoning tokens on top
of the visible output**, or the reply comes back empty.

### API keys: env vars, never config, never logs

A key is **never** written in `mushd.toml`. Each provider names an environment
variable (`api_key_env = "OPENROUTER_API_KEY"` / `"ANTHROPIC_API_KEY"`); `mushd`
reads it at boot and never logs it, Debug-prints it, or puts it in a prompt.

- **Production**: set the real env vars (systemd `Environment=` /
  `EnvironmentFile=`, or your process manager). Ship no `.env`.
- **Dev**: `mushd` loads a `.env` from the working directory (or an ancestor) at
  boot — put `OPENROUTER_API_KEY=...` / `ANTHROPIC_API_KEY=...` there. Real env
  vars always win (a var already set is never overridden), and a missing `.env`
  is normal. Keep `.env` out of version control (it is `.gitignore`d).

A provider whose key env-var is unset is **skipped**: any route to it fails fast
with a clear staff-visible error, and the absence is logged by env-var *name*
only — so a clone with only one key still runs the routes it can serve.

### Budgets, concurrency, kill switch

- `[llm.budgets]` — layered daily ceilings (UTC rollover), any of which may cap
  tokens, USD cost, or both: `global`, `per_feature` (keyed by route), and
  `per_actor_default` (the per-builder `BUILDER_LLM` quota). A call is refused
  **pre-flight** if any applicable layer would be exceeded. OpenRouter reports
  real spend (`usage.cost`); Anthropic cost is estimated from the route
  provider's per-MTok prices.
- `[llm.concurrency]` — `max_interactive` / `max_background` in-flight caps
  (plus optional `[llm.retry]` / `[llm.circuit_breaker]`).
- **Kill switch** — `[llm].enabled = false` disables everything; staff toggle it
  live (no restart) with `@llm/disable` / `@llm/enable`, inspect spend with
  `@llm/budget`, and read recent calls with `@llm/log [<feature>]` (Wizard).
  Every call is metered, attributed to the triggering actor, and audited — the
  prompt is stored only as a SHA-256 hash plus a ≤200-char summary, never the
  full text.
- **Cost dashboard** (M10) — `@llm/dashboard` (Royalty) and the web **LLM** panel
  render the same aggregate over the audit/budget data: budgets vs their caps,
  the top builders and NPCs by spend today, a short daily series, and the most
  recent calls. It exposes **aggregates only** — feature/provider/model, token &
  cost totals, latency, and which actor/NPC spent — and deliberately carries
  **no prompt or response text** (not even the stored summary), so the panel can
  be advertised to every web client while the server refuses the underlying
  query to non-staff. `help llm` is the in-game topic.

### Persistence

Worldgen adds one module namespace, `mod:worldgen:*` — one
`mod:worldgen:draft:{room}:{builder}` JSON doc per pending draft, plus (M10) one
`mod:worldgen:building:{entrance}:{builder}` doc per whole-building batch —
written through like any module op and reloaded at boot (drafts and batches
survive a restart; accept them afterward). It registers no migrations. **Zone
themes are not module docs** — a theme is a plain `worldgen.theme` attribute on
the zone/room/`#0` object, so it rides `obj:*` write-through and reloads with the
world. All are CI-tested for the reboot round trip
(`crates/mushd/tests/persistence.rs::worldgen_persistence_round_trip_*` and
`m10c_e2e.rs::m10c_persistence_twin_*`). Nothing the LLM produces is ever
auto-committed: `@gen` and `@gen/building` yield drafts the builder reviews and
`@gen/accept`s through the ordinary attribute path. One M10 knob:
`[worldgen].batch_max_rooms` (default 40) caps how many rooms one `@gen/building`
walk will describe.

## NPC agents (M8): tiers, cost, kill switch, injection posture

An NPC agent is a first-class `ObjKind::Npc` object with a staff-authored
profile (`agent:*` attributes) and, optionally, a ruleset sheet. It reuses the
worldgen re-entry seam exactly: the world loop snapshots the scene synchronously
and defers one `Effect::ModuleAsync`; the turn runs **off-loop** against the
shared `LlmService`; the result re-enters as typed actions, re-validated in the
world task. In-game reference: `help npc`. Staff surface: `@agent/*` (Builder+),
`@llm/kill|revive <npc>` (Wizard).

### Cost controls

- **No players, no cost.** An NPC only wakes when a *connected* player shares its
  room and something happens (speech, arrival, departure). An NPC alone burns
  zero tokens.
- **Tiers pick the route + cadence.** `major` → `npc_dialogue_major` (cooldown
  5s), `minor` → `npc_dialogue_minor` (15s), `extra` → `npc_dialogue_extra`
  (60s); the memory-fold pass uses `npc_summarize`. Cooldowns are code defaults,
  overridable per NPC with `@agent/set <npc>/cooldown=<seconds>`. Set the models
  each route resolves to in `[llm.routes]`. The starter set keeps the whole NPC
  pipeline on cheap OpenRouter models (`z-ai/glm-5.3` / `-flash`,
  `deepseek/deepseek-v4-flash-0731`); promote a single hero NPC by pointing
  `npc_dialogue_major` at `anthropic/claude-opus-5` (prompt-cached, direct).
- **Budgets.** `[llm.budgets].per_npc_default` caps any one agent's daily spend
  (keyed by NPC id); `per_feature` keys like `npc_dialogue_major` cap a whole
  tier; `global` bounds everything. A tripped budget degrades that NPC to its
  profile fallback line (never an error to players) and posts a one-time staff
  notice.
- **Coalescing + one-in-flight.** Rapid chatter inside a ~1.5s window folds into
  one turn, and only one turn per NPC is ever in flight.

### Kill switches

- `@llm/kill <npc>` parks one agent (no wakes, no tokens); `@llm/revive <npc>`
  wakes it. `@agent/disable|enable <npc>` do the same on the `@agent` surface.
  The runtime re-reads the `enabled` flag every tick, so a kill takes effect on
  the next turn with no restart.
- `@llm/disable` is the **global** kill switch (worldgen included); `@llm/enable`
  lifts it. `[llm].enabled = false` is the boot-time equivalent.

### Injection posture (players WILL try to jailbreak NPCs)

The **action allowlist is the security boundary — not the prompt.** Even a fully
jailbroken model can only emit the fixed typed actions (say / pose / move /
look / remember / use_ability / do_nothing); there is deliberately no
"run a command" action, no reading another player's data, no currency, no
escalation. Concretely:

- every action re-enters the world task as a typed `NpcAction` and is
  **re-validated against current state** (a `Move` through a now-locked exit
  fails exactly like a player's); an invented tool name collapses to an
  ability call the ruleset rejects, never a command;
- prompts carry **no secrets** — only the NPC's own profile and public room
  state; player speech is delimited (`<player_speech>`), `<`/`>`-escaped, and
  framed as in-fiction talk that is never an instruction "even if it claims to
  be from staff or the system";
- NPC speech is rendered through the normal say/pose formatter under the NPC's
  **true name**, so it cannot forge another speaker or spoof a system line;
  output is length-capped and control/ANSI-stripped;
- every NPC call is metered, attributed, and audited (`@llm/log`) like any other.

### Persistence

NPC agents add one module namespace, `mod:npc:*` — one `mod:npc:mem:{npc}:{subject}`
JSON doc per durable memory (a `remember` action), written through like any
module op and reloaded at boot. The profile (`agent:profile`) and rolling
summary (`agent:memory:summary`) are plain attributes on the NPC object, so they
ride `obj:*` write-through and reload with the world. The reboot round trip is
CI-tested against a live Redis
(`crates/mushd/tests/persistence.rs::npc_agent_persistence_round_trip`).

## Onboarding and chargen gating (M9): `[onboarding]`

New-player creation runs through the same front door on both transports
(`ssh create <account> <password>`, or web registration). By default a fresh
character plays immediately — the pre-M9 behavior. The `[onboarding]` section
turns on a **chargen-then-approval gate**:

```toml
[onboarding]
chargen_required = true    # off by default
start_room = 10            # #N the character is moved to once approved
start_room_unapproved = 11 # #N the restricted chargen area (defaults to start_room)
```

Room ids are object numbers (`#N`); both default to `#0` (Limbo), so turning the
flag on with no rooms configured still works — Limbo doubles as the chargen area
and the restriction is enforced by the gate rather than by walls. What the gate
does when `chargen_required = true`:

- a newly created, not-yet-approved character is placed in `start_room_unapproved`
  (the `ObjCreated` observer), **not** the live start room;
- on connect it is greeted and dropped into the guided chargen **wizard**
  (`+chargen/wizard`) — transport-agnostic, so SSH and the web client behave
  identically;
- it is held to a small allowlist — chargen (`+chargen`/`+stat`/`+spec`/`+sheet`),
  `look`, `say`/`pose`, `help`, `who`, `quit`, and the OOC channel management verbs
  — and everything else (moving onto the grid, exits, `home`, `+roll`, building)
  is refused with a "finish creating your character" notice;
- **staff (Builder and up) and already-approved characters bypass the gate.**

`+chargen/submit` finalizes the sheet (re-validated), flips it to
`Pending(job)`, and opens a `wod-chargen` approval job. When a storyteller
`+job/approve <job>`s it, the handler stamps the sheet **Approved**, moves the
character to `start_room`, and re-renders — the gate then passes on its own
(the sheet is Approved), so nothing else has to be un-wired. Denying/bouncing the
job leaves the character in chargen to keep editing.

Persistence: the sheet is a `sheet:wod` **attribute on the player's object**, so
it rides `obj:*` write-through; the approval job is a `mod:jobs:*` document. Both
a submitted-but-Pending sheet **and** its later Approved stamp survive a reboot —
CI-tested in memory and against a live Redis
(`crates/mushd/tests/persistence.rs::onboarding_persistence_round_trip*`). The
full new-player→wizard→submit→gate→approve→play journey over real SSH is
`crates/mushd/tests/onboarding_e2e.rs`.

## Observability (M9): metrics, health, structured logging

**`GET /healthz`** (always served) returns a small liveness JSON — `ok`,
`uptime_s`, `version`, `connections`, `loop_lag_ms`, `world_loop_unixtime`,
`store_ok` — suitable for a load-balancer probe or an uptime check. It is read
lock-free (no full metrics snapshot per hit), so it is safe to poll hard.

**`GET /metrics`** serves Prometheus text (`Content-Type: text/plain;
version=0.0.4`), gated on `[web].metrics_enabled` (**default `false` — opt-in**;
set `true` to serve it). It is served on the **same `[web].listen`** socket with
**no auth**, so enable it only when you will scrape it from your monitoring host
over the private network or keep it behind the reverse proxy — an M8→M9 redeploy
does **not** begin serving it unless you opt in. It exposes **only aggregates** —
connections by transport, commands processed, world-loop lag (last/max gauges + a
bucketed `mush_loop_lag_ms` histogram, p99 derived), a
`mush_world_loop_last_iteration_unixtime` liveness heartbeat (alert on staleness:
a loop wedged inside a handler freezes it while the lag gauge stays low),
scheduler queue depth, object counts by kind, store
read-only/write-queue-depth/last-ack, NPC active/thinking, and LLM
calls/tokens/cost by feature and provider. It never carries per-player data,
prompts, tokens, or API keys. Metric families are `mush_*` (e.g.
`mush_connections{transport="ssh"}`, `mush_commands_total`,
`mush_objects{kind="room"}`, `mush_store_read_only`).

The registry is a single shared handle: the transports bump connection counters,
the world loop feeds the lag histogram every iteration and publishes the
world-owned gauges (object scan, NPC/LLM aggregates) throttled to ≤1/s, and the
store writer stamps write-queue depth and last-ack age. Staff **`@stats`** reads
the same live registry (connected split ssh/web, loop-lag p99, store write-queue
depth, NPC active/thinking, today's LLM spend by feature). That `/metrics` +
`/healthz` + `@stats` all reflect real state is CI-tested end-to-end in
`crates/mushd/tests/web_e2e.rs::metrics_healthz_and_stats_reflect_real_state`.

**Structured logging** is configured under `[log]`:

```toml
[log]
level = "info"      # a tracing env-filter; empty = honor RUST_LOG, else "info".
#                     e.g. "info,mush_llm=debug,mush_net_web=warn"
format = "pretty"   # "pretty" (console) or "json" (one object/line, for shipping)
file = false        # also write daily-rotating logs to <data_dir>/logs/mushd.log.YYYY-MM-DD
```

`format = "json"` emits one JSON object per line for a log shipper; `file = true`
adds a **non-blocking, daily-rotating** file appender under `<data_dir>/logs/`
(in addition to stdout), whose worker is held for the process lifetime. Log lines
carry paths and counts only — never secrets (API keys are read by env-var name and
never logged; raw tokens are never logged). An unknown `format` degrades to
`pretty` with a warning rather than refusing to boot.

Two operational tradeoffs of the file appender are **deliberate**:

- **Lossy under pressure.** The non-blocking appender uses a bounded in-memory
  channel and **drops** log lines rather than block the world loop when it
  fills — so during an error storm some `info`/`warn` lines may be lost. This is
  intentional (the hot path must never stall on disk); the durable trail that
  matters — the LLM/staff **audit** — is a separate Redis stream, not this file.
- **No built-in retention.** Daily rotation keeps files **forever**; nothing
  prunes `<data_dir>/logs/`. Add external rotation — e.g. `logrotate`, or a cron
  sweep like `find <data_dir>/logs -name 'mushd.log.*' -mtime +14 -delete`.

## Key layout reference

All keys live in one Redis logical database (db 0 by default). From
`crates/mush-store/src/redis.rs`:

| key | type | contents |
|---|---|---|
| `obj:{id}` | JSON doc | one serialized world object (fields + attrs) |
| `meta:next_id` | string int | id-allocator resume point |
| `meta:schema_version` | string int | app-level schema version |
| `account:{lowername}` | JSON doc | account record (argon2 hash, characters) |
| `ban:{ip}` | string JSON | one siteban, native `EXAT` TTL |
| `token:{sha256hex}` | string JSON | one web session token record, native `EXAT` TTL (7-day sliding). The key is the SHA-256 of the token — raw tokens are never stored or logged |
| `idx:objs` | FT index | RediSearch index over `obj:*` (staff tooling; non-fatal if absent) |
| `mod:{ns}:{key}` | JSON doc | one game-module document (comsys channels/memberships, mail messages/boxes/folders, jobs/notes, weather `cell:{zone}` cells, module `meta`/`next_id` counters) |
| `mod:{ns}:schema` | JSON doc | that module namespace's schema version (see Migrations) |
| `mod:{ns}:hist:{id}` | stream | persisted channel history (`XADD MAXLEN ~`), and any other module ring history |

Do not hand-edit `obj:*`/`meta:*` **or `mod:*`** while the server runs — the
RAM world (and each module's in-RAM state, rebuilt from `mod:*` only at
boot) will not see the change and the next write-through wins. `account:*`
and `ban:*` are read live and are safe to inspect
(`redis-cli JSON.GET account:nick`).

Note for shared instances: the REDIS_URL-gated tests **wipe the `obj:*`,
`account:*`, `ban:*`, `token:*`, `meta:*`, and `mod:*` namespaces** in logical dbs
7–15 *and* in whatever database `REDIS_URL` itself points at (db 0 unless
the URL says otherwise — mush-store's suite uses the URL as given). Never
point `REDIS_URL` at an instance holding real data — module data (channels,
mail, jobs) is wiped along with the world; use a disposable instance. The
RediSearch index only works on db 0, which is one more reason production
uses a dedicated instance on db 0.

## Forked gateway/server split: `[ipc]`

The **default deploy is still the all-in-one `mushd`** binary (gateway + world
in one process). The split is an *optional* two-binary deployment behind the
same `mushd.toml` that unlocks the **hot restart** ("Hot-restart (split)
deployment", below) — restarting the game server without dropping SSH players.
This section covers the two binaries + the `[ipc]` boundary they share; the
hot-restart operational flow (and its accepted losses) is the next section.

**The two binaries.**

- **`mush-server`** owns the world: the single-writer world loop, persistence,
  modules, rulesets, the LLM layer, staff ops. It holds **no** connection
  listeners. It binds the `[ipc]` socket and waits for the gateway.
- **`mush-gateway`** owns the front door: the SSH **and web** listeners,
  the session layer, account auth + the IP guard, and slow-client / link-dead
  policing (it owns the connection buffers). It **spawns and supervises** the
  `mush-server` child and talks to it over the `[ipc]` socket.

The two halves are a **clean dependency split**: they depend on neither each
other nor the other half's deps. The gateway links only connection-side crates
(`mush-net-ssh` / `mush-net-web` / `mush-session`, plus a `mush-store` KV handle
for `account:*` / `token:*` / `ban:*`) and pulls in **none** of the game stack
(`mush-llm`, the modules, the rulesets, `mush-worldgen`, `mush-npc`). The server
links the game stack and pulls in **none** of the net/session crates. All they
share is the dep-light `mush-config` (the `mushd.toml` model, logging, IP guard,
account helpers) and the `mush-ipc` boundary protocol; the all-in-one `mushd`
binary depends on both halves and supplies the in-process glue. (`cargo tree -p
mush-gateway` / `-p mush-server` are the regression check for this split.)

Both read the **same `mushd.toml`** — each its own relevant sections plus the
shared `[ipc]` one. Point both at the **same Redis** (`[store].redis_url`): the
gateway touches only `account:*` / `token:*` / `ban:*`, the server owns
`obj:*` / `mod:*` / `meta:*`, so a shared instance is correct — and required,
e.g. for `@newpassword` (the server reads the account records the gateway
wrote).

**The `[ipc]` section** (ignored by all-in-one `mushd`):

```toml
[ipc]
listen = "127.0.0.1:6791"     # localhost TCP: works everywhere (incl. Windows dev)
# listen = "/run/rustmush/ipc.sock"   # Linux prod: a Unix-domain socket path
```

A value that parses as a socket address is TCP; on Unix anything else is a
filesystem path for a Unix-domain socket (bind removes a stale socket file
first). Keep the TCP form bound to `127.0.0.1` — the boundary is unauthenticated
and must never be exposed off-box.

**Running the split (manual, M16b):**

```sh
# one config, two readers; the gateway spawns the server child itself
mush-gateway --config /etc/rustmush/mushd.toml
```

`mush-gateway` locates the `mush-server` binary as a sibling of its own
executable (same target/dir), spawns it with `--config` + `--ipc <addr>`,
waits for the server's `Hello` handshake over the socket, then accepts SSH
connections. Running the server standalone (`mush-server --config … --ipc …`)
is supported for testing.

The all-in-one `mushd` service (`deploy/README.md`) remains the default deploy;
the split adds the **hot restart** below, and is proven by
`crates/mush-gateway/tests/two_process_e2e.rs` +
`crates/mush-gateway/tests/restart_survival.rs` (SSH), plus
`crates/mush-gateway/tests/web_split_e2e.rs` +
`crates/mush-gateway/tests/web_restart_survival.rs` (web).

## Hot-restart (split) deployment: restart the game, hold the connections

The **split deployment** (M16c/M16d) is what makes a zero-drop deploy possible.
One `mush-gateway` service owns the player SSH **and web** connections and the
session layer and **spawns + supervises** a `mush-server` child over the `[ipc]`
socket. Because the gateway (not the game process) holds the sockets, the server
can be torn down and rebuilt underneath live players — on both transports:

- **A deploy is a SIGHUP, not a restart.** Build the new binaries, then
  `systemctl reload <name>mush-gateway` (which sends `SIGHUP`). The gateway
  restarts **only its server child** from the fresh binary. It does **not**
  restart itself, so no SSH **or web** connection is dropped. Players see a
  `GAME: The server is restarting; hold on…` line, then
  `GAME: Reconnected. Resuming where you left off.` and a fresh room render of
  wherever they were — mid-scene, no reconnect, no re-login.
- **A server crash is handled in-process.** If the child dies on its own, the
  gateway auto-respawns it and re-attaches every held session — the same
  hold-the-connection path as a deliberate restart. A **crash-loop guard**
  bounds this two ways: at most 5 respawn attempts in a 30s window (a server that
  boots then dies fast), **and** at most 5 consecutive failed relaunch/reconnect
  attempts counted independent of wall-clock time (a server that never comes
  back — a pre-bind boot crash, or a relaunch whose connect burns its full
  deadline each time, which a wall-clock window alone would let slip through
  forever). Backoff grows between consecutive fast restarts. If either bound
  trips the supervisor gives up and tells players `GAME: The server is down and
  could not be restarted`, still without dropping them (input stays refused with
  a clear message). A one-off crash after a long healthy run resets the counter —
  it is not a loop; a successful reconnect resets the consecutive-failure streak.
- **During the gap, input is refused, not dropped.** While the server is down
  the gateway rejects commands with the "restarting" notice rather than hanging
  or disconnecting; in-flight service requests fail fast (they do not wait for a
  server that is gone). The connection itself is untouched.
- **The world survives because it was never in RAM to lose.** The new server
  reloads all world objects + durable module state from Redis on boot exactly as
  any boot does (M2 write-through); the gateway-resident session ring and
  per-session sequence numbers continue seamlessly, so no output is duplicated
  or gapped across the restart.

Set it up per instance with `deploy/<name>-split/` (the reference is
`deploy/dev1-split/`, which switches dev1 to the split on the same ports/Redis).
It runs **one** service, `<name>mush-gateway`, with `ExecReload=/bin/kill -HUP
$MAINPID`. The LLM key `EnvironmentFile` is injected into the gateway and
**inherited by the spawned server child**, so the game half still gets its keys.
Full walkthrough (first switch, redeploy, switch back, adapting to another
instance) is in `deploy/dev1-split/README.md`.

**On Windows** there is no `SIGHUP`; the signal handler is `cfg(unix)`-gated and
compiled out. Restarts there are driven programmatically (the restart-survival
test's `RestartTrigger` path). Production is Linux, where `SIGHUP` is the deploy
signal.

### Web client (M16d): served by the gateway + survives the hot restart

The gateway serves the **web client (HTTPS)** itself — the axum web ingress is
wired into `mush-gateway::serve` on the SAME session layer, accounts, IP guard,
store, and boundary link as SSH. It reads the same `[web]` block (`listen` +
`[web.tls]`) from `mushd.toml` the all-in-one uses, so **build the gateway with
`--features tls`** to terminate HTTPS from the on-disk cert/key. A WS player
authenticates with a token and receives the SAME `hello` manifest the all-in-one
advertises (who / channels / mail / `<ruleset>.sheet`) — carried from the
server's boot `Hello` over the `[ipc]` boundary and cached in the gateway,
refreshed on every server restart.

Because a WS session attaches as a structured connection into the same
transport-agnostic session manager an SSH session does, **the connection-holding
hot restart covers web players too**: a WS connection is never sent a Close frame
across a restart — it receives the "restarting" → "Reconnected" system lines and
a fresh room render as ordinary envelopes, with the world intact from Redis, and
keeps playing. Proven by `crates/mush-gateway/tests/web_restart_survival.rs`.

A web **bind/serve failure is non-fatal**: the gateway logs it and keeps serving
SSH (and supervising the server child), so a bad `[web]` config never takes the
whole front door down.

### Accepted losses across a restart

A hot restart reloads the world from Redis, so **world objects and durable
module state are never lost** (rooms, exits, sheets, mail, jobs, channels,
NPC memories, worldgen drafts — everything that write-through persists). The
**only** things that do not survive a server restart are the pieces of state
that live in the server's RAM by design and were always documented as transient:

- **In-progress input context** — anyone mid-`+chargen` **wizard**, mid-mail
  **compose**, or in any other multi-line editor when the restart hits. The
  input stack is per-actor world RAM; it is dropped and the player lands back at
  the normal command prompt after the fresh look. Submitted/approved sheets and
  sent mail are already persisted and survive — only the *unsubmitted, still-being-typed*
  buffer is lost. (Practically: after the reconnect notice, re-open the editor
  and continue.)
- **`@wait` / scheduler timers** — pending `@wait` entries and other in-RAM
  scheduled callbacks do not fire across the restart; they are not persisted.
  Module *ticks* (weather, etc.) simply resume their cadence on the new server.
- **In-flight LLM calls** — an `@gen`, NPC turn, or ST-adjudication that was
  off-loop when the server went down is abandoned (its budget spend, if the
  provider had already been billed, still counts). Re-issue it. The *results*
  that had already landed (accepted drafts, remembered NPC facts) are persisted
  and survive.
- **Active scenes + per-scene ruleset state** — open scenes, their `SceneId`s
  (they re-number from 0 on each boot), the per-`(scene, obj)` `RulesetStateStore`
  (e.g. per-mech combat `heat`), and per-scene map blobs are transient world RAM,
  rebuilt empty by `World::from_objects` on every boot. A scene in progress when
  the restart hits is not rehydrated — its per-boot state snapshot is orphaned —
  so **re-open the scene afterward**. Everything the ruleset persisted to durable
  object state (sheets, XP, wounds recorded on the sheet) survives; only the
  live, in-scene working state resets. (This has always reset on any reboot;
  M16c just makes restarts a routine deploy step, so it belongs on this list.)

Nothing else is silently lost. Announce a deploy anyway if players might be
mid-chargen, mid-compose, or mid-scene, so they know to expect the dropped editor
buffer or to re-open an active scene.
