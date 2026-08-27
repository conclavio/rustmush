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
- **Ruleset modules** — World of Darkness (CofD 2e) first; BattleTech (btmux-style) seams
  reserved and CI-tested from day one. Built-ins: comsys, @mail, weather, jobs.
- **Redis 8 persistence** — RedisJSON object documents with write-through per-command
  transactions; RediSearch indexes power `@find`/`@search`; crash recovery is just a restart.

## Status

Pre-alpha; scaffold + design phase. See [docs/PLAN.md](docs/PLAN.md) for the master plan and
milestones, and [docs/design/](docs/design/) for subsystem design docs.

## Development

Requirements: Rust (stable), Node 20+ (web client), and Redis 8+ for integration tests and
runtime from M2 on. On Windows, run Redis via Docker Desktop/WSL2 or
[Memurai](https://www.memurai.com/) (Redis 8.2-based native builds). Production target is
Ubuntu LTS with Redis 8 local (AOF `everysec` minimum; `always` for zero-loss).

```bash
cargo test --workspace
```

Redis-backed integration tests run only when `REDIS_URL` is set (e.g.
`redis://127.0.0.1:6379`).

Redis is tri-licensed (AGPLv3 option from Redis 8); running it alongside the server is fine —
we don't redistribute it.
