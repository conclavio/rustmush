# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Not built.

## MW.1

Not built.

## MW.2

Not built.

## MW.3

Not built.

## MW.4

Docs only (2026-11-11). Each ruling is written where its question was first asked, as "Ruled
2026-11-11 (doc 44 Q-44-n)"; no question was deleted and no surrounding text rewritten.

- `17-unit-minds.md`: D-UM-56 (Q-44-9); Q-UM-12, Q-UM-14, Q-UM-18 (Q-44-10).
- `17g-m7-completion-plan.md`: §3 C4 (Q-44-15); §4's paid Follow and §5 item 4 (Q-44-11); §5
  item 3 (Q-44-5); §5 item 5 (Q-44-9, Q-44-10); §7's renewal-retry finding (Q-44-14).
- `17h-m8-contraband-procurement-plan.md`: the heirloom (Q-44-12); the House term (Q-44-13).
- `36-ground-combat-after-the-field.md`: Q-36-41 (Q-44-7); Q-36-43 (Q-44-21).
- `03-llm-integration.md`: the status line, from `LlmFeature` and its callers (§4 G), with Q-44-29
  and Q-44-6.
- `27-ic-field-operations-design-and-survey.md`: Q-H5 (Q-44-35 to Q-44-38).
- `42b-surrender.md`: the yielded hull that keeps firing on a busy exit (§4c row 22), checked in
  `field/raiders.rs`.
- `23-ic-sorties.md`: §14 S.2's owed list, a wrecked ship's epoch (§4c row 26), checked in
  `port/epoch.rs`.
- `README.md`: the 17g row.

## MW.5

Not built.

## MW.6

The Registry's pace as a file key, and the two notice drains that stopped on one bad holding
(doc 44 §7.1, Q-44-41; second review, §4c rows 4 and 10). Built on Q-44-41's default, which is
open: the key exists with today's value as its default, and a dev box sets it low.

**Built.**

- **`[tacsim.minds].registry_pulse_ms`.** `mush_config::TacsimMindsCfg.bulk_maintenance_ms` was
  `serde(skip)`, so no file could set the pace of a Registry lease; only the acceptance stories'
  harness could, in code. It is now a file key under the name `registry_pulse_ms` (the Rust
  field keeps its name and is not a key). Absent, it is 1,000 ms, production's second, as doc
  43c records was decided. `MindsConfig::validate` (`crates/tacsim/src/config.rs`) refuses a
  value outside 10 to 60,000 by the key's name, ahead of the bulk model's own check. The key is
  documented, commented, in `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl`, and in
  the key table at the head of `crates/mush-config/src/config.rs`. `tacsim::MindsConfig` itself
  still skips the field in its own serde; the host's mapping (`mush_server::config_ext`) is the
  path a file takes.
- **Both notice drains skip one bad holding.** In `crates/mush-roll/src/commerce/registry_recall/notice.rs`
  and `registry_revocation/notice.rs`, `tick` returned the first error from `own` before the
  cursors were written back, so the next tick read the same page, met the same holding and
  stopped again: every notice behind it waited for ever. Now an error that is one holding's own
  (`one_holding`: `Missing`, `Mismatch`, `Invalid`) skips that holding, emits nothing for it,
  logs it once (`Runtime::note_skipped`, bounded like the remembered notices) and lets the
  cursor pass. An error that is the inventory's (`Unavailable`, `Stale`, `Capacity`) still stops
  the drain where it is, before anything is emitted, to be tried again. `page` had the same
  fault one level up: a row that failed `validate` failed the whole page. Such a row is now
  carried with no recipient, so the cursor passes it.

**Tests.** `mush-config`: `m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo` gains
the key (absent is 1,000; `registry_pulse_ms = 50` parses; `bulk_maintenance_ms` is an unknown
field). `mush-server`: `config_ext`'s refusal table gains 5 and 60,001, each refused by the key's
name, and the mapper carries 50 through to the layer's config. `mush-roll`: one test in each
notice module, `a_bad_holding_is_skipped_and_an_unavailable_inventory_is_not`. Local: `cargo test
-p mush-config` 141 + 14 passed; `cargo test -p mush-roll notice` 3 passed; `cargo test -p
mush-server --lib config_ext` 18 passed; `cargo test -p mush-contract-tests` green; clippy with
warnings denied on mush-roll, mush-config, tacsim and mush-server.

