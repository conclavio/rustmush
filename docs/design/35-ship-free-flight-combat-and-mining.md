# 35. Ship free flight, Expanse-style naval combat, asteroid field mining, and the bridge/docking views

**Draft 0.1 — 2026-11-01, owner rulings R1–R4 folded.** Designed to run in parallel with the IC ground combat field track ([doc 32](32-ic-combat-field.md)) with zero file or state-family overlap on `arena/`, `field/`, `surface/` or `detachment/`.

---

## 1. Owner rulings (2026-11-01, binding)

- **R1 — Hybrid two-tier flight model (D-35-1, D-35-2).** The filed-flight-plan autopilot (`+ship/undock`, `+ship/order depart`, `CarrierJourney`) stays intact for offline trade runs. Alongside it, **System Free Flight** (`+helm/undock`, `+helm/decouple`, `+helm/burn`, `+helm/course`) models a free-flying ship as a 3D heliocentric/planetocentric state vector ($\mathbf{r}_{\text{km}}, \mathbf{v}_{\text{m/s}}$) on the 24:1 IC clock across the full `Sky::Kepler` ephemeris (extended with **Uranus, Neptune, Triton**, and orbital **asteroid clusters**). When ships close within sensor/weapon range of another ship, an asteroid cluster, or a port approach zone, they enter a **Local Space Bubble** (`tacsim::space::bubble`) — a co-moving Newtonian tactical domain in metres and $\text{m/s}$ ticking at a 1 s wall-time pulse.
- **R2 — Active counterplay weapon triad (D-35-5..8).** Ship combat balances three weapon classes around relative velocity and vector physics at the 24:1 time ratio:
  - **Torpedoes** (long range, $500\text{–}10{,}000\text{ km}$, $10\text{–}30\text{ s}$ real flight time): inherit the launching ship's velocity vector $\mathbf{v}_{\text{ship}}$, accelerate under proportional navigation with a finite $\Delta v$ propellant budget, and deal heavy hull/section damage. Countered by **PDCs**, **defensive laser mode**, **ECM/decoys**, or out-burning the torpedo's $\Delta v$ envelope.
  - **Energy weapons / lasers** (medium range, $100\text{–}2{,}000\text{ km}$, instant hit): lightspeed, ammunition-free, governed by reactor draw, diffraction range falloff, and **hull thermal load**. Can toggle per mount between **Offense** (stripping sensors, radiators, and armour facets) and **Point-Defense** (intercepting incoming torpedoes).
  - **Railguns** (close-to-medium knife-fight range, $50\text{–}800\text{ km}$, rare/heavy mounts): unguided relativistic slugs ($15\text{–}25\text{ km/s}$ muzzle velocity) that **cannot be intercepted by PDCs or lasers**, punch through armour into internal sections/components, draw a capacitor spike and impart recoil impulse, and are countered by **evasive thrusting / jinking** during the slug's $2\text{–}20\text{ s}$ time of flight.
- **R3 — Zero-G velocity-match asteroid field mining (D-35-9..11).** Pilots fly into orbital asteroid clusters (Main Belt, Kirkwood, Pallas inclined belt, Saturn ring-shepherds), ping drifting **C-type** (volatiles/ice), **S-type** (metals/silicates) and **M-type** (heavy metals/strategic elements) rocks, match relative velocity ($|\Delta \mathbf{v}| < 5\text{ m/s}$ within $2\text{ km}$), and extract ore with **mining lasers** and **collector drones** directly into the ship's hold while managing **thermal/IR bloom** (which increases detection range to raiders/patrols) and avoiding tumbling debris.
- **R4 — 6-DOF Starfield-style docking mini-game + Holo-Tank & Bridge views (D-35-12..16).**
  - **Offline/queued ships** continue to auto-dock at ports via station tugs.
  - **Live pilots at ports** may opt into manual approach (`+helm/dock`) in the **6-DOF Docking Camera View**, waiving port tug fees and completing turnaround immediately upon hard-dock.
  - **Unassisted targets** — ship-to-ship boarding/salvage/fuel transfer, derelicts, and asteroid moorings — require manual 6-DOF docking alignment.
  - **Three `mush-pixel` visual views** (Kitty, Sixel, Web WASM): the **3D Holo-Tank** (`holotank.rs`), the **First-Person Space Bridge Cockpit** (`space_cockpit.rs`, with procedural planet/ring/atmosphere rendering so crews can fly out and admire Neptune, Saturn, Jupiter, Mars, etc.), and the **Docking Port Alignment Camera** (`docking.rs`).

