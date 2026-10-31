# 33. Cut notes — gateway generations

One `## GG.n` heading per cut of [33-gateway-generations.md](33-gateway-generations.md), in the
order they land: what landed, what it deferred and why, how it was verified.

## GG.0 (2026-10-30) — the IPC compatibility window, one step, one direction (D-33-1)

Built on `claude/dreamy-blackwell-838a12` against doc 33 Draft 0.2 (§4.1 and the GG.0 row of §9),
one PR, two commits: the golden-frame recorder with the version-12 set first, recorded by the
version-12 codec before anything else moved, then the bump and everything that rides on it.

- **The protocol** (`crates/mush-ipc`): `IPC_VERSION` 12 → 13. `ToGateway::Hello` gains a
  defaulted `min_proto`; the new `ToServer::Hello { proto, min_proto }` is the gateway's first
  frame. `IPC_MIN_PROTO` (= `IPC_VERSION - 1`) is the floor of the window this build serves, and
  `IPC_PRE_HELLO_VERSION` (= 12) the version a link is assumed to speak when its first frame is
  not a Hello — the bootstrap rule of §4.1, which lapses by itself when 12 leaves the window. The
  module docs of `protocol.rs` replace "a new arm is not a free extension" with the window's rules:
  additive only inside the window, a defaulted field additive on its own, a removal waits one
  release, a bump records its golden set. The version history says the 13 bump is the last that
  needs a lockstep deploy, and why (a 12 gateway accepts exactly its own version).
- **The mechanism** (`mush_ipc::window`): `ProtoWindow` (`announced(proto, min_proto)` reads a
  Hello with no `min_proto` as "that version only", so a 12 server reads as `[12, 12]`);
  `LinkVersion`, the per-link cell the server's reader fills from the first frame and the writer
  reads on every frame (the window floor until the peer has spoken); `ToGateway::introduced_at`
  (every current variant is `PRE_WINDOW`, 12), `ToGateway::downgrade` (the per-variant older
  forms; none yet) and `ToGateway::for_link`; `EmissionGate`, which the server's `writer_loop`
  passes every queued frame through, with `with_introduction_table` as the test hook that lets a
  proof declare a variant newer than it is. `IPC_VERSION_SKEW_MARKER` moves here, since both halves
  now log it, and stays re-exported from `mush-gateway`.
- **The server** (`crates/mush-server/src/lib.rs`): `admit_gateway` decides each link from its
  first frame — a `ToServer::Hello` whose `proto` lies in `ProtoWindow::OURS` is recorded as the
  link's version; one outside it is refused, the marker in the log line and in the reader's error;
  any other first frame is a 12 peer. A Hello after the first frame is a protocol fault. The
  greeting carries `min_proto`; the greeting and the health tick bypass the gate (both predate the
  window, and the greeting goes out before the gateway has spoken). The reader's lifetime rule is
  unchanged: a refused link ends the reader, and today that ends the server, as any link EOF does
  (GG.1 and GG.2 change that).
- **The gateway** (`crates/mush-gateway/src/lib.rs`): `connect_and_wire` reads the server's
  Hello, checks that the announced window contains `IPC_VERSION`, and only then writes its own
  `ToServer::Hello` on the raw half before the writer task starts — so a server the gateway refuses
  never receives a frame it may not decode, and nothing precedes the Hello on an accepted link.
  `BoundaryFailure::VersionSkew` carries the server's `ProtoWindow`; the skew log names the new
  operator action (deploy a server whose window contains this gateway; the server upgrades first).
  The retry path, the players' notice and the crash-guard exemption (WS22-S16) are as they were.
- **The recorded contract** (`crates/mush-ipc/tests/golden/`): `v12/` (31 `ToServer`, 43
  `ToGateway` frames) recorded by the 12 codec in the first commit, `v13/` (32 and 43) after the
  bump; one length-prefixed frame per sample of the new shared `tests/samples/mod.rs`, which both
  `roundtrip.rs` and the new `golden.rs` read, and which gained the nested arms the round-trip list
  lacked (`Invoke`, `Composing`, `Respond`, the Discord interaction and relay requests and replies,
  `DiscordControl`). `golden.rs` fails when the current version's set is not byte for byte what the
  build encodes (so any wire change under a fixed version re-records, and the diff of `golden/` is
  the record) and decodes the previous version's set under the current codec, both directions.
  The recorder is the `record_golden_frames` example binary (`cargo run -p mush-ipc --example
  record_golden_frames`): a tool, not an `#[ignore]`d test, which the gated-proof inventory in
  `mush-contract-tests` (the K8 kernel-contracts job) reads as a proof no CI job runs — the first
  CI run of this PR said so. `.gitattributes` marks `*.frame` binary.
