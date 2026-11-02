# 35. Ship free flight, Expanse-style naval combat, asteroid field mining, and the bridge/docking views

**Draft 0.3 — 2026-11-01; as built through SF.R8 (2026-11-02; per-cut notes and divergences in [35-cut-notes.md](35-cut-notes.md)).** Surveyed against `origin/main` at `830f2265`; reviewed at `497f2a78` in [doc 35a](35a-space-flight-review.md). `SF.R0`–`SF.R8` landed 2026-11-02 and were reviewed at `f1ea7715` in [doc 35b](35b-seam-first-train-review.md): right direction, seams real, four player-reachable holes and a re-plan `SF.R9`–`SF.R16` (35b §3, §8). Draft 0.1 captured the initial design and rulings; Draft 0.2 folded in the cross-review against landed code ([doc 18](18-trade-run-vertical-slice.md), [doc 19](19-astronomy-engine.md), [doc 20](20-surface-mining.md), [doc 21](21-graphics-engine.md), [doc 23](23-ic-sorties.md), [doc 27](27-ic-field-operations-design-and-survey.md), [doc 32](32-ic-combat-field.md)) and recorded the owner's binding rulings on `Q-35-1` through `Q-35-6`. **Draft 0.3** folds the eight design corrections from [doc 35a](35a-space-flight-review.md) §7, records the per-cut disposition of `SF.0`–`SF.20` (§10.1), and replaces the island vertical-slice cuts with the seam-first implementation plan `SF.R0`–`SF.R8` (§10.2).

---

## 1. Why this document exists

Today in `tacsim`, interplanetary travel is strictly a filed autopilot passage (`+ship/undock`, `+ship/order depart`, `CarrierJourney`): a captain files a flight plan at Port A, propellant and stores are debited once at departure (`D-18-46`), the hull stays `Unplaced` with a leg cursor, and it arrives at Port B on a timer.

To support **Expanse-style tactical space combat**, **zero-G asteroid field mining**, **sightseeing expeditions to the outer solar system (Saturn, Uranus, Neptune, Triton)**, and a **Starfield-style 6-DOF docking mini-game** — while working in parallel with the IC ground combat field track ([doc 32](32-ic-combat-field.md)) without causing git conflicts on `arena/*` or `field/*` — this document specifies:
1. What earlier documents already decided and what the tree has today (§2, §3).
2. The owner's rulings (`Q-35-1..6`) and the numbered decisions (`D-35-1..18`) (§4, §5).
3. The two-tier space model: site-anchored and encounter **Local Space Bubbles** on 1:1 wall time (`Q-CF-32`) + **System Free Flight** on the live `WorldTimeline` IC clock (§6).
4. The 1:1 wall-time weapon triad, point-defense, multicrew stations, light-lag sensing, customs encounters, and derelict/salvage lifecycle (§6).
5. Zero-G velocity-matched asteroid field mining integrated with [doc 20](20-surface-mining.md)'s ore goods, `stow.ore`, 4-drone leash, and the Freeport Registry claims engine (30 IC-day claim contest; 48 real-hour derelict salvage contest under [doc 27](27-ic-field-operations-design-and-survey.md) `Q-S15`) (§7).
6. Three `mush-pixel` views (`holotank.rs`, `space_cockpit.rs` + `planet_shader.rs` + `ship_shader.rs`, and out-of-time-loop `docking.rs`) rendered into `Frame`'s packed RGB framebuffer with a closed colour set of at most 256 colours per frame (`D-21-7`, amended by `D-35-15` to permit 4×4 Bayer ordered dithering on planetary and ship shaders) (§8).
7. Canonical acceptance stories (§9), the [doc 35a](35a-space-flight-review.md) disposition of `SF.0`–`SF.20` (§10.1), and the seam-first re-plan `SF.R0`–`SF.R8` (§10.2).

---

## 2. Already decided; not asked again