---

## 2. Parallelism & Non-Overlap Boundary

To ensure zero merge conflicts with the ongoing IC combat field and detachment work ([doc 32](32-ic-combat-field.md)):

| Subsystem | Owned by Doc 32 (Claude) | Owned by Doc 35 (This Track) | Shared Seam (Read-Only or Narrow Hook) |
|---|---|---|---|
| **Tactical Ground / Detachments** | `tacsim/src/arena/*`, `tacsim/src/field/*`, `tacsim/src/surface/*`, `tacsim/src/detachment/*` | *(untouched)* | None |
| **Space Flight, Combat & Asteroid Mining** | *(untouched)* | `tacsim/src/space/*` (`mod.rs`, `flight.rs`, `bubble.rs`, `weapons.rs`, `mining.rs`, `docking.rs`, `commands.rs`, `feeds.rs`) | Narrow hook in `port/voyage.rs` & `port_tick.rs` for decoupling/handing off a hull between `Berth::Docked` / `CarrierJourney` and `SpaceFlightState` |
| **Ephemeris & Outer System** | *(untouched)* | `tacsim/content/lane/ceres_phobos.json` (or `sky/outer_bodies.json` + asteroid belts) | `tacsim/src/sky.rs` (`Sky::position_km`, `Sky::velocity_mps`) |
| **Pixel Graphics (`mush-pixel`)** | `cockpit.rs` (ground), `heightfield.rs`, `iso25d.rs` | `holotank.rs`, `space_cockpit.rs`, `docking.rs`, `planet_shader.rs` | `Frame`, `Scene`, `Picture` trait (`picture.rs`) |
| **SSH TUI & Web Client** | Ground cockpit & detachment tabs | `tui/space_screen.rs` + `web/src/components/SpaceBridgeView.svelte` | Screen registry (`ScreenId` hotkey) |

---

## 3. Architecture: The Hybrid Two-Tier Space Model

### 3.1 Tier 1 — System Vector Cruise (`tacsim::space::flight`)

A ship in free flight outside a station berth or filed `CarrierJourney` is represented by a durable `SpaceFlightRecord` (`tacsim:space:flight:{hull}`):

```rust
pub struct SpaceFlightRecord {
    pub schema: u16,
    pub hull: ObjId,
    pub pilot: ObjId,
    /// Heliocentric ecliptic J2000 position in metres (i128 or km + m offset).
    pub pos_m: [i128; 3],
    /// Heliocentric velocity in millimetres per second (i64).
    pub vel_mmps: [i64; 3],
    /// Nose attitude: yaw (0..360_000 mdeg) and pitch (-90_000..=90_000 mdeg), plus roll.
    pub attitude_mdeg: [i32; 3],
    /// Current throttle in permille (0..=1000) and active guidance mode.
    pub throttle_permille: u32,
    pub guidance: GuidanceMode,
    /// Thermal reservoir (0..=1000 permille of radiator capacity) and EMCON state.
    pub heat_permille: u32,
    pub emcon: EmconPosture,
    pub last_ic: i64,
    /// Present when the hull is inside a Local Space Bubble.
    pub bubble: Option<BubbleId>,
}
```

**Guidance modes (`GuidanceMode`):**
- `Manual` — holds commanded attitude $(\text{yaw}, \text{pitch})$ and burns at `throttle_permille`.
- `Prograde` / `Retrograde` — aligns nose with $+\hat{\mathbf{v}}$ or $-\hat{\mathbf{v}}$ (relative to the nearest dominant gravity well / target body).
- `Intercept { target, flip_burn: bool }` — computes the brachistochrone / lead vector toward a body, port, asteroid field, or contact, automatically executing the midpoint **flip-and-burn** deceleration if `flip_burn` is enabled (`+helm/course <target> [throttle=N]`).
- `MatchVelocity { target }` — aligns thrust against $\Delta \mathbf{v} = \mathbf{v}_{\text{ship}} - \mathbf{v}_{\text{target}}$ until $|\Delta \mathbf{v}| = 0$.

