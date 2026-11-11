# 35d. Review of the SF.R17–SF.R24 train against doc 35c §9, and what space flight still needs before production

**2026-11-10.** Reviewed at `origin/main` `a76b1662`. Authority: [doc 35c](35c-remediation-train-review.md) §4 (the holes), §6 (the bugs), §9 (the re-plan and its acceptance cells) and §12 (the programme's space seams), and [doc 35](35-ship-free-flight-combat-and-mining.md) `D-35-1..19`. Cuts and merges: SF.R17 `7f64450a` (#374), SF.R18 `c0d123a9` (#376), SF.R19 `0c1408c6` (#386), SF.R20 `9dedc1c0` (#395), SF.R21 `3fb3e861` (#401), SF.R22 `1801b880` (#402), SF.R23 `d3a30948` (#404), SF.R24 `7a535806` (#409); beside them FR.6 `fe212f4a` (#363), RS.8 `f4ffa532` (#439) and CR.6 `77de4a64` (#453), which changed the same files.

Method: five parallel reviewers (SF.R17/R18; SF.R19/R20 with RS.8 and CR.6; SF.R21 and the store; SF.R22–R24; the eight acceptance stories and the documents), then every P0 and the headline P1s re-read in the tree by the lead. The PR bodies and [35-cut-notes.md](35-cut-notes.md) were treated as claims, not evidence. Line numbers are at `a76b1662`; paths are under `crates/tacsim/src/space/` unless written in full. A finding marked *(inferred)* was read but not traced end to end; everything else was read along its full path. Nothing was reproduced in a running server.

---

## 1. Verdict

- **Most of what 35c asked for landed, and this time the documents do not lie.** Five of 35c's eight holes are closed and three are part closed (§3). The multi-carrier bubble is per carrier; the 1,200 cr bounty and the 1970 stamps are gone; the store is split into per-entity keys with identity checks and per-entity quarantine; there is one docking kinematics; the cluster enum, the `c`/Space auto-solve key and the public server solver are gone. Every symbol the eight PR bodies name exists. The documents are **stale, not fictional**: the cut notes stop at SF.R21 and doc 35 still describes SF.R17–R24 as future work (§7).
- **Five player-reachable holes are open** (§2). Three were introduced by this train's own fixes (the cluster undock, the linkdead return, the prepaid-burn credit), one by RS.8's custody row, and one is the surviving form of 35c §4 #7.
- **The largest piece of 35c is still not built: the `authoritative` label is still a relabel** (§4). Every in-bubble verb changes state in its handler; the submission that follows writes an audit row, re-checks nothing, and is skipped when the actor already has an action pending.
- **Several acceptance stories still pass on test-side shortcuts** (§5): every docking tape comes from an auto-solver, sfr20 hand-sets a staff flag, sfr23's light-lag case types its own distance, and sfr22 never opens the bubble its acceptance names.
- **Nothing outside the unit-level stories has ever exercised space**: no bot lane over SSH, no gate dispatch that names a space story, no hand smoke on record (§8).
- **Recommendation**: a short docs correction, then `SF.R25`–`SF.R31` (§9), with §10's exit criteria as the definition of production-ready. `SF.R25` (the holes) is small and should go first.

---

## 2. Open holes (P0, player-reachable wherever tacsim is composed)

1. **Customs bypass through the sanctioned cluster departure.** `+ship/undock <cluster>` attaches an arbitrary corridor route to the voyage, `def.routes.values().next()` (`port/voyage.rs:3484`); at Ceres that is the `ahuna` route, which is not interdicted. The flight is a free-flight record, which `promote_due_interdictions` skips (tick.rs:4220). A later `+helm/autopilot stickney` recouples; the "preserve the uncrossed interdicted leg" check reads that arbitrary route's first leg (commands.rs:3256–3262), so the new route is one leg, and a one-leg route is never interdicted (tick.rs:4362). Ceres to Stickney with unlicensed `strategic_ore` and no picket. This is the second half of 35c §4 #2, now reachable by a normal verb.
2. **A free trip home by dropping the link.** `execute_linkdead_return_journey` (commands.rs:1948–2031) recouples a hull in free flight or at a cluster onto a hand-built route: a fixed `leg_secs = 3_600`, an origin that is "any other open port", no Δv or tank check, no debit. It fires at the 120 s grace expiry (tick.rs:2576), and `nearest_return_port_for_hull` ignores the hull and returns the first open port (1938–1943). A Dray with an empty tank at Neptune closes its terminal and is at a port one IC hour after the grace runs out. SF.R23 built this to satisfy "the queued return plan executes".
3. **Salvage title to a hull that was never recovered.** `+helm/tow release` removes `active_tows` and `clamped_derelicts` only (commands.rs:5465–5483); the `SalvageCustody` row and its filed claim stay. The claim tick awards on the window alone, with no delivery or location check (`surface/claim.rs:3134–3176`). Clamp, tow, release, move to the next derelict; each window closes with title awarded for a hull still adrift. A second tug that then does the work finds the row `held` and files nothing (5538–5543).
4. **One pilot can freeze a bubble for everyone** (the surviving form of 35c §4 #7). A second `+helm/dock` is refused, but `+helm/dock abort` costs nothing when no pulse was fired and a new session may start at once. Any carrier with an open session skips the whole bubble's step (tick.rs:3489–3494), and the hostile gate on opening a session is checked against the docking hull's own party only (commands.rs:4740–4757). A clean escort beside a derelict can hold a live inspection or fight still by re-opening a session every 59 s. Whether a victim can still act through instant thrust changes was not checked.
5. **The prepaid burn is never drawn down.** `+ship/undock <cluster|body>` sets `prepaid_unflown_t = plan.burn_t` (commands.rs:2756) and nothing reduces it as the flight proceeds; `+helm/decouple`, by contrast, prorates it (2443). Fly most of the way, then `+helm/autopilot <elsewhere>`: the new plan is charged `burn_t − prepaid` (3061, 3212), so a trip costing up to the whole original burn is free, and its crew-days are zero when the credit covers it (3062–3066).

P1 beside these: a towed derelict "lands" at whatever port the tug next docks at, from any distance, and the tow adds nothing to the tug's mass, tank or Δv (tick.rs:2812–2886); the dock gate now **skips** an undetected operational hostile where it should refuse without naming it, while the ordnance check still refuses on undetected torpedoes and so reveals them (commands.rs:4733–4749); `+tac/hail` takes any aboard player, in or out of a bubble, with a free `range_m=` and pushes onto an uncapped persisted queue (commands.rs:3519–3548; picket.rs:824–828).

---

## 3. 35c §4, hole by hole

| # | 35c hole | At `a76b1662` |
|---|---|---|
| 1 | `DOCK ABORT` resumes a journey | **Closed.** Both abort paths need an open session (commands.rs:4997–5048); `unpause_hull_after_docking` is empty. Side effect: a leg-end `+helm/dock <port>` promotes the journey and an abort or expiry now leaves it Promoted until the hull docks or decouples. |
| 2 | Autopilot from a berth | **Part.** The berth refusal is in (2776–2783); the one-leg recouple is §2 #1. |
| 3 | Expedition capped at the tank | **Closed** as a refusal (3061–3079). Undeclared: it first re-plans at 250‰ and then 180‰ throttle (3030–3060), below the D-19-9 floor. |
| 4 | Shared bubble knows only the primary | **Closed.** Per-carrier clear, detach and settle (tick.rs:418–456, 3040–3081, 3233–3325). |
| 5 | One submit clears every carrier | **Closed** in the verbs (commands.rs:4247, 4290; picket.rs:1476–1502). The interdictor's stand-down is still decided for everyone at once (§6.6). |
| 6 | Tow bounty pays twice | **Closed** by deletion (RS.8); §2 #3 is what replaced it. |
| 7 | Re-initiating docking freezes everyone | **Part.** §2 #4. |
| 8 | `holding_for_salvage` blocks everyone | **Part.** Per hull with a 60 s wall timeout, and other carriers detach past it; but a clamped hull is exempt (tick.rs:3020–3025), so clamp-and-never-tow keeps the bubble and its activation slot open for good. |
| P1 | Dock range gate | **Part.** Free flight measures range and speed (4914–4931); the leg-end branch still derives both from `remaining_s` (4876–4892). |

35c §6 by area is in §6 below. The seams 35c §12 handed to the programme (RS.8, CR.6) landed; nothing new was found in CR.6 by reading.

---

## 4. The `authoritative` label is still a relabel (35c §4b, `D-35-9`)

SF.R21's acceptance was one prepared action that the bubble verbs submit, whose prepare re-checks seat, aboard and detection and whose apply records the tape entry. What exists:

- `+tac/lock|beam|torp|railgun|pdc` and `+helm/heading|thrust|emcon|roll|guidance` still call `apply_bubble_command` in the handler and then `submit_bubble_input` (for example commands.rs:3734–3735). `+helm/submit|run` move the fine, the faction front and the relation in the handler (4287–4456). `+helm/dock`, `+helm/tow`, `+tac/hail`, `+mine/ping|laser` submit nothing. All three dispatchers still end in `persist_space_state` (1068, 3417, 5689).
- `submit_bubble_input` returns without submitting outside an active bubble (tick.rs:2071–2077) and when the actor already has an authoritative action pending (2120), which a thrust change that crosses a tonne boundary causes by itself.
- The prepare (tick.rs:2256–2334) checks that the pending entry's actor, hull and switch match the invocation, appends a row to a separate tape family, and stages drone records. It reads no seat, no location and no detection, and the bubble never reads what it wrote. The replay tape the bubble does use is written by the handler.
- The checks themselves exist, in the handlers (`require_aboard_and_seat`, `resolve_detected_contact`, `verify_hostile_fire_allowed`), so this is not a missing-authority hole. It is the same structural gap 35b §3b named: space is the one track whose player commands do not go through the engine's prepare and apply, so two crew typing in one tick, a refused debit, and a restart between the handler and the batch all behave differently here than anywhere else.
- The standing-order channels are read by the tick for Posture and for Thrust at or below zero only (tick.rs:3524–3565); heading and positive thrust are still the handler's. The private map is deleted.
- The input sequence is RAM-only (`next_bubble_input_seq`, tick.rs:2105) and restarts at 1, while its idempotency key is `tacsim.space.input/{hull}/{seq}/{switch}` (2126). If the engine keeps scoped keys durably, a post-restart drone launch can be dropped as a duplicate *(inferred)*.

---

## 5. Acceptance: what the eight stories do

Run locally in this worktree: `cargo test -p tacsim --test domain_world -- sfr` → 24 passed in 18.5 s; `cargo test -p mush-contract-tests` → 189 passed, 8 ignored. CI on `a76b1662` is green.

The basics hold in all eight: every actor is a spawned non-GOD player, time moves only through `set_now` and `fire_due`, there is no `#[ignore]`, sleep or early return. Lines are in `crates/tacsim/tests/domain_world/voyage.rs`.

| Story | Earned | Fiat or absent |
|---|---|---|
| sfr17 | berth autopilot refused; `+ship/undock <cluster>` submits the burn; abort without a session refused; second `+helm/dock` refused; expiry submits a burn; a second carrier resumes past a held one | the 30 s late tick equals the cap exactly, so nothing tests above it; the docstring's "free flight at 1 km refused" is never run; the "1 t to Neptune" tank is never verified; the picket is disabled with every `+tac/beam` output discarded |
| sfr18 | distinct parties; the escort's submit clears only the escort; bubble closes, escort a derelict, miner Cruising; four hulls checked in; `ActivationBook` count 0 | 51 t of contraband minted; nothing shows who disabled whom (60 empty ticks); the "not re-promoted" check looks vacuous, the miner being a leg past the waypoint *(inferred)* |
| sfr19 | clamp pins the real title; second tug refused; one claim, window equal to the economy's; custody survives a reboot; `+salvage` and its switches work; award after the window | every clamp is `play_to_clamp` on a challenge read out of the book; the expected window is production's own helper; "a repaired hull manoeuvres again" and the attach/tow/load capacity check are absent |
| sfr20 | two hulls under 1 m/s² accelerate differently; bubble closes with no leaked NPC Things; `@field/raiders` works after a picket | ROYALTY set by `world_mut().get_mut(..).flags.insert` (9753–9758); 80 t minted; the "rear beam split" is an NPC's hits checked by inequality; "still stripped after check-out" and Slow-`min` are direct library calls on a copy (9831, 9857–9860) |
| sfr21 | heading submits and writes the tape family; channel values; per-entity keys appear and are deleted; OutOfGate does not survive a reboot; a countdown tick writes only the derelict key; a corrupt document quarantines | "a gunner's beam is one submission" is absent; the heave-to has no before/after contrast; the dock-open assertion ORs two strings, one of which a refusal also matches (10060) |
| sfr22 | a malformed cluster fails `load_pack`; `+mine/drones` writes two `RigRecord`s | no second cluster on another body is opened; no interdictor bubble exists in the story, so "stands down on heave-to" is untested; `picket_cast_book()` and `all_orbital_clusters()` are called directly |
| sfr23 | ping, the standard-Dray refusal, the clerk at a host room, own-block mining unflagged, 4 t + 1 t from a 5 t rock, linkdead leaves the cluster, per-block manifest | the hail's distance is the player's own `range_m=600000000` (10887); the torpedo mask is a hand-built off-engine bubble and asserts a key the feed never emits (10876); the ghost-dock assertion passes on any refusal (10818) |
| sfr24 | the session is hydrated from the pushed panel; the submitted tape clamps; canopy panels pushed on the tick | the "human key tape" is `next_guidance_input()` reduced to one key a frame; the "wasm" replay is the same Rust type twice; the expected canopy value comes from production's own function (11145); no test pins that a client cannot reach the auto-solver |

35c §9's rule "the client session produces every tape" is met in letter only: the server solvers are `#[cfg(test)]` now, and the stories (sfr7, 8, 9, 12, 15, 16 and every clamp since) call mush-pixel's `play_to_clamp` on a `DockingChallenge` read from module state instead. No TUI key path and no web key path reaches the server in any test. sfr6 still mints propellant and provisions after its return (4669–4670).

**The lint** (`crates/mush-contract-tests/tests/engine_contracts/arena_library.rs:908–949`) now matches the qualified turbofish for any `tacsim::space::` book, two alias forms, `clear_interdiction_bubble(`, `apply_bubble_command(` and the two server solvers. It misses module aliases, a turbofish wrapped across lines, annotated calls with no turbofish, every non-space book, `world_mut().get_mut(..)` flag and field edits, off-engine bubble copies, book reads used as inputs, the client auto-solver, and `seed_pool` after departure, which 35c §9 said it would pin.

---

## 6. Bugs beside the holes

### 6.1 Store and persistence (P1)
- The per-entity keys are real: `tacsim:space:bubble:{domain}`, `flight:{hull}`, `derelict:{hull}`, `docking:{hull}`, `ledger:{hull}`, `custody:{domain}`, each with an identity rule and quarantine on a bad decode. `deny_unknown_fields` is on the envelopes only.
- The auxiliary `tacsim:space:state` is still unbounded and still hot. It carries `derelict_pins` (insert-only, commands.rs:5313), `cluster_tenure_orders` (never pruned), the whole salvage book and the whole standing-order book, and is rewritten whole every tick while any hull mines (mining.rs:1755–1762) and on each combat event. A bad aux or custody document holds the whole space state.
- There is no dirty flag. `persist_space_state` (tick.rs:1536–1829) clones the book, the salvage book and the standing-order book on every tick and every command and diffs them against the last write.
- The derelict countdown decrements once a second, so each derelict costs a write a second for 48 h (tick.rs:2938–2942); a deadline needs one. Each bubble document is rewritten every combat second.
- A quarantined bubble is not fenced: the refusal tests `active_for_hull(hull)` against `quarantined_bubbles` (commands.rs:377–380), but a quarantined bubble is never in `by_domain`, and nothing stops the next interdiction at that domain writing over the quarantined key *(overwrite inferred)*.
- The port dock clamp can return `Err` after the session is removed, the carrier pulled from its bubble and the domain stood down (5190–5276); the next tick persists a hull that is in no bubble, has no flight record and is not docked.
- Eviction by lowest `ObjId`: the 64-ledger cap can drop a live hull's unlanded debit (tick.rs:2772), `pending_space_burns` drains oldest-first to 16 (2745), and the black-box cap can evict evidence for a live claim (`mush-tactics` salvage.rs:869–874).
- The staged tape family `tacsim.space_bubble_tape` is never deleted.

### 6.2 Lifecycle leaks (P1)
- `+helm/decouple` on the first leg of `ceres_stickney` promotes the free-flyer into the Kirkwood domain (commands.rs:2446–2454) and nothing resumes it; every Kirkwood bubble then closes without `stand_down_domain`, the domain stays one of `MAX_ACTIVE_DOMAINS = 8`, and it keeps interdicting.
- Clamp-and-never-tow (§3 #8) holds a slot the same way.
- `active_tows` survives the tug's destruction and keeps the derelict record alive.
- A disabled NPC that nobody clamped gets a derelict record and then its Thing is destroyed; `+helm/dock derelict` walks `book.derelicts` in id order (4647–4657) and can pick the dead one, shadowing a real derelict. Outside a bubble the same token matches any derelict in the world and the refusal prints its name and id.
- Each promotion creates new NPC Things with location `None` and issues new economy titles; the destroy at close freezes those titles, so frozen assets accumulate.
- Drone `RigRecord`s are inserted and never closed (tick.rs:2349–2391); if the surface counts open excursions per operator, space drones can exhaust it *(inferred)*.

### 6.3 Custody and salvage (P1)
- 35c §6.3's first bullet is open and worse: a refused `check_out` still falls back to an uncustodied copy (tick.rs:4562–4598), and a check-in with no marker now only clears `deployed`, dropping the damage (custody.rs:1000–1005).
- The claim's base is the first lane port's host whatever port the tug docks at (tick.rs:2987–3003), and every in-space custody row is filed under the ground site `freeport` (`surface/claim.rs:1638–1655`). An open ground field at Freeport therefore refuses every space tow and holds every space award *(reachability of that field not traced)*.
- There is no load act and no capacity check beyond twice the tug's dry mass (commands.rs:5520–5528).
- `+helm/dock derelict` still writes `Promoted` with a `DomainId(1)` fallback (5390–5391); a failed free-flight undock promotes into `DomainId(hull.0)`.

### 6.4 Hull fidelity (P1)
- **Slow is inverted on bubble entry.** tick.rs:4631–4636 multiplies by `100 − slow_pct`; `recompute_drive_accel` (bubble.rs:630–631) treats `slow_pct` as the surviving share. A carrier that checked in at Slow 75 enters her next bubble at 25 %.
- **Slow still compounds.** Every Slow pushes `DriveDegraded` (bubble.rs:981–983), which halves the cap again in both burn paths (1679–1683, 1782–1786), and check-in turns it into a damaged `power` component, so the halving outlives a restart.
- `SpaceShipState::apply_damage` is still a private proportional splitter (bubble.rs:806–995); only the Slow consequence and the disable test are shared with the arena. PDC and sensor losses are still hard thresholds (929–956), though they now persist through the `tacsim.space.subsystems` blob.
- `ceil` on the rating survives at bubble.rs:641, tick.rs:4638–4642 and in free-flight `+helm/burn` (commands.rs:2562). NPCs and free flight still use whole m/s²; the interdictor still burns a fixed `[-10,0,0]` (tick.rs:4084–4097).
- There is no space raider unit file: `tacsim.unit.raider_heavy` is a Rust overlay over `tacsim.unit.corvette` (custody.rs:115–148; `content/space/picket_cast.json:16`). `hull_hold_capacity_kg` falls back to the Dray-P's capacity (mining.rs:117–129).

### 6.5 Tempo (P2)
- The step is `min(elapsed, 30 s)` and the excess is dropped; a 45 s stall loses 15 s. The source is `World::now()` in unix seconds; a backwards step now freezes the bubble until the wall clock catches up (tick.rs:3499).
- `last_stepped_wall_s` does not advance while paused or frozen, so a docking pause or a W-clock freeze is paid back as one 30 s step, and ordnance launched during the pause jumps 30 s.
- The Registrar cadence is still `Coarse(1 s)`, so the 500 ms ordnance cadence is unreachable; cruise still skips fires under 5 s; the derelict approach still snaps 6 km a fire off the tape; `[tacsim].space_pulse_ms` is still parsed and inert.

### 6.6 Minds and authoring (P1)
- Doctrines now run through the sealed host, but both live paths pass `store = None` (tick.rs:3974; picket.rs:1754–1761), so `TacsimStore::mind_source` is never read and only compiled-in digests resolve. There is no cadence: each think is a synchronous wait on the world thread, every tick, per ship.
- The interdictor's stand-down is keyed off a trace string (`INTERDICTOR_STAND_DOWN_CLEARED`, tick.rs:3985–4009) and off the first-ranked hostile alone; one stationary contact clears relations for every party *(inferred)*, which is the case SF.R18's acceptance forbids.
- Clusters are a `Vec<OrbitalClusterDef>` on a location, all four on `kirkwood_depot` (`content/lane/ceres_phobos.json:2129–2136`). When no applied pack has clusters, `all_orbital_clusters` silently falls back to the compiled-in lane and re-parses it on every call, which is per hull per tick (mining.rs:373–384); a load failure does not refuse boot, and a world cannot author clusters away.
- `picket_cast.json` is an `include_str!` behind a `LazyLock` with `.ok()` (picket.rs:2103–2111): not in the pack, no digest, silent on a parse failure, with Rust fallbacks and `unwrap_or(GOD)` commanders.
- Drones are still a timer paying a fixed 4 t (mining.rs:1542–1563) with a hand-built `ExcursionId` (commands.rs:6453); the mind's intents are discarded.

### 6.7 Mining and customs (P1)
- The clerk is still head-of-line: a draw that fails for a reason other than an empty rock blocks every hull at the cluster *(inferred)*. Clerk Things are still created as GOD, and the host room falls through to the first placed object of any applied pack and then `ObjId(0)` (mining.rs:709–783).
- Rates are unchanged (4 t per 60 s laser, 4 t per 120 s drone), not doc 20's rig numbers.
- `strategic_ore` is checked per block only when a mined record exists; bought or transferred ore falls back to "any one claim" (picket.rs:665–693), and the record is cleared only on seizure.
- Any fine error still seizes, and clearance is granted before the asynchronous seizure lands, its result discarded (commands.rs:4324–4359).

### 6.8 Sensing, docking and canopy (P1/P2)
- Light lag is applied only when it reaches one second, which is 300,000 km; every in-bubble hail is immediate. The only path that ever queues one is the player's own `range_m=`.
- The canopy's body list is still `default_canopy_bodies_for_site`, the substring table (bubble.rs:1165–1322); the ephemeris rewrites the first body's and the sun's bearing only, a moon sits at a fixed offset, and inside a bubble the list is set once and never updated.
- The cluster-arrival flight is a linear lerp of position to the target by the arrival time (tick.rs, `step_free_flight_records`), independent of the drive.
- The docking pose seed is still `seq + hull × 131` into fixed tables (commands.rs:5355–5358). `play_to_clamp` and `next_guidance_input` remain `pub` in mush-pixel, which both clients compile.

### 6.9 Output and conventions (P2)
- 35c §6.11 is unchanged except the `+helm/cluster` refusals: the `+tac` help line still names mounts with the word the owner ruled out (commands.rs:127); `IR {}, margin +{}` (3499); raw mm/s (1121, 1143, 1632, 4964, 5736); `paused_from_world_tick=` in player text (1192, 4964, 5423); the lead-pip fields in the cockpit feed (bubble.rs:2769–2770), still the owner's call.
- `docs/help/space.md` exists and names every switch but `+tac/hail`; it omits `+helm/tow release`, the berth refusal and `+ship/undock <cluster|body>`.

---

## 7. The documents

Nothing claims the train complete, and no cited SHA is dead (none is cited). What is wrong is staleness:

- [35-cut-notes.md](35-cut-notes.md) has no section for SF.R22, SF.R23 or SF.R24, cites no PR or commit for SF.R17–R21, and says "Divergences. None" on all five. Zero-hit or wrong names: `ActiveClusterSession` (`ActiveClusterMiningSession`), `paid_salvage_bounties` and the 1,200 cr settlement (deleted by RS.8), `tactics.space.subsystems` (`tacsim.space.subsystems`), `pending_mine_draws` (`pending_space_mine_draws`), `claim_window_s` (`claim_window_ic_days`), the lint's path, the bounds "completed ≤ 64" (8) and "pending burns ≤ 64" (16), and `tests/shard_10/voyage.rs` (`tests/domain_world/voyage.rs`).
- Doc 35 is still Draft 0.5: SF.R17–R24 as targets, §3 saying `corvette.json` has no drive law and `+helm` writes a private standing-orders map, no §10 dispositions for this train. The README rows match it.
- `legacy-handlers.toml`: the `+helm` note still describes the pre-RS.8 tow and names `deliver_towed_unit`, `SetHeading` and `SetRoll` (no hits); the `+mine` note stops at SF.R16; the `+tac` note omits `/hail`.
- The PR bodies overstate three things: #402's drone "excursions", #404's clerk placement and its "delivered at now + d/c", #409's "computed from the live Sky ephemeris".

---

## 8. What has never run

- **No client has docked against the server from keys.** Not the TUI, not the web; sfr16 and sfr24 use the library.
- **No bot lane.** mush-bot has no `+helm`, `+tac` or `+mine` vocabulary; `mining_cohort.rs:1149–1151` routes round Kirkwood because the bot lacks the picket's verbs.
- **No gate run.** `gate.yml` is dispatch-only and no dispatch has named a space story; the nightly paths do not include one.
- **No measured cost.** The whole-book clone and diff each tick, the per-second documents, the synchronous doctrine thinks on the world thread and the per-call lane re-parse have no number against them, alone or beside a field.
- **No hand smoke on record**, and no staff repair path for a quarantined document was found in this review.
- Site sizes (doc 43 §2.3 rule 5) do not apply yet: clusters are not sites, so there is no 256/512 to run.

Both dev instances carry this code with every verb live (Q-38-9: no enable flag). There are no players and the stores are wiped at each step, so §2 is a list of defects to fix, not an incident.

---

## 9. Re-plan: `SF.R25`–`SF.R31`

One seam per cut, as 35c §9. Every acceptance test through registered handlers with a non-GOD actor, no auto-solver, no mint after departure, no hand-set flag; `SF.R31` pins those in the lint and the others must already obey them.

| Cut | Scope | Acceptance |
|---|---|---|
| **Docs** (first, with `SF.R25`) | Cut-note sections for SF.R17–R24 from this review and the PR bodies, each with its PR, merge and a Divergences line; doc 35 Draft 0.6 (the status line, §3, `D-35-3/7/9/10/12/13/14/15/17` as built, §10 dispositions from §11 below); README rows; the three allowlist notes; `space.md` gains `/hail`, `/tow release`, the berth refusal and the cluster departure | every name the notes cite greps; `help_commands` green |
| **SF.R25** | **Hole close-out III.** The cluster departure records the real corridor it stands in for and a recouple re-derives interdiction from origin and destination, never from a stored leg (§2 #1); the linkdead return goes through the free-flight planner with the tank check and the debit, and a hull that cannot afford it drifts (§2 #2, Q-35d-2); `/tow release` withdraws the claim and the custody row unless the hull is already delivered, and an award requires delivery (§2 #3); a docking session pauses the docking pair, not the bubble, with a cooldown and a charge on abort, and the hostile gate refuses on any operational hostile in the bubble without naming an undetected one (§2 #4, Q-35d-3); `prepaid_unflown_t` is drawn down as the flight proceeds (§2 #5); the leg-end range gate measures; `+tac/hail` loses `range_m=`, needs a bubble or a Tier-1 contact, and its queue is capped; the three slot leaks of §6.2; the clamp's `Err`-after-teardown; `+helm/dock derelict` scoped to the hull's own bubble | sfr25: `+ship/undock ceres_halo` then `+helm/autopilot stickney` with contraband meets the picket; an empty-tank hull that drops its link is still adrift after the grace; tow-release-repeat ends with no title; an escort's dock-abort loop does not stop the corvette; a diverted cluster flight pays for the divert; after ten decouple and clamp cycles the `ActivationBook` count is zero |
| **SF.R26** | **The prepared mechanic, for real (35c §4b).** The state change moves into apply for every bubble verb, and for `/submit`, `/run`, `/dock`, `/tow`, `/hail`, `/decouple`, `/approach`, `/match` and `+mine/ping|laser`; prepare re-reads seat, aboard, detection and relation in the world; a pending action queues the input, it does not drop it; the sequence is persisted; the tick reads all four standing-order channels; `persist_space_state` is deleted in favour of writes staged by the action or the tick that made them; the staged tape family is the replay tape or is deleted | sfr26: a gunner's beam is exactly one submission and the damage appears only after it applies; two crew firing in one tick are two ordered submissions; a refused debit leaves no flight record; a restart between offer and apply applies once; the allowlist notes say "prepared" for every switch |
| **SF.R27** | **Store budget.** The aux document split into bounded per-entity keys (hail, tenure, pin, tow, clamp, cluster session) with `deny_unknown_fields` on the inner types; dirty flags, no clone-and-diff; the countdown a deadline; a bubble document written on events and at a checkpoint interval, not every second; quarantine fences the domain and the hull on every path; no eviction of live records; every key deleted with its entity | sfr27 and a write census: an idle derelict writes once; ten minutes of heave-to writes a bounded number of documents; a mining hull does not rewrite a shared document; a quarantined domain refuses a new bubble and its key is byte-identical afterwards |
| **SF.R28** | **Physical salvage and one damage table (Q-36-46, Q-36-47).** Towed mass in the tug's acceleration, tank and Δv; the derelict's position follows the tug and delivery is at the port the tug docks at, which is also the claim's base; attach, tow and load as separate acts with the bay or cradle capacity check; space custody rows off the `freeport` ground site; a refused `check_out` refuses the bubble, no uncustodied copy; section choice through the arena's `choose_section` and consequences from authored components; the Slow inversion and the `DriveDegraded` compounding fixed; no `ceil`, milli-m/s² for NPCs and free flight; a raider unit file with its own mounts; NPC hulls reused per domain with one title each | sfr28: a Dray towing 480 t is slower and burns more than she does empty; a hull at Slow 75 enters her next bubble at 75 %; a rear `+tac/beam` by a player splits per the unit file; a refused custody refuses promotion; the title count is flat across ten pickets |
| **SF.R29** | **Tempo and minds.** A monotonic source; elapsed time carried over, not dropped, with a sub-second cadence while ordnance flies; paused time excluded, not repaid; `space_pulse_ms` wired or deleted; the derelict approach on the tape; doctrines read `mind_source` from the store and think on MindPool cadence off the world thread; the interdictor steers by its intents and stands down per carrier on state, not on a trace string | sfr29: a 45 s stall advances 45 s; a 60 s docking pause advances nothing for the pair; ordnance steps at 500 ms; a store-edited doctrine changes the picket; an escort's heave-to does not clear the runner; a Dray can heave to against the interdictor |
| **SF.R30** | **Authoring and mining remainders.** Clusters as doc 20 §17 field-kind orbital sites in the applied pack on their own bodies, no compiled fallback, a load failure refuses boot; the picket cast in the pack with a digest; drones as real excursion sorties whose intents decide the yield and whose records close; clerks laid up by the site opener, not as GOD; rates on doc 20's rig numbers; `strategic_ore` per block on every path; seizure before clearance; the canopy from `Sky` at the hull's state vector in and out of a bubble; the cluster flight through the planner; the docking pose from a seeded draw | sfr30 at both site sizes: a second cluster on another body opens there; a world with no clusters has none; a drone sortie is in the excursion book and gone when it lands; bought contraband is not laundered by an unrelated claim; Neptune's bearing changes inside a bubble |
| **SF.R31** | **Acceptance that cannot be faked, and the surfaces.** Recorded key tapes from the TUI key path and from the web (Playwright) clamp on the server; the auto-solver behind a test feature in mush-pixel; the lint extended to `seed_pool` after departure, `world_mut` edits, off-engine bubbles, the client solvers and the other books; sfr1–sfr24's remaining fiat removed; a mush-bot space lane over SSH (depart, picket, submit, mine, dock) on the nightly at the real pace and on dispatch; a measured tick cost with eight bubbles beside a field; the §6.9 text; a staff verb to inspect and release a quarantine | the lint fails on each form in a fixture; the bot lane is green three nights running; the cost figure is in the cut note |

Order: Docs and `SF.R25` at once (independent, and `SF.R25` is the only one that closes player-reachable holes). `SF.R26` next, alone: it rewrites every dispatcher, so `SF.R27` and `SF.R28` wait for it. `SF.R29` and `SF.R30` after `SF.R26`, in parallel. `SF.R31`'s lint and bot lane can start beside `SF.R25`; its story rewrite lands last.

**Questions for the owner.**
- **Q-35d-1, light lag in a bubble.** At 6–120 km the delay is under a millisecond. Default: delete the in-bubble claim from `D-35-10` and `+tac/hail`'s `range_m=`, and apply lag only to Tier-1 traffic, where it is real.
- **Q-35d-2, the linkdead return.** Default: the plan runs only if the tank covers it and is debited like any autopilot; otherwise the hull drifts on its reserve and is a retrieval job for the port's house (Q-38-6/18) after the 28 IC days of the player-response rule.
- **Q-35d-3, what a docking session pauses.** Default: the docking hull and its target only; the rest of the bubble keeps its clock.
- **Q-35d-4, releasing a tow.** Default: release withdraws the claim. Alternative: the claim stands while any tug of the same party holds the line.
- **Q-35d-5, the lead pip** (carried from 35c §6.11): keep or drop under the no-targeting-maths rule.

---

## 10. Production-ready means

1. No open P0, and §2's five each pinned by a story that fails on the old code.
2. Every space verb that changes state is a prepared action; `persist_space_state` is gone.
3. No unbounded document and no steady-state per-second write; a measured tick cost.
4. No space story uses an auto-solver, a mint after departure, a hand-set flag or a direct library call for a thing the acceptance names; the lint enforces it.
5. A bot plays the picket and a mining run over SSH on the nightly, and a person has docked by hand on the TUI and in the browser.
6. Doc 35, its cut notes, the allowlist notes and the help topic describe the tree, checked by grep before each merge.
7. Staff can see and clear a quarantine without a store edit.

---

## 11. Per-cut disposition

| Cut | Verdict | Keep / fix / re-cut |
|---|---|---|
| SF.R17 | 35c §4 #1 and #3 closed, #2 part; the leg-indexed re-promotion guard, the per-hull hold, the submitted expiry burn and the cluster departure are real. New: §2 #1 and #5, the abort loop, the clamp exemption, the dropped step excess, throttle below the floor | **Keep; holes in `SF.R25`, tempo in `SF.R29`** |
| SF.R18 | Per-carrier party, inspection, clearance, detach and settle; every carrier checked in: real. Stand-down leaks through a Promoted free-flyer; the interdictor still clears everyone | **Keep; leaks in `SF.R25`, stand-down in `SF.R29`** |
| SF.R19 (+ RS.8) | One claim path, the real title holder, one window, the bounty and the 1970 stamps gone, derelict records removed on repair: real. No physical tow, no load act, the wrong base, title on release | **Keep the filing; `SF.R25` for the title, `SF.R28` for the physics** |
| SF.R20 | The drive from the rocket equation for player thrust, `corvette.json`'s drive law, crits persisted, NPC Things destroyed, the `RAIDER_ATTR` collision gone: real. Private splitter, hard thresholds, Slow inverted and compounding, `ceil`, no raider unit | **Keep; `SF.R28`** |
| SF.R21 | Per-entity keys, identity, quarantine, the one ledger, the OutOfGate fix, the Posture and zero-thrust channels: real. The prepare is an audit row; the aux document; clone-and-diff; per-second writes | **Keep the store; re-cut the mechanic (`SF.R26`) and the budget (`SF.R27`)** |
| SF.R22 | The enum gone, content-authored zone, anchor and label, the sealed host, the picket cast in a file: real. Compiled fallback, `store = None`, drones as timers, all clusters on one host | **Keep; `SF.R29` for minds, `SF.R30` for sites and drones** |
| SF.R23 | Tenure lookup, fine payer, idle expiry, the Dray-P check, cluster sessions in the linkdead cover, detection gates on approach and dock: real. The free return, the hail queue, the fail-open dock gate, rates, clerk as GOD, seizure ordering | **Keep; `SF.R25` for the three holes, `SF.R30` for the rest** |
| SF.R24 | One kinematics, the keys and the public solver gone, panels on the tick: real. No client key tape reaches the server; the canopy is still the table | **Keep; `SF.R30` for the canopy, `SF.R31` for the tapes** |

35c §6 at `a76b1662`, by its own headings: §6.1 part (channels half read, label open); §6.2 part; §6.3 one fixed, one part, one open, one not re-checked (the W-freeze boot); §6.4 six fixed, four part, one open; §6.5 one fixed, four part, two open; §6.6 open; §6.7 four fixed, three part, two open; §6.8 six fixed, two part, one not checked (the `v` key); §6.9 two fixed, three part, two open; §6.10 four fixed, two part, one open; §6.11 two fixed, the rest open.

**Not covered by this review:** mush-core's durable idempotency and whether a batch commits atomically (both bear on §4 and §6.1); custody across a boot under a W-clock freeze; a restart in the middle of a partial detach; whether the corvette's doctrine aims at the right carrier's party; the web render loop against the 3,600-frame tape cap; the older stories sfr1–sfr16 beyond their solver, mint and departure lines.
