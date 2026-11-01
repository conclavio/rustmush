# 35. Ship free flight, Expanse-style naval combat, asteroid field mining, and the bridge/docking views

**Draft 0.2 — 2026-11-01.** Surveyed against `origin/main` at `830f2265`. Draft 0.1 captured the initial design and rulings; Draft 0.2 folds in the cross-review against landed code ([doc 18](18-trade-run-vertical-slice.md), [doc 19](19-astronomy-engine.md), [doc 20](20-surface-mining.md), [doc 21](21-graphics-engine.md), [doc 23](23-unit-lifecycle-epoch.md), [doc 27](27-ic-field-operations-design-and-survey.md), [doc 32](32-ic-combat-field.md)) and records the owner's binding rulings on `Q-35-1` through `Q-35-6`.

---

## 1. Why this document exists

Today in `tacsim`, interplanetary travel is strictly a filed autopilot passage (`+ship/undock`, `+ship/order depart`, `CarrierJourney`): a captain files a flight plan at Port A, propellant and stores are debited once at departure (`D-18-46`), the hull stays `Unplaced` with a leg cursor, and it arrives at Port B on a timer.

To support **Expanse-style tactical space combat**, **zero-G asteroid field mining**, **sightseeing expeditions to the outer solar system (Saturn, Uranus, Neptune, Triton)**, and a **Starfield-style 6-DOF docking mini-game** — while working in parallel with the IC ground combat field track ([doc 32](32-ic-combat-field.md)) without causing git conflicts on `arena/*` or `field/*` — this document specifies:
1. What earlier documents already decided and what the tree has today (§2, §3).
2. The owner's rulings (`Q-35-1..6`) and the numbered decisions (`D-35-1..18`) (§4).
3. The two-tier space model: site-anchored and encounter **Local Space Bubbles** on 1:1 wall time (`Q-CF-32`) + **System Free Flight** on the IC clock (§5).
4. The 1:1 wall-time weapon triad, point-defense, multicrew stations, light-lag sensing, customs encounters, and derelict/salvage lifecycle (§6).
5. Zero-G velocity-matched asteroid field mining integrated with [doc 20](20-surface-mining.md)'s ore goods, `stow.ore`, 4-drone leash, and the Freeport Registry claims engine (§7).
6. Three 256-colour Bayer-dithered `mush-pixel` views (`holotank.rs`, `space_cockpit.rs` + `planet_shader.rs`, and out-of-time-loop `docking.rs`) (§8).
7. Canonical acceptance stories and the staged cut plan `SF.0`–`SF.8` (§9, §10).

---

## 2. Already decided; not asked again

