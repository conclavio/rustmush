# rustmush

A modern MUSH server in Rust — the TinyMUX/TinyMUSH/RhostMUSH tradition, rebuilt:

- **SSH access, not telnet** (`ssh play@yourgame`) — russh-based, with a classic welcome screen,
  account wizard, guest pool, and public-key logins.
- **Native web client** — Svelte SPA speaking a structured JSON protocol over WebSocket
  (scrollback + input center; channel tabs, character sheet, mail, who, map panels). Not a
  terminal emulator.
- **Native LLM integration** — describe a building in prose in-game and get consistent,
  zone-themed `@desc`s to review and accept (`@gen`); LLM-driven NPC agents with real stat
  sheets who act only through a validated tool allowlist and roll real dice.
- **No softcode** — the classic command surface (`@dig`, `@desc`, `look`, `say`, `pose`,
  channels, `@mail`) is native Rust; user scripting is sandboxed [Rhai](https://rhai.rs).
- **Ruleset modules** — Classic World of Darkness 20th Anniversary (oWoD20 — the Storyteller
  System, Vampire/Mage/Werewolf templates) first; BattleTech (btmux-style) seams reserved and
  CI-tested from day one. Config selects which ruleset(s) load. Built-ins: comsys, @mail,
  weather, jobs.
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
(they use logical dbs 11–15 plus the default db).

The web SPA builds with `npm install && npm run build` in `web/` (vitest: `npm test`).
Debug server builds serve `web/dist` straight from disk (rebuild the SPA, refresh the
browser — no cargo rebuild); **release builds embed it into the binary and fail to compile
until the SPA has been built**, so a shipped `mushd` can never silently 404 its own client.

Redis is tri-licensed (AGPLv3 option from Redis 8); running it alongside the server is fine —
we don't redistribute it.

## Running

Copy [mushd.toml.example](mushd.toml.example) to `mushd.toml`, adjust, and run `mushd`
(`mushd --repl` gives an offline in-memory REPL). In-game `help` is two-layered: per-command
usage comes from the command table, and prose topics (getting started, building, locks,
channels, mail, jobs, admin, scripting, weather, wod, rules-of-conduct) are markdown files
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

**Rulesets** are selected in `[rulesets].enabled` (default `["wod"]`). The **oWoD20** ruleset
brings the Storyteller System to the game: point-buy character generation (`+chargen` →
staff approval job), the classic dotted sheet (`+sheet`, staff `@sheet/set`), the d10 dice
pool (`+roll strength+brawl vs 6`, with `/spec` `/wp` `/priv` `/nowound` and inline `vs`/`ext`/
`job`/`contested`), the Bruised→Incapacitated health track with wound penalties (`+harm`/
`+heal`/`+health`), and XP spends applied atomically on approval (`+xp/spend`). Character
sheets are stored as object attributes, so they persist and reload with the world — no
separate save path. In-game: `help wod`.

It ships all four **templates** — Mortal plus the **Vampire** (V20), **Mage** (M20), and
**Werewolf** (W20) splats. Every template is always selectable (there is no per-splat
allow-list): a player picks one during chargen with `+chargen/template`, and each character
is exactly one template whose sheet carries only that splat's sections and per-template schema
(a vampire's Disciplines / Blood Pool, a mage's Spheres / Arete, a Garou's Rage / Gnosis /
Gifts). Splat commands (`+disc`/`+blood`, `+cast`/`+sphere`, `+shift`/`+rage`, …) are always
registered but self-gate on the actor's template and refuse everyone else.

The web client is served by the same process on `[web].listen` (default
`http://127.0.0.1:8080`): log in or register over `POST /api/auth/*`, then the SPA speaks
JSON envelopes over `/ws`. Sessions are opaque 256-bit tokens stored server-side as SHA-256
hashes in Redis with a 7-day sliding TTL — they survive server restarts and are revoked by
logout, `@newpassword`, and bans — revocation is live (a logout or password reset also
closes any WebSocket that token had open, not just future connects). In production put a reverse proxy (Caddy/nginx) in front
and list it in `[web].trusted_proxies`; SSH stays directly exposed. During frontend work,
`npm run dev` in `web/` proxies `/api` and `/ws` to the running server.
