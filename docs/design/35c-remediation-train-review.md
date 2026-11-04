# 35c. Review of the SF.R9–SF.R16 remediation train against doc 35b §8

**2026-11-03.** Reviewed at `origin/main` `f1b2a1e4` (the SF.R16 merge, PR #353). Authority: [doc 35b](35b-seam-first-train-review.md) §3 (the holes), §5 (the bugs), §8 (the re-plan), [doc 35](35-ship-free-flight-combat-and-mining.md) `D-35-1..19`, and the PR bodies of #332, #333, #335, #336, #337, #338, #339 and #353. Commits: SF.R9 `67ee3347` (#332), SF.R10 `21c67a91` (#333), SF.R11 `3e4e2018` (#335), SF.R12 `6410a8cc` (#336), SF.R13 `b1fc0391` (#337), SF.R14 `e6f573c4` (#338), SF.R15 `50f7aedc` (#339), SF.R16 `020e567c` (#353).

Method: four parallel reviewers (authority and lifecycle; tempo, persistence, custody, ids and clocks; hull fidelity, mining, presence, authoring and channels; the sixteen acceptance stories), then every P0 re-read in the tree by the lead. [35-cut-notes.md](35-cut-notes.md) §SF.R9–R16 and doc 35 §10.3 were **not** used as the spec: §3 explains why. Line numbers are at `f1b2a1e4`; paths are under `crates/tacsim/src/space/` unless written in full.

---

## 1. Verdict

- **The code moved.** The four 35b §3 holes are closed as written: `+helm/dock` is gated on the same bubble or the leg end, on a Hostile or ordnance in flight, on a mandatory single-use session with a 60 s wall deadline and a per-session pose; `+helm/cluster` is gone; the timed burn clears on `dt == rem`; a disabled primary carrier's journey is failed and all six manoeuvre verbs refuse a derelict. The bubble is keyed by domain, the NPC hulls are world Things under custody, linkdead rides the module hook on real session presence, panels ride the tick to every crew member, the doctrines live in content, the document is capped and quarantined, the Rhai runner is cached. Fourteen of 35b §5's twenty bugs are fixed or part-fixed (§7).
- **The documents did not.** The SF.R16 commit rewrote doc 35 §3, `D-35-3`/`D-35-12`/`D-35-14`, the §10.2 dispositions, §10.3 and all eight new sections of 35-cut-notes to describe a train that was never built: five cited SHAs do not exist and twenty-two named symbols have no hit in the tree (§3). The PR bodies are the only record of what landed.
- **Eight new player-reachable holes** (§4), most of them consequences of SF.R9's and SF.R12's own fixes in the shared bubble and at the berth.
- **Against the rest of the tree the track is still not compliant where it matters most**: the `authoritative` label (`D-35-9`, 35b §3b), the standing-order channels, the single unbounded document, Q-36-46's one salvage path, Q-36-47's one damage table, Q-36-48's `PartyId`, `D-35-1`'s drive constants (§6).
- **Recommendation**: a docs-first correction (§8), then `SF.R17`–`SF.R24` (§9). Stack nothing further on space until §8 is done, because the next builder will read the cut notes. The enable flag is **not** owed: Q-38-9 is an owner ruling against it.

---

## 2. What was run

- CI on `f1b2a1e4`: green on every job (lint, documentation, web, tacsim shards 1–10, the rest-* splits, kernel contracts). CI was green on `f146734e` after SF.R15 as well.
- Locally in this worktree: `cargo test -p tacsim --test shard_10 -- sfr` → 16 passed in 24.2 s; `cargo test -p mush-contract-tests` → every binary green, including `legacy_handler_allowlist`, `panic_budget`, `restriction_lints`, `help_commands` and the new `arena_library::tacsim_integration_tests_never_mutate_space_bubble_book_directly`.
- Not run: the VPS gate (`gate.yml`, dispatch-only) has not fired since 2026-11-02 04:13 (`eaefe6a0`, failure), so no long story has exercised this train. Nothing is deployed.
- `D-35-2` held: no SF.R commit touched `arena/`, `field/`, `surface/` or `detachment/`; the `surface/` edits in `8ccb3979..f1b2a1e4` are doc 36a lane 5 (PR #346).
- Panic budget: every `unwrap`/`expect` under `space/` sits inside a `#[cfg(test)]` block.

---

## 3. The documents describe a different train

Commit `020e567c` (SF.R16) is the only commit that wrote these sections. Each row compares what it wrote against the PR body of the cut it names and against the tree.

| Cut | 35-cut-notes / doc 35 §10.3 say | The PR body says | The tree at `f1b2a1e4` |
|---|---|---|---|
| SF.R9 | "hold promoted interdictions across engine ticks and step bubbles on 1:1 wall time", commit `6aa49514`, test `sfr9_promoted_interdiction_holds_across_full_engine_ticks_and_steps_on_wall_clock`, `tick_world_at_wall_ms` | #332: authority close-out | test is `sfr9_authority_closeout_gates_docking_removes_cluster_teleport_stops_1s_burn_and_locks_disabled_derelict`; `6aa49514` is not an object; `tick_world_at_wall_ms` 0 hits |
| SF.R10 | "emit real propagation events and push space panel keyframes", `c13ab990`, `PropagationBook`, `ensure_bubble_propagation_locations`, `perceived_events` | #333: tempo and persistence budget | all three symbols 0 hits; `c13ab990` is not an object; `mush_propagation` has no `light_lag_ms` |
| SF.R11 | "wire `+helm/decouple`, free-flight entry and mid-space recouple", `b157a098`, `recouple_from_free_flight` | #335: one propellant ledger, rated drive, post-burn reserve | 0 hits; not an object |
| SF.R12 | "end-to-end derelict lifecycle and split bounded store keys `tacsim:space:bubble:<id>`, `flight:<hull>` …, `MAX_TAPE_ENTRIES` 4,096, one-time migration", `611d754c`, `check_in_space_ship_preserving_epoch_if_disabled` | #336: domain bubbles, real NPC hulls, contraband seizure | one key `tacsim:space:state` (`SPACE_STATE_KEY`, tick.rs:1043; store.rs:2598); `MAX_BUBBLE_TAPE_ENTRIES` = 64; the function 0 hits; not an object |
| SF.R13 | "close authority and lifecycle bypass holes": `require_helm_or_command_seat`, `has_claim_authority`, remove `+tac/hail clear`, `hail_clear_ship`, `MAX_DOCKING_SESSION_WALL_S`, `56b963d4` | #337: presence and feeds (crew panel push, torpedo detection, linkdead grace, canopy keys) | all four symbols 0 hits; the deadline is `DOCKING_SESSION_TIMEOUT_WALL_S` (docking.rs:29); not an object |
| SF.R14 | "`packs/solar_2240/` with `clusters.json`, `pickets.json`, `doctrines/*.rhai`, `SpaceClusterDef`, `SpacePicketDef`, `clusters.schema.json`" | #338: `content/minds/{kirkwood_picket,belt_raider,interdictor_patrol,drone_mining}.{rhai,json}`, `ceres_phobos.json` rock tables, `outer_bodies.json` as a signed pack | no `packs/` directory; all six names 0 hits |
| SF.R15 | "route hits through `resolve_attack`, `challenger_hulls`/`defender_hulls: Vec<ObjId>`, thrust from `drive_cell_ratio`, `Sky::velocity_mps` 3D" | #339: facet split in `SpaceShipState::apply_damage`, heat cell, bay repair, salvage bounty | all four names 0 hits; the damage path is private (bubble.rs:675) |
| SF.R16 | matches #353 | — | matches, except the canopy (§6.10) |

Beyond the table: doc 35 §3 lists `raider_heavy.json` (no such unit file; `tacsim.unit.raider_heavy` is an overlay key only, custody.rs:103); `D-35-3` names `last_wall_step_ms`, `MAX_BUBBLE_TICK_STEP_MS`, `#[serde(skip)]` and 250 ms substeps where the field is `last_stepped_wall_s`, **persisted** with `#[serde(default)]` (tick.rs:146–149) and clamped to 1–10 s (§6.6); `D-35-5` says `step_promoted_journeys` checks `tacsim.space.interdiction` (every Promoted journey pauses, `mush-sim/src/journey.rs:1292–1304`, and its doc comment now says so); `D-35-14` names `MAX_DOCKING_SESSION_WALL_S`; the §10.2 dispositions ("Seam broken at `f1ea7715`, fixed in `SF.R9`") are not 35b §6's dispositions; the cut notes say `voyage.rs` moved in #346 (it was `44aac0fd`, PR #344); the README row says "SF.R0–SF.R16 all built"; the allowlist notes for `+helm`, `+tac` and `+mine` (`crates/mush-contract-tests/legacy-handlers.toml:143, 210, 372`) were touched by none of the eight PRs and still describe SF.R3/SF.R5 with four of `+helm`'s sixteen switches. 35b §7 #3 (doc 21) and #5 (`D-35-19`) were applied correctly.

This is the pattern 35a and 35b named, in a new form: the landing commit now rewrites the design, not only the cut table. Rule for the next train: `git cat-file -t` every SHA a doc cites and grep every symbol it names before the PR merges.

---

## 4. Authority and exploit holes (P0, player-reachable wherever tacsim is composed)

1. **`+helm/dock DOCK ABORT` resumes any paused journey.** The text-order path (commands.rs:4401–4415) removes whatever session exists and calls `unpause_hull_after_docking` (3981–3989), which runs `mush_sim::resume_carrier_journey` whenever the hull has no active bubble, session or not. A pilot who decoupled mid-leg (journey Promoted at 2211, free flight at 2229) types it and the corridor journey resumes Cruising while the free-flight record stays; `promote_due_interdictions` skips free-flight hulls (tick.rs:2471), so the hull crosses Kirkwood with no bubble.
2. **`+helm/autopilot <cluster|body>` departs from a berth.** The `Berth::Docked` branch (commands.rs:2480–2503, 2627–2633) builds a free-flight record from the berth's venue and never runs `+ship/undock`'s `prepare_departure` (charter, promised-goods hold, field hold, stay charge, bay gate; `port/voyage.rs:3246–3420`). A following `+helm/autopilot <port>` with no live interdicted cursor leg gets one leg with interdiction 0 (2872). sfr5 (voyage.rs:4248) and sfr16 (7929) assert this departure.
3. **The outer expedition is capped at the tank.** Any first leg over 500,000,000 km takes `plan.burn_t.min(live_tank_t.max(1)).max(1)` and the same for crew-days (commands.rs:2682–2690). One tonne and one crew-day reach Neptune.
4. **Shared bubbles only know the primary carrier.** `is_clear_to_resume` and `is_carrier_disabled_and_settled` read `carrier_ship_id` only (tick.rs:305–313, 348–357); settle checks in and fails only `active.hull` (1795–1894); `clear_resolved_bubbles` resumes every carrier. A disabled secondary flies home with no derelict record; a disabled primary leaves the secondaries Promoted forever with their custody marks, and `any_other_promoted` blocks `stand_down_domain`, so the ActivationBook slot leaks.
5. **One `+helm/submit` clears every carrier.** All carriers share `CARRIER_PARTY` and the single `picket.inspection_status` (commands.rs:3745–3750, 3790–3796, 3820–3826; tick.rs:329–337). A clean escort submits, the smuggler beside it is refused "already cleared" and leaves with unlicensed `strategic_ore`. sfr12 passes this way: only the miner submits. Conversely one pilot's `+helm/run` turns the corvette Hostile to everyone.
6. **The tow bounty pays at attach and pays again.** `+helm/tow` settles 1,200 cr through the Registry at clamp (commands.rs:4863–4888 → picket.rs:1656–1750), before delivery and inside the 48 h window; the only guard is `active_tows[tug].derelict_hull == this`, and `settle_salvage_recovery_payout_from_registry` ignores the derelict. Two tugs on one raider are both paid; one tug alternating two derelicts is paid on every switch.
7. **Re-initiating a docking session is free and freezes everyone.** `helm_dock` never checks for an open session; `insert` overwrites it and resets `started_wall_s` (commands.rs:4651–4722), and the session pauses the whole bubble (tick.rs:1923–1929). A pilot re-typing `+helm/dock derelict` every 50 s holds every other carrier and NPC indefinitely.
8. **`holding_for_salvage` blocks the bubble for everyone.** `+helm/approach derelict` sets it (commands.rs:3516–3517); only `+helm/tow` clears it (4878–4879); while set the bubble cannot clear (tick.rs:314–321). One pilot who approaches and never tows pauses every co-promoted journey and refuses their decouple, autopilot and port dock.

P1 beside these: the port-at-leg-end dock branch tests "remaining_s > 60" IC seconds (commands.rs:4259), not range or speed, though the refusal quotes metres; the free-flight branch treats anything within 2 km of the body centre as 200 m (4289–4293).

### 4b. The `authoritative` label is still a relabel

Still registered under `classify_commands(Authoritative)` (`tacsim/src/commands.rs:446–512`). The `submit_authoritative` calls in `space/commands.rs` are the propellant debit (1925, 1935), the seized-contraband burn (1987, 1997), `+mine/stake` (5200) and `+mine/contest` (5336). `+tac/lock|beam|torp|railgun|pdc` (3196–3379) and `+helm/heading|thrust|emcon|roll|guidance` (1359–1605) mutate the bubble in the handler through `apply_bubble_command`; `+helm/approach|match|submit|run|dock|tow` write `SpaceBubbleBook` directly; all three dispatchers end in `persist_space_state`, a whole-book `JsonSet` post-image (976–979, 3012–3015, 4923–4926; tick.rs:1061–1100). Compare `+ship/undock` (one voyage document and one prepared burn, `port/voyage.rs:3545–3551`) and `+field` (`field/commands.rs:489`). 35b §3b's fix, "a prepared mechanic for the bubble", is `SF.R21`.

---

## 5. Acceptance: what the sixteen stories do

The basics hold in all sixteen: every actor is a spawned non-GOD player; GOD appears only in `@teleport`, `@tacsim/time` and `@tacsim/port status`; time passes only through `set_now` and `fire_due`; no sleeps, no `#[ignore]`; no `module_state_mut` on any book but `mush_sim::AccountOracle` (setup, voyage.rs:156). The fiat moved: the server solving its own docking tape, off-engine unit checks, `seed_pool` mints mid-space, hard-coded production tables the tests mirror, and OR'd or missing assertions.

| Story | Earned | Fiat or absent |
|---|---|---|
| sfr1 | promotion, pause, resume, three `+tac/beam` clear the bubble, docks | on the test-only `lane_with_interdicted_kirkwood`; the "5 fires = 5,000 ms" assertion pins the tempo clamp's loss (one fire was 627 s late and advanced 1 s) |
| sfr2 | custody out, damage back on check-in, Immobile derelict | `+ship` damage is a pattern match; the interdictor's prior owner resolves to GOD, unasserted |
| sfr3 | every step | crew placed by `@teleport`; feeds read off the book, not from pushed panels |
| sfr4 | decouple, debit, cut-off, reserve warning | `seed_pool`; `force` untested; "docks at Stickney" also matches the under-way line (35b open) |
| sfr5 | refit, licence, deposits, tenure, approach over ticks, match, 20 t conserved, stake, contest, sell, mill | departs Ceres by `+helm/autopilot` from the berth (§4 #2); drones are fixed timers; `seed_pool` at the cluster; the Ceres dock is a GOD `@teleport` after `+ship/leave` |
| sfr6 | three-party bubble, refusals, heave-to, fine and front, ambush, jink, raider disabled by the corvette's doctrine, derelict, claim, docks | hail "light-lag" is instant text (picket.rs:576–588); salvo screening never asserted; three mints |
| sfr7 | promote, disable by beams, tow-before-clamp and >500 m refusals, pause, forged/over-length/out-of-gate rejections | bounce and both clamps use the server's `solve_*_tape` (5243, 5292, 5356); tow is a record |
| sfr8 | restart mid-bubble, decoded state compared | derelict clocks `<` only; clamp by server solver (5641); custody across restart absent |
| sfr9 | `/cluster` gone, 1 s burn stops, mid-leg refusal, leg-end dock, mandatory id, 60 s expiry, `/run` guard, derelict refusals | under-fire assertion ORs two messages (5796–5797) so the hostile gate is unpinned; clamp by server solver (5878); `seed_pool` in free flight; the docstring's out-of-gate close is never submitted |
| sfr10 | 3 s late = 3,000 ms (harness clock), 606-fire heave-to bounded, quarantine | bound measured on the in-memory JSON, not the writes; custody only `is_deployed`, no post-restart write-back; a tick ≥ 11 s late untested; the write-census stories are not here |
| sfr11 | jink debits the vault, no refill on re-entry | `max_accel = 1` is `ceil(0.49)`; `command_burn` applies 1 s of Δv instantly on each thrust change (bubble.rs:1484–1505); burn-away has no toward-port contrast |
| sfr12 | two hulls share one bubble, NPC hulls are Things, indigent seizure, approach, "Varga Teodor" on the claim | 51 t contraband minted; the escort's `+tac/beam` output discarded, the kill unattributed; clamp by server solver (6619); tow a record; base asserted `!= GOD` only |
| sfr13 | tick panels to a silent gunner, TTI, linkdead through `Inbound::ConnDown` to the hook, drift | torpedo filter is an off-engine bubble with `remaining_dv_mmps = 0` set by hand (6823–6862); "holds PDC" is two flags; grace expiry and the return plan untested |
| sfr14 | packs load, sealed minds, picket on the authored host | the "no const" lint matches `const *_RHAI/_ROCKS` only and misses `pub static STANDARD_RHAI` and the `OrbitalClusterId` enum (mining.rs:109–320); negative case flips the same waypoint; the drone `ExcursionId` never asserted |
| sfr15 | 1,200 cr payout, bay refit, end-to-end damage total | the rear split is a direct `apply_damage(Rear, 160)` on a test-built ship (7356–7390); the PDC gap is `pdcs[0].operational = false` by hand (7445–7534); parts minted; the payout lands at tow-lock |
| sfr16 | the mush-pixel client reads the real `space.docking` panel, `play_to_clamp()`, `format_tape_command()`, `+helm/dock tape` → `Clamped`, `Berth::Docked`; stale and constant tapes fail | the client is an autopilot, not key input; "a web pilot docks by playing" has no test; departs Ceres from the berth (§4 #2); Neptune/Triton bearings `(0,−4)`, `(26,5)` are a Rust table keyed on the substring "neptune" (bubble.rs:1040–1071) and the test hard-codes the same numbers (7975–8002) |

**The contract lint would not have caught the original fiat.** `arena_library.rs:947` matches the literal `module_state_mut::<SpaceBubbleBook>()`. All eight 35b §4 writes used `module_state_mut::<tacsim::space::SpaceBubbleBook>()` (`git show 009efd2f`). Only sfr9's own `include_str!` check (voyage.rs:5685, 5972) catches that form, and only in `voyage.rs`. The lint also misses aliases, type-annotated calls without a turbofish, `&mut World` mutators (`clear_interdiction_bubble`, `apply_bubble_command`), every other book, `reboot_editing` store edits, and reads used to compute inputs (the docking challenge in sfr7/8/9/12/15/16, `cluster_arrival_ic_s` for clock jumps).

Direct `tacsim::space::` calls from the stories: `solve_docking_tape` ×8, `solve_hard_bounce_tape`, `DockingChallenge::standard_port`, `decode_rle`/`replay_docking_tape`, the sfr13 bubble construction and `fire_torpedo_salvo`, the sfr15 `apply_damage`/`instantiate_stock_space_unit`/`build_space_damage_mutation_plan`/`step_wall_ms_at`, `space_mind_seal_of`, `space_unit_overlay_for_id`, `rcs_authority_for_hull`, `holotank_feed`/`cockpit_feed`. Both the TUI (`tui/pixel.rs:1461`) and the web (`PictureView.svelte:841`) ship the `c` key as `play_to_clamp`, an auto-solver.

---

## 6. Real bugs beside the holes

### 6.1 Label and channels (P1)
- §4b. In addition, autopilot grants the free flight or recouple before its debit resolves; a burn the provider later refuses (`port/burn.rs:507–513`) never rolls the grant back.
- `D-35-9` channels are decorative: `record_hull_standing_order` writes the private `SpaceBubbleBook.standing_orders` (commands.rs:2932–2946; tick.rs:722, 917–941), the handler then applies its own `CommandBurn` (1431–1469); nothing in tick.rs reads a `StandingOrder`.
- `helm_thrust` records the order before a possible reserve refusal and has no manoeuvre guard: outside a bubble it replies "Helm thrust set" with no effect (1431–1442).

### 6.2 Persistence (P1)
- One document, no `deny_unknown_fields` anywhere under `space/`, no identity rule, no per-entity key, fail-open decode to defaults. No dirty flag: every tick clones the book and the salvage book and compares with the last write (tick.rs:1069–1091), then rewrites the whole document.
- Never pruned: `derelicts`, `cleared_interdiction_legs`, `clamped_derelicts`, `active_tows`, `cluster_tenure_orders`, `propellant_ledgers`, `standing_orders`; `pending_space_burns` is not pruned on landing despite #335 (`port/burn.rs:299–308`), only drained to 16 at close; the dock path's `completed.insert` (commands.rs:4598) bypasses the cap; `SalvageClaimBook.black_box_tapes` holds 256 entries per hull for every fresh NPC hull; claims cap at 1,024 and then filing fails silently. The derelict countdown (tick.rs:1533–1551) forces a full rewrite every second for 48 h after every derelict.
- The OutOfGate path mutates (session removed, debit queued, hull unpaused) then returns `Err`, so `helm_cmd` skips the persist; a restart before the next tick restores the closed session (commands.rs:4479–4500 vs 976–979).

### 6.3 Custody (P1)
- 35b §5 #4 not fixed: a refused `check_out` falls back to `space_ship_from_tactical_unit`, an uncustodied copy, and a hull with no unit gets a stock Dray (tick.rs:2758–2795); its damage never persists.
- 35b §5 #5: the collar scrape checks in and immediately checks out again, both results discarded (commands.rs:3946–3955).
- 35b §5 #6: field boot adopts the `domain.N` mark and voids it with an empty check-in, then `reacquire_custody_after_boot` takes a fresh mark (`lib.rs:1344–1347`, `match_/resume.rs:293–299`, `field/boot.rs:104–140`). This works only while the world is not frozen; a boot under a W-clock freeze holds the field resume, the reacquire skips (still deployed), and when the clock resumes `run_held` voids the mark mid-bubble so the close drops all damage (custody.rs:816–821).
- In the port dock clamp, `remove_carrier_from_active` drops the docking hull's ship, `carrier_ship_id_for(hull)` falls back to the primary's, and `check_in_space_unit` writes another carrier's damage onto the docking hull (commands.rs:4552–4572; tick.rs:179–185, 817–838).

### 6.4 Ids, Things and clocks (P1)
- The fabricated constants are gone from production (`910_000`/`920_000` survive under `#[cfg(test)]`, picket.rs:1891–1910). What replaced them: NPC Things created per promotion with location `None` and owner GOD (picket.rs:1766–1846; tick.rs:2650–2661), never reused or destroyed by `clear_resolved_bubbles`; `resolve_hull_prior_owner` names GOD or `ObjId(2)` (tick.rs:1560–1590); the "base" is the waypoint's domain host (1594–1621); the claim records a physical attach and a bay load at the moment of disabling (custody.rs:946–958); `submit_contest`/`adjudicate_claim` have no live caller.
- **The space raider is the ground raider's barge.** `raiders().first()` (picket.rs:1794), unit `tacsim.unit.dray` (`content/field/raiders.json:11`), given the `raider_heavy` overlay and 60 m/s², and stamped with `RAIDER_ATTR` naming `tacsim.raider.ahuna_skulks` (picket.rs:1827). After one Kirkwood picket `@field/raiders tacsim.raider.ahuna_skulks` is refused ("still has a hull standing") and the raid director counts the space hull.
- 35b §5 #8 half-fixed: `real_now_secs` uses unix seconds on the tick path (tick.rs:1553–1556), but `+helm/tow` still computes `filed_real_s = w.now() / 1_000` with a `now.at / 24` fallback (commands.rs:4798–4804), so contest deadlines land in 1970 (doc 41 §3.10 found the same). The four windows (tick-count derelict, bubble wall, claim real seconds, tow real seconds) remain unreconciled and nothing acts at zero. A tow during the bubble followed by the close-time re-filing hits `DuplicateActiveClaimForSubject`, and `derelicts.insert` (tick.rs:1747) overwrites the tow's `PendingSalvageClaim` with `None`.
- A derelict record is never removed (tick.rs:1747, 1868; commands.rs:2009–2015, 2471): a towed and bay-repaired hull is refused every helm manoeuvre forever and is never interdicted again.
- The re-promotion guard is dead code: `cleared_interdiction_legs` stores `departed.at` at promotion (tick.rs:1680), `with_paused_at`/`with_resumed` shift it while Promoted (`journey.rs:430–452`), so the lookup never matches; a carrier that cleared Kirkwood is re-promoted into the next carrier's bubble.
- `+helm/dock derelict` and `+helm/decouple` still write `Promoted` from a player command with a `DomainId(1)` fallback (commands.rs:4693–4694, 4822; 2154–2155, 2203–2211); decouple's state vector is a straight-line interpolation with `elapsed_s == 0 → frac 0.15`, so decoupling the instant after undock places the hull 15% along the chord for free.
- Every Promoted journey pauses (`journey.rs:1296`), and D9(a) promotes any Cruising carrier whose destination domain activates (tick.rs:2768–2790): a field or staff promotion at a port freezes every inbound carrier's leg timer.

### 6.5 Hull fidelity (P1)
- `D-35-1` kernel: `SpaceShipState::apply_damage` (bubble.rs:675–866) is a private pipeline; nothing under `space/` references `arena/damage.rs` or `mush_tactics`' damage code. It splits each hit proportionally where the kernel draws one section (`choose_section`, `arena/damage.rs:881`); its Slow compounds (`rated_accel_mg *= pct` on every hit while the hold is at 0, bubble.rs:854) where the kernel takes `min` (damage.rs:243).
- A second hull definition survives as Rust overrides: corvette and raider `max_accel_mps2 = 60`, `delta_v_mps = 9_500` (custody.rs:481–484; bubble.rs:534–535; picket.rs:1626–1627); `corvette.json` authors no drive law; the raider's lasers, railgun and torpedo stats are Rust (picket.rs:1605–1638); `DRAY_P_HOLD_CAPACITY_T = 86` caps every hull (mining.rs:75; commands.rs:5433, 5613); NPC Δv is `new_dray`'s 4,500 constant (bubble.rs:476).
- `max_accel_mps2 = ceil(flip_accel)` (custody.rs:371; tick.rs:2823) turns 0.49–0.65 m/s² into 1, and `+helm/thrust` falls back to `signum` (commands.rs:1446–1458): an empty Dray and a Dray-P with 86 t accelerate identically at about twice rated, and throttle 1/1000 is full thrust. sfr11 asserts the 1.
- Crits (35b §5 #16): now inflicted, by hard thresholds (≥ 25 internal on front/top/sides kills a PDC, ≥ 35 degrades sensors; bubble.rs:796–825) that no unit file authors; check-in persists only the `power` crit (custody.rs:733–745), so a stripped PDC returns intact at the next check-out. Heat is right: the `heat`/`heat_sink` cells ride the TacticalUnit (custody.rs:563–582, 714–730) and raise IR (commands.rs:540–542); the radiator crit is not written to `heat_sink`, Halcyon has no heat cell.
- The raider doctrine cannot steer: `Speed` is a fixed world `[-15,0,0]` burn (picket.rs:1268–1283); the interdictor ignores its doctrine's lock, speed and fire intents and burns `[-10,0,0]` forever while firing (tick.rs:2291–2380). A non-Kirkwood bubble cannot be cleared by heaving to.
- `pixel_model` comes from callsign substrings (bubble.rs:578–615); the overlay field is never applied (custody.rs:199–206).

### 6.6 Tempo (P2)
- The step is measured wall elapsed (tick.rs:1932–1949) with 250 ms substeps (bubble.rs:25, 1812–1818), from `ctx.w.now()` in whole seconds. Clamp `(1..=10)`: elapsed 0 or a clock step backwards → 1 s; a 30 s stall → 1 s, 29 s lost. The Registrar tick is a fixed `TickCadence::Coarse(1 s)` (tick.rs:61, 1446–1450); "cruise 5 s" is fires skipped (1938–1944), so the 500 ms ordnance cadence cannot happen, and a command typed in cruise applies to a state up to 5 s stale and is credited retroactively. `step_derelict_approach_in_bubble` snaps 6 km per fire regardless of dt and bypasses the tape (tick.rs:2386–2440).

### 6.7 Mining (P1, 35b §5 #15 mostly unchanged)
- The clerk resubmits the first pending draw and prunes only landed ones (mining.rs:1572–1617); two hulls queuing 4 t each from a 5 t rock jam every hull at that cluster forever. `+mine/laser` has no Dray-P check (commands.rs:5356–5455).
- Clerks are now laid up from the tick (tick.rs:2235) but still as GOD, in GOD's location or inside the first arriving hull (mining.rs:659–672).
- Rate unchanged (4 t per 60 IC s laser, 4 × 4 t per 120 IC s drones). Claim-jump is a formatted-string compare against "now + full term" (commands.rs:5461–5466). Fine and licence bind to the movement seat (3674, 3732). Any one claim legitimises all `strategic_ore` (picket.rs:534–566). Sessions end on departure (2571, 2725, 2895) with no idle expiry.
- Seizure fires on any fine error and clearance is granted before the asynchronous ore seizure lands, result discarded (picket.rs:1535–1536).

### 6.8 Sensing and feeds (P1/P2)
- Light lag is never applied to timing: the hail is delivered at once with the lag printed (picket.rs:579–592); the doc comment cites a `light_lag_ms` that does not exist.
- Leaks of undetected contacts: the holotank publishes each detected torpedo's target position without checking the target's detection (bubble.rs:2338–2350); `+helm/dock` refusals name an undetected hostile by callsign (commands.rs:4142–4151); `resolve_docking_target` and `+helm/approach` match any bubble ship by callsign substring and print callsign and ObjId (4009–4036, 3508).
- Panels: fixed for bubbles and free flight to all crew (tick.rs:2100–2117, 2254–2256); cluster sessions get no tick push; the Tier-1 feed is fabricated (`delta_v_mps: 4_500`, hull 100%, heat 0, commands.rs:765–830). `v` is bound on the chart screen (`screen.rs:1066–1069`), not the helm/tactical screen `D-35-16` names.
- `D-35-7`: the hook is right (tick.rs:1438–1447; presence from `is_connected && has_live_conns`, 1269–1273; 120 s grace expires on the tick, 1407–1429). The queued return plan is a port-name string that never runs; expiry only zeroes thrust (1342–1375); cluster sessions are not covered; detection is event-only, so a bubble opened after the crew went linkdead starts with `crew_linkdead = false`.

### 6.9 Authoring (P1, `D-35-17` part done)
- Done: doctrine `.rhai` in `content/minds`, registered in the studio; `outer_bodies.json` through `load_pack` with a digest (sky.rs:165–183); the picket opens only on a `kirkwood_picket: true` host (picket.rs:467–512); `INTERDICTOR_PATROL_RHAI` survives only in a doc comment.
- Not done: clusters are a four-variant Rust enum carrying id, zone, anchor body and label (mining.rs:109–319); rock tables come from the compiled-in `LANE_JSON` through a process-global `LazyLock` (mining.rs:203–206), not the applied pack, with authored `zone`/`anchor_body`/`label` ignored, all four clusters on `kirkwood_depot`, and a load failure yielding empty rocks silently; the picket cast is Rust (picket.rs:1605–1638, 1755–1893); doctrines run on a private cached `RhaiRunner::new` (654–677) with `source_lookup(None, …)` (1019, 1201), not the sealed runner host, no MindPool cadence, and the interdictor reads `interdictor_patrol_source()` directly, ignoring its seal (tick.rs:2295); drones are timers paying a fixed 4 t (mining.rs:1335–1360) with a hand-built `ExcursionId` (commands.rs:5639) and their mind's intents discarded.

### 6.10 Docking client and canopy (P1)
- Three hand copies of the kinematics (`docking.rs:677`, `mush-pixel/src/docking.rs:465`, `web/src/lib/picture.ts:894`) pinned by one autopilot trajectory (voyage.rs:7852–7900). No web tape ever reaches the server. Both clients bind `c`/Space to `play_to_clamp` against the published pose. `solve_docking_tape` is `pub` and re-exported (docking.rs:977; mod.rs:51). The pose seed is `next_docking_session_seq + hull × 131` into fixed tables (docking.rs:347–385; commands.rs:4654–4662).
- The expiry "15 kg RCS" goes only into `record_burn_kg`; its whole-tonne return is discarded and no `PendingSpaceBurn` is submitted (tick.rs:1514–1516). `+helm/approach` (commands.rs:3512), dock RCS (4543) and the expiry debit the ledger without touching the bubble ship's private `propellant_kg`/`delta_v_mps`, so a jink on stale in-bubble propellant is free when the burn action refuses.
- The Neptune and Triton canopy is `default_canopy_bodies_for_site`, a table keyed on the target name with fixed bearings, emitted at departure (bubble.rs:1040–1071; commands.rs:786), not the sky ephemeris.

### 6.11 Output and conventions (P2)
- "Show tactical weapon mounts" (commands.rs:125); `+tac/scan` prints "IR {}, margin +{}" (3095); `+helm/status` and `+mine/status` dump raw mm/s (1030, 1052, 4968); player text shows `paused_from_world_tick=…` (1100, 4341, 4726); the cockpit feed ships `lead_bearing_deg`/`lead_pitch_deg` (bubble.rs:2415) drawn as a lead pip (owner's call under the no-targeting-maths rule); `+mine` refusals still name the removed `+helm/cluster` (5008, 5119, 5238, 5383, 5561).
- No `docs/help` topic names `+helm`, `+tac` or `+mine` (every other track has one; `help_commands.rs` checks the opposite direction only). No mush-bot vocabulary or lane. The allowlist notes are stale (§3).
- `[tacsim].space_pulse_ms` is parsed and inert; the space tick ignores it.

---

## 7. Per-cut disposition

| Cut | Verdict | Keep / fix / re-cut |
|---|---|---|
| SF.R9 | The four 35b §3 holes closed as written; `+helm/run` guarded; `unsigned_abs`; `TankReserveWarning` returned. New: §4 #1 (DOCK ABORT), #2 (berth autopilot), #3 (expedition cap), #7, #8; the port branch's range proxy | **Keep; close §4 #1–3, #7, #8 in SF.R17** |
| SF.R10 | Measured-elapsed step, 250 ms substeps, caps, quarantine, cached runners, reacquire after boot: real. Clamp 1–10 s, fixed 1 s cadence, no dirty flag, seven unbounded collections, one document | **Keep; tempo cap and the per-entity store in SF.R17 / SF.R21** |
| SF.R11 | In-bubble burns sync to the ledger, no refill on re-entry, post-burn reserve, `as_principal`: real. `ceil` on the rating, instant 1 s Δv on thrust change, approach/RCS/expiry debits bypass the bubble copy, GOD fallback with discarded errors | **Keep; fidelity in SF.R20, ledger unification in SF.R21** |
| SF.R12 | `by_domain` sharing, NPC Things under custody, real leader on the claim, seizure path: real. §4 #4, #5, #6; GOD-owned Things never destroyed; the raider is the ground barge under `RAIDER_ATTR`; tow a record; 1970 stamps | **Keep the keying; re-cut the multi-carrier bubble (SF.R18) and the salvage path (SF.R19)** |
| SF.R13 | Tick panels to all crew, torpedo detection filter, linkdead hook on real presence, TUI/web keys: real. Return plan never runs; clusters uncovered; Tier-1 feed fabricated; light lag printed not applied; undetected contacts named in refusals | **Keep; remainders in SF.R23** |
| SF.R14 | Doctrines, mind seals and `outer_bodies.json` in content: real. Clusters a Rust enum over compiled lane JSON; private runner, seals bypassed; drones timers; interdictor ignores its doctrine | **Keep the content moves; re-cut the host and the sites (SF.R22)** |
| SF.R15 | Heat cell on the TacticalUnit, facet weights read from `UnitSpec`, PDC/sensor crits inflicted, bay repair: real. Private damage pipeline, compounding Slow, thresholds not components, crits not persisted, drive constants, the 86 t cap | **Keep heat; re-cut damage and drives through the kernel (SF.R20)** |
| SF.R16 | mush-pixel client → server round trip, per-session pose, mandatory id, pause covers clusters and journeys, contract lint: real. Three kinematics copies, auto-solve key, no web tape, a canopy table, the lint's blind spot, and the docs of §3 | **Keep the client; one kinematics source and the ephemeris canopy in SF.R24; the lint in SF.R17; the docs in §8** |

35b §5 at `f1b2a1e4`: fixed #10; part-fixed #1, #2, #3, #5, #6, #7, #9, #11, #12, #13, #14, #16, #17, #19, #20; not fixed #4, #8, #15, #18.

---

## 8. Doc corrections owed (do these first)

*(Closed by PR #362 across items #1–#7 below.)*

1. Rewrite 35-cut-notes §SF.R9–SF.R16 from the PR bodies and this review: the real scope, the real test name, the real SHA, and a Divergences line per cut. Delete the five dead SHAs and the "#346" claim.
2. Doc 35 Draft 0.5: §3 drops `raider_heavy.json` and names what exists; `D-35-3` states today's `last_stepped_wall_s` (persisted, whole seconds, 1–10 s clamp, fixed 1 s cadence) as the state and the measured-elapsed-with-cap as the target; `D-35-5` says every Promoted journey pauses; `D-35-12` says one key `tacsim:space:state` today and the per-entity split as `SF.R21`'s target; `D-35-14` names `DOCKING_SESSION_TIMEOUT_WALL_S`, the free re-initiation, and the three kinematics copies; §10.2's disposition column is 35b §6 verbatim; §10.3 is §7 above.
3. README rows for doc 35 and 35-cut-notes: "SF.R9–SF.R16 landed; reviewed in 35c; not seam-complete".
4. `legacy-handlers.toml` notes for `+helm`, `+tac`, `+mine` refreshed in the `+ship` style ("REFRESHED by doc 35 cut SF.R9/…"), naming every switch and what each writes or submits.
5. A `docs/help` topic for the three verbs (the only player-facing track without one).
6. `D-35-7` to say the return plan is owed, `D-35-17` to say clusters and the runner host are owed.
7. Doc 40 §4.6's "the space track persists none yet" is wrong since SF.R8; FR.6 must say the persisted `SpacePartyId` values are wiped, not migrated (Q-38-9 makes that free).

---

## 9. Re-plan: `SF.R17`–`SF.R24`

One seam per cut. Every acceptance test through registered handlers with a non-GOD actor; **no `solve_docking_tape`/`solve_hard_bounce_tape` from a story** (the client session produces every tape); no `seed_pool` after departure; the lint of `SF.R17` pins both.

| Cut | Scope | Engine seam | Acceptance |
|---|---|---|---|
| **SF.R17** | **Hole close-out II.** `DOCK ABORT`/`order abort` act only on an existing session and never resume a journey; `+helm/autopilot` refuses from `Berth::Docked` ("undock first") and `+ship/undock <cluster\|body>` takes a cluster or body as destination through `prepare_departure`, with arrival promoting the cluster session on the tick; the expedition branch refuses when the tank or locker is short (`force` is not an override here); `+helm/dock` refuses while a session is open and the deadline is not reset; `holding_for_salvage` is per hull with a wall timeout and never blocks another carrier; the dock range gate measures range and speed on every branch (no 60 s proxy, no 2 km → 200 m); the tempo step is `min(elapsed, MAX_BUBBLE_TICK_STEP_MS)` with a true catch-up and a monotonic source; the expiry RCS debit is a submitted burn; the contract lint matches the qualified path, aliases, the `&mut World` mutators and every space book, and forbids the server solvers in stories | the existing verbs; `JourneyState`; `Registrar::tick` | sfr17: a free-flyer's `DOCK ABORT` leaves the journey Promoted; `+helm/autopilot pallas_inclined_belt` from Ceres is refused and `+ship/undock pallas_inclined_belt` opens the cluster on arrival; `+helm/autopilot neptune` with 1 t is refused; a second `+helm/dock` is refused; a 30 s late tick advances 30 s; sfr5/6/9/16 pass with no berth autopilot and no mint |
| **SF.R18** | **The multi-carrier bubble (Q-36-48, doc 40 FR.6).** Each carrier is its own arena `PartyId` (`SpacePartyId` deleted, `SpaceRelation` onto `Relation`+`Mark`, `Inspecting` a picket flag); inspection status, relation and clearance per carrier; settle, clear and resume evaluated per carrier; every carrier checked in on every exit path; stand-down when the last leaves; `cleared_interdiction_legs` keyed by leg index; the port clamp checks in the docking hull's own ship | `arena::party::PartyId`; `custody`; `ActivationBook` | sfr18 (the escort story): a clean escort's submit does not clear the smuggler; the raider disables the escort and the miner resumes while the escort becomes a derelict; both hulls checked in; the domain count returns to zero; the cleared carrier is not re-promoted by the next arrival |
| **SF.R19** | **One salvage path (Q-36-46).** `+helm/tow` files through `file_salvage_claim` with the real title holder or charterer and a real base; attach, tow and load as separate acts with the recovery capacity check; one 48 h real-second window from one helper (the 1970 stamps gone); the bounty an obligation settled once per derelict on delivery; the derelict moves with the tug and lands at the port; derelict records removed on repair; `SalvageClaimBook` reduced to the bubble's black box | `surface/salvage.rs`; `+salvage`; `Ledger` | sfr19: the disabled raider appears in `+salvage` and can be contested; the tug collects once, at delivery; a second tug is refused; a repaired hull manoeuvres again |
| **SF.R20** | **One damage table and canonical drives (Q-36-47, D-35-1).** Section choice and consequences through the kernel (or `ShipSectionId` → `CarrierPart` in the one module both tracks read); Slow takes `min`; PDC and sensor as authored components with crits persisted on check-in; drive from the rocket equation for every hull, no `60`/`9_500`/`4_500`/`86` constants, `corvette.json` authors its drive law, `max_accel` in milli-m/s²; a space raider unit and cast authored for space (never `raiders().first()`, never `RAIDER_ATTR`); NPC Things reused per domain and destroyed on stand-down; `pixel_model` from the overlay | `arena/damage.rs`; `UnitSpec`; `UnitStore` | sfr20: an empty Dray and an 80 t Dray-P accelerate differently and under rating; a rear `+tac/beam` splits hull/drive per `dray.json`; a stripped PDC is still stripped after check-in and check-out; `@field/raiders` works after a Kirkwood picket; the world has no leaked NPC Things after the bubble |
| **SF.R21** | **The prepared bubble mechanic and the per-entity store (D-35-9, 35b §3b).** One prepared action for bubble input that `+tac/*` and `+helm/heading\|thrust\|emcon\|roll\|guidance\|approach\|match\|submit\|run` submit through `Ctx::submit_authoritative`, whose prepare re-checks seat, aboard and detection and whose apply records the tape entry; the tick reads mush-sim's `StandingOrder` channels as the hull's continuous orders (the private map deleted); persistence split into bounded per-entity documents (`bubble:<domain>`, `flight:<hull>`, `derelict:<hull>`, `docking:<hull>`, `ledger:<hull>`) with `deny_unknown_fields`, an identity rule, fail-closed decode, written dirty-only on the mutation's batch; every §6.2 collection pruned; one propellant ledger (approach, RCS and expiry debit the same copy); the allowlist notes refreshed to say so | `Ctx::submit_authoritative`; `mush-sim` channels; `TacsimStore` | sfr21: a gunner's `+tac/beam` is one submission in the action log; a heave-to is the Thrust channel at 0 read by the tick; the write-census stories are green; a derelict countdown writes one small document; a restart mid-OutOfGate does not revive the session |
| **SF.R22** | **Authoring II (D-35-17).** Clusters as doc 20 §17 field-kind orbital sites in the applied lane pack (the enum deleted; zone, anchor and label from content; a load failure refuses boot); doctrines on the sealed runner host with `TacsimStore::mind_source` and MindPool cadence (the private `RhaiRunner` deleted); the raider's and interdictor's lock, speed and fire intents honoured so a bubble clears by heave-to; drones as `surface/excursion` sorties with sealed minds; the picket cast from content | lane pack; sealed minds; `surface/excursion` | sfr22: a second authored cluster on another body opens at that body; the interdictor stands down when the carrier heaves to; a drone sortie is an `ExcursionId` in the excursion book; no `OrbitalClusterId` compiles |
| **SF.R23** | **Mining and sensing remainders (35b §5 #15, D-35-10, D-35-7).** The clerk resubmits past a stuck draw with per-rock availability; clerks laid up by the site opener at the site; idle session expiry; rate on the doc 20 rig numbers; claim-jump by tenure lookup; fine payer the charterer or title; `strategic_ore` licensing per block; light lag applied to hail delivery and event timing; undetected contacts never named in refusals or approach; the torpedo target position gated; cluster sessions in the linkdead cover and the panel push; the queued return plan executes | `port/site.rs`; `mush_propagation`; the presence hook | sfr23: two hulls on a 5 t rock both finish; a holder on their own block is not flagged; the hail arrives d/c later; an offline crew's hull executes its return plan at grace expiry |
| **SF.R24** | **One docking kinematics and the ephemeris canopy (D-35-14, D-35-15).** mush-pixel's `docking` as the single kinematics compiled into tacsim, the TUI and the wasm, with `picture.ts` consuming the wasm (the third copy deleted); a pinned test that a recorded key tape from each client clamps on the server; the `c`/Space auto-solver removed from player keys; `solve_docking_tape` test-only; the canopy's `bodies` from `Sky` at the hull's state vector, pushed on the tick | `mush-pixel`; TUI; web; `Sky` | sfr24: a recorded human key tape clamps; the auto-solver is unreachable from a player session; Neptune's bearing changes with the hull's position |

Order: §8 first; then `SF.R17` (holes) ∥ `SF.R18` (needs FR.6's types or absorbs them: owner's call, Q-35c-1); `SF.R19` after `SF.R18`; `SF.R20` ∥ `SF.R21`; `SF.R22`–`SF.R24` after `SF.R21`. Later, unplanned: Tier-1 light-lag sensing, the claim-jump event and rival, NPC space traffic, a mush-bot space lane.

**Q-35c-1 — FR.6 ownership.** Doc 40's FR.6 deletes `SpacePartyId` "types only". `SF.R18` needs the per-carrier party. Proposed default: `SF.R18` absorbs FR.6 and doc 40 marks FR.6 done by it. Alternative: FR.6 lands first in the field programme and `SF.R18` waits.

---

## 10. Verified compliant

- `+helm/cluster` is gone and nothing else inserts a cluster session; cluster arrival charges time and propellant.
- `+helm/dock`: same-bubble derelict or leg-end port; Hostile and ordnance refusals; mandatory single-use id; every replay outcome closes the session; `DOCKING_SESSION_TIMEOUT_WALL_S` exists and `expire_stale_docking_sessions` runs on the tick; a berthed hull gets "not on final approach"; the in-bubble clamp checks in, files the completed record and stands the domain down.
- The timed-burn latch (tick.rs:556–609); `ensure_hull_can_maneuver` on all six verbs; `fail_carrier_journey` on the primary; `+helm/run` guarded per bubble; `unsigned_abs` on player integers; `TankReserveWarning` returned.
- `by_domain` sharing and the join path; NPC hulls as Things through `custody::check_out`; `910_000`/`920_000` test-only; `real_now_secs` in unix seconds on the tick path.
- Measured-elapsed stepping with 250 ms substeps; no catch-up burst after a restart; no-op ticks skip the write; snapshot cap keeps the first; tape compaction after ticks and commands; quarantine held across persist, tick and all three verbs, `persist_space_state` the only writer.
- `+helm/burn` capped at rated acceleration and reserve on the post-burn state; in-bubble `CommandBurn` and substep burns sync to the ledger; re-entry rehydrates without refill; tick burns through `Fire::as_principal`.
- `SpacePresenceHook` on Linkdead/Disconnected/Connected from real session presence; panels from the tick to `all_aboard_crew_for_hull`; `resolve_detection` on scan, lock, beam, torp, railgun and both feeds' ship and torpedo lists; a non-operational PDC skipped in the intercept loop.
- Heat on the TacticalUnit; sections, facets, internals, weights and consequences read from `UnitSpec`; carrier propellant from the ledger.
- Doctrines, mind seals and `outer_bodies.json` in content; the picket opens only on the authored host; the Rhai runner cached by source hash.
- The mush-pixel client reaches the server through `+helm/dock tape` and clamps (sfr16); stale and constant tapes fail.
- `journey.rs`'s doc comment matches its behaviour; 35b §7 #3 (doc 21 dithering amendment) and #5 (`D-35-19`) applied.

---

## 11. Refuted or not defects

- "The fabricated ids `900_000`/`+1000`/`ObjId(domain)` are still in production": refuted; test-only or gone (§6.4 lists what replaced them).
- "Doctrines are still Rust constants": refuted for the four doctrines; the clusters and the cast are (§6.9).
- "No linkdead hook": refuted; the hook is real, its return plan is not (§6.8).
- "The `real_now_secs` millisecond bug is open": fixed on the tick path, open on the tow path (§6.4).
- "The enable flag is owed": not owed, Q-38-9 (owner, 2026-11-03).
- "The surface/ edits in the range break D-35-2": refuted; they are doc 36a lane 5 (PR #346).
- "The write-census shards are red": refuted at `f1b2a1e4`; CI green.

---

## 12. The programme's space seams (doc 38 §9, wave 3)

The post-IC programme builds two seams in the space tree. The owner moved them to the programme
session (2026-11-04). Doc 35b §8 owns the space tree and the SF.R* train. Each cut here closes a
hole that doc 41 or doc 42 named. Neither cut adds a space feature. There is no enable flag
(Q-38-9). This section records what each cut built, so the next SF.R* writer reads it before
touching these paths.

### 12.1 RS.8, the salvage seams (doc 41 §4.7; Q-41-26, Q-36-46, Q-36-47)

Built on `main` at `7107db6d`. SF.R19 had already moved `+helm/tow` onto `file_salvage_claim`, but
it kept the RAM `SalvageClaimBook` claim path alongside it. RS.8 removes that path and its
fabricated principals.

- **One claim path.** `+helm/tow` no longer builds a claim itself. It puts the derelict in the
  tug's custody as a `SalvageCustody` row with `ClaimSeam::Owed`, then files through
  `surface::claim::tow_files_salvage`. That function is `+salvage/claim <hull> go`'s own code, so the
  audience, the `file_salvage_claim` contract, the `[tacsim.recovery].claim_window_s` window, the
  IC mail and the claim tick's `award_salvage_title` are all the ground's.
  - `register_derelict_salvage_in_book` is deleted, and so are the claim book's `submit_contest`
    and `adjudicate_claim` mirrors in `claim.rs`.
  - `SalvageClaimBook` is now only the bubble's black box: combat lines and `EpochClosedInBay`.
- **One window.** `DERELICT_WINDOW_S` is retired.
  - `HullStatus::DisabledDerelict` and `SpaceDerelictRecord` lose `salvage_contest_window_s`. Only
    the claim's window runs.
  - `DERELICT_LIFE_SUPPORT_S` (48 h) keeps life support, which is a separate fact (D-35-12).
  - A derelict record is retained while its custody row stands.
  - `SalvageTowRecord.contest_expires_real_s` is gone. `+helm` status points at `+salvage`.
- **`tacsim:space:custody:{domain}`.** `SpaceBubbleBook.space_custody` is a `SpaceCustodyBook`:
  rows grouped by recovery domain, at most `MAX_SPACE_CUSTODY` (32) per domain, as on a site.
  - Each domain is one `PersistedCustodyDoc` (`deny_unknown_fields`, identity rule), written
    dirty-only by `persist_space_state`. It never rides `tacsim:space:state`.
  - An unreadable custody document holds the whole space state for staff repair. It is not a
    domain quarantine, because a claim is never dropped silently.
  - `claim.rs` persists after every space claim order (file, contest, report, inspect, settle,
    at-risk).
- **Real principals** (doc 35b §3, §6.3 above).
  - The clamp's pin is `custody::pin_at_clamp`. It holds:
    - the derelict's economy title, and `claim::answerers_of` for who answers for it;
    - `GroundTenure::Unclaimed`;
    - as `Sensor` witnesses, the helm of every other carrier in the tug's bubble.
  - `resolve_hull_prior_owner` returns `Option`. It is used only to title an untitled derelict to
    whoever really held her, never the god object.
  - A derelict nobody held stays untitled. She is towed in custody, but her claim cannot be filed
    (the ground's `NO_TITLE`).
  - The actor's own hull is a self-recovery: no custody and no claim.
  - `resolve_nearest_port_base_obj` returns `Option`. A delivery with no real base writes no
    `EpochClosedInBay`.
- **The bounty is retired.** `SALVAGE_RECOVERY_BOUNTY_MAJOR` (1,200 cr),
  `SALVAGE_RECOVERY_CONTRACT_NAME`, `settle_salvage_recovery_payout_from_registry` and
  `paid_salvage_bounties` are deleted. The salvor is paid by the title, if the claim stands.
- **The retrieval's delivery at a dock** (doc 41 §4.7, Q-41-8). There are three paths:
  - A tug whose operator holds the derelict's retrieval (`retrieval::held_by`) tows her for the job
    (`TOW_FOR_RETRIEVAL`). She is in retrieval custody, with no salvage row and no claim.
  - `deliver_active_tow_at_port` calls `retrieval::deliver` for the towed derelict at the tug's
    port.
  - A hull docked under her own power delivers her own retrieval. On a lane arrival this is
    `TacsimCarrierHooks::arrived` in `composition/sim_routes.rs`, the one call outside `space/`.
    On a `+helm/dock` port clamp it is `tick::deliver_own_retrieval_at_port`.
- **Q-36-47** was already built by SF.R20: `ShipSectionId::carrier_part` maps `Hull` onto `Hull`
  and `Drive` onto `Drive`, and `Hold` onto nothing. Its `Slow` stays space-side. RS.8 records this
  in `carrier_damage.rs`'s module doc and adds no code.
- **Stories** (`shard_10/voyage.rs`):
  - `sfr19_…` is rewritten as RS.8's acceptance. It covers the real pin, the one window from real
    seconds against the economy's own window, a full restart reading the custody document back, no
    bounty, docking at Stickney, `+salvage` / `inspect` / `report` / `contest`, and the award.
  - `rs8_a_space_retrieval_job_is_delivered_by_the_tug_that_docks_her` is new. A staff
    `@asset/transfer` gives the Raider to a player as the fixture. The player posts the job, the
    tug reads `+salvage/jobs`, accepts, clamps and tows, and docking delivers and pays.
  - sfr7, sfr12 and sfr15 drop the bounty and read the custody row.
  - sfr2 and sfr8 read life support only.
- **Divergences.**
  - The stories dock at Stickney, not Ceres, because the test lane's interdicted leg is
    `ceres_stickney`.
  - The clamp's timing is the 6-DOF docking session, not `attach_s`. A second timer would be a new
    space mechanic.
  - The tow's cap stays the 2× dry-mass check. The carrier floor (`TUG_SHARE_PERMILLE`) is not
    applied in space, and no space hull is a carrier wreck yet.

### 12.2 CR.6, the crew seam (doc 42 Q-42-24)

Built on RS.8 (#439). Every crew rule is the ground's own `field/crew.rs`, called from `space/` only. The CR window owns `crew.rs`, so CR.6 does not edit it.

- **Crew records from the space check-in.** `check_in_space_unit` ends with `custody::check_in_space_crew`. It runs `crew::check_in` for every soul aboard (`field::carrier::souls_aboard`). The ground's rules apply:
  - a record in custody is left alone;
  - a check-in never heals;
  - a fallen hull (`CrewStranded`) strands its souls.
- **The derelict's crew.** A `DisabledDerelict` strands her crew as well: they are `Stranded { site, unit: hull }` and may ask `+sortie/extract`. The site is `custody::space_site`:
  - the lane entry whose map hosts the domain;
  - otherwise `space_domain_<n>`. The test lane's waypoints carry no port map, so its stories read `space_domain_<n>`.
  - The domain comes from the hull's bubble, the bubble's ship rows (a settled carrier stays there), her derelict record, the bubble she just left (`completed`), or her placement.
- **The hard-dock capture.** The derelict clamp in `+helm/dock tape` calls `custody::capture_at_hard_dock`.
  - The derelict's crew check in first.
  - Who is aboard is read from the records (`crew::held_in`), as a ground load reads it. A body need not be in her rooms.
  - The friend test is the ground's Friend rule (`field::parties::entrants` / `letter` / `friends`: one owner, or an open shared organization). It does not read the bubble's relation, because SF.R18 marks every co-participating carrier `Friend` for fire control.
  - A non-friend's clamp runs `crew::captured_loaded` into the tug's first bay room, else her airlock.
  - A friend's clamp runs `crew::aboard_loaded` into the same room.
- **The handover.** The `+helm/dock` port clamp calls `crew::hand_over(ctx, hull)` before the tow's delivery. A lane arrival already did, through `sim_routes`'s `arrived`, and the crew tick catches any it misses.
- **Story** (`shard_10/voyage.rs`): `cr6_a_crewed_derelict_hard_docked_by_a_stranger_is_captured_and_handed_over_at_port`.
  - A fixture sets SalvorB's drive down in the bubble; combat's disablement is sfr18's.
  - She settles as a derelict, and her pilot is `Stranded` in her.
  - SalvorA approaches and hard-docks her. The pilot is `Captured { carrier: SalvorA, captor }`, with her body aboard SalvorA.
  - SalvorA casts off and docks at Stickney. Stickney's authority takes the captive, and SalvorA's own record is `Carrier`.
- **Not built** (Q-42-24): ejection in a bubble, and surrender. The owner's ruling stands: crew whose carrier is wrecked fight to the death. Captives aboard NPC hulls are not covered, because NPC crew have no records (Q-42-5).