- **The lint** (`crates/mush-contract-tests/tests/ipc_golden_frames.rs`): fails when the newest
  recorded set is older than `IPC_VERSION`, when a set is newer than the protocol, when a set is
  empty in either direction, or when the previous version's set is gone; each case tripped on a
  fixture, and the real gate tripped once with `golden/v13/` hidden (see below).

**Verified** (Windows, this worktree, `cargo test -j 8` one binary at a time; then the same
binaries under nextest, the CI runner, as the last line says).

- `mush-ipc`: 9 lib tests (the window: its width, a Hello without `min_proto`, the link floor until
  the peer speaks, every variant predates the window, a newer frame withheld through the hook),
  15 `roundtrip` (now pinning 13 / 12 and the `ToServer::Hello` round trip, and a v12-shaped Hello
  decoding as a `[12, 12]` server), 3 `golden`: the v12 set (31 + 43
  frames) decodes under the 13 codec, and the v13 set (32 + 43) is byte for byte what the build
  encodes. Two recordings of the same build are identical.
- `mush-server` lib: `admit_gateway_pins_the_window_and_the_marker` (13 and 12 recorded, no Hello
  recorded as 12, 14 and 11 refused with the marker and the link left unannounced),
  `the_writer_withholds_a_frame_the_link_predates` (the real `writer_loop` with the hook; a 12 link
  never gets the "13" frame, gets everything else, and gets it once the cell records 13), and the
  two existing writer teardown tests.
