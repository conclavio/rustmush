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

## GG.1 — the server as its own unit (2026-10-30)

§4.2 and Q-33-5, built on main after GG.0 (PR #211) and GG.3 (PR #213). The gateway no longer spawns, supervises, respawns or gives up on the server: `<inst>mush-server.service` runs `mush-server` as its own `Type=notify` unit, systemd restarts it, and the gateway is a client that dials `[ipc].listen` and holds every connection while the unit is away. **The gap and the heal are unchanged** (`enter_gap`, the restarting notice, input refused not dropped, reconnect with backoff, `ReAnnounce`, the barrier, custody replay, `connect`); what changed is who brings the server back, and what the gateway says when nobody does.

- **Units** ([`deploy/dev1-split/dev1mush-server.service`](../../deploy/dev1-split/dev1mush-server.service) and [`deploy/dev2-split/dev2mush-server.service`](../../deploy/dev2-split/dev2mush-server.service), new; the two gateway units rewritten in place). The server unit: `Requires=`+`After=` the instance's Redis, `EnvironmentFile=` the LLM keys (the gateway never sees them now), `RuntimeDirectory=rustmush` / `rustmush-dev2` (mode 0750) for the socket, `ExecStart=mush-server --config <mushd.toml>` with no `--ipc` (both halves read the one `[ipc].listen` from the one rendered file), `Type=notify`+`NotifyAccess=main`, `TimeoutStartSec=300` (hydration and the 15 s lock wait come before `READY=1`), `TimeoutStopSec=60`, `Restart=on-failure`, `RestartSec=2`, `StartLimitBurst=5`, `StartLimitIntervalSec=60` (Q-33-5), and the same hardening block as before. The gateway unit: `Wants=`+`After=` the server unit, never `Requires=` (a crash-looping server must not take the connections down with it); `Type=simple`; no `EnvironmentFile`, no `RuntimeDirectory` (connecting to a unix socket needs no write access to `/run`), no `ExecReload`, no `KillMode` (one process, no child to sweep: the default control-group kill and stop timeout are right); `Restart=on-failure`, `RestartSec=5`; `LimitNOFILE=65536` kept from PR #203.
- **The gateway as client only** ([`crates/mush-gateway/src/lib.rs`](../../crates/mush-gateway/src/lib.rs)). Gone: `ServerSource::{Spawn,Connect}`, `ServerLauncher`, `SpawnLauncher`, `spawn_server_child`, `server_child_args`, `LaunchedServer`, `RestartTrigger`, `CrashGuard`, `MAX_CONSECUTIVE_FAILURES`, `SHUTDOWN_EXIT_WAIT`, `wait_for_exit`, `AbandonReason`, `SupervisionOutcome`, `abandon`, the `SUPERVISION_ABANDONED` marker, `--child-config` on the CLI and `[ipc].exit_on_abandoned_supervision` (`[ipc]` takes unknown keys, so a rendered config from before the cut still parses). In their place `GatewayOptions { store, ssh_ready, web_ready, siteban_refresh, give_up_after }` and `keep_boundary`, the keeper: serve until the link ends or `stop` fires; on EOF `enter_gap` + the restarting notice; reconnect with the existing backoff (`backoff(streak)`: 200 ms doubling to a 5 s cap) and `RECONNECT_TIMEOUT` per attempt; `resync_boundary` (`ReAnnounce` → barrier → custody replay) on every heal; the version-skew streak as GG.0 left it (refused, `IPC-VERSION-SKEW` logged, the skew notice once per streak, never a give-up). **Boot with no server is a gap from the start**: the first dial still has `INITIAL_CONNECT_TIMEOUT` (10 s), but its expiry is a warn + `enter_gap`, not an exit; the listeners bind and serve the restarting notice. **The give-up notice**: a gap that outlasts `SERVER_UNREACHABLE_AFTER` (60 s, the unit's start-limit bound; `give_up_after` is the test knob) flips `BoundaryHandle::unreachable`, raises the `server_unreachable` gauge (`/healthz` `server_ok: false` and `ok: false`, `/metrics` `mush_server_unreachable 1`), logs the `SERVER-UNREACHABLE` marker with `systemctl status <inst>mush-server` in the message, and broadcasts "GAME: The server is down and staff have been paged. You stay connected; play resumes by itself when it is back." The keeper **never stops dialing** (the skew streak excepted: a skewed server is a deploy done backwards, not an outage), so `systemctl start` by hand heals the gap on its own: `connect` clears the flag and the gauge, "Reconnected" goes out, the siteban mirror refreshes. [`bridge_common.rs`](../../crates/mush-gateway/src/bridge_common.rs): a login during the gap is refused as `ConnectErr::Restarting` ("the server is restarting; hold on and try again in a moment") and, past the bound, `ConnectErr::ServerDown` ("...down and staff have been paged; please try again later"); input and arena actions in the gap say the same two things in the same two states (`GatewayLink::server_unreachable`, defaulted `false` for the in-process link). `SIGHUP` is reserved (§4.5): logged at info and ignored. The keeper ending on its own is still a gateway exit (`Restart=on-failure` brings it back); `SIGTERM`/`SIGINT` stop the listeners, the web and discord bridges, then the keeper, bounded by `KEEPER_STOP_WAIT`.
- **The server** ([`crates/mush-server/src/lib.rs`](../../crates/mush-server/src/lib.rs), `boot.rs`, `gateway_sink.rs`, `main.rs`). `mush_drain::notify::ready()` right after the IPC socket is bound (a no-op without `NOTIFY_SOCKET`, so the test harness and the Windows dev path are untouched), so `systemctl start` returns when the gateway can dial and a boot that dies before that is a failed start. `systemctl stop`/`restart` now reach the server directly, so the boundary loop's select takes a third arm, `mush_config::signals::shutdown_signal()` → `BoundaryEnd::Signal`: "shutting down on signal", `stop_world` (`ControlMsg::Shutdown`, the writer drained, the quarantine spool released), exit 0. The socket bind and the 15 s quarantine-lock wait (`QUARANTINE_LOCK_WAIT`) are as they were. **The exit on gateway EOF is unchanged** (GG.2 removes it).
- **Scripts and docs.** [`deploy/dev1-split/deploy.sh`](../../deploy/dev1-split/deploy.sh) and `dev2-split/deploy.sh` install both units; a redeploy is `systemctl restart <inst>mush-server` (the gateway is left alone), `GATEWAY_RESTART=1` restarts the gateway as before, and the one deploy that drops connections is the first: a running gateway from before the cut is stopped (it kills its child) before the pair is enabled and started; a server unit that exhausted its start limit is `reset-failed` before `start`. [`deploy/dev1-split/README.md`](../../deploy/dev1-split/README.md), [`deploy/README.md`](../../deploy/README.md), the `[ipc]` comment in `deploy/instance/mushd.toml.tmpl` and [`docs/ops.md`](../../docs/ops.md) (the "When the server stays down" section replaces the supervisor one; the knob block is gone) say plainly that `systemctl stop <inst>mush-gateway` no longer stops the server, and that until GG.2 a gateway restart still bounces the server through its unit. `mushd` is untouched.
- **Tests.** [`split_halves/harness.rs`](../../crates/mush-gateway/tests/split_halves/harness.rs) is the stand-in for the unit: `ServerUnit` boots real `mush-server` halves on their own thread and runtime at one remembered endpoint (the first boot binds `127.0.0.1:0`; `at(reserve_endpoint())` fixes one before any boot) over one shared store, with `start`/`stop`/`restart`/`wait_exit`/`boots`; a dropped handle stops nothing. Every module that handed the gateway a launcher and drove restarts through the trigger now aims a `Config` at the unit and restarts it externally (`restart_survival`, `web_restart_survival`, `server_panic_relaunch`, `arena_custody_split`, `arena_story_split`, `front_door_split`, `tui_split`, `two_process_e2e`, `web_split_e2e`, `two_process_llm`, `authoritative_process_kill`). [`split_halves/server_unit.rs`](../../crates/mush-gateway/tests/split_halves/server_unit.rs) (new, in the nextest `load-sensitive` group in place of the deleted `supervision_abandoned`): a gateway started before its server refuses the login with the restarting text and reports `server_ok: true`, then the unit starts and the same player logs in and plays; and a server stopped past a 1.5 s bound (`give_up_after`) is announced as "restarting" then "staff have been paged" in that order, input is refused with the down text, `/healthz` shows `server_ok: false`, a late login is refused with the down text, and the unit's next start heals all of it ("Reconnected", `server_ok: true`, play, logins admitted, two boots). `restart_survival` keeps the story (gap notice, refused input, replay, barrier, resumed play) and swaps the crash-loop case for `a_server_that_stays_away_keeps_the_player_held_and_input_refused`. `server_panic_relaunch`'s skew story boots six skewed servers in a row against a held session and asserts no give-up was announced. [`mush-server/tests/split_boundary/unit_signals.rs`](../../crates/mush-server/tests/split_boundary/unit_signals.rs) (new, `cfg(unix)`): a real `mush-server` process with a gateway attached, sent `SIGTERM`, exits 0 and names the signal.

**Verified.**

- `cargo clippy --all-targets -- -D warnings` on `mush-config`, `mush-ipc`, `mush-server` and `mush-gateway`; `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --document-private-items` on the same four; `rustfmt --edition 2024 --check` on every changed `.rs` file; `bash -n` on both `deploy.sh`.
- Under nextest (profile `default`; `server_unit::` in the `load-sensitive` group in place of `supervision_abandoned::`): the `mush-gateway` lib (68), `mush-gateway::split_halves` (all 35: the boot-into-the-gap story 12.1 s, most of it the 10 s first dial; the give-up story 4.7 s; the skew story's six skewed boots 12.9 s; the arena story 55 s, pre-existing), `mush-server::split_boundary` (all 260; `unit_signals` is `cfg(unix)`, so it compiles to nothing on this box and runs on the PR's Linux CI), the `mush-config` and `mush-ipc` libs (143), and the four `mush-contract-tests` guards `feature_matrix_inventory`, `panic_budget`, `restriction_lints`, `ci_gated_test_inventory` (31) — 537 tests across 9 binaries, 537 passed, 0 skipped.
- Nothing Linux-only ran here: the units, `deploy.sh` and the `SIGTERM` story are proven by review, `bash -n` and the Linux CI cells of the draft PR.

**Not done here, and why.**

- **`RestartForceExitStatus=0` on the server unit is an interim deviation from §4.2's plain `Restart=on-failure`.** Until GG.2 the server still exits 0 when the gateway closes the boundary, so a gateway restart or crash would otherwise leave the server dead and the gateway in its gap for good. The line makes that clean exit a restart too (an in-game `@shutdown` comes straight back as well, which is what it did under the gateway's supervision). GG.2 removes the exit and the line together; the unit file says so.
- **The server's `SIGTERM` arm is an addition beyond §4.2.** Until this cut `systemctl stop` reached the gateway, which closed the boundary, and the server only ever saw EOF; now the signal reaches the server, and without the arm the default action killed it mid-write and the unit read that as a failure. The handler is installed at the boundary loop's select, after a gateway has attached: a `SIGTERM` before that takes the default action, which systemd counts as a clean stop. `unit_signals` runs on Linux CI only.
- **A boot into the gap leaves the web `hello` manifest empty until the gateway is next restarted.** The web client's features and panels are copied from the manifest cache once, when the web bridge is built at boot, and a gateway that booted with no server has nothing in the cache yet; the heal refreshes the cache but not the running bridge. SSH is unaffected. The comment at the first dial in `serve` records it; a bridge that reads the cache per `hello` is the fix, and it is not this cut's.
- **`INITIAL_CONNECT_TIMEOUT` delays the listeners by up to 10 s** when no server answers at boot: the gateway tries the first dial for that long before it enters the gap and binds. A shorter first budget is a one-line change if GG.5 finds it felt.
- **`authoritative_process_kill` no longer asserts a gateway exit.** It killed the server process out from under the supervisor and watched the gateway abandon and exit; the gateway now rides the gap, so the module spawns and respawns its worker itself and the proof is the custody story across the kill. Its gateway is left dialing a private endpoint when the test ends (there is no stop handle in `GatewayOptions`, deliberately: the units own that).
- **`dev2-redeploy.sh` lives on the box, not in the tree**; GG.5 updates it with the owner (the `systemctl restart dev2mush-server` line is the whole change).
- **No deploy.** The first deploy of this cut installs the server unit and must stop the old gateway, which kills its child, before the pair comes up; `deploy.sh` does that and says so, and GG.5 runs it on the box with the owner. dev1 and dev2 are the owner's.
- **Doc 33 itself** is not edited here. The dated record in `docs/interface/03-discord-owner-questions.md` §7a (2026-09-06) still describes the gateway as the server's supervisor and the one place that can page "crash-looping"; that page now comes from the unit's start limit and the gateway's give-up notice, and the Discord programme's on-call seam is where that record gets rewritten, not here. The one code comment outside the touched crates that named `CrashGuard` (`tacsim/src/match_/resume.rs`, the resume-once rule) now names the unit's start limit.