**Divergences.** The unit's row asks for "the measured time a rating takes at both paces". It
was not measured in this cut. The figures on file are `ci.yml`'s (about 1,070 leases a
certificate, about three hours a story at 1,000 ms) and the acceptance stories' (minutes at
their 50 ms). MW.1's bring-up is where a dev box's rating is first timed.

**Not built.** Registry pacing beyond the pulse: a lease still takes 3 to 7 s of work at the
stock pace, and nothing here shortens the work a lease does. The key is not set in
`deploy/dev1` or `deploy/dev2`; MW.1 sets it when it renders the launch.

**Not pinned by a story.** The drains end to end: no rig corrupts one holding and shows the
holder behind it getting their notice. The tests pin which errors skip and which stop, and that
the skipped keys stay bounded. A row that fails `validate` in `page` is not pinned at all.

## MW.7

Not built.

## MW.8

Not built.

## MW.9

Not built.

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

## MW.14a

Not built.

## MW.14b

Not built.

## MW.15

Not built.

## MW.16

Surveyed as [doc 45](45-the-absent-player.md): every way a player can be absent while something of theirs is exposed, what the game does and what an attacker can do today, checked in the tree.
Twenty-five owner questions, Q-45-1 to Q-45-25, open at their defaults; no units of work until they are ruled.
PR #533.

## MW.17

The last decision in the staff view of an NPC (doc 44 §7.1, Q-44-30; first review, finding 8;
second review, §4c row 19).

**Built.** In `crates/mush-npc`:

- **A turn records what woke it.** `NpcHostState::begin_turn` (`runtime.rs`) drains the mailbox,
  so it is the one place that knows; it now writes a `LastDecision` on the NPC's runtime: the
  correlation, when it began, and for the first `LAST_DECISION_WOKE_MAX` (8) stimuli of the batch
  their kind (`Stimulus::kind_name`) and whom each was about. `fire_dialogue` (`module.rs`) adds
  how many public facts and recalled memories the snapshot showed the model
  (`note_decision_context`); guarded and secret facts never cross the async boundary, so the
  count is of public ones.
- **A completion records what the turn came to.** `NpcCompletion::on_complete` writes a
  `DecisionResult`: the model answered with these actions (their tool names), the model did
  nothing, the profile's fallback line with the staff-facing reason (capped at 160 characters),
  or the payload did not decode. A completion for another turn changes nothing
  (`finish_decision`).
- **`@agent/show` prints it**, in two lines after the runtime line (`LastDecision::lines`), and
  `docs/help/npc.md` describes them. The verb is Builder and up, as before.

The record holds kinds, ids and counts. It holds no prompt, no reply and no stimulus text, in
keeping with the audit's rule (`docs/help/llm.md`).

