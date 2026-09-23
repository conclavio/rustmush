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
- **TacSim** — one game, set in 2350, with pilot character sheets, a playable tank
  simulator, human and AI teams, and cockpit views shared by SSH and the web client.
  The platform supplies reusable systems; the `tacsim` crate composes the game.
  Built-ins include channels, mail, weather, jobs, and building tools.
- **Redis 8 persistence** — RedisJSON object documents with write-through per-command
  transactions and durable action journals, with recovery before serving players.

## Status

Pre-alpha with a playable combat simulator. Two pilots can select teams, board,
start their tanks, drive, detect, lock and fire, settle the match, and play again;
a solo pilot can play against the AI fill when configured. The in-process and
split gateway story tests exercise this loop.

Start with [the combat design](docs/design/16-combat-arena.md), its
[implementation record](docs/design/16b-combat-implementation-plan.md), and the
[current backlog](docs/design/16c-simulator-backlog.md). [The original master
plan](docs/PLAN.md) is historical.

## Development

Requirements: the Rust version pinned in `rust-toolchain.toml`, Node 22 (web client),
and Redis 8+ for persistent runtime and live-backend integration tests. Production target is Ubuntu LTS with Redis 8 local (AOF `everysec`
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
cargo test --workspace --exclude mush-bot
```

Run `cargo test -p mush-bot` separately: its JSON feature set must not unify into
the engine tests. Everything runs against the in-memory store by default; the Redis-backed integration tests
(store behavior suite, boot round trip, the kill-9 durability test) run only when `REDIS_URL`
is set, and skip with a note otherwise. The store behavior suite is also ignored
by default: run `cargo test -p mush-store --test redis_suite -- --ignored` explicitly
against that disposable store.

```bash
REDIS_URL=redis://127.0.0.1:6379 cargo test --workspace --exclude mush-bot          # bash
```

```powershell
$env:REDIS_URL = "redis://127.0.0.1:6379"; cargo test --workspace --exclude mush-bot  # PowerShell
```

Point `REDIS_URL` at a **disposable** instance: the gated tests wipe their key namespaces
(they use logical dbs 1–15 plus the default db).

The web SPA builds with `npm ci && npm run build` in `web/` (vitest: `npm test`).
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
channels, mail, jobs, admin, scripting, weather, worldgen, simulator, chargen, rules-of-conduct) are markdown files
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

The game is composed by `mush_server::boot::game_modules`. `[tacsim].enabled` controls the
TacSim layer; there is no ruleset selector. Pilot sheets use `+chargen`, `+stat`
and `+sheet`, with staff approval through the `pilot-chargen` queue. See
[character generation](docs/help/chargen.md) and [the simulator guide](docs/help/simulator.md).
The shipped instance template enables TacSim and seeds the simulator through
versioned content packs. A first-boot pack failure stops startup with its cause.

For deployment, reset and first-staff bootstrap, use [the deployment guide](deploy/README.md).

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
fallback lines). NPC actions use the game's registered abilities and sheet grounding. When a **connected** player shares its room and speaks, the agent takes a turn off
the world loop (rapid chatter coalesced, one call in flight, tier cooldowns) and acts through a
small **typed action allowlist** — say / pose / move / look / remember / use_ability /
do_nothing. That allowlist, re-validated in the world task, is the security boundary, *not* the
prompt: player speech is delimited and escaped and framed as never-an-instruction, and a fully
jailbroken NPC still can't run a command, read another player's data, or escalate. An NPC alone
in a room costs nothing; a budget trip or `@llm/kill <npc>` degrades it to a canned fallback
line. Tiers route to cheap models by default (`npc_dialogue_{major,minor,extra}`,
`npc_summarize` in `[llm.routes]`); reserve `anthropic/claude-opus-5` for a promoted hero NPC.
In-game reference: `help npc`.

**Onboarding** can require approved pilot chargen before play. With
`[onboarding].chargen_required` enabled, a new character starts in the configured
chargen room with a restricted command set. `+chargen/submit` files a
`pilot-chargen` approval job; approval moves the character to the live start room.
See [chargen](docs/help/chargen.md) and `[onboarding]` in the example config.

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