| Decision | Where | What it fixes for space flight, combat & mining |
|---|---|---|
| Strategic travel sets encounter geometry; combat runs in a bounded local-space domain; multicrew stations (`helm`, `tactical`, `ew`, `engineering`, `flight_ops`, `command`) use continuous channels (latest-order-wins) and discrete intents | doc 10 §8 | Two-tier model; verbs map onto `mush-sim` multicrew stations from day one |
| Fight or evade is a propellant ($\Delta v$) decision; torch drives have high thermal bloom; civilian hulls are slow, armed warships/corvettes rare | doc 11 | $\Delta v$, relative velocity, and IR signature drive encounter geometry |
| Light-lag and observer-bounded contact projection via `mush-propagation` | doc 10 §14, doc 15 | System-scale contacts carry light-lag; EMCON and thermal bloom govern detection |
| One data/damage/party kernel for IC and OOC (`PartyId`, relations, sections, facets, heat, custody, deterministic step, tape, snapshot) | doc 32 Q-CF-1, Q-CF-3, Q-CF-19 | Space hulls reuse the exact unit definitions (`dray.json`, `dray_p.json`, `halcyon.json`), sections, facet bands, heat cells, `PartyId` relations, and `custody::check_out` / `check_in` seam |
| Engaged fields run on 1:1 wall time (adaptive 5000/1000/500 ms step with 250 ms substeps, `D-16-2`) and stamp IC instants; W-clock outage freeze (`Q-CF-33`) applies | doc 16 D-16-2, doc 27 Q-H1, doc 32 Q-CF-32, Q-CF-33 | Local Space Bubbles run on 1:1 wall time; all weapon flight times are authored in 1:1 real seconds |
| No player permadeath (`FO-12`); recoverable disablement keeps the hull epoch open; disabled hulls persist as physical salvage/derelicts with a **48 real-hour** non-owner recovery contest (`Q-S15`) | doc 23, doc 27 Q-H8/H9/H12/S15, doc 27a | Disabled ships are the system's derelicts; life-support clock + Tycho re-float / tow contracts under the 48 real-hour salvage window (`salvage.rs:18`) |
| Link loss holds last safe posture (`Q-H5`, `P-2`); NPC pickets and raiders use `MindRunner` Rhai doctrine over frame fields (`Q-CF-42`, `D-UM-4`) | doc 17, doc 27 Q-H5, doc 32 Q-CF-42 | No bespoke C-style AI loop; pickets and raiders run Rhai doctrines |
| Four ore goods (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`) stored in `stow.ore` (kg / L); Freeport Registry prospecting licence (2500 cr / 168 IC days), claim stakes, **30 IC-day** claim contest (`tenure.rs:34`), Pallas Syndicate outer paper; 4 drone bays per `Dray-P` sealed under `ExcursionId` | doc 20 §3.4, §7.3, §9, §17 | Asteroid mining outputs the four canonical ore goods into `stow.ore` under Registry claim rules (30 IC-day claim contest, distinct from doc 27's 48 real-hour salvage contest) |
| `mush-pixel`'s `Frame` is a packed RGB framebuffer with a closed colour set of at most 256 colours per frame (`D-21-7`, no alpha blending); 13 digit screen slots (`MAX_SCREENS`) are full; digits `1–9`/`0` are global screen switches in the TUI | doc 21 §1, D-21-7 | `D-35-15` amends `D-21-7` for planetary and ship shaders to permit 4×4 Bayer ordered dithering over fixed stepped palette ramps within the closed 256-colour per-frame cap; space views mount on lettered screens / contextual view cycles; combat hotkeys avoid bare screen digits |

---

## 3. What the tree has today (checked at `830f2265`, updated at `497f2a78`)

- **Hulls & sections (`crates/tacsim/content/units/dray.json`, `dray_p.json`, `halcyon.json`)**: The `Dray` and `Dray-P` already define sections `hull`, `hold`, and `drive` with facet bands `front`, `left`, `right`, `rear`, and `top`. The `Halcyon` carrier defines eight berths and its own section list. These unit definitions are the single source of truth for hull mass, hold capacity, drive thrust/ISP, sections, facets, and heat (`D-35-1`).
- **Standing-order channels (`crates/mush-sim/`)**: The standing-order book already carries 3D `Heading { x, y, z }`, `Thrust`, and `Emcon { 0..=3 }` channels plus multicrew seat bindings waiting for a space consumer.
- **Passage & interception seams (`crates/tacsim/src/port/passage.rs`, `voyage.rs`, `mush-sim/src/journey.rs`)**: `plan_passage` plans body-to-body flip-and-burn transits with a five-pass intercept solver. `LegDef.interdiction`, `PromotionTrigger::Interception`, `JourneyState::Promoted`, and `MAX_ACTIVE_DOMAINS` (8) exist as seams, though `Promoted` does not yet pause the leg timer or materialize a 3D bubble.
- **Sky (`crates/tacsim/src/sky.rs`, `crates/tacsim/content/sky/fetch_elements.py`, `outer_bodies.json`)**: `Sky::Kepler` computes `PosKm` (whole km in 3D ecliptic J2000) and `Sky::velocity_mps(body, ic_t) -> [i32; 3]` for inner, mid, and outer-system bodies including Uranus, Neptune, Triton, the Galilean moons (`io`, `europa`, `ganymede`, `callisto`), and the four orbital asteroid cluster reference anchors (`ceres_halo`, `kirkwood_gap_3_1` at 2.50 AU, `pallas_inclined_belt`, `saturn_b_ring_edge`), folded into `Sky::revision()` and validated through `BodyDef`.
- **Chart & space pixel views (`crates/mush-pixel/src/{chart,holotank,space_cockpit,planet_shader,ship_shader,docking}.rs`)**: `mush-pixel` and `mush-pixel-wasm` implement the 3D Holo-Tank (`FEED_SPACE_HOLOTANK = 6`, `KIND_HOLOTANK = 7`), the First-Person Space Bridge Canopy (`FEED_SPACE_COCKPIT = 7`, `KIND_SPACE_COCKPIT = 8`) with 12 baked `128×64` equirectangular planetary maps (`planets/*.png`) and 10 baked `512×64` (`8 × 64×64` yaw facings, single elevation pitch) 3D ship/station/asteroid sheets (`ships/*.png`), and the 6-DOF Docking Camera (`FEED_SPACE_DOCKING = 8`, `KIND_DOCKING = 9`).

---

## 4. Owner survey & rulings (2026-11-01)

### Q-35-1 — Kernel unification vs. parallel worktree isolation
- **Ruling (owner, 2026-11-01):** **Unified data & damage kernel, isolated 3D space kinematics (`crates/tacsim/src/space/`).**
  - Reuse the existing hull/section/facet/component/heat/party/custody types (`dray.json`'s `hull`/`hold`/`drive` sections and `front`/`left`/`right`/`rear`/`top` facets, `PartyId` relations, `custody::check_out`, deterministic input tape, snapshots, `WorldTimeline` outage freeze, and `MindRunner` Rhai host).
  - Run 3D continuous kilometer-scale space kinematics, orbital vector integration, and torpedo/railgun/laser ballistics in `crates/tacsim/src/space/*` so zero files collide with the ongoing `arena/*` and `field/*` refactoring in [doc 32](32-ic-combat-field.md).

### Q-35-2 — Tier-1 System Free Flight vs. site-attached bubbles first
- **Ruling (owner, 2026-11-01):** **Keep full 3D System Free Flight (`+helm/decouple` + outer planets) with live propellant debit and return-reserve checks, AND stage the cut order so site-anchored space bubbles (Kirkwood, Pallas, Saturn Rings) land first before mid-leg decoupling.**
  - Supersedes `D-14-5` (node-only orbital regions) and `D-18-46` (frozen passage plan) when a ship enters a promoted `LocalSpaceBubble` or `FlightMode::FreeFlight`.

### Q-35-3 — Asteroid mining integration with Doc 20 & the Freeport Registry
- **Ruling (owner, 2026-11-01):** **Unify both.** Use [doc 20](20-surface-mining.md)'s canonical ore goods (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`), `stow.ore` stowage (`Dray-P` and hoppers), four ore families with `barren`/`lean`/`fair`/`rich` richness grades, 4-drone leash per `Dray-P` (`ExcursionId`), and Freeport Registry prospecting licences, claim stakes, **30 IC-day** claim contests (`tenure.rs:34`), and Pallas Syndicate paper claims — while playing inside a 3D velocity-matched space bubble around orbital asteroid clusters.

### Q-35-4 — Time base & Point-Defense under "no auto-fire"
- **Ruling (owner, 2026-11-01):**
  1. **Wall-time bubbles (`Q-CF-32`)**: Engaged Local Space Bubbles run on **1:1 wall time** (adaptive step tempo `D-16-2`), while Tier-1 system free flight runs on the runtime `WorldTimeline` IC clock (`dt_ic` from `world_timeline().current().rate`, never hardcoding `24`).
  2. **Automatic Point-Defense toggle permitted**: Full automatic PDC and defensive-laser interception toggles (`+tac/pdc auto` and per-mount `Offense`/`Defense` mode) are explicitly **approved**. Since the game already permits conditional fire doctrines such as `"FIRE ALL WHEN ODDS >= 60"`, an automatic point-defense posture toggle is natural and avoids impossible manual typing races against incoming torpedo salvos.

### Q-35-5 — 6-DOF Starfield-style docking mini-game execution & time loop
- **Ruling (owner, 2026-11-01):** **Run the 60 fps 6-DOF alignment HUD client-side in `mush-pixel` / the gateway, and step the docking hull out of the live time loop while the player plays the mini-game.**
  - Taking the docking ship out of the time loop eliminates SSH latency issues and prevents time-compression drift while the pilot carefully aligns translation, pitch/yaw, roll, and closure rate; when the mini-game finishes, the client submits its bounded RCS control input tape, which the server deterministically replays from the session's initial pose to verify gate satisfaction and merge `Docked { quality, rcs_spent_kg, hull_scrape }` back into the world state (`D-35-14`).
  - Mandatory for unassisted targets (disabled derelicts, ship-to-ship boarding/salvage, unpowered field moorings) and opt-in at ports (saving terminal approach time / RCS reaction mass).

### Q-35-6 — Visual views, Bayer dithering, and cut priority
- **Ruling (owner, 2026-11-01):** **Keep all three views (`holotank.rs`, `space_cockpit.rs` + analytical raycast `planet_shader.rs` + `ship_shader.rs`, and `docking.rs` wireframe collar overlay) within `Frame`'s closed 256-colour per-frame budget (`D-21-7` amended by `D-35-15`), and build visuals early in parallel with backend cuts.**

---

## 5. Numbered decisions (`D-35-1` – `D-35-18`)

- **D-35-1 (Shared damage kernel, canonical unit definitions, isolated 3D space module):** Space combat reuses the canonical unit definitions (`dray.json`, `dray_p.json`, `halcyon.json`, etc. in `crates/tacsim/content/units/`), sections (`hull`/`hold`/`drive` on `Dray`/`Dray-P`; section list on `Halcyon`), facet bands (`front`/`left`/`right`/`rear`/`top`), component crits, heat cells, `PartyId` relations, `custody::check_out` / `check_in`, deterministic input tape, snapshots, W-clock outage freeze, and `MindRunner` host. **There is no second hull definition file**: any space-specific overlay keyed to a canonical `UnitId` may bind only 3D RCS control parameters, space weapon mount bindings, and `pixel_model` visual archetype ids, and **may never redefine** hull mass, hold volume/mass, drive acceleration/ISP, sections, facets, or heat capacity. All 3D kilometer-scale kinematics and space verbs live in `crates/tacsim/src/space/*`.
- **D-35-2 (Parallel file boundary):** No cut in this track modifies `crates/tacsim/src/arena/*`, `crates/tacsim/src/field/*`, `crates/tacsim/src/surface/*`, or `crates/tacsim/src/detachment/*`.
- **D-35-3 (Two-tier clock rule):** Tier-1 System Free Flight advances on a registered world tick (`reg.tick`) by the live `WorldTimeline` IC delta (`dt_ic`, never hardcoding `24`). Tier-2 Local Space Bubbles advance on **1:1 wall time** (`D-16-2` adaptive 5000 / 1000 / 500 ms pulse, substeps at most 250 ms) and stamp caller-supplied W-clock IC instants (`Q-CF-32`). No player command may step the world or bubble clock in production.
- **D-35-4 (Outer-system ephemeris & orbital velocity):** `Sky::Kepler` gains `Sky::velocity_mps(body, ic_t) -> [i32; 3]` (central-difference derivative of `position_km`, with continuous `f64` phase on the circular sky) and elements for **Uranus**, **Neptune**, **Triton**, and **Io, Europa, Ganymede, Callisto** (`fetch_elements.py`, using propagated rows post-2250 where Horizons ends, matching Titan), validated through `pack::BodyDef` and folded into `Sky::revision()`.
- **D-35-5 (Bubbles at sites first, mid-leg decoupling second):** A `LocalSpaceBubble` (`BubbleId`) attaches first to orbital sites/nodes (Kirkwood Picket approach, Pallas Cluster, Ceres Halo, Saturn Ring Edge) using the existing `LegDef.interdiction` / `PromotionTrigger::Interception` / `JourneyState::Promoted` seam (`Promoted` pauses the leg timer while the bubble is open). Mid-leg free-flight decoupling (`+helm/decouple`) lands after site bubbles.
- **D-35-6 (Live propellant debit & return-reserve check):** While in free flight or a bubble, burns debit propellant live from the hull's `stow.propellant` pool in kg. A from-coordinate passage estimator (`plan_passage_from(pos_km, vel_mps, body)` built on `port/passage.rs`) continuously computes the minimum $\Delta v$ required to reach the nearest open port (`RETURN_RESERVE`) and warns (`TANK_RESERVE_WARN`) before a burn would dip below the return margin; `+helm/burn force` is required to burn into stranding reserve.
- **D-35-7 (Offline / linkdead rule in space):** Under `Q-H5` / `P-2`, if all crew on a free-flying ship go linkdead (detected from session presence, never a manual toggle), main thrust cuts to zero (ballistic drift), active point-defense remains armed for the reclaim grace window, and if unclaimed inside a safe corridor the hull either holds ballistic with a life-support notice or executes its queued return plan.
- **D-35-8 (1:1 wall-time weapon triad & automatic PDC toggle):** Weapon ranges, velocities, and flight times are authored in 1:1 wall time (§6.2). `+tac/pdc auto` and per-mount `Offense`/`Defense` laser mode are valid automatic defensive postures (`Q-35-4`). PDC mounts enforce fire-rate cooldowns and per-burst intercept capacity so multi-torpedo salvos can saturate point defense; torpedoes fly 3D proportional navigation while $\Delta v > 0$ and coast ballistically after burnout up to `max_flight_ms`; railgun lead solutions solve in world coordinates (`v_slug_world = v_shooter + v_muzzle_vec`).
- **D-35-9 (Multicrew channels & seats):** `+helm`, `+tac`, and `+mine` are `authoritative` verbs (`Ctx::submit_authoritative`) that write to `mush-sim`'s existing standing-order channels (`Heading{x,y,z}`, `Thrust`, `Emcon{0..=3}`) and seat roles (`helm`, `tactical`, `ew`, `engineering`, `flight_ops`, `command`), requiring verified physical presence aboard the hull and seat authority. A solo pilot occupying command operates all channels directly.
- **D-35-10 (Light-lag sensing & EMCON):** System-scale detection routes through `mush-propagation` light-lag and compares target IR output (passive hull floor + radiator heat + torch thrust bloom + weapon/mining laser flash) against distance squared and observer sensor aperture. Undetected contacts are never leaked in text or panel feeds.
- **D-35-11 (Customs encounter & multi-party bubble):** Bubbles are multi-party (`PartyId` / relation). The canonical non-lethal/escalating encounter is the **Kirkwood Customs Picket** driven by a `MindRunner` Rhai doctrine (hail, heave-to for inspection, live `stow.ore` + Registry claim check, Ledger fine or seizure advancing the Customs Assertion front, or running the picket under fire).
- **D-35-12 (Derelicts, disablement & salvage):** No permadeath (`FO-12`). A hull disabled in space keeps its epoch open (`doc 23`) and becomes a persistent store-backed drifting derelict on a life-support clock, eligible for distress beacons, Tycho re-float contracts, or non-owner salvage/tow (subject to `doc 27/27a`'s 2× tonnage tow cap and **48 real-hour** salvage contest window in `salvage.rs:18`).
- **D-35-13 (Asteroid sites, canonical ores & Registry claims):** Orbital asteroid clusters are authored as `field`-kind orbital sites (`doc 20 §17`) with `OreDeposit` pools for `metal_ore`, `ice_ore`, `carbon_ore`, and `strategic_ore`, loaded into `stow.ore` (on `Dray-P` and drone/TV hoppers) under Freeport Registry prospecting licences (2,500 cr / 168 IC days via `port/licence.rs`), claim stakes (`surface/claims.rs`), and **30 IC-day** claim contests (`surface/tenure.rs:34`). Inside the bubble, extraction uses 3D Euclidean velocity matching ($|\Delta \mathbf{v}| < 5\text{ m/s}$ within $2\text{ km}$) via ship mining lasers or up to 4 sealed-mind collector drones (`ExcursionId`).
- **D-35-14 (Out-of-time-loop client-side 6-DOF docking with server-replayed tape verification):** Entering manual docking (`+helm/dock`) transitions the hull into a store-backed `DockingSession` paused outside the world tick (`DockingChallenge { session_id, start_pose, rcs_authority, max_frames }`) while the gateway/`mush-pixel` runs the 60 fps 6-DOF collar alignment mini-game locally. Upon completion or abort, the client submits the bounded `DockingInputTape` (`<= 3,600` frames at 60 Hz of 6-DOF RCS control bitmasks); the server deterministically replays the integer 6-DOF kinematics from `start_pose`, verifies `evaluate_docking_gates` (`|dx|, |dy| <= 250 mm`, `|pitch|, |yaw| <= 2,000 mdeg`, `|roll| <= 3,000 mdeg`, `dz <= 500 mm`, `100 <= vz <= 800 mm/s`), computes `rcs_spent_kg` (`rcs_pulse_count * kg_per_pulse`, bounded by and debited from the hull's `stow.propellant`) and `hull_scrape` server-side, and merges the ship into `Berth::Docked` (or airlock hard-dock with a derelict/ship).
- **D-35-15 (Closed 256-colour per-frame budget, Bayer-dithered shaders & 8×64 ship sheets — amends `D-21-7` and `D-22-9` for space):** All three space views (`holotank.rs`, `space_cockpit.rs` + `planet_shader.rs` + `ship_shader.rs`, `docking.rs`) render into `mush-pixel`'s packed RGB `Frame` keeping a closed colour set of at most 256 distinct colours per frame (`D-21-7`). `D-21-7`'s no-dithering rule is amended for `planet_shader.rs` and `ship_shader.rs`, which use deterministic 4×4 Bayer ordered dithering over fixed 6-step colour ramps so curved planetary limbs, rings, and 3D hull normals shade smoothly within the 256-colour cap (`holotank.rs` and `docking.rs` use flat stepped colours). Space hull/station/asteroid sheets (`crates/mush-pixel/ships/*.png`, `SF.14`/`SF.20`) are ruled as **`512×64` indexed PNG sheets containing 8 yaw facings at `64×64` px per cell and one canonical elevation pitch (`18°`, or `0°` for axial projectiles)**, distinct from ground units' `D-22-9` `8 × 6 × 96 px` pose sheets.
- **D-35-16 (TUI screen & key discipline):** Space views mount on a lettered TUI screen (`v` view mode toggle inside the tactical/helm screen) and never consume the 13 global digit screen slots (`MAX_SCREENS`); weapon group hotkeys in the TUI use function/letter bindings (`z`/`x`/`c` or `F1–F3`) rather than bare screen-switch digits, and forward/reverse RCS translation uses `r`/`f` rather than a lone `Shift` modifier.
- **D-35-17 (Content studio authoring):** Asteroid clusters, picket casts, and patrol doctrines are authored through the lane pack and [doc 34](34-content-studio.md)'s content studio schemas, never an unvalidated ad-hoc loader.
- **D-35-18 (Seam-first cut ordering):** Foundation assets, shaders, bakes, and the pure deterministic weapon/kinematics library (`SF.R0`) land first; server gameplay then lands seam-first (`SF.R1`–`SF.R8`, §10.2), with one real engine seam per cut and each acceptance test written in `tests/shard_*` driving registered handlers with a non-GOD actor.

---

## 6. Two-Tier Space Architecture & Expanse Combat

### 6.1 Tier 1 (IC Clock) vs. Tier 2 (1:1 Wall-Time Local Space Bubble)

1. **Filed Autopilot (`CarrierJourney`, default for offline trade):**
   - Works as built in [doc 18](18-trade-run-vertical-slice.md). When a leg with `LegDef.interdiction` triggers `PromotionTrigger::Interception` (e.g. passing the Kirkwood Picket or arriving at an orbital asteroid cluster), `JourneyState::Promoted` pauses the leg cursor and a registered world tick (`reg.tick`) materializes the hull into the site's `LocalSpaceBubble` via `custody::check_out`.
2. **Tier 2 — `LocalSpaceBubble` (`crates/tacsim/src/space/bubble.rs`, 1:1 wall time):**
   - Origin $\mathbf{R}_0(t), \mathbf{V}_0(t)$ co-moves with the site body, station, asteroid cluster centre, or interception barycentre.
   - Local position $\mathbf{x}$ in metres (`[i64; 3]`, $\pm 100{,}000\text{ km}$ domain) with micrometre substep remainders (`pos_rem_um: [i64; 3]` so sub-4 m/s motion is never truncated at 250 ms substeps), local velocity $\mathbf{v}$ in $\text{mm/s}$ (`[i64; 3]`).
   - Steps on **1:1 wall time** (`D-16-2` adaptive 5000 / 1000 / 500 ms step with 250 ms substeps), stamps caller-supplied W-clock IC instants (`Q-CF-32`), records a deterministic `BubbleInputTape`, and supports mid-battle `BubbleSnapshot` capture and bit-identical replay.
3. **Tier 1 — System Free Flight (`WorldTimeline` IC clock):**
   - For ships that undock manually (`+helm/undock`) or decouple mid-leg (`+helm/decouple`) to explore the outer solar system or fly off-corridor:
     - Position `pos_m: [i128; 3]` (heliocentric ecliptic J2000 metres), velocity `vel_mmps: [i64; 3]`.
     - Consumes `mush-sim` channels (`Heading{x,y,z}`, `Thrust`, `Emcon{0..=3}`) on each world tick (`dt_ic`), debiting propellant live from `stow.propellant` (`D-35-6`).
     - Supports guidance modes: `Manual`, `Prograde`, `Retrograde`, `Intercept { target, flip_burn }`, and `MatchVelocity { target }` (evaluated in target-relative frame).
     - `+helm/autopilot <port>` invokes `plan_passage_from(pos_km, vel_mps, port)` (`port/passage.rs`) to re-enter a filed `CarrierJourney` once unengaged and above `RETURN_RESERVE`.

### 6.2 The 1:1 Wall-Time Weapon Triad (`D-35-8`)

All combat inside a `LocalSpaceBubble` runs on **1:1 wall time**:

| Weapon Class | Effective Range | Kinematics (1:1 Wall Time) | Flight / Resolution Time | Active Counterplay |
|---|---|---|---|---|
| **Torpedoes** (`torpedo_tube`) | $10\text{ – }500\text{ km}$ | Inherits $\mathbf{v}_{\text{ship}}$; accelerates at $15\text{–}30\text{ g}$ ($150\text{–}300\text{ m/s}^2$) under 3D proportional navigation while $\Delta v > 0$ ($8\text{–}15\text{ km/s}$), then coasts ballistically up to `max_flight_ms` (`90_000 ms`) | **$12\text{ – }90\text{ real seconds}$** (tracked live across steps on the Holo-Tank) | Automatic **PDCs** (`+tac/pdc auto`), **Defensive Lasers**, target-locked **ECM/decoys**, or high-G burn |
| **Energy Weapons** (`laser_mount`, `pulsed_beam`) | $20\text{ – }500\text{ km}$ | Speed of light ($c$, instant hit) | **Instant per step**; builds hull heat cell (`heat_permille`) | Range diffraction ($1 / (1 + (R/R_0)^2)$), radiator management, attitude rolling across `front`/`left`/`right`/`rear`/`top` facets |
| **Railguns** (`railgun_spinal`, `railgun_turret`) | $10\text{ – }250\text{ km}$ | Unguided tungsten slug at world velocity $\mathbf{v}_{\text{slug}} = \mathbf{v}_{\text{ship}} + \mathbf{v}_{\text{muz}}$ aimed at world intercept $\mathbf{r}_{\text{target}} + \mathbf{v}_{\text{target}} \cdot t_{\text{tof}}$ | **$1.0\text{ – }15\text{ real seconds}$** time-of-flight | **Cannot be intercepted by PDCs or lasers.** Dodged by **lateral jinking thrust** during time-of-flight outside the shooter's predicted lead pip |
| **PDCs / CIWS** (`pdc_rotary`) | $0\text{ – }15\text{ km}$ (anti-ship) / terminal envelope (anti-torpedo) | $3\text{ km/s}$ rotary bursts with per-mount cooldown (`cooldown_ms`) and deterministic hit accumulation | Automatic terminal intercept pass when toggled on (`+tac/pdc auto`) | Saturation salvos that exceed PDC fire-rate capacity; stripping PDC mounts with lasers; magazine depletion (`W15`) |

### 6.3 Hull Sections, Facets & Subsystem Crits (`D-35-1`)

Rather than inventing a second damage or hull schema, space combat resolves hits directly against the hull's canonical unit definition (`dray.json`, `dray_p.json`, `halcyon.json`, etc.):
- **Facet Bands**: Incoming bearing in the ship's local frame selects `front`, `left`, `right`, `rear`, or `top` armour facets; rolling the ship (`+helm/roll` or `q`/`e`) rotates fresh armour facets toward the threat.
- **Sections (`hull`, `hold`, `drive` on `Dray`; berth/hull sections on `Halcyon`) & Components**:
  - **Radiator / Heat Cell Crit**: Lowers thermal dissipation per step, choking laser fire rate and raising IR signature.
  - **Drive Crit**: Degrades `accel_mg` or leaks propellant.
  - **Sensor / PDC Crit**: Degrades contact locks or opens a gap in the automatic PDC defense arc.

---

## 7. Zero-G Asteroid Field Mining & Registry Claims (`D-35-13`)

### 7.1 Orbital Asteroid Sites (`doc 20 §17` + 3D Bubble)

Four orbital asteroid clusters are authored:
1. **`ceres_halo`** — Inner Ceres co-orbital rubble cloud ($2.77\text{ AU}$, Core zone; Freeport Registry commons).
2. **`kirkwood_gap_3_1`** — 3:1 Jupiter resonance scatter at **$2.50\text{ AU}$** ($P \approx 1443.8\text{ days}$, Frontier/Contested; patrolled by the Kirkwood Customs Picket and raiders).
3. **`pallas_inclined_belt`** — High-inclination ($i = 34.8^\circ$) cluster near Pallas (Frontier; contested Pallas Syndicate paper claims).
4. **`saturn_b_ring_edge`** — Outer-system shepherd cluster near Titan/Saturn (Frontier; deep-space expedition site).

### 7.2 Spectral Classes $\rightarrow$ Canonical Ore Families & `stow.ore`

Each cluster contains seeded, slowly drifting asteroid pockets backed by `OreDeposit` pools (`doc 20 §3.4`) with richness grades `barren`, `lean`, `fair`, and `rich`:
- **C-Type (Carbonaceous Chondrite)** $\rightarrow$ `ice_ore` ($1000\text{ L/t}$) and `carbon_ore` ($1200\text{ L/t}$).
- **S-Type (Stony-Metallic)** $\rightarrow$ `metal_ore` ($500\text{ L/t}$).
- **M-Type (Nickel-Iron / Platinum Core)** $\rightarrow$ rich `metal_ore` ($500\text{ L/t}$) and `strategic_ore` ($600\text{ L/t}$).

All mined matter enters **`stow.ore`** (measured in `kg` and `L` using `goods.json`'s canonical specific volumes), requiring an ore-capable hull (`Dray-P`'s $86{,}000\text{ kg}$ hold or drone/TV hoppers) and respecting `hold_unreserved` (`D-20-9`).

### 7.3 The Zero-G Mining Loop & Registry Consequences

1. **Rights & Claims**: Just as on surface sites (`D-20-11`, `D-20-12`), scanning (`+mine/ping`) and flying are free, while extraction and claim-staking consult the **Freeport Registry** (`port/licence.rs` prospecting licence at 2500 cr / 168 IC days, block/rock stake via `surface/claims.rs`, and **30 IC-day** claim contest window via `surface/tenure.rs:34`). Mining an unleased or Syndicate-claimed rock without paper works in frontier space but spikes thermal bloom and records an actionable claim-jump event.
2. **Velocity Match (`+helm/match <rock_id>`)**: The pilot flies within **$2{,}000\text{ m}$** of the target asteroid pocket and nulls 3D Euclidean relative speed to **$|\Delta \mathbf{v}| < 5\text{ m/s}$** (`5,000 mm/s`).
3. **Extraction (`+mine/laser <rock_id>` or `+mine/drones launch`)**:
   - **Ship Mining Laser**: Direct extraction over a timed run; generates high **thermal/IR bloom** visible across the cluster bubble and risks volatile outgassing $\Delta \mathbf{v}$ kicks if overheated.
   - **Collector Drones (`doc 20 §17`)**: Up to **4 drones per `Dray-P`** (`maximum_items = 4`), launched and sealed from bay to bay under a real `ExcursionId`, running the stock `mining` doctrine (`scan()`, `sound()`, `goto()`, `mine()`, `recover()`).

---

## 8. The Three Visual Views (`mush-pixel`, `D-35-15`)

All three views render into `mush-pixel`'s packed RGB `Frame` keeping a closed colour set of at most **256 distinct colours per frame** (`D-21-7`, required for indexed PNG and DEC sixel encoding) and avoiding any runtime 3D triangle mesh pipeline (`doc 21 §1`). Per `D-35-15`, `planet_shader.rs` and `ship_shader.rs` use **4×4 Bayer ordered dithering** over fixed 6-step colour ramps, while `holotank.rs` and `docking.rs` use flat stepped palette colours.

### 8.1 View 1 — The 3D Holo-Tank (`crates/mush-pixel/src/holotank.rs`)

- Consumes `sky.chart` (at system scale, rendering its `z_au` and `lat_deg` fields) and `space.holotank` (inside a `LocalSpaceBubble`).
- **Rotatable Cylindrical Plot**: Configurable azimuth ($\theta$) and elevation tilt ($\phi$, default $30^\circ$) with range rings.
- **Elevation Stalks**: Vertical dashed stalks from each ship, torpedo, asteroid, or planet down/up to its $(x, y, 0)$ ecliptic/orbital plane footpoint.
- **Tactical Symbology**: Velocity vectors ($\mathbf{v}$), orange thrust plumes ($\mathbf{a}$), red torpedo tracks with **TTI (`T-14s`)** tags, PDC defense rings, and railgun lead lines.

### 8.2 View 2 — First-Person Bridge Canopy, Planet Shader & 8×64 Ship Sheets (`space_cockpit.rs`, `planet_shader.rs`, `ship_shader.rs`)

- **Analytical Sphere & Ring Raycaster (`planet_shader.rs`)**: Pure 2D per-pixel sphere + 3D ring-plane intersection shader (`ry_plane = -ny / sin_tilt`, `rz = ry_plane * cos_tilt`) with solar phase illumination, cloud-top ring shadows, 12 baked `128×64` equirectangular planetary albedo maps (`crates/mush-pixel/planets/*.png`), and 4×4 Bayer dithering:
  - **Neptune**: Cobalt-azure methane gradient, zonal storm bands, Great Dark Spot, bright high-altitude cirrus streaks, faint ring arcs, and **Triton** in orbit.
  - **Saturn & Titan**: Golden-ochre banded globe with perspective **A/B/C/Cassini Division rings** casting shadow bands onto the cloud tops, plus Titan's orange nitrogen haze limb.
  - **Jupiter & Galileans**, **Uranus** ($97.8^\circ$ tilted rings), **Mars / Phobos / Deimos**, **Ceres & Vesta**: Distinct dithered palettes and crater/albedo maps.
- **Canopy HUD & Contacts (`space_cockpit.rs`, `ship_shader.rs`)**:
  - Bridge canopy struts and cowling gauges (facet armour doll, capacitor, radiator heat bar, $\Delta v$ reserve).
  - **Prograde ($\oplus$) / Retrograde ($\otimes$) reticles** and **Railgun Lead Pip** ($\mathbf{r}_{\text{target}} + \mathbf{v}_{\text{target}} \cdot t_{\text{tof}}$).
  - Distant ships render as tactical bracket symbology + thruster flare sprites; close-range hulls render their baked **`512×64` (`8 × 64×64` yaw facings, single elevation pitch)** sprite sheets (`crates/mush-pixel/ships/*.png`, honouring explicit `pixel_model` contact bindings); top-left corner embeds a mini Holo-Tank inset.

### 8.3 View 3 — Out-of-Time-Loop 6-DOF Docking Camera (`docking.rs`, `D-35-14`)

- Runs **client-side at 60 fps** in `mush-pixel` / the gateway while the docking hull is paused in a `DockingSession` outside the world tick (`Q-35-5`).
- Renders the target's illuminated **geometric wireframe/vector docking collar**, decimated 3D hull feature edges (`ships/index.json`), and approach chevrons in true 6-DOF perspective from the bow collar camera.
- **Alignment HUD & Gate Tolerances**:
  - Lateral translation box ($\Delta x, \Delta y$ within **`±0.25 m` / `±250 mm`**),
  - Pitch/Yaw needles (**`±2.0°` / `±2,000 mdeg`**),
  - Roll alignment ring (`Q`/`E`, **`±3.0°` / `±3,000 mdeg`**),
  - Axial collar distance ($0 \le \Delta z \le \mathbf{0.50\text{ m}}$ / `500 mm`), and
  - Closure velocity tape (**$0.10\text{–}0.80\text{ m/s}$ / `100–800 mm/s`** green latch zone; $> 0.80\text{ m/s}$ triggers a hard-contact hull scrape).
- **Controls (no bare `Shift` dependency)**: `W`/`S`/`A`/`D` lateral/vertical RCS translation, `Arrow Keys` pitch/yaw trim, `Q`/`E` roll, `R`/`F` forward/reverse closure pulse, `X` RCS dampeners.
- **Deterministic Tape Verification (`D-35-14`)**: Upon clamp lock (or abort/collision), the client submits its bounded 60 Hz `DockingInputTape`; the server replays the integer 6-DOF kinematics from the session's `start_pose`, verifies `evaluate_docking_gates`, debits `rcs_spent_kg` from `stow.propellant`, and merges the hull back into `Berth::Docked` (or hard-docked to a derelict/ship for salvage).

---

## 9. Canonical Acceptance Stories

### Story 1 — The Kirkwood Customs Picket (Multi-Party Interception & Triad Combat)
1. A player `Dray-P` returning from `pallas_inclined_belt` with `strategic_ore` in `stow.ore` reaches the Kirkwood leg boundary (`LegDef.interdiction`), promoting into the Kirkwood `LocalSpaceBubble` (`JourneyState::Promoted`) alongside a **Republic Customs Corvette** (Party A, Neutral-Inspecting, driven by a `MindRunner` Rhai doctrine) and a lurking **Belt Raider** (Party B, Hostile).
2. The picket hails on light-lagged radio (`+helm` / `tacsim:notice`), ordering the player to heave-to ($|\Delta \mathbf{v}| < 10\text{ m/s}$).
3. When the raider lights its torch and launches a 2-torpedo salvo at the player, the bubble steps on 1:1 wall time: the player toggles `+tac/pdc auto`, coordinates with the customs corvette's screen, jinks laterally to dodge a 120 km world-frame railgun shot, and either submits to customs inspection (paying a Ledger fine if unlicensed) or burns clear to resume passage.

### Story 2 — Pallas Claim-Jump, Derelict Salvage & Outer-System Neptune Expedition
1. A crew flies into `pallas_inclined_belt`, pings an M-type rock (`strategic_ore` + `metal_ore`), matches velocity ($|\Delta \mathbf{v}| < 5\text{ m/s}$), and deploys 4 collector drones (`ExcursionId`) + the ship mining laser into `stow.ore`.
2. Their thermal bloom attracts a rival claim-jumper; after disabling the attacker's drive in a laser/railgun duel, the attacker's hull remains in the bubble as an open-epoch **derelict**.
3. The pilot approaches within $500\text{ m}$, triggers `+helm/dock <derelict>`, plays the **out-of-time-loop 6-DOF docking mini-game** (`docking.rs`) whose replayed input tape hard-docks with the hull, and attaches a tow/salvage line under `doc 27a`'s **48 real-hour** salvage contest rules (`salvage.rs:18`).
4. With the payout, the captain refits via the port refit bay (`doc 31`), decouples into **System Free Flight** (`+helm/decouple`), burns out past Saturn and Uranus to **Neptune & Triton** on the live `WorldTimeline` IC clock, switches to the **First-Person Bridge Canopy** (`space_cockpit.rs`), and watches Neptune's banded azure storms and Triton rise across the canopy glass.

---

## 10. Review Disposition (`35a`) & Seam-First Implementation Plan (`SF.R0` – `SF.R8`)

### 10.1 Disposition of the `SF.0`–`SF.20` Train ([doc 35a](35a-space-flight-review.md) §6)

| Original Cut | Review Verdict (`35a`) | Disposition |
|---|---|---|
| **SF.0** | Design doc Draft 0.2 | **Superseded by Draft 0.3** (this document) and [doc 35a](35a-space-flight-review.md). |
| **SF.1** (Outer System Sky & Velocity) | **Mostly real.** Horizons rows for Uranus, Neptune, Galileans; Triton propagated 2198 row; Kepler `velocity_mps` sound. Needed `BodyDef` validation, `Sky::revision()` coverage, 2.50 AU `kirkwood_gap_3_1` fix, and unquantised circular `velocity_mps`. | **Kept & fixed in `SF.R0`** (`crates/tacsim/src/sky.rs`, `outer_bodies.json`, `tests/shard_4/sky.rs`). |
| **SF.2** (`mush-pixel` Space Views) | **Credible determinism & palette budget.** Needed ring-plane intersection fix (`planet_shader.rs`), small-frame clamp guards, `pixel_model` contact wiring, and golden frame hash pins. | **Kept & fixed in `SF.R0`** (`crates/mush-pixel/src/{planet_shader,space_cockpit,holotank,docking}.rs`, `tests/space_golden.rs`). |
| **SF.13 / SF.14 / SF.20** (Planet & Ship Bakes) | **Real SHA-pinned assets, zero raw `.fbx`/`.unitypackage` committed.** Needed pure-Python reproducible PNG decode in `planet-bake`, `bake_params` provenance, no hardcoded local paths, and compact `ships/index.json`. | **Kept & fixed in `SF.R0`** (`scripts/{planet,ship}-bake/bake.py`, `planets/`, `ships/`). |
| **SF.7 / SF.18** (WASM ABI & Clients) | **WASM ABI additive and safe.** Fixed sticky `spaceMode` in `picture.svelte.ts`; TUI screen/key wiring and docking tape submission wait for server seams (`SF.R3`, `SF.R7`). | **WASM ABI kept in `SF.R0`; client verbs reworked in `SF.R3` / `SF.R7`.** |
| **SF.3** (`bubble.rs`, `weapons.rs`) | **Weapon maths partly right, island state.** Fixed railgun world-frame lead, sub-metre `pos_rem_um` truncation, W-clock IC stamp, PDC cooldown/saturation, decoy target check, torpedo proportional nav + ballistic coast, `i128` thrust clamp, and added `BubbleInputTape` + `BubbleSnapshot` replay. | **Pure weapon/kinematics library kept & fixed in `SF.R0`; picket/kernel consumer re-cut in `SF.R1` / `SF.R6`.** |
| **SF.4** (`mining.rs`) | Self-contained island; did not touch `stow.ore`, `OreDeposit`, `port/licence.rs`, or `surface/claims.rs`. | **Removed in `SF.R0`; re-cut in `SF.R5`.** |
| **SF.5** (`space/docking.rs`) | Server-side auto-win verb; kept pure 6-DOF gate predicates (`DockingPose`, `DockingGateStatus`, `evaluate_docking_gates`) and `mush-pixel` renderer. | **Verb path removed in `SF.R0`; gate predicates kept; tape-verified session re-cut in `SF.R7`.** |
| **SF.6 / SF.17** (`flight.rs`, `pulse.rs`) | Manual player-stepped pulse, toy planner, no `reg.tick` or `JourneyState::Promoted` hook. | **Removed in `SF.R0`; re-cut in `SF.R1` and `SF.R4`.** |
| **SF.8 / SF.9 / SF.11 / SF.12 / SF.15 / SF.16 / SF.19** (`commands.rs`, `stations.rs`, `hulls.rs`, `autopilot_bot.rs`) | Unauthenticated RAM-only `SpaceBook` verbs registered as `legacy-post-image`, duplicate seat/hull tables, scripted bot. | **Removed in `SF.R0`; re-cut seam-first across `SF.R1`–`SF.R8`.** |

### 10.2 Seam-First Implementation Plan (`SF.R0` – `SF.R8`, `35a` §8)

> **Reviewed in [doc 35b](35b-seam-first-train-review.md) (2026-11-02, at `f1ea7715`).** The "(Built)" markers below record what the landing commits claimed, not what the review accepted; 35b §6 is the disposition per cut, 35b §3 the holes that must close first, and 35b §8 the plan `SF.R9`–`SF.R16` that follows this table.

Every cut after `SF.R0` targets **one real engine seam** and proves its acceptance through a `tests/shard_*` integration test driving registered `authoritative` handlers with a non-GOD actor:

| Cut | Scope | Engine Seam | Acceptance Criteria |
|---|---|---|---|
| **SF.R0** | **Review 35a Foundation, Fixes & Island Cleanup (Built)**: Add `doc 35a` and `doc 35` Draft 0.3; fix `SF.1` (`BodyDef` validation, `Sky::revision()`, 2.50 AU Kirkwood, circular `velocity_mps`, moon/derivative pins), `SF.2` (ring geometry, small-frame clamps, `pixel_model` wiring, `space_golden.rs`), `SF.13/14/20` (reproducible pure-Python planet bake, `bake_params`, compact `ships/index.json`), and `SF.3` pure library (world-frame railgun lead, `pos_rem_um`, W-clock stamp, PDC cooldown/saturation, proportional-nav + coasting torpedoes, `BubbleInputTape` + `BubbleSnapshot` replay); delete island modules (`commands.rs`, `stations.rs`, `pulse.rs`, `flight.rs`, `mining.rs`, `hulls.rs`, `autopilot_bot.rs`, `SpaceBook`) | `tacsim::sky`, `mush-pixel`, `tacsim::space::{bubble,weapons,custody}` | All `sky`, `mush-pixel` (`space_golden`), `mush-pixel-wasm`, `tacsim::space`, and web tests green; zero P0 authority holes in `crates/tacsim/src/space/`. |
| **SF.R1** | **Tick & Interdiction Promotion Seam (`reg.tick` + `JourneyState::Promoted`) (Built)**: Register a world tick consumer (`tacsim.space`, `crates/tacsim/src/space/tick.rs`) that opens a `LocalSpaceBubble` when a `CarrierJourney` leg promotes (`PromotionTrigger::Interception`), pauses the leg timer while the bubble is active (`D-35-3`, `D-35-5`), steps the bubble on 1:1 wall time (`step_wall_ms_at` with live W-clock stamps, input tape, and snapshots), and resumes the leg (`resume_carrier_journey` + `stand_down_domain`) on clear | `mush_sim::journey::CarrierJourney`, `mush_sim::domain::ActivationBook`, `Registrar::tick` | `tests/shard_2/voyage.rs` (`sfr1_interdicted_leg_promotes_pauses_leg_timer_steps_wall_bubble_and_resumes_to_dock`): a real chartered ship (*Salt Moth*) on an interdicted leg promotes on the world tick, pauses its leg timer across a 2,000,000 IC-second hold, steps its bubble on 1:1 wall time, and resumes the leg on clear to dock at Stickney Yards. |
| **SF.R2** | **Unit Custody & Canonical Hull Seam (`custody::check_out` / `check_in`) (Built)**: Build `SpaceShipState` directly from canonical `UnitSpec` (`dray.json`, `dray_p.json`, `halcyon.json`) via `custody::check_out` and write section/facet/component/heat mutations and black-box entries back via `custody::check_in` (`D-35-1`, `D-35-12`) | `mush_tactics::custody`, `TacticalUnit`, `SalvageClaimBook` | `tests/shard_2/voyage.rs` (`sfr2_space_bubble_custody_checks_out_unit_persists_damage_on_checkin_and_opens_48h_salvage_window`): damage taken in a space bubble writes back to the real hull's `TacticalUnit` cells on check-in (`+ship` reports `Damage: hull 480/600, drive 190/250.`), and disabling a hull's drive keeps its epoch open and opens a 48 real-hour (`172,800 s`) salvage window. |
| **SF.R3** | **Authoritative `+helm` / `+tac` Verbs & `mush-sim` Standing-Order Channels (Built)**: Register `+helm` and `+tac` as `authoritative`, gating on physical aboard presence (`hull_aboard_for_actor`) and `mush-sim` bridge seats/channels (`SpatialState.assignments`, `World::command_subjects_of`), writing validated `mush_sim::tick::StandingOrder` records (`Heading`, `Thrust`, `Posture`), gating sensor/weapon targeting through `mush_propagation::resolve_detection`, and wiring the TUI `v` space view cycle and `z`/`x`/`c` (`F1`–`F3`) weapon keys (`D-35-9`, `D-35-10`, `D-35-16`) | `mush_sim` channels/seats, `mush_propagation`, TUI/web views | `tests/shard_2/voyage.rs` (`sfr3_authoritative_helm_and_tac_enforce_aboard_seats_write_standing_orders_and_gate_propagation_detection`): non-GOD crew aboard a hull assign stations and submit `+helm` and `+tac` orders through the authoritative lane; unrostered/off-ship callers are refused; undetected contacts are hidden from `+tac/scan` and refused by `+tac/lock` / `+tac/beam`. |
| **SF.R4** | **Live Propellant Pool & `passage.rs` Free-Flight Planner (`D-35-6`) (Built)**: Debit free-flight and bubble burns from the hull's live `stow.propellant` pool (`HullPropellantLedger` + `PendingSpaceBurn` through `tacsim_port_burn`), implement `plan_passage_from(sky, pack, pos_km, vel_mps, target_place, ...)` and `return_reserve_from` on top of `port/passage.rs` (`D-35-6`), fix timed-burn cutoff and Tier-1 solar gravity + target-relative flip (`35a` §5 #4–#5), and recouple into a real `CarrierJourney` via `+helm/autopilot` (`recouple_carrier_journey`) | `stow.propellant`, `port/passage.rs`, `CarrierJourney` | `tests/shard_2/voyage.rs` (`sfr4_free_flight_decouples_debits_live_propellant_warns_return_reserve_and_recouples_autopilot`): a non-GOD pilot decouples from corridor transit, executes a timed burn that debits `stow.propellant` and cuts off cleanly on expiry, triggers `RETURN_RESERVE` warning at the computed return margin, and recouples via `+helm/autopilot stickney` into a real `CarrierJourney` that docks at Stickney Yards. |
| **SF.R5** | **Asteroid Field Mining on the Doc 20 Engine (`+mine`, `D-35-13`) (Built)**: Wire orbital asteroid clusters (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`) to `SiteOpener` (`OreDeposit` pools), `SurfaceTenure` (`pallas_syndicate` seeds, `CLAIM_TERM_IC_SECS = 168d`, `CONTEST_WINDOW_IC_SECS = 30d`), `port/licence.rs` (`licence_gate`, 2,500 cr), 3D Euclidean velocity matching (`<= 2,000 m`, `|Δv| < 5.0 m/s`), timed laser + 4-drone `ExcursionId` sorties, and `SiteWorkProvider` (`tacsim_site_work`) transferring conserved ore into the hull's `stow.ore` hold (`86 t` on `Dray-P`) and updating `PocketCursor` in `tacsim.pockets` | `port/licence.rs`, `surface/{tenure,claims}.rs`, `stow.ore`, `ExcursionId` | `tests/shard_2/voyage.rs` (`sfr5_chartered_dray_p_buys_licence_matches_velocity_in_pallas_mines_into_stow_ore_and_sells_at_ceres_mill`): a chartered `Dray-P` buys a 2,500 cr licence, matches velocity in `pallas_inclined_belt`, mines 20 t of `metal_ore` into `stow.ore` via laser and 4 collector drones, stakes/contests Block `C` under the 30-IC-day window, and sells the ore at Ceres Freeport's Exchange where `tacsim_port_mill` mills it into 8 t of `concentrate`. |
| **SF.R6** | **Rhai Customs Picket Doctrine & Ledger Fine (`D-35-11`) (Built)**: Run the Kirkwood Customs Picket through `MindRunner` Rhai doctrine with real `PartyId` relations, inspecting live `stow.ore` and Registry claims and assessing a real `mush-economy` Ledger fine / Customs Assertion front advance | `MindRunner`, `PartyId`, `mush-economy` Ledger, campaign fronts | `tests/shard_2/voyage.rs` (`sfr6_kirkwood_customs_picket_story_1_end_to_end_rhai_doctrine_triad_combat_and_ledger_fine`): Story 1 end-to-end through registered handlers with a non-GOD pilot. |
| **SF.R7** | **Client-Side 60 fps 6-DOF Docking Game & Server Tape Verification (`D-35-14`) (Built)**: Implement client-side 60 fps 6-DOF kinematics in `mush-pixel` / TUI / web, submit the bounded `DockingInputTape`, replay on the server from `start_pose`, debit `rcs_spent_kg` from `stow.propellant`, merge into `Berth::Docked`, and support `+helm/tow` salvage towing of clamped derelicts | `mush-pixel`, `mush-sim::journey::dock_carrier_at_destination`, `web`, `port::ship::Berth` | `tests/shard_2/voyage.rs` (`sfr7_docking_session_pauses_ship_replays_valid_tape_into_berth_docked_rejects_forged_tape_and_tows_derelict`): a docking session pauses the ship outside the tick, replays a valid client tape into `Berth::Docked` (or clamps onto a disabled derelict for `+helm/tow`), and rejects an out-of-gate or forged tape. |
| **SF.R8** | **Store-Backed Space Persistence Across Restarts (`D-35-12`) (Built)**: Persist active interdiction bubbles, free-flight records, open-epoch derelicts (`SalvageClaimBook` + decrementing life-support and contest clocks), orbital mining sessions, and paused docking sessions in `TacsimStore` (`tacsim:space:state`) so nothing is lost on server restart | `TacsimStore` | `tests/shard_2/voyage.rs` (`sfr8_space_state_persists_in_tacsim_store_and_survives_full_engine_restart_mid_bubble_free_flight_derelict_and_docking`): Story 1 and Story 2 survive a full engine restart mid-bubble and mid-free-flight with bit-identical state. |