**Tests.** `runtime::tests::mw17_the_last_decision_records_what_woke_a_turn_and_how_it_ended`
(the record, the cap, another turn's completion, the two lines carrying no text) and
`mw17_staff_read_what_woke_a_turn_and_whether_the_model_or_the_fallback_spoke`
(`crates/mush-npc/tests/agents.rs`): through `@agent/show`, a turn in flight, the same turn
answered by the model, the next turn starved of budget and answered by the fallback, and a
player refused the verb. No test makes a provider call. Local: `cargo test -p mush-npc` 150 +
72 + 34 passed; clippy clean with warnings denied.

**Divergences.**

- **The NPC decision call still carries no NPC.** The row asks for `decision_llm.rs` to pass its
  NPC so the per-NPC budget and `@llm/kill <npc>` reach a decision. It cannot be done at that
  call site: the worker lease it is handed (`mush_core::decision::DecisionWorkerLease`) is
  public-safe by design, "no world or private witness reference can cross the async boundary",
  and names a controller and a decision, not an object. Carrying an audit attribution on the
  lease is a change to that contract in `mush-core` and belongs to its own cut.
- **"The knowledge tiers the NPC was allowed"** is reported as a count of public facts and of
  recalled memories. The snapshot carries no other tier, so there is nothing else to report.

**Not built.** The record does not survive a restart (it is runtime state, like the mailbox).
It keeps one turn, not a history. It does not cover a summarise turn or a scene-turn decision
(`npc_decision`), only dialogue turns.

**Not pinned by a story.** A completion whose payload does not decode (the `Unreadable`
result) through `@agent/show`.

## MW.18

Not built.

## MW.19

Not built.

## MW.20

Not built.

## MW.21

Not built.

## MW.22

Not built.

## MW.23

Not built.

## MW.24

SSH `mind push`, `mind pull` and `mind check` without a presence (doc 44 §7.1; §4c row 23;
Q-44-35 to Q-44-37, whose hand-back at reconnect is why this had to land first, §7.4).

**Built.** A **bare connection**: one bound to a character so that an invoke on it runs as them
and its reply reaches it, and nothing else. It is carried end to end as its own pair of
messages, not as a flag on `ConnUp` (see Divergences).

- **The engine** (`crates/mush-core/src/engine.rs`). `Inbound::BareUp { conn, actor }` and
  `Inbound::BareDown { conn, actor }`. `bare_up` binds the connection in the world
  (`World::bind_connection`, the one thing `handle_invoke` checks) and records it in the engine's
  connection map with `ConnInfo::bare`. It does not touch `conn_count`, `set_live_conns`,
  `set_connected` or `session_start`, announces nothing, and fires no hook and no world event;
  `bare_down` undoes exactly that. So `World::is_connected`, `has_live_conns`, `connected()`,
  `Engine::connections_of` and `WHO` cannot see one. `Engine::actor_of` does answer for it,
  which is what the gateway's readiness probe asks. A `BareDown` for a connection that is not
  bare is ignored, and a `BareUp` on a live connection's id is ignored, so a stray frame cannot
  take a player's real connection down or over. `conn_down` on a bare connection is silent
  (a link that ends takes everything it owned down as `Lost`), and `session_gone` leaves a bare
  connection bound, so an exec that outlives its player's grace still gets its reply.
- **The wire** (`crates/mush-ipc`). `WireInbound::BareUp` and `BareDown`, mirrored field for
  field. Two new `ToServer` arms, so **`IPC_VERSION` 13 → 14**, the first bump inside doc 33's
  window: the golden set `tests/golden/v14/` is recorded (34 `ToServer`, 43 `ToGateway` frames;
  the two new samples are last among the `Inbound` ones, so every older frame keeps its number)
  and the v13 set still decodes. Nothing is gated toward the gateway (no `ToGateway` frame
  changed). **Deploy order is the window's: the server first.** A 14 server serves a 13 gateway,
  which never sends either frame and attaches its exec in full as before; a 14 gateway refuses
  a 13 server. 12 leaves the window, so a gateway that sends no `Hello` is now refused.
- **The server** (`crates/mush-server`). The two frames are classed with the lifecycle
  messages in `world_task/mode.rs` (`ConnectionHooks`, held in the lifecycle slot behind a
  pending commit, allowed in an outage countdown), so they keep `ConnUp`'s ordering against a
  commit and against the invoke behind them. `LinkRegistry::claim_bare` (`links.rs`): the link
  owns the connection for routing and a duplicate id is refused as for any other, but the claim
  does not clear a `SessionGone` the link owes for that actor, and a link that ends with a bare
  connection up does not reap its actor on that account.
- **The session manager** (`crates/mush-session/src/manager.rs`). `attach_bare`,
  `detach_bare`, `bare_conns`, `character_of_conn`. A bare connection lives in its own map:
  no session is made, joined or revived, the link-dead epoch is not read or written, the
  link-dead observer is not called, and it receives nothing addressed to the character, a
  session or everyone. `send_to_conn`, `conn_handle` and the flood bucket work on it by id.
- **The gateway** (`crates/mush-gateway`). `GameBridge::enter_bare`: `attach_bare`, a
  `BareUp`, then the same exact readiness probe a login makes (`ServiceReq::ConnectionReady`);
  a refusal sends a `BareDown` and undoes the attach. `GameBridge::disconnected` ends a bare
  connection with a `BareDown` and nothing else: no `ConnDown`, no reaper. `SshBridge::connect_exec`
  makes the same ownership check as `connect_character` and then `enter_bare`; the exec is
  still watched by a gateway drain (doc 33 §4.6). After a server restart `resync_boundary`
  sends a `BareUp` for each bare connection behind the roster and ahead of the barrier, so the
  invoke the gateway replays from input custody finds its binding and an exec caught by a
  restart completes, as it did when it was an ordinary connection. `ReAnnounce` itself is
  unchanged, and a bare connection is never in it.
- **The SSH transport** (`crates/mush-net-ssh`). `SshHooks::connect_exec`, called by
  `start_mind_exec` in place of `connect_character`. Who may run the exec is untouched: verified
  public key, an owned character.
- **The doorway** (`crates/mush-sshdoorway/src/server.rs`). The pump writes an injected line
  only into a channel whose client asked for a shell (`PumpCtx::shell`, set by `shell_request`
  before it relays, and at once for the menu's channel). Until then the lines wait in the
  queue; in an exec or a subsystem they are never written. The session is still closed at the
  drain's deadline.

**Tests.** All run on Windows and Linux unless marked.

- `mush-core`: `a_bare_connection_is_never_presence` and
  `a_bare_frame_never_touches_a_live_connection` (`tests/module_api.rs`; the toy module now
  records the `revived` and `linkdead` hooks as well): offline, link-dead, grace expiring under
  a bare connection, a `Lost` teardown, and the six hook and event lists compared before and
  after. `engine::typed_invoke_tests::an_invoke_runs_over_a_bare_connection`.
- `mush-session` (`tests/session.rs`): `a_bare_connection_has_no_session_and_leaves_none`,
  `a_bare_connection_does_not_touch_a_linkdead_window` (four minutes in; the expiry instant is
  compared), `a_bare_connection_beside_a_live_session`.
- `mush-ipc`: the round trip and conversion cases gain both frames; the golden suite; the
  version pins moved to 14. `window::tests::a_frame_newer_than_the_link_is_withheld` is now
  written against the window instead of the numbers 12 and 13.
- `mush-server`: `links::tests::a_bare_claim_routes_and_owes_no_reaping`;
  `admit_gateway_pins_the_window_and_the_marker` now pins that a gateway with no `Hello` is
  refused. `tests/split_boundary/ipc_window.rs` is rewritten for the window as it now stands:
  a gateway one step behind is served in full, one at the server's version is served, one with
  no `Hello` and one a step ahead are refused.
- `mush-gateway`: `ssh_bridge::exec_tests` (four, against a link that records every frame):
  an offline exec sends `BareUp`, the invoke and `BareDown` and never `ConnUp`, `ConnDown` or
  `SessionGone`; an exec four minutes into a link loss leaves the expiry instant where it was;
  a server that does not bind it refuses and undoes it; a character the account does not own
  is refused before anything is sent. `resync_orders_full_roster_before_readiness_barrier`
  gains the bare connection's `BareUp` between the roster and the barrier.
- `mushd`: `mw24_a_mind_exec_is_never_presence` (`tests/world_journeys/ssh_e2e.rs`), over real
  SSH on the all-in-one server with a second player watching the room: an offline player's
  `mind pull` answers as them, nobody is told anything and `WHO` has no row; a connected
  player's own exec works; an exec halfway through a real link loss is announced as nothing,
  and the character is disconnected when the original grace ends, less than one grace after
  the exec.
- `mush-sshdoorway`: `a_drain_warns_a_shell_and_writes_nothing_into_an_exec`
  (`tests/e2e.rs`): a spliced shell reads the warning while a `mind pull` through the same
  doorway gets its one result line and no other byte.
- The existing SSH mind stories are unchanged and are the "still works" half:
  `mush-net-ssh` `tests/loopback.rs` (`mind_exec_*`) and `mush-bot`
  `arena_mind_author_story.rs`.

Run on the Windows box: the `mush-core`, `mush-session` and `mush-ipc` tests above; the
doorway's `tests/e2e.rs` (34 passed; with the pump's condition taken out the new test fails on
the warning in the exec's output, so it tests the fix); `mush-net-ssh` `mind_exec` (3); the
gateway's `exec_tests` and the resync test (5); the `mushd` story (with the old full attach put
back it fails on "Nick has connected."); `mush-contract-tests` (35 + 80 + 76); clippy with
warnings denied and rustdoc on the eight changed crates. Left to the Linux CI: everything in
`mush-server`, and the gateway's `tests/split_halves/*` and
`tests/split_boundary/unit_signals.rs` (`cfg(unix)`, so the `Hello` added to
`server_panic_relaunch.rs` was not even type-checked here). CI ran all of them green on the
first full run but one: `mush-server`'s
`ic_slice_loop::ic12_part_two_the_queue_runs_the_return` ("a sale is never negative"), a
trade story that builds no bare connection. It passed when its cell was run again on the
same commit, so it is a flake of that story's own (it ran 93 s on a loaded runner), owed a
look by doc 18's owner and not touched here.

**Divergences.**

- **Two messages, not a flag on `ConnUp`.** The row says "a flag from the SSH hook through the
  gateway bridge and the IPC message to `Engine::conn_up` and `conn_down`". A field on
  `Inbound::ConnUp` would have edited about 110 sites that build one, in fifty files, most of
  them other chains' tests, for a connection that shares none of `ConnUp`'s behaviour. As its
  own variant every exhaustive match had to choose what a bare connection is, which is where
  the server's classes and the link registry's rule came from. **Costly to undo** only in the
  sense that it is now wire vocabulary (version 14).
- **A bare connection takes no slot of the split gateway's lifecycle ledger**
  (`LifecycleCustody`). That ledger counts sessions and the `SessionGone` each may owe, and
  reserving in it converts a link-dead player's owed `SessionGone` into the new connection,
  which is the bug this unit removes. A bare connection is bounded by the SSH accept gates and
  by public-key authentication instead.
- **`SshHooks::connect_exec` has a default** that attaches in full, so the test doubles of four
  crates keep compiling and keep their old behaviour. The gateway is the one production
  implementer and overrides it.
- **A subsystem channel through the doorway gets no banner either.** The rule is "only a
  shell", which is wider than "not an exec" by that one case.
- **Bare frames are refused by a world mode that refuses connections** (booting, recovering,
  integrity-frozen): the exec is then refused at the readiness probe with "Character
  attachment was refused.", as it was before.
- **An exec no longer holds its character connected, so it no longer shields them.** The
  workshop cancels a character's authoring jobs when they go link-dead or disconnect
  (`WorkshopCommands::linkdead`). While the exec was a live connection, a player whose shell
  dropped during their own `mind check` was not link-dead until the exec ended; now they are
  at once, and that check is cancelled with their other jobs. In the other direction, the end
  of an offline player's exec used to fire that hook and now fires nothing: a job the client
  abandoned runs to its own bound and its result is dropped by the connection lease
  (`structured.rs`), which is what already happened for a player connected elsewhere.

**Not built.**

- A story on the split pair (gateway and server as two processes) for the exec, and one for an
  exec caught by a server restart. The resync order is pinned by a unit test; the replayed
  invoke reaching its binding on a fresh server is not run anywhere.
- A story that a ship or a field unit held by a backup mind stays held through an exec. It is
  true by construction (the space seat and the arena bridge act on the `connected`, `revived`
  and `linkdead` hooks and on `has_live_conns`, and none of those moves), and the engine test
  pins exactly those; no tacsim story flies it.
- The exec's refusal still reads "Character attachment was refused." whatever the reason (a
  restart gap, a busy world).
- A second shell channel opened after an exec on the same doorway connection gets no drain
  warning (the inject queue belongs to the first channel). The connection is still closed at
  the deadline.

**Not pinned by a story.** `BareUp` and `BareDown` behind a pending commit (the lifecycle
slot); a duplicate bare id across two gateway generations; a gateway drain's deadline closing
an exec in flight; a link's end with a bare connection up, end to end (the registry half and
the engine half are each pinned).

## MW.17b

Not built.