| Decision | Where | What it fixes for space flight, combat & mining |
|---|---|---|
| Strategic travel sets encounter geometry; combat runs in a bounded local-space domain; multicrew stations (`helm`, `tactical`, `ew`, `engineering`, `flight_ops`, `command`) use continuous channels (latest-order-wins) and discrete intents | doc 10 §8 | Two-tier model; verbs map onto multicrew stations from day one |
| Fight or evade is a propellant ($\Delta v$) decision; torch drives have high thermal bloom; civilian hulls are slow, armed warships/corvettes rare | doc 11 | $\Delta v$, relative velocity, and IR signature drive encounter geometry |
| Light-lag and observer-bounded contact projection via `mush-propagation` | doc 10 §14, doc 15 | System-scale contacts carry light-lag; EMCON and thermal bloom govern detection |
| One data/damage/party kernel for IC and OOC (`PartyId`, relations, sections, facets, heat, custody, deterministic step, tape, snapshot) | doc 32 Q-CF-1, Q-CF-3, Q-CF-19 | Space hulls reuse the exact unit sections, facet bands, heat cells, `PartyId` relations, and `custody::check_out` / `custody_in` seam |
| Engaged fields run on 1:1 wall time (adaptive 5000/1000/500 ms step with 250 ms substeps, `D-16-2`) and stamp IC instants; W-clock outage freeze (`Q-CF-33`) applies | doc 16 D-16-2, doc 27 Q-H1, doc 32 Q-CF-32, Q-CF-33 | Local Space Bubbles run on 1:1 wall time; all weapon flight times are authored in 1:1 real seconds |
| No player permadeath (`FO-12`); recoverable disablement keeps the hull epoch open; disabled hulls persist as physical salvage/derelicts with a 48 h non-owner recovery contest | doc 23, doc 27 Q-H8/H9/H12, doc 27a | Disabled ships are the system's derelicts; life-support clock + Tycho re-float / tow contracts |
| Link loss holds last safe posture (`Q-H5`, `P-2`); NPC pickets and raiders use `MindRunner` Rhai doctrine over frame fields (`Q-CF-42`, `D-UM-4`) | doc 17, doc 27 Q-H5, doc 32 Q-CF-42 | No bespoke C-style AI loop; pickets and raiders run Rhai doctrines |
| Four ore goods (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`) stored in `stow.ore` (kg / L); Freeport Registry prospecting licence (2500 cr / 168 IC days), claim stakes, 48 h contest, Pallas Syndicate outer paper; 4 drone bays per `Dray-P` sealed under `ExcursionId` | doc 20 §3.4, §7.3, §9, §17 | Asteroid mining outputs the four canonical ore goods into `stow.ore` under Registry claim rules |
| `mush-pixel` uses a planar 2D framebuffer with a closed 256-colour indexed palette (no alpha blending); 13 digit screen slots (`MAX_SCREENS`) are full; digits `1–9`/`0` are global screen switches in the TUI | doc 21 §1 | Views use 4×4 Bayer ordered dithering and stepped palette ramps; space views mount on lettered screens / contextual view cycles; combat hotkeys avoid bare screen digits |

---

## 3. What the tree has today (checked at `830f2265`)

- **Hulls & sections (`crates/tacsim/content/units/dray.json`, `Halcyon`)**: The `Dray` and `Dray-P` already define sections `hull`, `hold`, and `drive` with facet bands `front`, `left`, `right`, `rear`, and `top`. The `Halcyon` carrier defines eight berths and its own section list.
- **Standing-order channels (`crates/mush-sim/`)**: The standing-order book already carries 3D `Heading { x, y, z }`, `Thrust`, and `Emcon { 0..=3 }` channels plus multicrew seat bindings waiting for a space consumer.
- **Passage & interception seams (`crates/tacsim/src/port/passage.rs`, `voyage.rs`, `mush-sim/src/journey.rs`)**: `plan_passage` plans body-to-body flip-and-burn transits with a five-pass intercept solver. `LegDef.interdiction`, `PromotionTrigger::Interception`, `JourneyState::Promoted`, and `MAX_ACTIVE_DOMAINS` (8) exist as seams, though `Promoted` does not yet pause the leg timer or materialize a 3D bubble.
- **Sky (`crates/tacsim/src/sky.rs`, `scripts/fetch_elements.py`)**: `Sky::Kepler` computes `PosKm` (whole km in 3D ecliptic J2000) for inner and mid-system bodies through Saturn/Titan (`2340–2400`). There is no `velocity_mps` helper yet. Uranus, Neptune, Triton, and the Galilean moons can be added via `fetch_elements.py` (noting that Triton and the Galileans, like Titan, lack Horizons ephemerides past 2250 and use propagated rows).
- **Chart feed (`crates/mush-pixel/src/chart.rs`)**: The `+chart` (`sky.chart`) feed already carries `z_au` and `lat_deg` per body, which the 2D chart renderer currently flattens.

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
- **Ruling (owner, 2026-11-01):** **Unify both.** Use [doc 20](20-surface-mining.md)'s canonical ore goods (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`), `stow.ore` stowage (`Dray-P` and hoppers), four ore families with `barren`/`lean`/`fair`/`rich` richness grades, 4-drone leash per `Dray-P` (`ExcursionId`), and Freeport Registry prospecting licences, claim stakes, 48 h contests, and Pallas Syndicate paper claims — while playing inside a 3D velocity-matched space bubble around orbital asteroid clusters.

### Q-35-4 — Time base & Point-Defense under "no auto-fire"
- **Ruling (owner, 2026-11-01):**
  1. **Wall-time bubbles (`Q-CF-32`)**: Engaged Local Space Bubbles run on **1:1 wall time** (adaptive step tempo `D-16-2`), while Tier-1 system free flight runs on the runtime `WorldTimeline` IC clock (default `IC_RATIO=24`, configurable via `@tacsim/time rate=`).
  2. **Automatic Point-Defense toggle permitted**: Full automatic PDC and defensive-laser interception toggles (`+tac/pdc auto` and per-mount `Offense`/`Defense` mode) are explicitly **approved**. Since the game already permits conditional fire doctrines such as `"FIRE ALL WHEN ODDS >= 60"`, an automatic point-defense posture toggle is natural and avoids impossible manual typing races against incoming torpedo salvos.

### Q-35-5 — 6-DOF Starfield-style docking mini-game execution & time loop
- **Ruling (owner, 2026-11-01):** **Run the 60 fps 6-DOF alignment HUD client-side in `mush-pixel` / the gateway, and step the docking hull out of the live time loop while the player plays the mini-game.**
  - Taking the docking ship out of the time loop eliminates SSH latency issues and prevents time-compression drift while the pilot carefully aligns translation, pitch/yaw, roll, and closure rate; when the mini-game finishes, the bounded outcome (`Docked { quality, rcs_spent_kg, hull_scrape }`) merges back into the world state.
  - Mandatory for unassisted targets (disabled derelicts, ship-to-ship boarding/salvage, unpowered field moorings) and opt-in at ports (saving terminal approach time / RCS reaction mass).