- `mush-server::split_boundary::ipc_window`: 3 (a 12 gateway served in full, a 13 gateway served
  in full, a 14 gateway refused and answered nothing); the sibling fake gateways, which send no
  Hello and so walk the bootstrap path: `ipc_authoritative_action` 8, `ipc_teardown` 1,
  `gateway_link_parity` 3; `world_task::dead_world_reader` 6 (the reader's new argument).
- `mush-gateway::split_halves`: `server_panic_relaunch` 5 (the skew story now against a v12-shaped
  Hello with no `min_proto` on the wire: refused, `IPC-VERSION-SKEW` logged, sessions kept, healed
  by a 13 server), `two_process_e2e` 4, `restart_survival` 7; 74 lib tests.
- `mush-contract-tests`: `ipc_golden_frames` 8 (the gate and seven fixture cases),
  `feature_matrix_inventory` 3, `panic_budget` 10, `restriction_lints` 11. With `golden/v13/`
  moved aside the gate failed for real — "IPC_VERSION is 13 but the newest recorded golden set is
  v12: a bump records its contract. Run `cargo run -p mush-ipc --example record_golden_frames` at
  version 13 and commit crates/mush-ipc/tests/golden/v13/" — and passed with the set back.
- `cargo clippy --all-targets -- -D warnings` on the four crates (the two boundary loops take the
  house `#[allow(clippy::too_many_arguments)]`, each now eight arguments), `RUSTDOCFLAGS=-D
  warnings cargo doc --no-deps --document-private-items` on `mush-ipc`, `mush-server` and
  `mush-gateway`, `rustfmt --edition 2024 --check` on every changed `.rs` file.
- Under nextest (profile `default`, `ipc_window::` in the `load-sensitive` group): the whole of
  `mush-ipc`, `mush-server::split_boundary` (all 260) and `::world_task`, the `mush-server` lib
  minus its Redis-bound tests, `mush-gateway::split_halves` (all 35) and lib, and the four
  `mush-contract-tests` binaries above — 799 tests across 12 binaries, 799 passed, 8 skipped (the
  recorder, then still an ignored test, and the usual ignores), one slow (`dt_window_scenes`, pre-existing).
- After PR #211's first CI run: the K8 kernel-contracts and rest-libs cells failed on
  `ci_gated_test_inventory::every_backend_gated_proof_is_named_by_a_ci_job`, which read the
  `#[ignore]`d recorder as a proof no job runs (it keys on the attribute, so a reason string would
  not have helped). The recorder became the `record_golden_frames` example; then: the inventory 7
  passed under `cargo test` and under nextest, the example re-recorded v13 byte for byte (no change
  under `golden/`), `mush-ipc` 9 / 3 / 15 (no ignored test left), `ipc_golden_frames` 8 with the
  hidden-v13 trip repeated for the quoted message, clippy and rustdoc on `mush-ipc` and
  `mush-contract-tests` clean.

**Not done here, and why.**

- **Nothing is gated for real.** Every `ToGateway` variant predates the window, so the gate admits
  everything to a 12 link; the mechanism is proven through the test hook (`mush-ipc`'s window
  tests and the writer-level twin in `mush-server`). The first real entry in `introduced_at` comes
  with the first post-13 variant.
- **The refused link still ends the server.** A gateway outside the window is refused by ending its
  reader, and the server's lifetime is the link's today; GG.1 makes the server its own unit and GG.2
  makes a link's end detach that link's connections only. The proof of the refusal is therefore the
  link ending with nothing answered on it (no reply, no disposition, nothing to a connection; the
  shutdown broadcast to `All` and the `Bye` that precede the close are the server ending, not an
  answer), plus the marker on `admit_gateway`'s error.
- **A gateway-side marker on the server's refusal** does not exist: when the server refuses, the
  gateway sees EOF after its Hello and takes its ordinary reconnect path. The server upgrades first,
  so a gateway ahead of its server is a deploy done backwards, and the server's log names it.
- **No deploy.** This bump needs both halves together, once (the 12 gateway refuses a 13 server);
  from 13 on the server deploys first and the gateways follow. dev1 and dev2 are the owner's.
- **Doc 33 itself** (on main as of PR #205, Draft 0.2 with the owner's rulings) is not edited
  here; this file is the first cut note of the series it names.

## GG.3 — `mush-drain`, the shared drain crate (2026-10-30)

D-33-6 (§4.5), built in parallel with GG.0 on disjoint files. The doorway's graceful restart is now one crate that the gateway takes from GG.4 on. **The doorway's behaviour is unchanged**; its e2e suite ([`crates/mush-sshdoorway/tests/e2e.rs`](../../crates/mush-sshdoorway/tests/e2e.rs), `draining_warns_on_schedule_and_closes_at_the_deadline` included) and its unit tests are the proof, on Linux CI.

- **`crates/mush-drain`** (lib; tokio, futures, socket2, tracing; all six panic-shaped restriction lints denied at the root, in `restriction_lints.rs` `MEASURED` and in the panic budget's ratchet tier, at zero):
  - [`drain.rs`](../../crates/mush-drain/src/drain.rs): `Drain<S>`, generic over the session through `Session` — an inject sink (`inject(line)`, a warning line to show) and a close (`close()`, with the daemon's own reason) — in place of the russh `Handle`/`ChannelId` pair; entries are held as `Arc<S>`, so a broadcast never holds the registry lock across an await. `enter`/`live`, `register`/`track`, `abort`/`abort_all`, `phase`/`watch`/`begin`, `wait_empty`, `broadcast`, `close_all`, `run` and `finish_now` as the doorway had them; `registered()` added for the tests. `schedule`, `Warning` and `parse_duration` moved here. The doorway's deadline, warning offsets and grace are the one default config: `DEFAULT_DEADLINE` (24 h), `DEFAULT_WARNINGS` (12h/6h/2h/1h/15m/1m), `DEFAULT_GRACE` (30 s), `DrainConfig::default()`. `DEFAULT_MESSAGE` is neutral wording: the doorway's text is operator config (`config::DEFAULT_DRAIN_MESSAGE`, unchanged).
  - [`kill.rs`](../../crates/mush-drain/src/kill.rs) (`KillSwitch`, `Killable<S>`), [`listen.rs`](../../crates/mush-drain/src/listen.rs) (`bind`, `migrate_req_enabled`, `BACKLOG`, plus `warn_if_migrate_req_off`, the doorway's startup warning) and [`notify.rs`](../../crates/mush-drain/src/notify.rs) (`notify`, plus `ready()`): moved verbatim.
  - [`marker.rs`](../../crates/mush-drain/src/marker.rs): `check_generation` (the `--generation` rule), `file_name`, `dir` (the first `$STATE_DIRECTORY` entry, else the daemon's fallback directory), `path`, `leave` (writes the marker; logged, never fatal) and `found` (the start-time rule: log, `READY=1`, then `true`, on which the daemon exits 0). Nothing in the crate removes a marker.
  - [`lifecycle.rs`](../../crates/mush-drain/src/lifecycle.rs): `DrainConfig`; the `Host` trait (`drain_config()`, read when a drain or a finish begins so a reload counts; `reload()` for `SIGHUP` and `reopen_logs()` for `SIGUSR2`, both a logged no-op by default); `Signals::install()` (first, before anything slow); `start_drain`; `finish` (one grace, never restarted; `STOPPING=1` here and never on a drain); `run` (the select loop: `SIGUSR1` leaves the marker and drains, `SIGTERM`/`SIGINT` finish, exit when empty, the 60 s `HARD_EXIT_AFTER`). The log lines are the doorway's, word for word.
- **The doorway, re-based** (`crates/mush-sshdoorway`): [`drain.rs`](../../crates/mush-sshdoorway/src/drain.rs) is the adapter — `DoorwaySession` (handle, channel, inject queue) implementing `mush_drain::Session` with the pump-or-channel inject and the `CLOSE_REASON` disconnect, `type Drain`, `type LiveGuard`, `INJECT_QUEUE`; `kill`, `listen` and `notify` are `pub use mush_drain::{kill, listen, notify}`, so `crate::kill::KillSwitch` and `mush_sshdoorway::listen::bind` resolve as before; `config.rs` re-exports `Warning` and `parse_duration` and takes its `[limits]` defaults from the shared constants; `server.rs` registers a `DoorwaySession` at its three sites; [`main.rs`](../../crates/mush-sshdoorway/src/main.rs) keeps the argument parsing, readiness and `DoorwayHost` (the reload, the `SIGUSR2` reopen, the drain settings from `[limits]`) and calls `mush_drain::lifecycle::run`. `socket2` left the doorway's dependencies.
- **[`deploy/drain/swap.sh`](../../deploy/drain/swap.sh)**, parameterised: `--unit <stem>`, `--marker-dir <dir>`, `--check <cmd>` (the pre-start check), `--symlink <link> --symlink-target <path with %g>` (re-pointed by `rename` of a fresh temporary link after the 5 s stay-up check, and only if the target exists, else the successor is put down like one that did not stay up), `--after <cmd>` (a warning on failure), `--lock`, `--drain-note`, `--force`. The flock, the two-generation guard, `--force` finishing a still-draining generation, the marker cleared right before `systemctl start`, stop/`reset-failed`/disable of a successor that did not stay up, and exactly one generation enabled are as the doorway's script had them. [`deploy/sshdoorway/swap.sh`](../../deploy/sshdoorway/swap.sh) is the thin call: `[--force]` only, the doorway's units, `/var/lib/sshdoorway`, the binary's `--check`, `check-logging.sh` as `--after` when present; it finds the shared script at `../drain/swap.sh` (`/opt/rustmush/drain/swap.sh` beside `/opt/rustmush/sshdoorway/`) or `$DRAIN_SWAP`.
- **Docs**: [`deploy/sshdoorway/README.md`](../../deploy/sshdoorway/README.md) (the install table and the swap section) and 29a §3.4 (the GG.3 note under the module table) name the move.

Verified: `mush-drain`'s portable unit tests (the schedule and the defaults, duration parsing, the generic registry — broadcast, replace, forget on guard drop, close, track/abort — a run to its deadline and an early return, a finish, the marker directory and generation rules, a found marker never removed, the kill switch, the loopback bind); `cargo clippy --all-targets -- -D warnings` and `RUSTDOCFLAGS=-D warnings cargo doc` on both crates; the three contract guards; `bash -n` on both scripts; the doorway's `cfg(unix)` unit tests and its e2e suite on Linux CI through the draft PR.
