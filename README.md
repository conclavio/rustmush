# rustmush

**A persistent multiplayer world in Rust, with MUSH roots and a science-fiction game set in 2350.**

Build places, play characters, crew vehicles, and take part in a world of trade,
mining, organizations, and tactical combat. rustmush combines the social and
worldbuilding tools of a traditional MUSH with an authoritative simulation and
shared views over SSH and the web.

[Design overview](docs/design.md) · [Crate map](crates/README.md) ·
[Documentation](docs/README.md) · [Bot laboratory](docs/bot.md)

## What you can explore

| Area | What's in the project |
| --- | --- |
| **A shared text world** | Accounts and characters, rooms and exits, building tools, locks, channels, mail, staff jobs, scenes, factions, and canon history. |
| **TacSim** | Pilot sheets, a playable tank simulator with human and AI teams, cockpit controls, sensors and weapons, plus persistent in-world combat fields and recovery. |
| **A working world** | Ports and passages, trade runs, surface mining, cargo and inventory, organizations, contracts, ownership, production, and repair systems. |
| **Several ways to play** | SSH line mode and full-screen TUI, a Svelte web client, shared pixel rendering, and optional Discord integration. |
| **Authoring tools** | Neighborhood geography, maps, content packs, a web Content Studio, unit design tools, and sandboxed Rhai scripting. |
| **AI and experiments** | Budgeted LLM services, reviewed worldbuilding drafts, NPC agents, programmable unit minds, and an independent SSH bot laboratory for playtesting. |
| **Persistent operation** | Redis storage, durable action journals and recovery, health and metrics endpoints, and separate world/gateway processes with generation-based deployment support. |

**Status: active pre-alpha.** The combat simulator and substantial world systems
are implemented; the larger game continues to grow. Space flight, industry,
crew, and other programmes have their own implementation records and open work.
The [design overview](docs/design.md) points to those records and distinguishes
working systems from planned behavior.

TacSim is the current game. Early documents discuss World of Darkness,
Shadowrun, and a loadable-ruleset architecture; those choices have been
[superseded](docs/design/README.md#superseded-decisions).

## How it fits together

The platform crates provide world state, persistence, transport, graphics, and
reusable game systems. [`tacsim`](crates/tacsim/) supplies the game's content and
behavior; [`mush-composition`](crates/mush-composition/) wires the complete
server's modules and their adapters together. State has one writer per authority:
the world and running combat arenas communicate through explicit boundaries.

[`mushd`](crates/mushd/) runs the server in one process for a straightforward
local setup. Split deployments run [`mush-gateway`](crates/mush-gateway/) and
[`mush-server`](crates/mush-server/) separately, so connections can stay up while
the world process restarts. The [crate map](crates/README.md) shows these
relationships and explains every workspace crate by responsibility.

The browser application lives in [`web/`](web/).
[`mush-bot`](docs/bot.md) is a separate player client with its own model and
decision loop; it interacts with the game through SSH.

## Run locally

You need Rust through rustup (the version is pinned in
[rust-toolchain.toml](rust-toolchain.toml)), Node.js 22.12+ with npm, and Redis 8+
with JSON and Search support. From the repository root:

```sh
rustup toolchain install
rustup target add wasm32-unknown-unknown
npm --prefix web ci
npm --prefix web run build
docker compose up -d redis
```

Copy [mushd.toml.example](mushd.toml.example) to `mushd.toml`. To enable TacSim
and the simulator, set `enabled = true` in its `[tacsim]` section, then run:

```sh
cargo run -p mushd
```

With the example's listener settings, open [localhost:8080](http://127.0.0.1:8080)
or connect to the SSH welcome screen:

```sh
ssh -p 2222 play@localhost
```

Register an account and start with in-game `help getting-started` and
`help simulator`. LLM features are disabled by default, so no model API key is
required. The [development guide](docs/development.md) covers configuration,
in-memory operation, the offline core REPL, frontend iteration, and tests.

## Read more

- **Design:** [guided overview](docs/design.md) and [full design index](docs/design/README.md).
- **Code:** [crate map](crates/README.md) and [development guide](docs/development.md).
- **Play:** [getting started](docs/help/getting-started.md),
  [simulator](docs/help/simulator.md), [trade](docs/help/trade.md),
  [mining](docs/help/mining.md), and [all help topics](docs/help/).
- **Operate:** [operations guide](docs/ops.md) and [deployment guide](deploy/README.md).
- **Experiment:** [bot overview](docs/bot.md), with character, cast, campaign,
  and evaluation guides under [`docs/bot/`](docs/bot/).

The [documentation index](docs/README.md) also links specialist references,
implementation records, and archived plans.