**Transitions between Filed Autopilot and Free Flight:**
1. **Undock into Free Flight (`+helm/undock`)**: Instead of filing a port-to-port `CarrierJourney`, the ship casts off into the port's local approach bubble at the port body's orbital position $\mathbf{r}_{\text{body}}(t)$ and orbital velocity $\mathbf{v}_{\text{body}}(t)$.
2. **Decouple Mid-Passage (`+helm/decouple`)**: While a `CarrierJourney` is `Cruising`, the pilot on the Bridge can decouple the autopilot. The hull's current interpolated 3D position $\mathbf{r}(t)$ and velocity vector $\mathbf{v}(t)$ along the active leg are materialized into a `SpaceFlightRecord`, and the `CarrierJourney` transitions to `Diverted` / handed off.
3. **Resume Filed Plan (`+helm/autopilot <port>`)**: Once clear of engagements and inside a valid corridor envelope with sufficient propellant in the tank, the pilot can hand the ship back to the passage planner.

### 3.2 Outer Solar System & Sightseeing (`Sky::Kepler` extension)

To make flying out to the ice giants and ringed worlds a real expedition:
- Add osculating Keplerian element tables (`2340–2400`) to the sky pack for:
  - **Uranus** ($a \approx 19.19\text{ AU}$, axial tilt $97.8^\circ$, faint ring system)
  - **Neptune** ($a \approx 30.07\text{ AU}$, deep azure methane atmosphere, banded storms, Great Dark Spot)
  - **Triton** (retrograde, inclined moon of Neptune, $a \approx 354{,}759\text{ km}$, nitrogen geysers)
  - **Callisto, Ganymede, Europa, Io** (Galilean moons of Jupiter)
- Add **High-Burn Torch / Drop-Tank Refits** and **Magnetic Ram-Scoop / Deep-Space Cruise Modes** so an explorer or long-range corvette can reach Saturn ($9.5\text{ AU}$), Uranus ($19.2\text{ AU}$) and Neptune ($30.1\text{ AU}$), or stage from Ligeia Terminal on Titan!

### 3.3 Tier 2 — Local Space Bubble (`tacsim::space::bubble`)

When a free-flying ship:
- Approaches within **$10{,}000\text{ km}$** of a station, asteroid field, or another ship with a hostile/interdiction vector, or
- Is interdicted along a corridor leg,

the ships share a **`LocalSpaceBubble`** (`BubbleId`), whose origin $\mathbf{R}_0(t), \mathbf{V}_0(t)$ co-moves with the reference body (station, asteroid cluster centre, or encounter barycentre).
- **Local coordinates**: $\mathbf{x}$ in metres (`i64`, supporting bubbles up to $\pm 100{,}000\text{ km}$ with millimetre/metre precision), local velocity $\mathbf{v}$ in $\text{mm/s}$ (`i64`).
- **Cadence**: Runs on a **1 s wall-time pulse** (matching the 24:1 IC time ratio where $1\text{ s}$ wall = $24\text{ IC s}$ of long-range kinematics, or $1\times$ tactical time during close-in docking/knife-fighting; see §4.1).

---

## 4. Expanse-Style Ship Combat & Weapon Balance

### 4.1 Time Ratio & Engagement Envelope (24:1 Scaling)

Because the world runs at **24 IC seconds per real second**, long-range orbital kinematics feel brisk and tactical without multi-hour dead time:

