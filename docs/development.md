# Development

[Project overview](../README.md) · [Crate map](../crates/README.md) ·
[Documentation](README.md)

## Set up the toolchain

Use Rust through rustup, Node.js 22.12+ with npm, and Redis 8+ with JSON and
Search support for persistent servers. Run commands from the repository root:

```sh
rustup toolchain install
rustup target add wasm32-unknown-unknown
npm --prefix web ci
npm --prefix web run build
```

[rust-toolchain.toml](../rust-toolchain.toml) pins the compiler and components.
The frontend build also compiles `mush-pixel-wasm` with Cargo, so it needs the
WebAssembly target even when you are only working on the browser client.

## Start a local world

Start the repository's [development Redis](../docker-compose.yml):

```sh
docker compose up -d redis
```

Copy [mushd.toml.example](../mushd.toml.example) to `mushd.toml` and edit it for
your instance. Set `enabled = true` under `[tacsim]` to enable the game layer,
including the simulator's arena and stock units. LLM and Discord features are
separate opt-ins and can stay disabled.

```sh
cargo run -p mushd -- --check-config
cargo run -p mushd
```

The config check reports preflight errors without opening the store; it does
not replace a successful boot. Use `-- --config <path>` to select another config
file. Relative paths, including `data/` and `docs/help/`, use the working directory.

The example listens for SSH on port `2222` and serves the browser client at
[localhost:8080](http://127.0.0.1:8080). Connect with `ssh -p 2222 play@localhost`
or register in the browser, then read `help getting-started` and `help simulator`.

Two modes work without Redis:

- Set `store = "memory"` under `[store]` in your config for a server with an
  ephemeral world. World state does not survive a restart.
- Run `cargo run -p mushd -- --repl` for the synchronous core command REPL,
  without networking or the full game-module assembly.

Help topics load at boot from `[server].help_dir`, normally `./docs/help`.
Restart after editing those files. Exact command names take precedence over
prose topics because command usage also comes from the registry.

## Iterate on the browser client

With the server running, start Vite in another terminal:

```sh
npm --prefix web run wasm
npm --prefix web run dev
```

Vite proxies `/api` and `/ws` to the game server. The explicit `wasm` step prepares
the renderer and asset files for a fresh development checkout; `build` and `test`
already run that step automatically. Rerun it after changing the Rust renderer.

Debug server builds serve `web/dist` from disk. Rebuild the SPA and refresh the
browser to update that bundle. Release builds embed the web client and require
it to be built before the server:

```sh
npm --prefix web run build
cargo build --release -p mushd
```

Shared wire types originate in `mush-proto`. Regenerate the committed TypeScript
bindings after changing them:

```sh
cargo gen-ts
git diff -- web/src/lib/proto
```

See the [web client guide](../web/README.md) for its layout and protocol details.

## Run checks

The repository uses cargo-nextest to schedule Rust tests and a separate Cargo
invocation for doc tests. Install the version required by
[.config/nextest.toml](../.config/nextest.toml); the current CI pin is 0.9.148:

```sh
cargo install cargo-nextest --locked --version 0.9.148
cargo nextest run -p <crate>
cargo test --doc -p <crate>
```

For a workspace sweep:

```sh
cargo nextest run --workspace
cargo test --doc --workspace
```

The nextest configuration preserves the repository's serialization and
concurrency limits. `cargo test` is also supported. Build `web/dist` first for
tests that serve the SPA. Frontend checks are:

```sh
npm --prefix web test
npm --prefix web run build
```

Most Rust tests use in-memory fixtures. Most Redis-backed tests skip without
`REDIS_URL`; explicitly enabled live-only suites require it. Use a
**disposable test instance**, separate from any game
you want to keep: tests wipe key namespaces across multiple logical databases,
including the database in the supplied URL.

In Bash:

```sh
REDIS_URL=redis://127.0.0.1:6379 cargo nextest run --workspace
```

In PowerShell:

```powershell
$env:REDIS_URL = "redis://127.0.0.1:6379"
cargo nextest run --workspace
Remove-Item Env:REDIS_URL
```

The store behavior suite is also ignored by default. With the disposable
`REDIS_URL` set, run it explicitly:

```sh
cargo test -p mush-store --test redis_suite -- --ignored
```

Use focused checks for the area changed. The [CI workflow](../.github/workflows/ci.yml)
defines the complete set of shards, feature combinations, lint gates, and
scheduled acceptance stories. Its ordinary Rust checks include:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mush-proto --features ts --test ts_export
git diff --exit-code -- web/src/lib/proto
```

For test coverage and longer-term improvements, read the
[testing strategy](design/43-testing-strategy.md). For backups, recovery, and
split deployment, use [operations](ops.md) and the [deployment guide](../deploy/README.md).
