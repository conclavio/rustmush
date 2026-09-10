# rustmush

A modern MUSH server in Rust — the TinyMUX/TinyMUSH/RhostMUSH tradition, rebuilt:

- **SSH access, not telnet** (`ssh play@yourgame`) — russh-based, with a classic welcome screen,
  account wizard, guest pool, and public-key logins. Register a key in-game with
  `@sshkey/add <your .pub line>` (see `help sshkey`), then `ssh -i <key> <account>@game` logs in
  with no password and `ssh -i <key> <account>+<character>@game` drops straight into that
  character. Login is by account on every surface.
- **Native web client** — Svelte SPA speaking a structured JSON protocol over WebSocket
  (scrollback + input center; channel tabs, character sheet, mail, who, map panels). Not a
  terminal emulator.
- **Native LLM integration** — describe a building in prose in-game and get consistent,
  zone-themed `@desc`s to review and accept (`@gen`); LLM-driven NPC agents with real stat
  sheets who act only through a validated tool allowlist and roll real dice.
- **No softcode** — the classic command surface (`@dig`, `@desc`, `look`, `say`, `pose`,
  channels, `@mail`) is native Rust; user scripting is sandboxed [Rhai](https://rhai.rs).
- **Ruleset modules** — two full game systems on one plugin API: Classic World of Darkness
  20th Anniversary (oWoD20 — the Storyteller System, Vampire/Mage/Werewolf templates) and
  **Shadowrun 5th Edition** (SR5 — the d6 hit pool, Limits, Edge, priority chargen), proving
  the API scales across very different dice engines; TacSim (btmux-style) seams reserved
  and CI-tested from day one. Config selects which **one** ruleset loads. Built-ins: comsys,
  @mail, weather, jobs.
- **Redis 8 persistence** — RedisJSON object documents with write-through per-command
  transactions; RediSearch indexes power `@find`/`@search`; crash recovery is just a restart.

## Status

Pre-alpha; scaffold + design phase. See [docs/PLAN.md](docs/PLAN.md) for the master plan and
milestones, and [docs/design/](docs/design/) for subsystem design docs.

## Development

Requirements: Rust (stable), Node 20+ (web client), and Redis 8+ for integration tests and
runtime from M2 on. Production target is Ubuntu LTS with Redis 8 local (AOF `everysec`
minimum; `always` for zero-loss) — see [docs/ops.md](docs/ops.md) for backups, restore,
migrations, and shutdown/crash semantics.

Getting a dev Redis, in order of preference:

- **Docker** (any platform): `docker compose up -d redis` — the repo-root
  [docker-compose.yml](docker-compose.yml) runs `redis:8` with `appendonly yes` +
  `appendfsync everysec` and a persistent volume on port 6379.
- **Windows without Docker**: [Memurai](https://www.memurai.com/) (Redis 8.2-based native
  builds), or Redis inside WSL2 (`sudo apt install redis-server`).
- **No local option**: tunnel to a disposable remote instance —
  `ssh -N -L 6379:127.0.0.1:6379 you@your-vps`.

```bash
cargo test --workspace
```

Everything runs against the in-memory store by default; the Redis-backed integration tests
(store behavior suite, boot round trip, the kill-9 durability test) run only when `REDIS_URL`
is set, and skip with a note otherwise:

```bash
REDIS_URL=redis://127.0.0.1:6379 cargo test --workspace          # bash
```

```powershell
$env:REDIS_URL = "redis://127.0.0.1:6379"; cargo test --workspace  # PowerShell
```

Point `REDIS_URL` at a **disposable** instance: the gated tests wipe their key namespaces
(they use logical dbs 1–15 plus the default db).

The web SPA builds with `npm install && npm run build` in `web/` (vitest: `npm test`).
Debug server builds serve `web/dist` straight from disk (rebuild the SPA, refresh the
browser — no cargo rebuild); **release builds embed it into the binary and fail to compile
until the SPA has been built**, so a shipped `mushd` can never silently 404 its own client.

Redis is tri-licensed (AGPLv3 option from Redis 8); running it alongside the server is fine —
we don't redistribute it.

## Running

For an external playing agent, see [the standalone bot](docs/bot.md): `mush-bot`
logs in over SSH, plays from a JSON character sheet and a goal, exposes a local
control REPL or local TUI, tracks model costs, and replays JSON scripts or captured
model decisions. Its JSON cast harness coordinates independent bots and runs
experiment matrices with observation assertions and joined traces. JSON prompt
composition exposes each contribution; organized casts add roles, private briefings,
group phases and a terminal monitor. Start with
[mush-bot.toml.example](mush-bot.toml.example). The bot intentionally develops
independently from the in-game NPC/LLM implementation so the two approaches can be
compared through the game interface.

Copy [mushd.toml.example](mushd.toml.example) to `mushd.toml`, adjust, and run `mushd`
(`mushd --repl` gives an offline in-memory REPL). In-game `help` is two-layered: per-command
usage comes from the command table, and prose topics (getting started, building, locks,
channels, mail, jobs, admin, scripting, weather, worldgen, wod, shadowrun, chargen, rules-of-conduct) are markdown files
loaded at boot from `[server].help_dir` (default `./docs/help` — edit or add `*.md` files there and
restart to change them; `help <topic>` serves them, with exact command names taking
precedence).
Admin tooling includes `@boot`, `@wall`, `@ps` (scheduler inspection), `@stats` (uptime,
loop lag, store health), `@siteban`, `@dump`, `@shutdown`, `@newpassword` (staff password
reset — also revokes every web session token of the account and boots its live web
connections), and a player-side `@ignore`
mute list.

Game systems: an **IC game clock** (`time`/`@time`) runs at a configurable IC:OOC ratio over
a fictional calendar — set `[game].ic_ratio` and `[game].ic_epoch` in `mushd.toml`. Builders
attach **sandboxed Rhai scripts** to objects as `script:*` attributes (budget-capped, auto-
HALTED on repeated overrun — `help scripting`), and staff drive **zone weather** with the
`@weather` family over `OUTDOORS`/`SHELTERED` rooms (`help weather`). See
[docs/ops.md](docs/ops.md) for the operator view of all three.

**Rulesets** are selected in `[rulesets].enabled` (default `["wod"]`). A server runs **one**
ruleset — `wod` *or* `shadowrun` — since both register the same core surface (`+roll`/`+sheet`/
`+chargen`); enabling two is a clear boot error (per-character cross-system dispatch is a future
concern). The **oWoD20** ruleset
brings the Storyteller System to the game: point-buy character generation (`+chargen` →
staff approval job), the classic dotted sheet (`+sheet`, staff `@sheet/set`), the d10 dice
pool (`+roll strength+brawl vs 6`, with `/spec` `/wp` `/priv` `/nowound` and inline `vs`/`ext`/
`job`/`contested`), the Bruised→Incapacitated health track with wound penalties (`+harm`/
`+heal`/`+health`), and XP spends applied atomically on approval (`+xp/spend`). Character
sheets are stored as object attributes, so they persist and reload with the world — no
separate save path. In-game: `help wod`.

It ships seven **templates** — Mortal plus the **Vampire** (V20), **Mage** (M20), **Werewolf**
(W20), and **Changeling** (C20) splats, and the two mortal-plus templates **Ghoul** (a mortal
bound to a vampire) and **Kinfolk** (a Garou's mortal kin). Every template is always
selectable (there is no per-splat allow-list): a player picks one during chargen with
`+chargen/template`, and each character is exactly one template whose sheet carries only that
splat's sections and per-template schema (a vampire's Disciplines / Blood Pool, a mage's
Spheres / Arete, a Garou's Rage / Gnosis / Gifts, a changeling's Arts / Realms / Glamour).
Splat commands (`+disc`/`+blood`, `+cast`/`+sphere`, `+shift`/`+rage`, `+cantrip`/`+glamour`,
…) are always registered but self-gate on the actor's template and refuse everyone else; a
Ghoul shares the vampire `+disc`/`+blood` surface and a Kinfolk the werewolf `+gnosis`/`+gift`.

The **Shadowrun 5th Edition** ruleset (`enabled = ["shadowrun"]`, M10b) is the second system on
the same `mush-ruleset` + `mush-dice` plugin API — the deliberate proof it scales past oWoD's
d10 Storyteller pool. Its dice are a **d6 hit pool** (each 5/6 is a hit; more than half 1s
**glitch**, zero hits then is a **critical glitch**) with SR5's two signatures: a **Limit** caps
counted hits, and **Edge** can push past it (add Edge dice, waive the Limit, Rule of Six). A
runner has a flat sheet (eight attributes + Edge/Essence/Magic/Resonance, active-skill /
skill-group / knowledge maps, two Condition Monitors), built with the classic **priority**
chargen (`+chargen/priority`, A–E across Metatype/Attributes/Magic/Skills/Resources → a
`shadowrun-chargen` approval job). Play is `+roll agility+pistols` (auto Limit + wound modifier,
`/edge` to push), `+damage`/`+heal`/`+condition` (Stun→Physical overflow), `+cast`/`+thread`
(spells/complex forms — sheet-tracked, pool-rolled, with **Drain**/**Fading**), `+init`, and
**Karma** advancement (`+karma/spend` → approval). The sheet is the same schema-generic object
attribute, so it renders on the web Sheet panel with **no** web change. In-game: `help shadowrun`.

**LLM worldbuilding** (M7) is configured in `[llm]` and is **off by default**
(`enabled = false`) — a keyless clone boots and `@gen` just answers "LLM features are
disabled." A builder with the `BUILDER_LLM` power sets a **zone theme** (`@theme <zone>=<field>:<value>`
— style, vocabulary, banned words, POV, length), then `@gen here=<prose>` drafts a room
description in that theme, seeded with neighboring rooms' accepted descs for consistency. The
call runs off the world loop (the game never blocks) and the draft comes back to review —
`@gen/accept [fields]`, `@gen/edit`, `@gen/retry [guidance]`, `@gen/discard`; **nothing the
LLM writes is ever auto-committed** (`help worldgen`). Providers are routed by **model-id
prefix** (`anthropic/…` → the Anthropic Messages API; anything else, e.g. `z-ai/glm-5.3`, →
OpenRouter); API keys resolve from env vars named in config (`api_key_env`), never inlined and
never logged — in dev `mushd` loads them from a `.env` at boot, in prod set real env vars.
Layered daily budgets (global / per-feature / per-builder, over tokens and USD) are enforced
pre-call, and staff hold a live kill switch (`@llm/disable`, `@llm/budget`, `@llm/log`). **M10**
adds whole-building generation (`@gen/building <entrance>=<prose>` — a reviewed brief, then a
coherent room draft per room; `help worldgen`), an **LLM flavour top-up** for the deterministic
weather machine (`@weather/generate` drafts extra ambient/transition lines that enter play only
after `@weather/lines/approve`; `help weather`), and a staff **cost dashboard** (`@llm/dashboard`
and the web **LLM** panel — budgets vs caps, top spenders, recent calls; aggregates only, never
any prompt text; `help llm`). See [docs/ops.md](docs/ops.md) for the full operator view and
[mushd.toml.example](mushd.toml.example) for a ready `[llm]` block.

**LLM NPC agents** (M8) turn a first-class `ObjKind::Npc` into an LLM-driven character. Staff
`@agent/create <name>` one, `@agent/set <npc>/persona=…` (plus goals, knowledge, tier,
fallback lines), and `@agent/sheet <npc>=<template>` gives it a real WoD sheet so it can roll
its own dice (`wod_roll` — the engine rolls on the shared seeded RNG, so an NPC can never fake
a result). When a **connected** player shares its room and speaks, the agent takes a turn off
the world loop (rapid chatter coalesced, one call in flight, tier cooldowns) and acts through a
small **typed action allowlist** — say / pose / move / look / remember / use_ability /
do_nothing. That allowlist, re-validated in the world task, is the security boundary, *not* the
prompt: player speech is delimited and escaped and framed as never-an-instruction, and a fully
jailbroken NPC still can't run a command, read another player's data, or escalate. An NPC alone
in a room costs nothing; a budget trip or `@llm/kill <npc>` degrades it to a canned fallback
line. Tiers route to cheap models by default (`npc_dialogue_{major,minor,extra}`,
`npc_summarize` in `[llm.routes]`); reserve `anthropic/claude-opus-5` for a promoted hero NPC.
In-game reference: `help npc`.

**Onboarding** (M9) wires new-player creation through to approved play. Chargen has a guided
terminal **wizard** (`+chargen/wizard`) layered over the classic `+chargen`/`+stat` commands —
same sheet, same point budget, step-by-step with a live budget and `~q` to save-and-resume
(`help chargen`). An optional gate, `[onboarding].chargen_required` (**off by default**, so a
fresh character plays immediately as before), holds a not-yet-approved character to a small
command set (chargen / look / say / help; channel speech and movement are gated), places it in a
`start_room_unapproved` chargen area, and drops it into the wizard on connect over **both** SSH
and the web client. Submitting files a `wod-chargen` approval job; when a storyteller
`+job/approve`s it the gate lifts and the
character is moved to the live `start_room` with full access. See `[onboarding]` in
[mushd.toml.example](mushd.toml.example).

**Observability** (M9). `GET /healthz` returns a small liveness JSON (`ok`, `uptime_s`,
`version`, `connections`, `loop_lag_ms`, `world_loop_unixtime`, `store_ok`) and `GET /metrics`
serves Prometheus text (`[web].metrics_enabled`, **opt-in / off by default** since it binds this
same possibly-public listener with no auth) — aggregate counters and gauges only (connections by
transport, commands, world-loop lag with a p99 histogram, a loop liveness heartbeat, object
counts, store-write health, NPC activity, and LLM calls/tokens/cost by feature), **never**
per-player data, prompts, or
keys. Staff `@stats` reads the same live registry (connected split, loop-lag p99, store queue
depth, today's LLM spend). Structured logging is configured under `[log]`: `level` (a `tracing`
env-filter, or `RUST_LOG`), `format` (`pretty` for the console or `json` for shipping), and
`file` to also write daily-rotating logs under `<data_dir>/logs/`. See
[docs/ops.md](docs/ops.md) for the operator view.

The web client is served by the same process on `[web].listen` (default
`http://127.0.0.1:8080`): log in or register over `POST /api/auth/*`, then the SPA speaks
JSON envelopes over `/ws`. Sessions are opaque 256-bit tokens stored server-side as SHA-256
hashes in Redis with a 7-day sliding TTL — they survive server restarts and are revoked by
logout, `@newpassword`, and bans — revocation is live (a logout or password reset also
closes any WebSocket that token had open, not just future connects). In production put a reverse proxy (Caddy/nginx) in front
and list it in `[web].trusted_proxies`; SSH stays directly exposed. During frontend work,
`npm run dev` in `web/` proxies `/api` and `/ws` to the running server.
