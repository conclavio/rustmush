# 35a. Review of the SF.1–SF.20 train against doc 35 Draft 0.2

**2026-11-01.** Reviewed at `origin/main` `497f2a78` (PR #291). Authority: [doc 35](35-ship-free-flight-combat-and-mining.md) Draft 0.2 (`Q-35-1..6`, `D-35-1..18`, §9 stories, §10 cut table). Commits: SF.1 `c28088e8`, SF.2 `b66ea2f3`, SF.3 `39847014`, SF.4 `abd9cd9a`, SF.5 `87378778`, SF.6 `82b78197`, SF.7 `269a5bab`, SF.8 `679ab7e4`, SF.9 `e6554051`, SF.10 `d44b37ab`, SF.11 `c87ddf9d`, SF.12 `dc02a868`, SF.13 `096470d2`, SF.14 `f14af5bc`, SF.15 `bec81dea`, SF.16 `b4df37eb`, SF.17 `2756b04b`, SF.18 `7bea1581`, SF.19 `33d1e325`, SF.20 `b4fd988a`. Five parallel reviewers covered the cuts by theme; every P0 below was re-read in the tree by the lead before it was written down.

---

## 1. Verdict

**The train does not implement doc 35. It implements a self-contained simulation that uses doc 35's vocabulary.** `crates/tacsim/src/space/` is 10,498 lines with 19 tests, and it reaches none of the seams the owner's rulings name:

| Ruling | Requires | What shipped |
|---|---|---|
| D-35-3, Q-35-4(1) | Tier-2 bubbles on 1:1 wall time (adaptive 5000/1000/500 ms), Tier-1 on the live `WorldTimeline` delta | **No tick at all.** No `reg.tick` in `space/`. Time moves only when a player types `+helm/pulse [dt_ic] [dt_wall]` or `+tac/step [ms]`, by whatever amounts they type, for every ship in the world |
| D-35-1, §6.3 | Reuse unit definitions, sections, facets, components, heat, `PartyId`, `custody::check_out`/`check_in`, input tape, snapshots, outage freeze, `MindRunner` | Hand-typed `SpaceShipState::new_dray` constants (drive 25 m/s² vs `dray.json` 0.49 m/s²), own `SpacePartyId`/`SpaceRelation`, own crit thresholds, no tape, no snapshot, `outage_frozen` never set, zero `MindRunner`/Rhai |
| D-35-9 | Write to `mush-sim` standing-order channels and seat roles | Zero `mush_sim` references. A second `BridgeSeat`/`ShipCrewRoster` in `SpaceBook` |
| D-35-10 | Light-lag via `mush-propagation`, IR vs distance² and aperture gating detection | Zero `mush_propagation` references. `+tac/scan` prints `true_distance_m` for undetected contacts; the holotank/cockpit feeds publish every ship's true state to every observer |
| D-35-5, §6.1 | Hook `LegDef.interdiction` / `PromotionTrigger::Interception` / `JourneyState::Promoted`; Promoted pauses the leg | `mush-sim/src/journey.rs` untouched. `InterdictionLegWatch` is a player-armed list (`+helm/watch`) that no `CarrierJourney` consults. `+helm/decouple` uses a hardcoded epoch `74_152_540_800` and a default `ceres→mars` leg; `+helm/autopilot` returns a struct and re-enters nothing |
| D-35-6 | Live kg debit from the hull's tank; `plan_passage_from(pos, vel, body)` reusing `passage.rs`; computed `RETURN_RESERVE`; `burn force` | Debit hits only `SpaceFlightRecord.propellant_kg` (every record starts at 48,000 kg / 24,000 m/s). `plan_passage_from` is `sqrt(dist_km) * 0.85` clamped, no velocity input. Reserve is the constant 6,000 m/s. Thrust latches after a timed burn and burns through the reserve on the next pulse |
| Q-35-3, D-35-13 | Ore into the real `stow.ore`; `OreDeposit` pools; Freeport Registry licence (2,500 cr), stakes, contests; 4 drones under the real `ExcursionId` | Space-only `OreHoldState` nothing else reads; own `AsteroidOrePool` with three hardcoded rocks per cluster; licence is a free record keyed by ship; stake/contest is a flag table; `ExcursionId(u64)` lookalike; drones are counters that deliver 2,500 kg each in one command |
| Q-35-5, D-35-14 | Client-side 60 fps 6-DOF game, ship paused outside the tick, verified outcome merging into `Berth::Docked` | Physics is server-side; both clients send `DOCK TRANS UP` text lines that **no server handler parses**. `+helm/dock clamp` steps 200 frames of `DockingControlInput::default()` from a start pose already inside every gate. Nothing reaches `Berth`; the port bonus is printed |
| D-35-12 | Disabled hull keeps its doc 23 epoch, persists as a derelict, 48 real-hour salvage contest | `epoch_open: bool`; life-support and contest clocks never decrement; `SpaceBook` is `#[derive(Clone, Debug)]`, no `Serialize`, registered with `reg.state(...)`: **everything is lost on restart** |
| D-35-11 | Multi-party bubble, Rhai picket, fine/seizure advancing the Customs Assertion front | Hardcoded Rust state machine; manifest hardcoded (`strategic_ore_kg: 18_000, valid_registry_claim: true`) so a live player can never be fined; `Fined` is a returned number, no Ledger, no front; `RanThePicket` never constructed; the ambush fires on the first `+tac/step` after heave-to, so "submit to inspection" is unreachable |
| D-35-16, SF.7 | Space views on a lettered TUI screen with a `v` toggle; weapon keys on `z/x/c` or F1–F3 | The TUI cannot open the space views: `toggle_for` is only ever called with Chart, Doll, Replay, Cockpit (`app.rs:2823-2834`). No `v`, no lettered screen, no weapon-group keys anywhere |
| D-35-17 | Clusters, pickets, doctrines through the lane pack / doc 34 schemas | `outer_bodies.json` has its own `include_str!` + `OnceLock` loader outside `RulesPack`, excluded from `Sky::revision()`; cluster content is Rust constants; `hulls.json` hot-publish writes `studio_tables`, but `+helm/refit` reads `SpaceHullCatalog::embedded()` |

The §10 table says every cut is "Built" and every acceptance criterion met. The acceptance criteria are not met for SF.3–SF.12 and SF.15–SF.19; see §4.

---

## 2. What was run

| Suite | Result |
|---|---|
| `cargo test -p tacsim --lib -- space::` | 19 passed |
| `cargo test -p tacsim --test shard_4 -- sky` | 16 passed |
| `cargo test -p mush-pixel -p mush-pixel-wasm -p mush-bot` | 888 passed, 0 failed |
| `npx vitest run` (web) | 890 passed, 1 failed on a stale gitignored `mush_pixel.wasm`; after `npm run wasm` the failing file passes 8/8. The rebuilt module is 1,527 KB against 880 KB before the SF train (the embedded planet maps, ship sheets and `ships/index.json`) |
| `npx svelte-check` | 0 errors |

Only 19 tests cover the 10.5k-line tacsim module; 14 of them are in `space/mod.rs`, none under `tests/shard_*`, none drive the registered handlers through a composed world with a non-GOD actor.

---

## 3. Authority and exploit holes (all P0, all live today behind `NOT_ENABLED`)

1. **Any player advances the whole space world.** `+helm/pulse <dt_ic> <dt_wall>` (`commands.rs:1339-1354`) iterates every flight, bubble, picket, cluster and watch for every player. Default seat for an unrostered actor is `Command` (`commands.rs:342`). `+tac/step 4294967295` runs about 17 M substeps on the single-writer world thread.
2. **Any player commands any ship.** `+helm/station <ship_id> command` (`commands.rs:1132-1152`) binds with no ownership or aboard check; `seat_for` defaults to `Command` (`stations.rs:154`); `station` and `linkdead` are always permitted.
3. **`+helm/enter <anything>` conjures a free, fuelled, armed Dray-P anywhere** and re-entering heals damage, refills propellant, wipes ore and clears derelict status (`commands.rs:738-818`).
4. **`+helm/refit <hull>` is free, instant, works mid-combat, and turns any ship into a Halcyon** (`commands.rs:1222-1308`); bypasses doc 31 and `port/refit.rs`.
5. **`+helm/droptanks`, `+helm/expedition`** are free, need no port, always pass `force_reserve=true`, and teleport the ship to the destination (`flight.rs:566-583`).
6. **`+mine/laser <rock> 100000`** fills the hold in one command; `+mine/drones` repeated gives 10 t per command with no cooldown.
7. **Any occupant can `+tac/beam` / `+tac/torp` any ship id in a shared cluster**, including another player's; `fire_*` never consults the relation table. Every PC is hardcoded `SpacePartyId(1)`.
8. **`+helm/linkdead on`** force-arms auto-PDC for the caller (`stations.rs:181-188`).
9. **`+mine/laser` in frontier space spawns a pre-crippled rival (`drive.internals = 10`, rear facet 0) and teleports the miner to 250 m of it** (`commands.rs:1966-1990`). This is the "laser/railgun duel" of Story 2 step 2, scripted inside the production verb.
10. **Ship ids map onto real dbrefs**: `ship.<n>` → `ObjId(n)`, others hash into 10,000..910,000 (`commands.rs:204-221`); black-box tapes are written against the actor's own dbref; collisions with live objects are possible.
11. **`+helm/tow`** registers `prior_owner_obj = ObjId(actor + 100)`, `port_base_obj = ObjId(900)`, hardcoded tonnages, passes IC seconds as `filed_at_real_s`, and swallows the cap error (`commands.rs:1100-1118`).

## 3b. Contract-test classification

SF.9 registers `+helm`, `+tac`, `+mine` as `legacy-post-image` (`legacy-handlers.toml:137,205,362`; `commands.rs:419`) with the note "no second money or quantity ledger". The file header excludes "anything that changes action legality"; these verbs run damage, derelict status, ore extraction, claims, tows, and keep `propellant_kg`, `delta_v_mps`, `lots_kg` as second quantity ledgers beside the hull tank and `stow.ore`. `+mine/licence|stake|contest` duplicate `+claim`, which is `authoritative`. The lints cannot catch this (the allowlist test checks presence and order only). **They must be `authoritative`**, submit through `Ctx::submit_authoritative`, and reply `Queued.` like every other submitting verb.

---

## 4. Acceptance: what the stories actually do

Story 1 (`mod.rs:292-420`, SF.9 verb test `:1216`, SF.11 bot `:1485`) and Story 2 (`mod.rs:1040-1213`) against §9:

| §9 step | How the tests do it |
|---|---|
| Real interdicted leg promotes | **fiat** `promote_interception(...)` with a literal manifest; no journey anywhere |
| Light-lagged hail | **absent**; constant string, no lag, not a `tacsim:notice` |
| Heave-to under 10 m/s | struct test earned; verb and bot tests never reach it (about 20–23 m/s) |
| 2-torpedo salvo, `+tac/pdc auto` | struct: `trigger_raider_ambush()` direct, `pdc_auto` pre-set; bot: toggles live but no torpedoes exist |
| 120 km railgun dodged by jink | struct earned, **but vacuous**: the railgun lead ignores shooter velocity (§5 item 1), so a 50 m/s shooter misses by about 300 m against a 60 m hit radius with no jink |
| Submit to inspection or burn clear | **absent** in all three |
| Raider disabled into derelict | **fiat** `raider.apply_damage(Rear, 350)` |
| Fly into Pallas, ping | fiat conjure; ping earned |
| Velocity match < 5 m/s in 2 km | **teleport**: `+helm/match` sets `pos = rock - 600 m, vel = rock_vel` from any distance (`mining.rs:689-718`) |
| 4 drones under `ExcursionId` | counters; launch and recovery in one call, no time passes |
| Bloom attracts rival; duel disables its drive | **fiat** spawn pre-crippled (test and production verb) |
| `+helm/dock` 6-DOF game | **auto-won**: default input from an in-gate start pose |
| Tow under 48 h contest | record with invented owner; nothing resolves |
| Payout, drop tanks, decouple | no payout; free tanks; hardcoded `ceres→mars` leg |
| Burn to Neptune on the IC clock | **teleport** ×3, each ship's private clock jumps, propellant lump-debited |
| Canopy shows Neptune and Triton | feed JSON `bodies.len() == 2`; no render |

SF.11's `SpacePilotDoctrine` is a hardcoded Rust open-loop script (every stage advances unconditionally after emitting a fixed batch, `autopilot_bot.rs:614-736`), not a Rhai doctrine, used nowhere outside tests. SF.12's `SpaceLane` "contract test" feeds hand-written server replies that have already drifted from the real server text (`M` vs `M-TYPE`, 3,200 vs 10,000 kg, 420 vs 600 m).

---

## 5. Real bugs worth fixing whatever happens to the module

1. **Railgun frame bug** (`weapons.rs:270-273`, `bubble.rs:1004-1009`): lead = target-relative, slug velocity = (lead − shooter)/tof, shooter velocity never added. A moving shooter always misses by |v_shooter|·tof. Masks the Story 1 jink criterion.
2. **Sub-metre motion truncates to zero** (`bubble.rs:1103`): `dpos = vel_mmps * step_ms / 1_000_000` with no remainder; under 4 m/s per axis nothing moves at 250 ms substeps. Breaks heave-to (<10 m/s), velocity match (<5 m/s) and docking approach. Same for torpedoes and slugs.
3. **IC stamp drift** (`bubble.rs:1057`): `is_multiple_of(1_000)` on `wall_elapsed_ms` skips seconds for any step not a multiple of 250 ms, and advances at a hidden 1:1 ratio instead of stamping the W-clock (Q-CF-32).
4. **Timed burn never ends** (`flight.rs:383-407`, `:494-501`): duration is checked against the reserve but not stored; thrust latches until Δv = 0.
5. **Tier-1 has no gravity** (`flight.rs:493-525`); Intercept/MatchVelocity use heliocentric speed for the flip test, so a ship co-moving with Ceres flips at rest relative to its target.
6. **Circular-sky `velocity_mps` is quantised to garbage** (`sky.rs:345-365`): millidegree phase over ±60 s gives Earth 21.8 or 43.5 km/s; outer bodies 0. Kepler path is fine; the circular fallback in `commands.rs:624-692` is not.
7. **Docking is not paused inside clusters or generic bubbles** (`pulse.rs:209-215` vs `:248-279`), and `paused_ships` entries are never removed, so a free-flight ship that has ever docked is frozen forever.
8. **PDC and defence lasers kill every torpedo in range in one substep** (`bubble.rs:1218-1274`): no Pk, no arc, no rate; saturation is impossible. Decoys spoof any torpedo within 35 km regardless of target.
9. **Torpedoes are pure pursuit, not proportional navigation, and are deleted at Δv = 0**; at 12 km/s / 250 m/s² their reach is about 288 km, so the §6.2 100–2,500 km band is unreachable.
10. **Ring-plane geometry swapped** (`planet_shader.rs:330-333`): in-plane coordinate and depth exchanged; minor axis is `R·tan(tilt)` not `R·sin(tilt)`; at Uranus's 97.8° the ring is a vertical smear; occlusion compares against the wrong coordinate.
11. **Velocity gate truncates per axis to whole m/s** (`mining.rs:680-683`): |Δv| ≈ 7.5 m/s passes the 5 m/s gate.
12. **Contest windows**: `48 * 3_600` IC seconds for both claims (`mining.rs:42`) and salvage (`docking.rs:43`). Doc 20's claim contest is 30 IC days (`tenure.rs:34`); doc 27's salvage window is 48 **real** hours (`salvage.rs:18`). At rate 24 the salvage window is 2 real hours.
13. **Hold volumes** (`mining.rs:94-101`, `:252-259`): ice 1100 / carbon 750 / metal 400 / strategic 350 mL/kg vs `goods.json` 1000 / 1200 / 500 / 600 L/t; Dray-P hold 100,000 kg vs `dray_p.json` 86,000 kg.
14. **Overflow on player input**: `thrust * max_a` and `ax*ax+ay*ay+az*az` in i64 (`bubble.rs:798-802`, `commands.rs:853,865,869`); `dt_ic_s as i64` wraps (`commands.rs:1341`). Panic in debug, wrap in release.
15. **Every non-Dray-P hull persists damage onto a `tacsim.unit.dray` unit** (`commands.rs:229-233`).
16. **Web `spaceMode` never clears** (`stores/picture.svelte.ts:77-80`): after one `space.*` keyframe the arena cockpit's draw and keys are lost for the session. Web canopy keys send absolute attitudes (`+helm/roll -15 0 0` sets yaw to −15, not −15 relative); `w`/`s` is a world-X burn.
17. **Planet bake is not reproducible**: `bake.py:207-288` takes a Tk subsample path or a pure-Python centre-sample path depending on the environment; the two produce different bytes and SHA-256s, and `index.json` does not record which ran. Both bake scripts default to `C:/Users/faeni/3d_assets`.
18. **`.clamp(24, w)` / `.clamp(2, h*3/4)`** panic on small frames (`docking.rs:382-385`, `space_cockpit.rs:252`); reachable through the WASM `render` size range.
19. `+helm/pulse` computes `now_ic = world_ic_now + dt_ic_s`, a future instant, without moving the world clock; `+helm/burn` steps one ship `dur_s` ahead of world time.
20. `embedded()` turns a `hulls.json` parse error into an empty catalogue silently (`hulls.rs:249-258`); same for `outer_bodies.json` (`sky.rs:114-161`).

---

## 6. Per-cut disposition

| Cut | Verdict | Keep / rework / remove |
|---|---|---|
| SF.1 sky | **Mostly real.** Horizons rows for Uranus, Neptune, Galileans; Triton one propagated 2198 row (Titan precedent); `velocity_mps` sound on the Kepler path. Clusters are sky rows, not doc 20 §17 sites; `kirkwood_gap_3_1` sits on Ceres's orbit (2.77 AU) not at 2.50 AU; ad-hoc loader outside the pack and outside `Sky::revision()` | **Keep**, fold `outer_bodies.json` into the pack loader, fix the Kirkwood elements, add derivative-vs-position and Galilean velocity pins |
| SF.2 renderers | Determinism credible (f64 + libm guard), Bayer matrix correct, colour budget ~173/256. Ring geometry bug; no golden frames; canopy projection linear; holotank has no dithering and a stretched plot | **Keep**, fix rings, add the goldens the row promises |
| SF.13/14/20 bakes | SHA tests real; no pack sources committed (licensing OK); provenance thin; non-reproducible decode path; `pixel_model` binding read by nothing; 1 pitch not 2; 6291-line pretty JSON should be compact | **Keep**, fix reproducibility, record bake params, compact the JSON |
| SF.7/18 clients | WASM ABI additive and safe for deployed dev1/dev2. TUI cannot open the views; `DOCK` orders hit no handler; web/TUI key tables disagree; web space mode sticks | **Rework** once a server exists to talk to |
| SF.3 bubble/weapons/picket | Island. Weapon maths partly right (velocity inheritance, laser falloff, heat gate); railgun frame bug; PDC always-kill; no tape/snapshot; picket hardcoded | **Rework as a kernel consumer**; salvage the weapon library only |
| SF.4 mining | Island; nothing reaches `stow.ore`, `OreDeposit`, Registry, Ledger | **Remove and re-cut** on the doc 20 engine |
| SF.5 docking | Server-side physics with a scripted win; no client game, no outcome, no `Berth` merge | **Remove the verb path**; keep the gate predicates and renderer |
| SF.6 flight | Fixture decouple, toy planner, teleport expedition, no gravity | **Remove and re-cut** on `passage.rs` and `CarrierJourney` |
| SF.8/9/11 verbs, stories, bot | RAM-only verbs misclassified as legacy-post-image; stories by fiat; bot is a Rust script | **Remove**; re-register as `authoritative` when there is a mechanic to submit |
| SF.10 custody | Real 27a types imported; written to a write-only shadow keyed by fake ObjIds; no `check_out`/`check_in` | **Rework** onto the field/match seam (`field/admit.rs:352`, `field/close.rs:223`) |
| SF.12 catalogue + bot lane | Read-only listing from code; parser self-test | **Remove** the lane test's claim; keep the catalogue |
| SF.15 stations | Second seat system; sensing gates nothing; linkdead manual | **Remove**; D-35-9 means `mush-sim` seats |
| SF.16/19 hulls | Third hull definition contradicting `dray.json`/`halcyon.json` (Dray 25 vs 0.49 m/s², Halcyon 1,200 vs 4,000 t); `+helm/enter` does not even use it; hot-publish not read by runtime | **Remove `hulls.json` as a hull source**; a space-only *overlay* (mounts, RCS) keyed to the real unit id is the most it should be |
| SF.17 pulse | Parallel watch list, no journey hook, no tick | **Remove**; the real cut is a `reg.tick` consumer of `JourneyState::Promoted` |

D-35-2 boundary: held, except a whitespace-only reflow of `field/admit.rs:344` in SF.12 (P3).

---

## 7. Doc 35 corrections owed

1. §2 row 8 and §7.3 attribute a "48 h contest" to doc 20. Doc 20's claim contest is 30 IC days; 48 real hours is doc 27 Q-S15's salvage window. Split the two.
2. The SF.16 row ("canonical space hull definitions") contradicts D-35-1. Either hulls are the unit definitions, or the doc records a space overlay and says what it may and may not redefine.
3. §1 item 7 still says `SF.0`–`SF.8`; §10's header says "Built". Replace "Built" with this review's dispositions.
4. README index says "24:1 IC clock"; D-35-3 says never hardcode 24.
5. §2 calls the framebuffer "planar 256-colour indexed"; `Frame` is packed RGB with a closed colour set per frame (D-21-7), and D-21-7 says "no dithering". Amend D-21-7 for D-35-15 or drop the dithering claim.
6. §8.2 "8-facing/2-pitch" and doc 22 D-22-9 (8 × 6 × 96 px) both differ from what SF.14 bakes (8 × 64 px, one pitch per sheet). Record the ruling.
7. §8.3 gives no roll tolerance; the code uses ±3°.
8. D-35-14's "verified/signed outcome" has no design: who signs, what the server replays, what bounds `rcs_spent_kg`. Write it before the client game is built.

---

## 8. Recommendation

Do not deploy this to dev1/dev2 and do not build SF.21+ on it. The module is `NOT_ENABLED`-gated, so main is not broken, but every one of the eleven authority holes in §3 is live the moment it is enabled.

Proposed shape of the re-plan, as doc 35 Draft 0.3:

1. **Keep** SF.1, SF.2, SF.13, SF.14, SF.20 and the WASM ABI from SF.7, with the §5 fixes (rings, Kirkwood elements, reproducible bake, goldens).
2. **Extract** the pure weapon/kinematics library from `bubble.rs`/`weapons.rs` after fixing §5 items 1, 2, 8, 9, and give it the tape + snapshot + replay test §6.1 asks for.
3. **Delete** `commands.rs`, `SpaceBook`, `stations.rs`, `pulse.rs`, `flight.rs`, `mining.rs`, `docking.rs` (tacsim), `hulls.rs`, `autopilot_bot.rs`, the three `legacy-handlers.toml` entries, and the `SpaceLane` contract claim.
4. **Re-cut seam-first**, one seam per cut, each acceptance test a `tests/shard_*` story through the registered handler with a non-GOD actor: (a) `reg.tick` consumer that opens a bubble when `JourneyState::Promoted` fires and pauses the leg; (b) `SpaceShipState` built from `UnitSpec` via `custody::check_out` and written back via `check_in`; (c) `+helm` as a writer to `mush-sim` channels/seats; (d) propellant on the hull's `stow.propellant` pool with `passage.rs` as the only planner; (e) `+mine` as a consumer of `surface/tenure.rs`, `surface/claims.rs`, `port/licence.rs`, `OreDeposit` and `stow.ore`; (f) the picket as a `MindRunner` doctrine with real `PartyId` and a Ledger fine; (g) docking as a designed client game with a server-replayed tape; (h) persistence of whatever space state is left as a store-backed slot.

Everything above is consistent with the 2026-09-30 unwired survey's finding ("the arena is an island") and with the owner's 2026-09-16 choice of a thin restart over patching a vertical slice that had drifted from its design.

---

## 9. Verified compliant

- D-35-2 file boundary (one whitespace reflow aside). No literal `24` in `space/`. No `rand`, `SystemTime`, `Instant` or `HashMap` in the step path. i128 squares do not overflow at solar-system scales.
- `Sky::velocity_mps` is a correct ±60 s central difference on the Kepler path, including the parent's velocity for moons; Earth and Neptune pins are tight.
- Torpedoes inherit launcher velocity; salvos are bounded by tube count and magazine; laser damage follows `D0/(1+(R/R0)²)` and is heat-gated; railgun slugs have no PDC/laser intercept path; `+tac/pdc auto` and per-mount Offense/Defense exist in the library; body-frame facet selection with yaw/pitch/roll is correct.
- The doc 27a `SalvageClaimBook`, `BlackBoxFlightEntry` and `apply_ic_condition_handoff` types are genuinely reused (into a private instance).
- Good ids `metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore` match `goods.json`; the C/S/M → ore-family mapping matches `surface::geology::OreFamily`; `CLAIM_TERM_IC_SECS` is used as an IC duration.
- Docking gates ±250 mm, ±2°, 0.10–0.80 m/s match §8.3; docking is mandatory off-port and opt-in at ports.
- No `.fbx`, `.unitypackage` or source textures are committed; PNG indices are albedo/shade codes mapped onto fixed ramps, nothing remaps the global palette; the 4×4 Bayer matrix is standard; no alpha blending; no `localStorage` in the space code.
- WASM feeds 6–8 and kinds 7–9 appended, existing discriminants unchanged, `wasm.ts` in sync, `.wasm` rebuilt per web build; deployed clients are safe.
- `NOT_ENABLED` is honoured by every mutating space verb.
