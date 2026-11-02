# 35b. Review of the SF.R0–SF.R8 seam-first train against doc 35 Draft 0.3

**2026-11-02.** Reviewed at `origin/main` `f1ea7715` (the SF.R8 merge, PR #321); PR #324 (`1edc47b3`, idle-tick write skip) landed during the review and is noted where it matters. Authority: [doc 35](35-ship-free-flight-combat-and-mining.md) Draft 0.3 (`Q-35-1..6`, `D-35-1..18`, §9 stories, §10.2 cut table) and [doc 35a](35a-space-flight-review.md) (§1 seam table, §3 holes, §4 fiat audit, §5 bugs, §8 re-plan). Commits: SF.R0 `db251406`/`c4d6e533` (PR #297), SF.R1 `9c32adae` (#302), SF.R2 `5943fd78` (#309), SF.R3 `6bc1a9a1` (#311), SF.R4 `3b498c42` (#313), SF.R5 `6b090b4b` (#316), SF.R6 `859c5cd7` (#317), SF.R7 `3f8db79f` (#318), SF.R8 `009efd2f` (#321).

Method: twelve parallel reviewers, one per seam or theme (SF.R1 through SF.R8, test honesty, the 35a §3 holes, doc and boundary hygiene, the gap inventory). Every P0–P2 finding was then handed to independent readers told to refute it (two readers for P0/P1, one for P2), and every P0 was re-read in the tree by the lead. 231 raw findings, 191 after dedup, 170 stand (164 confirmed, 6 plausible), 7 refuted (§10).

---

## 1. Verdict

**The train is heading the right way and is no longer an island.** Every seam the owner's rulings name is now really called, the D-35-2 boundary held across all nine PRs, and 35a holes #1, #2, #6, #7 and #8 are closed. **But the cuts are only partly seam-first.** Three of the 35a patterns came back under new names, and the acceptance tests lean on two of them.

| Ruling | What is real now | What is not |
|---|---|---|
| D-35-3, D-35-5, Q-CF-32 | `tacsim.space` is a registered `reg.tick`; a real `CarrierJourney` crossing an interdicted leg raises `Interception`, `Promoted` pauses the leg timer, `resume_carrier_journey` + `stand_down_domain` close it (tick.rs:1328–1371, journey.rs:1294, 1386). Live W-clock stamps, ≤250 ms substeps, the Q-CF-33 freeze (tick.rs:871, 1155) | **Not 1:1.** A fixed 1 s coarse tick advances the bubble by its *step size* (5000/1000/500 ms) whatever wall time passed (tick.rs:62, 1159): cruise runs 5× fast, ordnance at half speed. Bubbles are keyed per hull, not per domain (tick.rs:485, 1370). The shipped lane has no interdicted leg (`ceres_phobos.json`), so nothing promotes in production |
| D-35-1, D-35-12 | `custody::check_out`/`check_in` are the field's own calls (custody.rs:443, field/admit.rs:393); logistics and anatomy read from the compiled `UnitSpec` and live cells; damage lands in `UnitStore` and shows in `+ship`; `hulls.json` and `SpaceHullCatalog` are gone | `SpaceShipState::new_dray` still hand-types what the overlay does not overwrite: the bubble drive cap is 25 m/s² against `dray.json`'s 50 mg (bubble.rs:345), heat is private (bubble.rs:355), mounts are hardcoded (bubble.rs:400), damage is a private pipeline that ignores facet weights and consequences (bubble.rs:500). Picket hulls are a second definition, `tacsim.unit.corvette`, on fabricated `ObjId(910_000+domain)` (bubble.rs:392, picket.rs:1249). Three custody leaks (§5 #4–#6) |
| D-35-9 | Aboard and seat gating is real and hull-scoped; seats ride mush-sim's `declare_station`/`assign_station` and publish `ChannelGrant`s; passengers, stowaways and berth-room bystanders are refused (commands.rs:258–345) | **The authoritative label is a relabel.** `+tac` submits nothing: `apply_bubble_command` resolves fire inside the handler (commands.rs:2286). `+helm` submits only propellant tonnage. Standing orders use mush-sim's struct type but live in a private map on `SpaceBubbleBook` that nothing consumes (tick.rs:495, 658); the ship moves because the handler applies `BubbleInputCommand`s directly (§3b) |
| D-35-10 | `resolve_detection` gates scan, lock and every fire switch; the bubble IR model is the designed one (EMCON floor, heat and thrust bloom, flash, aperture; commands.rs:495–555); ship contacts in both feeds are filtered | Light lag is computed and thrown away (the hail is instant, picket.rs:484); the holotank torpedo list is unfiltered and carries each target's true position (bubble.rs:1772); there is no Tier-1 sensing; mining bloom is outside the model |
| D-35-6 | Free-flight burns, autopilot and docking RCS read the hull's live `stow.propellant` and submit `PendingSpaceBurn` through the real `tacsim_port_burn` provider (burn.rs:448); whole tonnes charged up front, the remainder persisted; `plan_passage_from` / `return_reserve_from` really reuse `passage.rs` (passage.rs:998); `+helm/droptanks` and `/expedition` are gone | In-bubble burns debit a private `propellant_kg` against a constant 900 m/s reserve and never reach the vault (bubble.rs:1337–1346, 354). Timed burns latch when `dt == remaining` (§3 #3). Solar gravity truncates to zero at production cadence (tick.rs:387). The flip flag is write-only (tick.rs:345). `+helm/burn` accepts 100 m/s² per axis (commands.rs:1716) |
| Q-35-3, D-35-13 | `OreDeposit` pools open through the real `DepositOpen` action; stake, contest and the Syndicate seed go through `SurfaceTenure` with doc 20's term and window; `licence_gate` at 2,500 cr; ore moves deposit vault → hull hold through `tacsim_site_work`, conserved (site.rs:1276); the 3D velocity gate is exact | **`+helm/cluster` is a teleport** (§3 #2). `+helm/approach` snaps to a 600 m standoff in one tick (mining.rs:1277). Drones are timers on GOD-created DARK Things, not excursions (commands.rs:4289). Hold room ignores `hold_unreserved` and is the Dray-P's 86 t for every hull (mining.rs:1141, 1299). Clusters are Rust constants (D-35-17). Bloom and claim-jump are dead counters |
| D-35-11 | The two Rhai doctrines run on `RhaiRunner`, read contacts and readiness, and emit lock/fire/EMCON; `+helm/submit` reads the live `stow.ore`, licence and tenure and opens a real public E4 contract settled to `freeport_registry` on the sweep; the Customs Assertion front and clock advance in mush-faction; fire on the Inspecting corvette is refused; the railgun dodge and the PDC screen are emergent | The encounter is a Rust state machine (hail, heave-to, run boundary, relation flips, ambush timing in `step_kirkwood_picket`, picket.rs:796–821); the Rhai `this.stage` is never read. Not a doc 17 host: a bespoke synchronous loop with no seal, pool or cadence, faults silently dropped (picket.rs:577). The picket opens for any Dray-P at any waypoint (picket.rs:407). No seizure path for a broke pilot. `+helm/run` has no status guard |
| D-35-14, Q-35-5 | The server replay is integer-only, bounded (≤3,600 frames, RLE parse safe), starts every session outside the gates, debits RCS through the SF.R4 ledger, clears the pause on every terminal outcome, and `dock_carrier_at_destination` really merges into `Berth::Docked` | **`+helm/dock` has no range gate** (§3 #1). The client half was never wired: `DockingClientSession` and the web encoders have no callers, both clients still send bare `DOCK TRANS UP` lines no command parses (pixel.rs:1336, PictureView.svelte:813). The challenge is the same fixed pose for every session, the session id is optional, out-of-gate retries are free (commands.rs:3216, docking.rs:289). Tow only writes a record |
| D-35-12 (SF.R8) | `tacsim:space:state` is written in the same outbox as the mutation (atomic), restored in `on_world_loaded` before the first tick, and the sfr8 reboot really drops module RAM; docking sessions, ledger remainders, cluster sessions and Rhai memory ride the document | Unbounded: a full bubble snapshot is appended every tick and every command, completed bubbles are never pruned, the whole document is rewritten every second (tick.rs:685, 879, 1181); this broke the SM.12 write census (§2). An unreadable document becomes an empty book and is overwritten within a second (tick.rs:839; the 35a §5 #20 pattern). A restart mid-bubble lets the field boot adopt and void the space custody mark (§5 #6) |

---

## 2. What was run

| Suite | Result |
|---|---|
| `cargo test -p tacsim --lib -- space::` | 14 passed |
| `cargo test -p tacsim --test shard_2 -- sfr` | 8 passed (sfr1–sfr8), 9.4 s |
| CI on `f1ea7715` (run 41527199335) | **red on all six `tacsim-integration` shards.** `cost_lane::sm12c_every_act_kind_counts_its_writes_at_the_port`, `sm12c_a_restart_inside_the_cohort_changes_nothing`, `drones::sm12b_a_delivery_is_one_receipted_bundle`, `field_carrier::f6_…`: every write-census story sees `tacsim:space:state` written on fires that have nothing to do with space. PR #321 merged with the same shards red |
| PR #324 `1edc47b3` | skips the write on idle ticks; its CI run was still in progress at the time of writing. It does not bound the document (§5 #9) |

The space module is `crates/tacsim/src/space/` at 14,087 lines (bubble 2,112, commands 4,336, custody 649, docking 1,056, mining 1,682, mod 882, picket 1,454, tick 1,572, weapons 344). `commands.rs`, `mining.rs`, `docking.rs` and `picket.rs` were deleted in SF.R0 and are back under the same names; what matters is what they call, and §1 says which calls are real.

---

## 3. Authority and exploit holes (P0, player-reachable today wherever tacsim is composed)

Every item below was confirmed by two independent readers. The nine P0 findings collapse to four holes and one test consequence.

1. **`+helm/dock <any open port>` from anywhere.** The only gate is `require_aboard_and_seat(ctx, Some("movement"))` (commands.rs:3082). `resolve_docking_target` matches any `open_port_places` entry by name and the first derelict in `book.derelicts` for the token `derelict`, with no distance, relative-speed, berth, leg or hostility test (commands.rs:2951–3033); `MAX_DOCKING_INIT_RANGE_M` and `MAX_DOCKING_INIT_REL_SPEED_MPS` (mod.rs:84, 87) have no callers. On `Clamped` the handler runs `book.by_hull.remove(&hull)` and `mush_sim::dock_carrier_at_destination` (commands.rs:3309–3311), which sets `journey.destination` and the cursor past the last leg (journey.rs:1571) or, with no journey, moves the carrier subtree (journey.rs:1584). The start pose is the constant `standard_port` (docking.rs:289) and `solve_docking_tape` is public, so one pasted line always works. Effects: a pilot skips the remaining legs, propellant and IC time; an active bubble is dropped without `check_in_space_unit`, a completed record or `stand_down_domain`, leaking the custody marker and an `ActivationBook` slot; and because the session sets `paused_from_world_tick` and `step_active_bubbles` skips the whole bubble (tick.rs:1156) with no deadline (`started_ic_s` is written and never read), **a pilot under fire types `+helm/dock stickney` and the raider, the picket and every other party stop indefinitely, through restarts too**. sfr7 (voyage.rs:5307) and sfr8 (5569) assert this jump as success.
2. **`+helm/cluster <id>` from a berth.** `helm_cluster` checks aboard and seat, then inserts a session at `ship_pos_m: [3_800, 0, 0]` (commands.rs:2508–2531). It never reads the berth, the journey, the pose or an active bubble; `anchor_body()` (mining.rs:369) has no caller; sessions are only ever inserted, never removed. A ship docked at Ceres mines Pallas or Saturn ore with no transit, no propellant, no interdiction exposure. sfr5 does exactly this and sells at Ceres (voyage.rs:4316–4506).
3. **Timed burn latches free Δv when `dt == remaining`.** `burn_s` takes the `dt_ic_s >= rem` branch and clears `burn_remaining_ic_s` (tick.rs:353–356); the only line that zeroes thrust requires `burn_s < dt_ic_s` (tick.rs:399), and the `burn_s == dt_ic_s` branch holds a comment (401–405). On every later tick `None => dt_ic_s` applies full thrust with no further debit. At rate 1 every burn hits this; at any rate a player picks `dur` to hit it. sfr4 steps 600 s past a 400 s burn and never takes the path.
4. **A disabled derelict can fly home.** `settle_disabled_carrier_bubbles` removes the bubble and inserts a derelict but never resumes, fails or captures the journey (tick.rs:1047–1133), so it stays `Promoted`, which `is_under_way()` accepts (journey.rs:137–141). `helm_decouple` refuses only an active bubble or an existing free flight (commands.rs:1538–1560); `helm_burn`, `helm_autopilot` and `helm_dock` never consult `book.derelicts` or the drive cells, and the recouple uses the canonical drive spec rather than the damaged section. The crew of a drive-dead hull with an open 48 h claim leaves while the claim is pending; the stale record also keeps it out of every future interdiction (tick.rs:1347).
5. **Consequence:** the sfr5 acceptance criterion is met literally and reached only through hole #2; the sfr7/sfr8 criteria through hole #1 (§4).

Two more P1 items belong next to these. `+helm/run` has no status guard and advances the shared Customs Assertion front by one per call (commands.rs:2840, mush-faction lib.rs:979). `+helm/dock derelict` clamps from across the system and `+helm/tow` only writes a record: the derelict never moves and the tow record is never cleared (commands.rs:3005, 3328, 3538).

### 3b. The Authoritative label

`+helm`, `+tac` and `+mine` are registered inside `classify_commands(Authoritative)` (tacsim/src/commands.rs:463) and `legacy-handlers.toml` says `authoritative`. mush-core's definition (cmd.rs:113–115) is "an ingress adapter for the authoritative action coordinator; the handler itself does not implement or persist the mechanic." `+tac` calls no `submit_authoritative`: lock, beam, torp, railgun and pdc resolve through `apply_bubble_command` inside the handler and reply with prose (commands.rs:2278–2300). `+helm` heading, thrust, emcon, roll, guidance, decouple, submit, run and tow mutate `SpaceBubbleBook` directly; burn, autopilot, approach, match and dock mutate directly and submit only the whole-tonne propellant debit. All three end with `persist_space_state`, a whole-book `JsonSet` post-image (tick.rs:789). `+mine` genuinely submits tenure, site work and the opener. So +tac is the 35a §3b misclassification under a new label, and the toml notes overclaim the standing-order seam. The fix is not a relabel back: it is a prepared mechanic for the bubble (SF.R9/SF.R11), after which the label is true.

---

## 4. Acceptance: what the tests actually do

The basics are honest in all eight: every actor is a spawned non-GOD player; GOD appears only in `@teleport` and `@tacsim/time` setup; time passes only through `TestWorld::fire_due`; no `#[ignore]`, no sleeps; queued work lands through real legal-action resolution. The problem is what they write by hand. Across sfr1/2/3/6/7/8 there are eight `world_mut().module_state_mut::<SpaceBubbleBook>()` writes plus direct calls into `clear_interdiction_bubble` and `apply_bubble_command`.

| §9 step | How the tests do it |
|---|---|
| Real interdicted leg promotes on the tick | **earned** (sfr1, voyage.rs:3057–3200), but on a test-only lane mutation (`lane_with_interdicted_kirkwood`, 3033) |
| Leg timer pauses across the hold, resumes to dock | **earned** |
| Encounter clears | **fiat**: `clear_interdiction_bubble(world_mut(), hull)` (3202); the production interdictor is mindless and never fires, so no production path clears it short of disabling it |
| Damage lands on the hull on check-in | write-back **earned**; the damage itself is **fiat** (`hull_sec.internals = 480; drive_sec.internals = 190` and `DisabledDerelict` set by hand, 3396–3425) |
| Off-ship, stowaway, non-captain refusals; channel grants; detection gating | **earned** (3837–3897) |
| Contact geometry for the detection test | **fiat**: `interdictor.pos_m = …; emcon = Ghost` written into the book (3976, 4039) |
| Decouple, timed burn, reserve warning, autopilot recouple | **earned** through handlers, but propellant is minted twice with `seed_pool` (4108, 4209), the reserve is tested with a burn 100× the drive's rating, `force` is never tested, and the final "docks at Stickney" string also matches the under-way line (964) |
| Fly into Pallas, ping, velocity match | **teleport** via hole #2, then `+helm/approach` snaps to 600 m |
| 4 drones under `ExcursionId` + laser into `stow.ore` | ore conservation, licence, stake, contest window, sale and mill **earned**; the drones are timers delivering 4 t each |
| Light-lagged hail, heave-to under 10 m/s | hail **instant**; heave-to **earned**, with `+helm/thrust 600` at about 30× rated acceleration (4782) |
| Salvo, `+tac/pdc auto`, jink dodges the railgun | pdc and jink **earned** (4898–4933); the salvo is **never resolved**: torpedoes and slugs are cleared by hand (4970) |
| Submit to inspection, Ledger fine, front | **earned** (fine: 200,000 minor delta into `freeport_registry`) |
| Raider disabled into a derelict | **fiat**: raider moved to 3 km, facets zeroed, `drive.internals = 10`, lasers and heat reset, then one `+tac/beam` (4937–4985) |
| Corvette screens the salvo | **absent** |
| Bloom attracts a rival; duel disables its drive | **absent** / **fiat** (sfr2, sfr7: `iship.status = DisabledDerelict` by hand, 5103–5113) |
| Approach within 500 m, `+helm/dock`, tape game | session, pause, id/length/out-of-gate rejection and bounce damage **earned**; no 500 m gate exists; the "client tape" is the server's own `solve_docking_tape` against the constant challenge (5268, 5322) |
| Tow under the 48 h contest | **earned** through `+helm/tow` against a hull that is not a world object (`interdictor_hull = ObjId(900_000+domain)`, 5296) |
| Payout, doc 31 refit, decouple, Neptune on the IC clock, canopy shows Neptune and Triton | **absent** in every test; SF.R8's criterion nonetheless says "Story 2 survives a restart" |
| Restart mid-bubble and mid-free-flight | round trip **earned** (`reboot_with` drops module RAM, decoded structs compared); zero downtime is simulated, the derelict clocks are asserted only `<`, and custody across the restart is never checked (§5 #6) |

---

## 5. Real bugs worth fixing whatever happens to the cuts

1. **Tempo** (tick.rs:62, 1159): the step size adapts, the cadence does not. `+helm/thrust` also reports success when the bubble refused the burn (`TankReserveWarning` dropped, commands.rs:1228).
2. **Per-hull bubbles** (tick.rs:485, 1370): two carriers at one waypoint fight separate copies of the same interdictor; a cleared carrier is re-promoted on the same tick while the other is still promoted (tick.rs:1032–1040, 1328–1411).
3. **ActivationBook leak** (tick.rs:1409): settle, dock-clamp and past-boundary paths never stand down; after eight, every new interception queues and the carrier flies through, silently.
4. **Custody leak 1**: a refused `check_out` falls back to an uncustodied copy, and a hull with no unit gets a conjured stock Dray (tick.rs:1503–1529, the 35a §3 #3 hull off the player path). *Plausible, not unanimous.*
5. **Custody leak 2**: a collar scrape mid-bubble checks the hull in while the bubble keeps running; later damage is dropped (commands.rs:2899–2915).
6. **Custody leak 3**: at restart, `match_/resume.rs:296` adopts the `DeployedMark::field("domain.N")` the bubble borrowed, `field/boot.rs:137` finds no live record and voids it, and the restored bubble's close finds no marker (custody.rs:443, 502–507). SF.R8's "nothing is lost" is false for damage after a restart.
7. **Fabricated ids** (35a §3 #10/#11 back): interdictor `ObjId(900_000+domain)` (tick.rs:1452), picket hulls 910_000/920_000 (picket.rs:1249), prior owner `hull+1000` and base `ObjId(domain)` in every salvage claim (tick.rs:1002, commands.rs:3495), third contacts `hull+1000` (commands.rs:2978). The claim is filed at bubble close with a physical attach and a bay load that never happened (custody.rs:632–647); nothing outside space reads, contests or adjudicates it.
8. **Four unreconciled 48 h clocks** (tick.rs:882–897, bubble.rs:28, 1297; `filed_at_real_s`): counted in ticks, in bubble wall time (5× in cruise), and in real seconds; none does anything at zero. `real_now_secs` treats `w.now()` (unix seconds) as milliseconds (tick.rs:902), so filing times are 1000× too small; its `/ 24` fallback (also commands.rs:3468) is a hardcoded rate, reachable only when `w.now() == 0`.
9. **Persistence growth** (tick.rs:685, 879, 1181; port/burn.rs:448): snapshot per tick and per command, completed bubbles and `pending_space_burns` never pruned, `completed_space_mine_draws` never pruned; any seated player can grow the document with `+helm/roll 1`. The unreadable-document path replaces and overwrites (tick.rs:839). `RhaiRunner::new` compiles on every step (picket.rs:577).
10. **Reserve checked against the pre-burn velocity** (commands.rs:1768, 2479): a burn straight away from port passes while the pre-burn reserve fits.
11. **Tick-context debits are submitted as GOD** (mining.rs:1295): the burn offer requires the actor aboard, so cluster approach braking never lands, and the error is discarded.
12. **Decouple** writes `JourneyState::Promoted` with a sentinel `DomainId(1)` from a player command (commands.rs:1652), breaking mush-sim's one-writer rule; an unrelated stand-down resumes it to Cruising while the free-flight record also steps. The state vector is a straight-line interpolation with Ceres→Mars fallbacks (1581–1640). Nothing refunds the prepaid unflown leg.
13. **Free flight is never interdicted** and the autopilot recouple builds a single leg with interdiction 0 (tick.rs:1346, commands.rs:1928): decouple on leg 0 and autopilot to Ceres skips the picket for the burn cost.
14. **Docking session**: one constant challenge, optional session id, free out-of-gate retries (commands.rs:3216); the pause does not cover cluster sessions or a journey when docking a derelict (mining.rs:1238, commands.rs:3359); server and client kinematics are two hand copies with the RCS authority hard-coded on the client and no equivalence test (mush-pixel docking.rs:418).
15. **Mining**: one over-drawn rock jams the cluster's single clerk for every hull (mining.rs:1314–1334, site.rs:1457); cluster clerks, block places and deposit-vault containers are created by a player command, actor swapped to GOD, in that player's own ship room (mining.rs:670–681); sessions never end; 86 t fills in under a wall minute at rate 24; claim-jump detection compares formatted strings so a holder on their own block is flagged (commands.rs:4111); the fine payer and licence are whoever holds the movement seat, and any one claim anywhere legitimises all `strategic_ore` (picket.rs:453).
16. **Sensor and PDC crits can never be inflicted** (bubble.rs:535); §6.2's PDC-stripping counterplay does not exist. The raider's doctrine cannot steer: `Speed` is a fixed world −X thrust (picket.rs:898).
17. **TUI**: `v` is bound on the chart screen, where `pixel_sync` hides the canopy and docking views at once (app.rs:946–969), so the canopy's z/x/c keys are unreachable live; the web has no fire keys. Panels are pushed only as a reply to whoever typed (commands.rs:618); no tick publisher, no second crew member, no `bodies` or `pixel_model` in the cockpit feed.
18. **mush-sim**: the SF.R1 change pauses *every* Promoted journey, not only space bubbles (mush-sim tick.rs:2808, journey.rs:1294), and journey.rs:101–109 still says Promoted is only a marker.
19. **D-35-7** linkdead is not started: `crew_linkdead` is never set and the module `linkdead` hook is unused (bubble.rs:266).
20. **D-35-17** is not started: clusters, picket ships, both doctrine sources and `outer_bodies.json` are compiled in (mining.rs:192, picket.rs:112, sky.rs:49).

---

## 6. Per-cut disposition

| Cut | Verdict | Keep / fix / re-cut |
|---|---|---|
| SF.R0 | Fixes real (railgun frame, remainder, PDC cooldown, PN torpedoes, tape/snapshot, rings, bake). The "zero P0 holes" criterion is not met: four new ones (§3) | **Keep** |
| SF.R1 | Promotion chain real; tempo is not 1:1; per-hull bubbles; no production interdicted leg; placeholder interdictor cannot be cleared by play | **Keep the chain; re-cut tempo and keying** (SF.R10, SF.R12) |
| SF.R2 | Seam real on the happy path; 25 m/s² cap, private heat and damage, second hull definition, three custody leaks, write-only salvage half | **Keep the seam; fix the leaks now; fidelity in SF.R15** |
| SF.R3 | Seats and gating real; channels decorative; `+tac` resolves in the handler; TUI views unreachable | **Keep gating; re-cut the channel half** (SF.R9/SF.R11) |
| SF.R4 | Free-flight ledger and planner real; latch bug; bubble propellant private; no interdiction in free flight; decouple heuristics | **Keep; fix §3 #3 now; one ledger in SF.R11** |
| SF.R5 | Deposits, tenure, licence, conserved ore, sale and mill real; cluster entry and approach are teleports; drones are timers; clusters are constants | **Keep the engine wiring; close the teleports now; sites and drones in SF.R12/SF.R14** |
| SF.R6 | Fine, front, Rhai runner, PDC screen and dodge real; encounter is Rust; picket opens by hull type; phantom hulls | **Keep the economics; real hulls in SF.R12, doctrine host in SF.R14** |
| SF.R7 | Server replay real; range gate missing; no client; fixed challenge; tow by record | **Fix the gate now (§3 #1); client game in SF.R16** |
| SF.R8 | Store, atomicity, boot restore real; unbounded growth; fail-open decode; custody voided on restart | **Keep; budget in SF.R10** |

---

## 7. Doc 35 corrections owed

1. §10.2 stamps every row "(Built)". Replace with the dispositions above, as 35a §7 #3 asked for the first train.
2. §3 "What the tree has today" still says `Promoted` does not yet pause the leg timer and lists `MAX_ACTIVE_DOMAINS` as a seam; update to `f1ea7715`.
3. 35a §7 #5 was applied on doc 35's side only; [doc 21](21-graphics-engine.md) D-21-7 still says no dithering. Amend doc 21.
4. D-35-14 now states the replay, the frame bound and the RCS bound, but not: session single-use and expiry, how `start_pose` is chosen per session, the tape wire format, the per-hull `rcs_authority` in the challenge, or the lateral-velocity and angular-rate gates the code adds (docking.rs:31, 34).
5. Story 2 step 3's "approaches within 500 m" is story prose the code ignored. Make it a numbered decision: docking initiates only inside `MAX_DOCKING_INIT_RANGE_M` at under `MAX_DOCKING_INIT_REL_SPEED_MPS`, in the hull's current frame, against a target in the same bubble or at the end of the current leg.
6. D-35-3 should say "step by measured wall elapsed" rather than "adaptive 5000/1000/500 ms step", since the step size was read as the cadence.
7. D-35-5 should say whether every Promoted journey pauses (the semantic SF.R1 landed in mush-sim) or only space bubbles, and mush-sim's journey.rs docs must match.
8. README row for doc 35 says "SF.R0 built, SF.R1–SF.R8 planned".

---

## 8. Re-plan: `SF.R9`–`SF.R16`

One real seam per cut; every acceptance test in `tests/shard_*` through registered handlers with a non-GOD actor, **and no `module_state_mut::<SpaceBubbleBook>()` from a test** (pin it with a contract lint).

| Cut | Scope | Engine seam | Acceptance |
|---|---|---|---|
| **SF.R9** | **Authority close-out.** `+helm/dock`: target must be in the hull's current bubble or at the end of its current leg, inside `MAX_DOCKING_INIT_RANGE_M` and under `MAX_DOCKING_INIT_REL_SPEED_MPS`; refused while any operational party in the bubble is Hostile; the session pauses only the docking hull, or the whole bubble with a wall deadline after which it aborts with RCS spent; a port clamp from a bubble runs `check_in_space_unit`, files the completed record and stands the domain down; mandatory session id, single use, out-of-gate closes the session. `+helm/cluster` is removed: cluster entry is a site promotion at the cluster's body (D-35-5), or at minimum refused unless the hull's free-flight or bubble frame is anchored there. Timed burn clears thrust on `burn_s == dt`. Settling a disabled carrier fails or captures its journey, and decouple, burn, autopilot and dock refuse a hull in `derelicts`. `+helm/run` guarded by inspection status. `+helm/approach` flies rather than snaps. Remove the fiat from sfr1–sfr8 | the existing verbs; `JourneyState`; `custody` | sfr9: a pilot under fire cannot dock away; dock from the leg-end pose succeeds; `+helm/cluster` is gone; a 1 s burn at rate 1 stops; a derelict crew's decouple is refused; sfr5–sfr8 pass without a teleport |
| **SF.R10** | **Tempo and persistence budget.** Step bubbles by measured wall elapsed since the last fire (engine tick timestamp), with the adaptive period as the cadence, not the step; snapshots capped or taken on command only; tapes, completed bubbles, pending burns and finished draws pruned on close; persist only when dirty (after #324) and never the full history; quarantine an unreadable document like `JourneyBook::held`; cache compiled runners | `Registrar::tick` cadence; `TacsimStore` | write-census stories green on main; a ten-minute heave-to grows the document by a bounded amount; a tick fired 3 s late advances the bubble 3 s |
| **SF.R11** | **One ledger, rated drive.** In-bubble `CommandBurn` debits through `HullPropellantLedger` / `PendingSpaceBurn` with the rocket equation from `exhaust_kms`; `max_accel_mps2` from `rated_accel_mg` via `flip_accel_mps2`; `+helm/burn` capped at the rated value; reserve via `return_reserve_from` on the post-burn state; tick-context debits submitted as the hull's captain; decouple credits the unflown leg or autopilot charges only the difference; `+helm/thrust` reports refusals | `stow.propellant`; `passage.rs` | a bubble jink shows in the vault; re-entering a bubble does not refill; a burn away from every port warns |
| **SF.R12** | **Domain bubbles and real hulls.** `by_domain` keying: every carrier promoted at a waypoint shares one bubble; NPC hulls are world Things with `TacticalUnit`s from unit files (a corvette unit file; the raider from `raiders.json`) checked out through custody; the private relation map removed in favour of the arena party kernel; salvage claims name the real title holder or charterer and a real base; the interdictor gets a doctrine so a bubble can clear by play; stand-down on every exit path; the re-promotion loop fixed; a seizure path for a pilot who cannot pay | `ActivationBook` / `DomainId`; `custody`; `SalvageClaimBook` | the escort story: two chartered hulls at the Kirkwood waypoint share one bubble; the escort disables the raider; the derelict is a world object the miner later tows; the claim lists the raider's real owner |
| **SF.R13** | **Presence and feeds.** D-35-7 through the module `linkdead`/`revived` hooks (thrust to zero, PDC held through the grace window, queued return plan); panels published from the tick to every crew member aboard; `bodies` and `pixel_model` in the cockpit feed; torpedo list filtered by detection; space kinds reachable on the TUI home screen | module hooks; feed publisher; TUI app | a gunner sees the TTI count down without typing; an offline crew's hull drifts and holds its PDC |
| **SF.R14** | **Authoring (D-35-17).** Clusters as doc 20 §17 field-kind orbital sites in the lane pack (rock pockets over `OreDeposit`); picket casts and doctrine sources through doc 34 schemas and `TacsimStore::mind_source` on the sealed runner host with `MindPool` cadence; `outer_bodies.json` through the pack loader; the shipped lane gets the interdicted Kirkwood leg; `kirkwood_picket_domains` written from content; drones as real excursions on the doc 20 drone engine | lane pack; doc 34; sealed minds; `surface/excursion` | no Rust constant defines a cluster or a doctrine; the picket opens only on the Kirkwood leg; a drone sortie is an `ExcursionId` with a sealed mind |
| **SF.R15** | **Hull fidelity.** Damage through the kernel (`arena/damage.rs` section choice, facet weights, consequences, hold); unit heat cells; a space mount overlay keyed to `UnitId` within D-35-1's allowed scope; sensor and PDC crits; refit through the doc 31 bay with Story 2's payout | `arena/damage.rs`; `UnitSpec`; doc 31 refit | a rear hit on a Dray splits hull/drive per `dray.json`; a stripped PDC opens a gap |
| **SF.R16** | **Client docking game and the Neptune canopy.** One kinematics source shared by server and clients (or a pinned replay test); TUI and web play the 60 Hz game and submit `+helm/dock tape <id> <rle>`; per-hull `rcs_authority` and a per-session start pose in the challenge; Tier-1 canopy with bodies for the expedition | `mush-pixel`; TUI; web | a web pilot docks by playing; a hand-typed tape against a randomised pose fails; Neptune and Triton render on the canopy |

Later, unplanned here: Tier-1 light-lag sensing, the claim-jump event and rival, NPC space traffic, a mush-bot space lane.

---

## 9. Verified compliant

- D-35-2: no file under `arena/`, `field/`, `surface/` or `detachment/` was edited by any SF.R PR; the mush-sim, mush-faction, mush-pixel, `port/*`, `composition/*`, `store.rs` and `lib.rs` edits sit at the seam owners.
- No player command steps the world or bubble clock; `/pulse`, `/step`, `/enter`, `/refit`, `/droptanks`, `/expedition`, `/linkdead` and the frontier rival spawn are gone.
- `custody::check_out` with `DeployedMark::field` and `check_in` with a never-`None` IC plan through `apply_ic_condition_handoff` (Q-CF-35); port hulls carry a real `TacticalUnit`.
- `plan_passage_from` / `return_reserve_from` reuse `leg_seconds`, `passage_burn`, `crew_days`, `detour_km` and the intercept passes, and pick the cheapest open port; the sub-tonne ledger charges tonnes up front and survives restart.
- `OreDeposit` via `DepositOpen`; `SurfaceTenure` term and 30-day contest window; `licence_gate`; `tacsim_site_work` conserved transfer with a `PocketCursor`; ore specific volumes match `goods.json`; the velocity gate is exact 3D in i128.
- The E4 customs fine and the mush-faction front; manifest inspection reads the live pools and tenure; fire on Inspecting or Friendly parties refused; PDC and defence lasers screen them; the jink dodge comes out of the kinematics; submit-before-ambush is reachable (plausible).
- D-35-8 triad mechanics: PDC cooldown and saturation, laser Offense/Defense, PN torpedoes with coast, world-frame railgun; moving-shooter lead pinned by test.
- Server docking replay integer-only, bounded, RLE parse safe; RCS debited through the SF.R4 ledger; pause cleared on abort, bounce and clamp.
- Persist rides the mutation's batch; `boot_read` precedes the first tick and the front door; the restart test compares decoded state.
- TUI key discipline: no digit slot consumed, F1–F3 taken before the global switch.
- 35a §7 corrections 1, 2, 3, 4, 6 and 7 applied in Draft 0.3.
- D-35-4 outer-system ephemeris.

---

## 10. Refuted during verification

Not defects; recorded so nobody chases them: the absence of a literal `Queued.` in `space/` (the engine prints it on every accepted submission, engine.rs:5075); relations not sourced from faction standing (Q-CF-5 forbids the lookup in the first version); promotion inferred from the cursor's previous leg (no IC catch-up exists to skip it); the W-clock fallback `/ 24` as a production violation (unreachable once `set_now` has run; the millisecond bug in the main branch is the real one, §5 #8); "no second hull catalogue" and "+ship shows the damage" (true, but the damage was set by hand); "relations live on the arena table" (false, the private map is read first, bubble.rs:802); and "no panics reachable from player integers" (false: `i64::abs` on `i64::MIN` at commands.rs:1716 wraps in release).