### Q-35-6 — Visual views, 256-colour dithering, and cut priority
- **Ruling (owner, 2026-11-01):** **Keep all three 256-colour Bayer-dithered views (`holotank.rs`, `space_cockpit.rs` + analytical raycast `planet_shader.rs`, and `docking.rs` wireframe collar overlay), and build visuals early in parallel with backend cuts.**

---

## 5. Numbered decisions (`D-35-1` – `D-35-18`)

- **D-35-1 (Shared damage kernel, isolated 3D space module):** Space combat reuses the unit definitions, sections (`hull`/`hold`/`drive` on `Dray`/`Dray-P`; section list on `Halcyon`), facet bands (`front`/`left`/`right`/`rear`/`top`), component crits, heat cells, `PartyId` relations, custody check-out/in, input tape, snapshots, W-clock outage freeze, and `MindRunner` host. All 3D kilometer-scale kinematics and space verbs live in `crates/tacsim/src/space/*`.
- **D-35-2 (Parallel file boundary):** No cut in this track modifies `crates/tacsim/src/arena/*`, `crates/tacsim/src/field/*`, `crates/tacsim/src/surface/*`, or `crates/tacsim/src/detachment/*`.
- **D-35-3 (Two-tier clock rule):** Tier-1 System Free Flight advances by the live `WorldTimeline` IC delta (`dt_ic`, never hardcoding `24`). Tier-2 Local Space Bubbles advance on **1:1 wall time** (`D-16-2` adaptive 5000 / 1000 / 500 ms pulse, substeps at most 250 ms) and stamp IC instants (`Q-CF-32`).
- **D-35-4 (Outer-system ephemeris & orbital velocity):** `Sky::Kepler` gains `Sky::velocity_mps(body, ic_t) -> [i32; 3]` (central-difference derivative of `position_km`) and elements for **Uranus**, **Neptune**, **Triton**, and **Io, Europa, Ganymede, Callisto** (`fetch_elements.py`, using propagated rows post-2250 where Horizons ends, matching Titan).
- **D-35-5 (Bubbles at sites first, mid-leg decoupling second):** A `LocalSpaceBubble` (`BubbleId`) attaches first to orbital sites/nodes (Kirkwood Picket approach, Pallas Cluster, Ceres Halo, Saturn Ring Edge) using the existing `LegDef.interdiction` / `PromotionTrigger::Interception` / `JourneyState::Promoted` seam (`Promoted` pauses the leg timer while the bubble is open). Mid-leg free-flight decoupling (`+helm/decouple`) lands after site bubbles.
- **D-35-6 (Live propellant debit & return-reserve check):** While in free flight or a bubble, burns debit propellant live from the hull's tank in kg. A from-coordinate passage estimator (`plan_passage_from(pos_km, vel_mps, body)`) continuously computes the minimum $\Delta v$ required to reach the nearest open port (`RETURN_RESERVE`) and warns (`TANK_RESERVE_WARN`) before a burn would dip below the return margin; `+helm/burn force` is required to burn into stranding reserve.
- **D-35-7 (Offline / linkdead rule in space):** Under `Q-H5` / `P-2`, if all crew on a free-flying ship go linkdead, main thrust cuts to zero (ballistic drift), active point-defense remains armed for the reclaim grace window, and if unclaimed inside a safe corridor the hull either holds ballistic with a life-support notice or executes its queued return plan.
- **D-35-8 (1:1 wall-time weapon triad & automatic PDC toggle):** Weapon ranges, velocities, and flight times are authored in 1:1 wall time (§6.1). `+tac/pdc auto` and per-mount `Offense`/`Defense` laser mode are valid automatic defensive postures (`Q-35-4`). Torpedo salvos are bounded by tube count and magazine stowage (`W15`).
- **D-35-9 (Multicrew channels & seats):** `+helm`, `+tac`, and `+mine` write to `mush-sim`'s existing standing-order channels (`Heading{x,y,z}`, `Thrust`, `Emcon{0..=3}`) and seat roles (`helm`, `tactical`, `ew`, `engineering`, `flight_ops`, `command`). A solo pilot occupies command and operates all channels directly.
- **D-35-10 (Light-lag sensing & EMCON):** System-scale detection routes through `mush-propagation` light-lag and compares target IR output (passive hull floor + radiator heat + torch thrust bloom + weapon/mining laser flash) against distance squared and observer sensor aperture.
- **D-35-11 (Customs encounter & multi-party bubble):** Bubbles are multi-party (`PartyId` / relation). The canonical non-lethal/escalating encounter is the **Kirkwood Customs Picket** (hail, heave-to for inspection, contraband/claim check, fine or seizure advancing the Customs Assertion front, or running the picket under fire).
- **D-35-12 (Derelicts, disablement & salvage):** No permadeath (`FO-12`). A hull disabled in space keeps its epoch open (`doc 23`) and becomes a persistent drifting derelict on a life-support clock, eligible for distress beacons, Tycho re-float contracts, or non-owner salvage/tow (subject to `doc 27/27a`'s 2× tonnage tow cap and 48 h salvage contest window).
- **D-35-13 (Asteroid sites, canonical ores & Registry claims):** Orbital asteroid clusters are authored as `field`-kind orbital sites (`doc 20 §17`) with `OreDeposit` pools for `metal_ore`, `ice_ore`, `carbon_ore`, and `strategic_ore`, loaded into `stow.ore` (on `Dray-P` and drone/TV hoppers) under Freeport Registry prospecting licences, claim stakes, and 48 h contests. Inside the bubble, extraction uses 3D velocity matching ($|\Delta \mathbf{v}| < 5\text{ m/s}$ within $2\text{ km}$) via ship mining lasers or up to 4 sealed-mind collector drones (`ExcursionId`).
- **D-35-14 (Out-of-time-loop client-side 6-DOF docking):** Entering manual docking (`+helm/dock`) transitions the hull into a `DockingSession` paused outside the world tick while the gateway/`mush-pixel` runs the 60 fps 6-DOF collar alignment mini-game; completion submits a verified outcome intent (`Docked { quality, rcs_spent_kg, hull_scrape }`) that merges the ship into `Berth::Docked` (or airlock hard-dock with a derelict/ship).
- **D-35-15 (256-colour Bayer-dithered views & ABI):** All three space views (`holotank.rs`, `space_cockpit.rs` + `planet_shader.rs`, `docking.rs`) render into `mush-pixel`'s planar 256-colour indexed framebuffer using 4×4 Bayer ordered dithering and stepped palette ramps, bumping the `mush-pixel-wasm` feed/kind enum cleanly.
- **D-35-16 (TUI screen & key discipline):** Space views mount on a lettered TUI screen (`v` view mode toggle inside the tactical/helm screen) and never consume the 13 global digit screen slots (`MAX_SCREENS`); weapon group hotkeys in the TUI use function/letter bindings (`z`/`x`/`c` or `F1–F3`) rather than bare screen-switch digits, and forward/reverse RCS translation uses `r`/`f` rather than a lone `Shift` modifier.
- **D-35-17 (Content studio authoring):** Asteroid clusters, picket casts, and patrol doctrines are authored through the lane pack and [doc 34](34-content-studio-and-gm-workspace.md)'s content studio schemas, never a third ad-hoc loader.
- **D-35-18 (Cut ordering):** Visual renderers (`holotank.rs`, `planet_shader.rs` + `space_cockpit.rs`, `docking.rs`) and backend cuts (`Sky` outer bodies, site-anchored bubbles, customs & weapon triad, asteroid mining, mid-leg decoupling) are staged in small, parallel-safe cuts (`§10`).

---

## 6. Two-Tier Space Architecture & Expanse Combat

### 6.1 Tier 1 (IC Clock) vs. Tier 2 (1:1 Wall-Time Local Space Bubble)

1. **Filed Autopilot (`CarrierJourney`, default for offline trade):**
   - Works exactly as built in [doc 18](18-trade-run-vertical-slice.md). However, when a leg with `LegDef.interdiction` triggers `PromotionTrigger::Interception` (e.g. passing the Kirkwood Picket or arriving at an orbital asteroid cluster), `JourneyState::Promoted` pauses the leg cursor and places the hull into the site's `LocalSpaceBubble`.
2. **Tier 2 — `LocalSpaceBubble` (`crates/tacsim/src/space/bubble.rs`, 1:1 wall time):**
   - Origin $\mathbf{R}_0(t), \mathbf{V}_0(t)$ co-moves with the site body, station, asteroid cluster centre, or interception barycentre.
   - Local position $\mathbf{x}$ in metres (`[i64; 3]`, $\pm 100{,}000\text{ km}$ domain), local velocity $\mathbf{v}$ in $\text{mm/s}$ (`[i64; 3]`).
   - Steps on **1:1 wall time** (`D-16-2` adaptive 5000 / 1000 / 500 ms step with 250 ms substeps), deterministic integer math, hashed input tape, and periodic snapshots.
3. **Tier 1 — System Free Flight (`crates/tacsim/src/space/flight.rs`, `WorldTimeline` IC clock):**
   - For ships that undock manually (`+helm/undock`) or decouple mid-leg (`+helm/decouple`) to explore the outer solar system or fly off-corridor:
     - Position `pos_m: [i128; 3]` (heliocentric ecliptic J2000 metres), velocity `vel_mmps: [i64; 3]`.
     - Consumes `mush-sim` channels (`Heading{x,y,z}`, `Thrust`, `Emcon{0..=3}`) on each world tick (`dt_ic`), debiting propellant live (`D-35-6`).
     - Supports guidance modes: `Manual`, `Prograde`, `Retrograde`, `Intercept { target, flip_burn }`, and `MatchVelocity { target }`.
     - `+helm/autopilot <port>` invokes `plan_passage_from(pos_km, vel_mps, port)` to re-enter a filed `CarrierJourney` once unengaged and above `RETURN_RESERVE`.

### 6.2 The 1:1 Wall-Time Weapon Triad (`D-35-8`)

All combat inside a `LocalSpaceBubble` runs on **1:1 wall time**:

| Weapon Class | Effective Range | Kinematics (1:1 Wall Time) | Flight / Resolution Time | Active Counterplay |
|---|---|---|---|---|
| **Torpedoes** (`torpedo_tube`) | $100\text{ – }2{,}500\text{ km}$ | Inherits $\mathbf{v}_{\text{ship}}$; accelerates at $15\text{–}30\text{ g}$ ($150\text{–}300\text{ m/s}^2$) under proportional navigation with finite $\Delta v$ ($8\text{–}15\text{ km/s}$) | **$12\text{ – }45\text{ real seconds}$** (tracked live across steps on the Holo-Tank) | Automatic **PDCs** (`+tac/pdc auto`), **Defensive Lasers**, **ECM/decoys**, or high-G burn to exhaust torpedo $\Delta v$ |
| **Energy Weapons** (`laser_mount`, `pulsed_beam`) | $20\text{ – }500\text{ km}$ | Speed of light ($c$, instant hit) | **Instant per step**; builds hull heat cell (`heat_permille`) | Range diffraction ($1 / (1 + (R/R_0)^2)$), radiator management, attitude rolling across `front`/`left`/`right`/`rear`/`top` facets |
| **Railguns** (`railgun_spinal`, `railgun_turret`) | $10\text{ – }250\text{ km}$ | Unguided tungsten slug at $v_{\text{muz}} = 15\text{–}20\text{ km/s}$ + $\mathbf{v}_{\text{ship}}$ | **$1.0\text{ – }15\text{ real seconds}$** time-of-flight | **Cannot be intercepted by PDCs or lasers.** Dodged by **lateral jinking thrust** during time-of-flight outside the shooter's predicted lead pip |
| **PDCs / CIWS** (`pdc_rotary`) | $0\text{ – }15\text{ km}$ (anti-ship) / terminal envelope (anti-torpedo) | $3\text{ km/s}$ rotary bursts | Automatic terminal intercept pass when toggled on (`+tac/pdc auto`) | Saturation salvos from multiple bearings; stripping PDC mounts with lasers; magazine depletion (`W15`) |

### 6.3 Hull Sections, Facets & Subsystem Crits (`D-35-1`)

Rather than inventing a second damage schema, space combat resolves hits directly against the hull's existing unit definition (`dray.json`, `Halcyon`, etc.):
- **Facet Bands**: Incoming bearing in the ship's local frame selects `front`, `left`, `right`, `rear`, or `top` armour facets; rolling the ship (`+helm/roll` or `q`/`e`) rotates fresh armour facets toward the threat.
- **Sections (`hull`, `hold`, `drive` on `Dray`; berth/hull sections on `Halcyon`) & Components**:
  - **Radiator / Heat Cell Crit**: Lowers thermal dissipation per step, choking laser fire rate and raising IR signature.
  - **Drive Crit**: Degrades `accel_mg` or leaks propellant.
  - **Sensor / PDC Crit**: Degrades contact locks or opens a gap in the automatic PDC defense arc.

---

## 7. Zero-G Asteroid Field Mining & Registry Claims (`D-35-13`)

### 7.1 Orbital Asteroid Sites (`doc 20 §17` + 3D Bubble)

Four orbital asteroid clusters are authored in the lane pack:
1. **`ceres_halo`** — Inner Ceres co-orbital rubble cloud (Core zone; Freeport Registry commons).
2. **`kirkwood_gap_3_1`** — 3:1 resonance scatter near Kirkwood Depot (Frontier/Contested; patrolled by the Kirkwood Customs Picket and raiders).
3. **`pallas_inclined_belt`** — High-inclination ($i = 34.8^\circ$) cluster near Pallas (Frontier; contested Pallas Syndicate paper claims).
4. **`saturn_b_ring_edge`** — Outer-system shepherd cluster near Titan/Saturn (Frontier; deep-space expedition site).

### 7.2 Spectral Classes $\rightarrow$ Canonical Ore Families & `stow.ore`

Each cluster contains seeded, slowly drifting asteroid pockets backed by `OreDeposit` pools (`doc 20 §3.4`) with richness grades `barren`, `lean`, `fair`, and `rich`:
- **C-Type (Carbonaceous Chondrite)** $\rightarrow$ `ice_ore` and `carbon_ore`.
- **S-Type (Stony-Metallic)** $\rightarrow$ `metal_ore`.
- **M-Type (Nickel-Iron / Platinum Core)** $\rightarrow$ rich `metal_ore` and `strategic_ore`.

All mined matter enters **`stow.ore`** (measured in `kg` and `L`), requiring an ore-capable hull (`Dray-P` or drone/TV hoppers) and respecting `hold_unreserved` (`D-20-9`).

### 7.3 The Zero-G Mining Loop & Registry Consequences

1. **Rights & Claims**: Just as on surface sites (`D-20-11`, `D-20-12`), scanning (`+mine/ping`) and flying are free, while extraction and claim-staking consult the **Freeport Registry** (prospecting licence at 2500 cr / 168 IC days, block/rock stake, 48 h contest window). Mining an unleased or Syndicate-claimed rock without paper works in frontier space but spikes thermal bloom and records an actionable claim-jump event.
2. **Velocity Match (`+helm/match <rock_id>`)**: The pilot flies within **$2{,}000\text{ m}$** of the target asteroid pocket and nulls relative velocity to **$|\Delta \mathbf{v}| < 5\text{ m/s}$**.
3. **Extraction (`+mine/laser <rock_id>` or `+mine/drones launch`)**:
   - **Ship Mining Laser**: Direct extraction for a seated crew; generates high **thermal/IR bloom** visible across the cluster bubble and risks volatile outgassing $\Delta \mathbf{v}$ kicks if overheated.
   - **Collector Drones (`doc 20 §17`)**: Up to **4 drones per `Dray-P`** (`maximum_items = 4`), launched and sealed from bay to bay under an `ExcursionId`, running the stock `mining` doctrine (`scan()`, `sound()`, `goto()`, `mine()`, `recover()`).

---

## 8. The Three 256-Colour Visual Views (`mush-pixel`)

All three views render into `mush-pixel`'s planar 256-colour indexed framebuffer using **4×4 Bayer ordered dithering** and stepped palette ramps (`D-35-15`), avoiding any 3D triangle mesh pipeline (`doc 21 §1`).

### 8.1 View 1 — The 3D Holo-Tank (`crates/mush-pixel/src/holotank.rs`)

- Consumes `sky.chart` (at system scale, finally rendering its existing `z_au` and `lat_deg` fields!) and `space.holotank` (inside a `LocalSpaceBubble`).
- **Rotatable Cylindrical Plot**: Configurable azimuth ($\theta$) and elevation tilt ($\phi$, default $30^\circ$) with range rings.
- **Elevation Stalks**: Vertical dashed stalks from each ship, torpedo, asteroid, or planet down/up to its $(x, y, 0)$ ecliptic/orbital plane footpoint.
- **Tactical Symbology**: Velocity vectors ($\mathbf{v}$), orange thrust plumes ($\mathbf{a}$), red torpedo tracks with **TTI (`T-14s`)** tags, dithered PDC defense spheres, and railgun lead lines.

### 8.2 View 2 — First-Person Bridge Canopy & Planet Shader (`space_cockpit.rs`, `planet_shader.rs`)

- **Analytical Sphere & Ring Raycaster (`planet_shader.rs`)**: Pure 2D per-pixel integer/fixed-point sphere + ring-plane intersection shader with solar phase illumination and 4×4 Bayer dithering:
  - **Neptune**: Cobalt-azure methane gradient, zonal storm bands, Great Dark Spot, bright high-altitude cirrus streaks, faint ring arcs, and **Triton** in orbit.
  - **Saturn & Titan**: Golden-ochre banded globe with perspective **A/B/C/Cassini Division rings** casting shadow bands onto the cloud tops, plus Titan's orange nitrogen haze limb.
  - **Jupiter & Galileans**, **Uranus** ($97.8^\circ$ tilted rings), **Mars / Phobos / Deimos**, **Ceres & Vesta**: Distinct dithered palettes and crater/albedo maps.
- **Canopy HUD & Contacts**:
  - Bridge canopy struts and cowling gauges (facet armour doll, capacitor, radiator heat bar, $\Delta v$ reserve).
  - **Prograde ($\oplus$) / Retrograde ($\otimes$) reticles** and **Railgun Lead Pip** ($\mathbf{r}_{\text{target}} + \mathbf{v}_{\text{rel}} \cdot t_{\text{tof}}$).
  - Distant ships render as tactical bracket symbology + thruster flare sprites; close-range hulls render their baked 8-facing/2-pitch sprites (`doc 22/31`); top-left corner embeds a mini Holo-Tank inset.

### 8.3 View 3 — Out-of-Time-Loop 6-DOF Docking Camera (`docking.rs`, `D-35-14`)

- Runs **client-side at 60 fps** in `mush-pixel` / the gateway while the docking hull is paused in a `DockingSession` outside the world tick (`Q-35-5`).
- Renders the target's illuminated **geometric wireframe/vector docking collar** and approach chevrons in true 6-DOF perspective from the bow collar camera.
- **Alignment HUD**:
  - Lateral translation box ($\Delta x, \Delta y$ within `±0.25 m`), Pitch/Yaw needles (`±2.0°`), Roll alignment ring (`Q`/`E`), and Closure velocity tape ($0.1\text{–}0.8\text{ m/s}$ green latch zone).
- **Controls (no bare `Shift` dependency)**: `W`/`S`/`A`/`D` lateral/vertical RCS translation, `Arrow Keys` pitch/yaw trim, `Q`/`E` roll, `R`/`F` forward/reverse closure pulse, `X` RCS dampeners.
- Upon clamp lock (or abort/collision), the client submits the signed session outcome and the hull merges back into the live world state (`Berth::Docked` at a port with approach time/burn saved, or hard-docked to a derelict/ship for salvage).

---

## 9. Canonical Acceptance Stories

### Story 1 — The Kirkwood Customs Picket (Multi-Party Interception & Triad Combat)
1. A player `Dray-P` returning from `pallas_inclined_belt` with `strategic_ore` in `stow.ore` reaches the Kirkwood leg boundary (`LegDef.interdiction`), promoting into the Kirkwood `LocalSpaceBubble` (`JourneyState::Promoted`) alongside a **Republic Customs Corvette** (Party A, Neutral-Inspecting) and a lurking **Belt Raider** (Party B, Hostile).
2. The picket hails on light-lagged radio (`+helm` / `tacsim:notice`), ordering the player to heave-to (`|\Delta \mathbf{v}| < 10\text{ m/s}`).
3. When the raider lights its torch and launches a 2-torpedo salvo at the player, the bubble runs on 1:1 wall time: the player toggles `+tac/pdc auto`, coordinates with the customs corvette's screen, jinks laterally to dodge a 120 km railgun shot, and either submits to customs inspection or burns clear to resume passage.

### Story 2 — Pallas Claim-Jump, Derelict Salvage & Outer-System Neptune Expedition
1. A crew flies into `pallas_inclined_belt`, pings an M-type rock (`strategic_ore` + `metal_ore`), matches velocity ($|\Delta \mathbf{v}| < 5\text{ m/s}$), and deploys 4 collector drones (`ExcursionId`) + the ship mining laser.
2. Their thermal bloom attracts a rival claim-jumper; after disabling the attacker's drive in a laser/railgun duel, the attacker's hull remains in the bubble as an open-epoch **derelict**.
3. The pilot approaches within $500\text{ m}$, triggers `+helm/dock <derelict>`, plays the **out-of-time-loop 6-DOF docking mini-game** (`docking.rs`) to hard-dock with the tumbling hull, and attaches a tow/salvage line under `doc 27a`'s 48 h contest rules.
4. With the payout, the captain refits long-range drop tanks, decouples into **System Free Flight** (`+helm/decouple`), burns out past Saturn and Uranus to **Neptune & Triton**, switches to the **First-Person Bridge Canopy** (`space_cockpit.rs`), and watches Neptune's banded azure storms and Triton rise across the canopy glass.

---

## 10. Implementation Cut Plan (`SF.0` – `SF.12` — Built)

Staged so visuals and backend cuts proceed cleanly with **zero edits** to `arena/*`, `field/*`, `surface/*`, or `detachment/*`:

| Cut | Scope | Key Files | Acceptance Criteria |
|---|---|---|---|
| **SF.0** | Design doc Draft 0.2 & README index | `docs/design/35-ship-free-flight-combat-and-mining.md`, `docs/design/README.md` | Rulings `Q-35-1..6`, `D-35-1..18`, and acceptance stories recorded. |
| **SF.1** | **Outer System Sky, Orbital Velocity & Asteroid Sites**: Add Uranus, Neptune, Triton, Galilean moons via `fetch_elements.py`; add `Sky::velocity_mps`; author 4 orbital asteroid cluster definitions | `crates/tacsim/src/sky.rs`, `crates/tacsim/content/sky/outer_bodies.json`, `tests/shard_4/sky.rs` | `Sky::position_km` and `Sky::velocity_mps` track Uranus, Neptune, Triton, Galileans, and 4 clusters; all existing `tests/sky.rs` pins green. |
| **SF.2** | **3D Holo-Tank & Analytical Planet/Canopy Shaders (`mush-pixel`)**: Build `holotank.rs` (3D elevation stalks on `sky.chart` and `space.holotank`), `planet_shader.rs` (256-colour Bayer-dithered raycast Neptune, Saturn+rings, Jupiter, Mars, Ceres), and `space_cockpit.rs` | `crates/mush-pixel/src/holotank.rs`, `planet_shader.rs`, `space_cockpit.rs` | Deterministic 640×480 / 1280×960 golden frames for Holo-Tank and Bridge Canopy (Neptune, Saturn rings, HUD reticles). |
| **SF.3** | **Site-Anchored Local Space Bubbles, Customs Picket & 1:1 Wall-Time Weapon Triad (`+tac`)**: Hook `PromotionTrigger::Interception` / orbital sites into `tacsim::space::bubble`; reuse unit sections/facets/heat/`PartyId`/`MindRunner`; implement Torpedoes, Lasers, Railguns, and automatic PDC/Laser defense toggle (`+tac/pdc auto`) | `crates/tacsim/src/space/mod.rs`, `bubble.rs`, `weapons.rs`, `picket.rs` | Story 1 (Kirkwood Customs Picket + 1:1 wall-time torpedo/PDC/railgun-jink combat + open-epoch derelict disablement) passes deterministically. |
| **SF.4** | **Zero-G Asteroid Field Mining & Registry Integration (`+mine`)**: Connect orbital asteroid bubbles to `OreDeposit` (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`), `stow.ore`, Freeport Registry licences/claims/contests, velocity-match gate ($|\Delta \mathbf{v}| < 5\text{ m/s}$), mining laser IR bloom, and 4-drone `ExcursionId` sorties | `crates/tacsim/src/space/mining.rs` | Ship enters `pallas_inclined_belt`, pings C/S/M rocks, matches velocity, mines canonical ores into `stow.ore` under Registry claim rules, and triggers IR bloom. |
| **SF.5** | **Out-of-Time-Loop 6-DOF Docking Mini-Game (`docking.rs` + `tacsim::space::docking`)**: Client-side 60 fps 6-DOF wireframe collar renderer & physics (`W/S/A/D`, `Arrows`, `Q/E`, `R/F`, `X`), paused `DockingSession` outside the world tick, outcome merge for ports and derelict/ship hard-docking | `crates/mush-pixel/src/docking.rs`, `crates/tacsim/src/space/docking.rs` | Manual 6-DOF alignment clamps at `< 0.8 m/s` within tolerances while paused from the world clock; mandatory for derelicts/ship-to-ship, opt-in at ports. |
| **SF.6** | **Tier-1 System Free Flight & Mid-Leg Decoupling (`+helm/decouple`)**: `SpaceFlightRecord` on the `WorldTimeline` IC clock, live propellant debit, `plan_passage_from` return-reserve check (`TANK_RESERVE_WARN`), light-lag contacts (`mush-propagation`), and outer-system expeditions to Neptune/Saturn | `crates/tacsim/src/space/flight.rs` | A pilot decouples mid-leg or undocks into free flight, burns to Saturn/Neptune with live propellant debit and reserve guard, and re-couples via `+helm/autopilot`. |
| **SF.7** | **SSH TUI & Web WASM Integration**: Wire `holotank`, `space_cockpit`, and `docking` feeds and non-colliding keybindings in `mush-net-ssh`, `mush-pixel-wasm`, and `web/src/lib/pixel/` | `crates/mush-net-ssh/src/tui/`, `crates/mush-pixel-wasm/src/lib.rs`, `web/src/lib/pixel/wasm.ts` | End-to-end playable over SSH (Kitty/Sixel) and Web browser. |
| **SF.8** | **End-to-End Acceptance Stories**: Automated story tests for Story 1 (Kirkwood Customs Picket) and Story 2 (Pallas Mining, Derelict Docking & Neptune Expedition) | `crates/tacsim/src/space/mod.rs` | Both canonical stories green in CI. |
| **SF.9** | **Live MUSH Space Verbs & Panel Feeds (`+helm`, `+tac`, `+mine`)**: Register `SpaceBook` state slot, `space.holotank`, `space.cockpit`, and `space.docking` panel feeds, and `+helm`, `+tac`, `+mine` commands | `crates/tacsim/src/space/commands.rs`, `docs/help/ships.md` | Live MUSH verbs drive Stories 1 and 2 and emit panel keyframes. |
| **SF.10** | **TacticalUnit Custody, Black-Box Forensics & SalvageClaimBook Bridge**: Bidirectional `TacticalUnit` <-> `SpaceShipState` damage round-trip (`UnitMutationPlan`), black-box forensics recording, and 48-hour `SalvageClaimBook` registration | `crates/tacsim/src/space/custody.rs` | Space combat damage persists onto `TacticalUnit` cells and `SalvageClaimBook` settles 48h salvage claims. |
| **SF.11** | **Autonomous Panel-Driven Space Pilot Doctrine (`SpacePilotDoctrine`)**: Closed-loop bot pilot that ingests `space.holotank`, `space.cockpit`, and `space.docking` JSON keyframes and 60 fps 6-DOF RCS controls | `crates/tacsim/src/space/autopilot_bot.rs` | Autonomous pilot completes Stories 1 and 2 from panel feeds. |
| **SF.12** | **Content Studio Space Catalog & Standalone `mush-bot` Space Lane (`SpaceLane`)**: Expose `space_clusters`, `outer_bodies`, and `+helm`/`+tac`/`+mine` in `content_studio.rs`; add standalone SSH line-mode `SpaceLane` in `mush-bot` | `crates/tacsim/src/content_studio.rs`, `crates/mush-bot/src/mine/space.rs` | `content_studio` catalog and `mush-bot` line-mode contract test green in CI. |