| Weapon / Phase | Engagement Range | Projectile Speed / Kinematics | Real-Time Flight / Resolution | Tactical Counterplay |
|---|---|---|---|---|
| **Torpedoes** (`torpedo_tube`) | $500\text{ – }10{,}000\text{ km}$ | Inherits $\mathbf{v}_{\text{ship}}$; burns at $20\text{ g}$ ($196\text{ m/s}^2$), $\Delta v = 45\text{ km/s}$ | **$8\text{ – }30\text{ real seconds}$** (tracked live across pulses on the Holo-Tank) | PDCs, Defensive Lasers, ECM/Chaff decoys, or hard evasive burn to exhaust torpedo $\Delta v$ |
| **Energy Weapons** (`laser_mount`, `particle_lance`) | $100\text{ – }2{,}500\text{ km}$ | Speed of light ($c$, instant hit) | **Instant per pulse** (1 s cadence; builds ship heat) | Range diffraction ($1 / (1 + (R/R_0)^2)$), radiator management, ablative armour, attitude rolling to spread thermal damage across facets |
| **Railguns** (`railgun_spinal`, `railgun_turret`) | $50\text{ – }800\text{ km}$ | Unguided slug at $v_{\text{muz}} = 20\text{ km/s}$ + $\mathbf{v}_{\text{ship}}$ | **$2\text{ – }15\text{ real seconds}$** time-of-flight | **Cannot be shot down by PDCs/lasers.** Dodged by **jinking** (changing thrust vector during time-of-flight outside the shooter's lead solution) |
| **PDCs / CIWS** (`pdc_rotary`) | $0\text{ – }50\text{ km}$ (anti-ship) / terminal envelope (anti-torpedo) | $3\text{ km/s}$ rotary bursts | Automatic terminal intercept pass + close-in shredding | Saturation salvos (firing 4+ torpedoes from multiple bearings), disabling PDC mounts with lasers |

### 4.2 Relative Velocity ($\Delta \mathbf{v}$) & Vector Tactics

Every weapon inherits the firing ship's velocity vector $\mathbf{v}_{\text{ship}}$:
1. **Closing Velocity Boost**: Launching a torpedo or firing a railgun while burning toward the target at $+10\text{ km/s}$ adds $+10\text{ km/s}$ to the projectile's closing speed, shrinking torpedo flight time and leaving the torpedo extra $\Delta v$ for terminal high-G jinking.
2. **Stern Chase Advantage**: A fleeing ship dropping torpedoes astern forces the pursuing ship to fly *into* the torpedoes' basket; the pursuer's own closing speed works against their PDC reaction window.
3. **Slash Attack vs. Matched-Vector Duel**:
   - Two ships crossing at $|\Delta \mathbf{v}| = 15\text{ km/s}$ have only a brief, high-intensity railgun/laser window before flashing past each other and needing a high-G flip-and-burn to re-engage.
   - To board, salvage, or demand surrender, the interceptor must burn to match vectors ($|\Delta \mathbf{v}| \to 0$).
4. **Thermal Signature & EMCON (`+helm/emcon quiet|standard|active`)**:
   - A ship burning its Epstein/fusion torch is visible across the solar system on passive IR.
   - A ship drifting ballistic (`throttle = 0`) with radiators throttled (`emcon = quiet`) and active radar off is nearly invisible at long range until heat builds up or it fires a weapon/mining laser.

### 4.3 Hull Damage, Sections & Crew Stations

Ships use `mush-tactics`' compositional `TacticalUnit` model adapted for spacecraft:
- **Four Structural Sections**: `Bow` (sensors, forward torpedo tubes, docking collar), `Port` & `Starboard` (PDC sponsons, radiators, cargo bays, habs), `Stern` (main fusion drive, reactor, propellant tanks).
- **Subsystem Crits**:
  - **Radiator damage** reduces heat dissipation per pulse, forcing the crew to throttle back lasers or risk thermal shutdown.
  - **Drive hit** degrades maximum thrust (`accel_mg`) or leaks propellant.
  - **PDC / Sensor hit** opens blind spots in the torpedo defense umbrella (shown as red/dark arcs on the Holo-Tank).

---

## 5. Zero-G Asteroid Field Mining (`tacsim::space::mining`)

### 5.1 Orbital Asteroid Fields

Unlike Doc 20's planetary surface sites (where a Dray-P lands on a 256×256 ground grid and deploys wheeled/tracked TVs), **Asteroid Field Clusters** are persistent orbital zones in `Sky::Kepler`:
1. **`ceres_halo`** — Inner Ceres co-orbital rubble cloud (Core zone; C-type ice-rich and S-type stony).
2. **`kirkwood_gap_3_1`** — Unstable resonance scatter near Kirkwood Depot (Frontier/Contested; S-type metals and M-type nickel-iron pockets; frequented by Republic pickets and raiders).
3. **`pallas_inclined_belt`** — High-inclination ($i = 34.8^\circ$) cluster near Pallas (Frontier; rich M-type strategic elements and platinum-group cores).
4. **`saturn_b_ring_edge`** — Outer-system ice/silicate shepherd cluster near Titan/Saturn (Frontier; ultra-pure water/ammonia volatiles and carbon chondrites).

### 5.2 The Zero-G Mining Loop

1. **Enter & Ping (`+helm/ping`)**: Inside the field's Local Space Bubble, drifting asteroids ($50\text{ m}$ to $2\text{ km}$ across) move on slow relative trajectories with spin rates. Active radar/spectrographic ping classifies nearby rocks by spectral type:
   - **C-Type (Carbonaceous)**: yields `water_ice` (`volatiles`) + `carbon_feedstock`.
   - **S-Type (Stony-Metallic)**: yields `metal_ore` (`metals`).
   - **M-Type (Metallic Core)**: high-grade `metal_ore` + `strategic_elements` (`rare_earths`).
2. **Approach & Match Velocity (`+helm/match <rock_id>`)**: The pilot flies within **$2{,}000\text{ m}$** of the target asteroid and nulls relative velocity to **$|\Delta \mathbf{v}| < 5\text{ m/s}$**.
3. **Fracture & Scoop (`+mine/laser <rock_id>`, `+mine/drones launch`)**:
   - **Mining Laser**: Cuts ore chunks from the asteroid's facets. If the asteroid is spinning rapidly, keeping the beam on a rich seam requires matching its rotation or blasting a spin-dampening charge.
   - **Collector Drones (*Occator-pattern*)**: Scoops severed chunks into the hull's `stow.bulk` cargo pool (respecting both mass in tonnes and volume in $\text{m}^3$, R7).
4. **Risk & Signature**:
   - Running a multi-megawatt mining laser spikes the ship's **thermal/IR bloom**, making the ship detectable across the entire cluster to pirate raiders or rival claim-jumpers.
   - Overheating a volatile-rich C-type pocket without pulsing the beam can trigger an **outgassing blow-off jet**, imparting a sudden $\Delta \mathbf{v}$ kick to both the rock and the ship!

---

## 6. The Three Visual Views (`mush-pixel`, SSH TUI, Web WASM)

All three views follow Doc 21's architecture (`mush-pixel` producing a `Frame` encoded via Kitty Graphics `a=t`/`a=p`, Sixel, and `mush-pixel-wasm` in the browser).

### 6.1 View 1 — The 3D Holo-Tank (`crates/mush-pixel/src/holotank.rs`)

An interactive, rotatable 3D cylindrical/spherical holographic tactical plot inspired by *The Expanse*'s CIC holo-table:
- **3D Projection**: Configurable azimuth ($\theta$) and elevation tilt ($\phi$, default $30^\circ$) with logarithmic or linear range rings ($10\text{ km}$, $100\text{ km}$, $1{,}000\text{ km}$, $10{,}000\text{ km}$).
- **Elevation Stalks**: Every 3D contact (ship, torpedo, asteroid, station) projects a vertical dashed stalk down (or up) to its $(x, y, 0)$ footpoint on the reference ecliptic disc so 3D altitude ($+z / -z$) is immediately legible at a glance.
- **Vectors & Envelopes**:
  - Cyan/amber **velocity vector arrows** ($\mathbf{v}$) and bright orange **burn plumes** ($\mathbf{a}$) on ships.
  - Red dashed **torpedo tracks** with pulsing warhead pips and **TTI (Time-To-Impact)** countdown tags (`T-14s`).
  - Translucent **PDC defense spheres**, **laser range rings**, and **railgun lead cones**.
- **Controls**: `[` / `]` or mouse drag to rotate the holo-tank azimuth; `+` / `-` to step range scale; `t` to cycle target lock.

### 6.2 View 2 — First-Person Space Bridge Cockpit (`crates/mush-pixel/src/space_cockpit.rs`)

A 640×480 / 1280×960 20 Hz first-person view from the Bridge canopy (`v` toggles between First-Person Bridge and Full Holo-Tank, with a mini Holo-Tank inset in the top-left corner just like G.13's ground minimap inset!):
- **Starfield & Ephemeris Skybox**:
  - Deterministic starfield with Milky Way band and constellations rotated by the ship's true 3D attitude $(\text{yaw}, \text{pitch}, \text{roll})$.
  - **Procedural Raycast Planets, Moons & Rings (`planet_shader.rs`)**:
    - Any solar-system body in view is raycast as a true 3D sphere illuminated by the Sun's vector!
    - **Planet shaders**:
      - **Neptune**: Deep cobalt-azure atmosphere, zonal storm bands, the Great Dark Spot, bright high-altitude methane cirrus streaks, faint ring arcs, and **Triton** orbiting nearby.
      - **Saturn & Titan**: Golden-ochre banded globe with perspective-projected multi-band **A/B/C/Cassini Division rings** casting shadows on the planet, plus Titan's orange nitrogen haze limb.
      - **Jupiter**: Banded ammonia clouds + Great Red Spot.
      - **Mars, Phobos & Deimos**: Rust-ochre terrain, Valles Marineris scar, polar ice cap, and lumpy cratered moon silhouettes.
      - **Earth & Luna / Ceres / Vesta / Uranus**: Distinct palettes, limb Rayleigh glow, and crater/albedo maps.
- **Tactical & Flight HUD**:
  - Bridge canopy struts and cowling gauges (Hull integrity doll, Capacitor charge, Radiator heat bar, Propellant $\Delta v$ remaining).
  - **Prograde ($\oplus$) and Retrograde ($\otimes$) vector reticles** on the canopy glass.
  - **Target Box & Railgun Lead Pip**: Computes the exact 3D intercept lead point $\mathbf{r}_{\text{target}} + \mathbf{v}_{\text{rel}} \cdot t_{\text{tof}}$ so the pilot can align the nose and fire railguns manually.
  - **Visual FX**: Fusion drive torches (blue-white Epstein-style actinic plumes), pulsed laser beams, railgun muzzle flash + tungsten plasma trail, PDC tracer streams, and asteroid tumbling sprites/wireframes.

### 6.3 View 3 — Starfield-Style 6-DOF Docking Mini-Game (`crates/mush-pixel/src/docking.rs`)

Triggered when within $500\text{ m}$ of a station berth (`+helm/dock`) or a target ship/derelict (`+helm/dock <contact>`):
- **Perspective Docking Camera**:
  - Looks straight out from the ship's bow docking collar at the target's illuminated docking port ring and approach chevrons.
- **6-DOF Alignment HUD**:
  - **Center Crosshair & Target Translation Box**: Shows horizontal ($\Delta x$) and vertical ($\Delta y$) offset in metres (`±0.25 m` green zone).
  - **Pitch / Yaw Attitude Needles**: Shows angular misalignment between the two collars (`±2.0°` green zone).
  - **Roll Alignment Ring**: Rotating collar indicator (`Q` / `E` to roll into alignment with the target's latch keys).
  - **Closure Rate Tape ($v_z$) & Range Readout**: Distance in metres ($500\text{ m} \to 0.0\text{ m}$) and closing speed in $\text{m/s}$ (safe latch envelope: $0.1\text{ – }0.8\text{ m/s}$; closing faster than $2.0\text{ m/s}$ triggers a `COLLISION WARNING` klaxon and scrapes hull armour if unbraked!).
- **RCS Thruster Controls**:
  - `W` / `S` / `A` / `D`: Translate Up / Down / Left / Right (RCS thrusters).
  - `Arrow Keys`: Pitch / Yaw attitude trim.
  - `Q` / `E`: Roll collar left / right.
  - `Space` / `Shift` (or `R` / `F`): Forward / Reverse RCS closure pulse.
  - `X`: Auto-kill lateral drift & rotational rates (RCS dampeners).
- **Reward & Integration**:
  - At ports: Successful manual clamp lock transitions the hull immediately to `Berth::Docked` and waives port tug fees.
  - Ship-to-ship / Derelicts: Hard-docking opens an airlock-to-airlock umbilical so crews or boarding parties can cross (`+ship/board <target>`) or transfer cargo/fuel/salvage!

---

## 7. Verbs & Controls Summary

### Line-Mode / Command Verbs (`+helm`, `+tac`, `+mine`)
- **`+helm`** — Status readout: 3D position, velocity, nearest body/port/field, attitude, throttle, $\Delta v$ reserve, heat, EMCON, and bubble contacts.
- **`+helm/undock`** — Cast off from a station berth or site pad directly into free flight (manual approach bubble).
- **`+helm/decouple`** — Drop out of a filed `CarrierJourney` autopilot into System Free Flight at the ship's current leg position and velocity.
- **`+helm/course <target> [throttle=N] [flip=on|off]`** — Set guidance toward a planet, moon, port, asteroid field, or contact.
- **`+helm/burn <throttle_permille> [yaw=<deg> pitch=<deg>]`** — Set manual thrust and attitude.
- **`+helm/flip`** / **`+helm/brake`** — Flip nose $180^\circ$ to retrograde / burn to zero relative velocity.
- **`+helm/match <contact|rock>`** — Match velocity vector with a ship, station, or asteroid.
- **`+helm/dock [<target>] [auto]`** — Enter the 6-DOF docking alignment camera (or request station tugs at a port with `auto`).
- **`+helm/emcon quiet|standard|active`** — Set sensor & radiator emission posture.
- **`+tac/lock <contact>`** / **`+tac/fire <slot|torpedo|laser|railgun>`** / **`+tac/pdc <offense|defense|auto>`** / **`+tac/ecm`** — Ship combat fire control and point-defense doctrine.
- **`+mine/ping`** / **`+mine/laser <rock>`** / **`+mine/drones <launch|recall>`** — Zero-G asteroid field prospecting and extraction.

### Interactive Cockpit / Holo-Tank / Docking Keys (SSH TUI & Web)
- **`v`**: Cycle view mode: **First-Person Bridge** $\leftrightarrow$ **3D Holo-Tank** $\leftrightarrow$ **6-DOF Docking Camera** (when within docking approach).
- **`w` / `s`**: Step main drive throttle up / down (in flight) or RCS vertical translation (in docking mode).
- **`a` / `d`**: Yaw left / right (in flight) or RCS lateral translation (in docking mode).
- **`q` / `e`**: Pitch or Roll trim.
- **`f`**: Flip $180^\circ$ to retrograde (`FLIP & BURN`).
- **`x`**: All stop / RCS dampeners.
- **`t` / `T`**: Cycle target contact / asteroid rock.
- **`1` / `2` / `3` / `g`**: Fire weapon group 1 (Torpedo), 2 (Lasers), 3 (Railgun), or volley (`g`).
- **`p`**: Toggle laser/PDC defensive intercept posture.
- **`[` / `]` / `+` / `-`**: Rotate & zoom the 3D Holo-Tank.

---

## 8. Implementation Cut Plan (`SF.0` – `SF.8`)

Each cut is self-contained, testable, and touches zero files in `arena/`, `field/`, `surface/`, or `detachment/`.

| Cut | Scope | Key Files | Acceptance Criteria |
|---|---|---|---|
| **SF.0** | Design doc & README index | `docs/design/35-ship-free-flight-combat-and-mining.md`, `docs/design/README.md` | Rulings R1–R4 and cut plan recorded. |
| **SF.1** | **Outer System Sky & Asteroid Clusters**: Add Uranus, Neptune, Triton, Galilean moons, and 4 orbital asteroid fields to the sky/lane data; add `Sky::velocity_mps` (orbital velocity vector by central difference) | `crates/tacsim/src/sky.rs`, `crates/tacsim/content/lane/ceres_phobos.json` (or sky table extension), `tests/sky_outer.rs` | `Sky::position_km` and `Sky::velocity_mps` track Uranus, Neptune, Triton, and asteroid clusters; all existing `tests/sky.rs` pins green. |
| **SF.2** | **System Free Flight & Autopilot Decouple (`+helm`)**: `SpaceFlightRecord`, 3D Newtonian integration on the 24:1 IC clock, `+helm/undock`, `+helm/decouple`, `+helm/course`, `+helm/burn`, `+helm/flip`, `+helm/brake`, and handoff back to port/autopilot | `crates/tacsim/src/space/mod.rs`, `flight.rs`, `commands.rs` | A pilot can undock into free flight or decouple mid-route, burn toward Neptune or an asteroid cluster, flip-and-burn to brake, and inspect `+helm`. |
| **SF.3** | **Local Space Bubble & Expanse Weapon Triad (`+tac`)**: Co-moving Newtonian bubble (`BubbleId`), 1 s pulse kinematics, Torpedo guidance/flight & PDC/Laser interception, heat-limited Lasers (Offense/Defense toggle), uninterceptable Railguns with TOF & jinking evasion, EMCON | `crates/tacsim/src/space/bubble.rs`, `weapons.rs`, `ship_combat_tests.rs` |Deterministic tests proving torpedo inheritance of launcher velocity, PDC/Laser interception, railgun TOF evasion by jinking, and section/radiator damage. |
| **SF.4** | **Zero-G Asteroid Field Mining (`+mine`)**: Procedural/seeded C/S/M-type drifting asteroids in field bubbles, `+mine/ping`, velocity-matching gate ($|\Delta \mathbf{v}| < 5\text{ m/s}$), mining laser thermal bloom, collector drone ore transfer into ship hold | `crates/tacsim/src/space/mining.rs`, `asteroid_mining_tests.rs` | A ship enters `kirkwood_gap` or `ceres_halo`, pings rocks, matches velocity, mines C/S/M ore into `stow.bulk`, and triggers IR bloom. |
| **SF.5** | **3D Holo-Tank Renderer (`mush-pixel::holotank`)**: 3D wireframe cylinder/sphere projection, elevation stalks, velocity/burn vectors, torpedo tracks with TTI countdowns, PDC/railgun envelopes, asteroid markers | `crates/mush-pixel/src/holotank.rs`, `space.holotank` feed | Renders deterministic 640×480 / 1280×960 frames for Kitty, Sixel, and WASM; unit & golden tests. |
| **SF.6** | **First-Person Space Bridge Cockpit & Planet Shaders (`mush-pixel::space_cockpit`)**: 3D starfield, raycast spherical planets/moons with atmospheric glow, cloud bands, Neptune's Great Dark Spot & rings, Saturn's rings, ship/asteroid sprites, prograde/retrograde & railgun lead HUD, Holo-Tank inset | `crates/mush-pixel/src/space_cockpit.rs`, `planet_shader.rs` | Flying to Neptune, Saturn, Mars, or Ceres renders the planet/moons/rings in 3D perspective from the Bridge canopy with full flight/combat HUD. |
| **SF.7** | **6-DOF Starfield-Style Docking Mini-Game (`mush-pixel::docking` + `tacsim::space::docking`)**: Docking state machine, RCS translation/attitude/roll physics, perspective docking collar camera, alignment bars & closure velocity tape, tug-fee waiver at ports, ship-to-ship & derelict hard-docking | `crates/tacsim/src/space/docking.rs`, `crates/mush-pixel/src/docking.rs` | Manual 6-DOF alignment clamps at `< 0.8 m/s` within lateral/angular tolerances; auto-dock preserved for offline queues; ship-to-ship boarding/transfer enabled on hard-dock. |
| **SF.8** | **SSH TUI & Web Client Integration**: Bind `space.cockpit`, `space.holotank`, and `space.docking` feeds + keyboard controls (`v`, `WASD`, `QE`, `f`, `x`, `t`, `1-3`) in `mush-net-ssh`, `mush-pixel-wasm`, and `web/src/components/` | `crates/mush-net-ssh/src/tui/`, `crates/mush-pixel-wasm/src/lib.rs`, `web/src/components/` | End-to-end playable in both w11term/WezTerm (Kitty/Sixel) and Web browser! |
