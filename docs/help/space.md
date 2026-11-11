# Space Flight, Tactical Combat, and Asteroid Mining

While aboard a carrier hull, bridge crew operate the vessel through three station-gated command families:

- **`+helm`** — bridge stations, standing orders, attitude, propulsion, flight guidance, docking, and towing.
- **`+tac`** — passive and active infrared sensors, fire-control locks, the space ordnance triad, and point defence.
- **`+mine`** — zero-G orbital cluster prospecting, Freeport Registry claims, mining lasers, and collector drones.

All three commands require you to be physically aboard a carrier hull and seated at a bridge station (`command`, `helm`, `tactical`, `ew`, `engineering`, or `flight_ops`) that grants the corresponding control channel (`movement`, `sensors`, or `fire`; a captain or crew member at `command` controls all three). On the graphical terminal and web client, press `v` on the chart screen to cycle between the bridge holotank (`space.holotank`), the pilot canopy (`space.cockpit`), and the docking approach view (`space.docking`).

---

## Bridge Stations and Flight Controls (`+helm`)

Assign or claim a bridge station before issuing flight or tactical orders:

```text
+helm/status
+helm/station <seat>[=<crew>]
```

- `+helm/status` (or bare `+helm`) shows bridge station assignments, standing orders, attitude, velocity, propellant reserves, and active bubble or free-flight state.
- `+helm/station <seat>[=<crew>]` assigns an aboard crew member (or yourself when `=<crew>` is omitted) to a bridge station (`command|helm|tactical|ew|engineering|flight_ops`), or `off` to vacate.

### Attitude, Thrust, and Flight Guidance

 Commands on the `movement` channel update the ship's continuous standing orders and maneuver state:

```text
+helm/heading <yaw_deg>[,<pitch_deg>]
+helm/roll <roll_deg>
+helm/thrust <permille>[,<x>,<y>,<z>] [force]
+helm/burn <ax>,<ay>,<az> [<s_ic>] [force]
+helm/guidance <attitude_deg|mode>
+helm/decouple
+helm/emcon <ghost|passive|standard|combat>
```

- `+helm/heading <yaw_deg>[,<pitch_deg>]` sets nose attitude in degrees (`yaw_deg` `0..360`, optional `pitch_deg` `-90..90`) and writes `StandingOrder::Heading`.
- `+helm/roll <roll_deg>` sets bank angle in degrees (`-180..180`) to rotate fresh armour facets toward incoming fire.
- `+helm/thrust <permille>[,<x>,<y>,<z>] [force]` sets main-drive throttle in permille (`0..1000`), with an optional 3D direction vector and `force` to override the return-reserve warning, and writes `StandingOrder::Thrust`.
- `+helm/burn <ax>,<ay>,<az> [<s_ic>] [force]` executes a timed reaction-mass burn in m/s² (`-100..100` per axis; a burn asked above the drive's rating for the ship's mass flies at that rating, which is under 1 m/s² for a laden Dray, and a derelict on your tow line counts as mass) for `<s_ic>` IC seconds (`1..86400`, default `60`) against `stow.propellant`. Pass `force` to burn into the return reserve.
- `+helm/guidance <attitude_deg|mode>` sets a guidance yaw (`0..359`) or flight-computer mode (`manual`, `prograde`, `retrograde`, `radial_in`, `radial_out`, `normal`, `anti_normal`, `intercept`, `match`).
- `+helm/decouple` decouples the ship from a corridor transit leg into 3D heliocentric free flight.
- `+helm/emcon <ghost|passive|standard|combat>` sets emissions posture (`silent`, `nominal`, and `full` are accepted aliases) and writes `StandingOrder::Posture`.

### Navigation, Customs Pickets, Docking, and Towing

```text
+helm/autopilot <port|cluster_id>
+helm/approach <rock|derelict>
+helm/match <rock>
+helm/submit
+helm/run
+helm/dock [<port|derelict> | tape <session_id> <rle> | abort]
+helm/tow [<derelict_id> | release]
```

- `+helm/autopilot <port|cluster_id>` plans and flies a passage from live `(pos, vel)` to an open port, an orbital asteroid cluster (`ceres_halo`, `kirkwood_gap_3_1`, `pallas_inclined_belt`, `saturn_b_ring_edge`), or an outer-system body (`ceres`, `jupiter`, `saturn`, `titan`, `enceladus`, `uranus`, `neptune`, `triton`).
- `+helm/approach <rock|derelict>` executes a braking approach toward asteroid pocket `A`, `B`, or `C`, or toward a disabled derelict in the current bubble.
- `+helm/match <rock>` locks an RCS velocity match (`|Δv| < 5 m/s` within `2,000 m`) with rock `A`, `B`, or `C`.
- `+helm/submit` heaves to and submits the hull's cargo manifest to a customs picket inspection; `+helm/run` refuses inspection and runs the picket.
- Customs answers in two steps. `+helm/submit` tells you what the corvette found; the ruling itself (clearance, a fine, or the unlicensed ore taken from your hold) follows a moment later, once it is on record, and you are not cleared until it does. `+helm/run` is logged the same way, and the corvette turns on you when the run is on record. While a ruling is on its way, a second `+helm/submit` or `+helm/run` is refused. If a ruling cannot be recorded you are told, nothing is settled, and you may submit again. If a fine cannot be collected you are not cleared; submit again.
- `+helm/dock [<port|derelict> | tape <session_id> <rle> | abort]` initiates a 60 fps 6-DOF docking approach (`space.docking`) against an open port at leg end or a disabled derelict within `500 m`, submits a recorded `<rle>` input tape for `<session_id>`, or aborts an open docking session.
- `+helm/tow [<derelict_id>]` attaches a salvage tow line to a derelict after a verified hard-dock clamp, and files your salvage claim on her when someone answers for her title. A clamp with no tow line lets go after a minute.
- `+helm/tow release` lets the derelict go. If she has not been brought into a port, your custody of her ends and the salvage claim on her is withdrawn; a claim is only ever awarded on a derelict a port has.
- An open docking session cuts your own main drive and leaves you coasting; every other ship in the bubble keeps flying. `+helm/dock` is refused while a hostile contact is operational in the bubble, whether or not your sensors hold it.
- A passage's reaction mass and provisions are drawn from the hold when it is engaged. `+helm/autopilot` is refused while an earlier action of the captain's is still committing; type it again. If the draw cannot be made, the drive is cut and the ship drifts where she is.
- If every crew member aboard loses their link, the drive is cut for a two-minute grace. Past it, a ship in free flight or at a cluster flies the autopilot's own passage to the nearest open port she can afford, paid from her tank and locker like any other; a ship that can afford none drifts where she is.

---

## Tactical Sensors and Combat (`+tac`)

Tactical commands require a bridge station with the `sensors` channel (`+tac/status`, `+tac/scan`, `+tac/lock`) or the `fire` channel (`+tac/beam`, `+tac/torp`, `+tac/railgun`, `+tac/pdc`).

### Sensors and Target Locks

Vacuum detection resolves through light-lagged infrared propagation, comparing target thermal signature, drive plume bloom, and emissions posture against inverse-square range and observer sensor aperture. Undetected contacts do not appear on scans or bridge displays and cannot be locked.

```text
+tac/status
+tac/scan
+tac/lock <contact|off>
```

- `+tac/status` (or bare `+tac`) shows tactical mounts, active locks, and sensor status.
- `+tac/scan` runs a passive/active infrared sensor pass with light-lag across the active space bubble.
- `+tac/lock <contact|off>` acquires or clears a fire-control lock on a detected contact.
- `+tac/hail <contact|port>[=<message>]` sends a radio hail from any bridge station. In a bubble it goes to a contact on your scope; from free flight or a cluster it goes to an open port. The range is measured, and a hail to a distant port arrives after the light has crossed (minutes between Ceres and Mars). At most four of your hails are in flight at once.

### Space Ordnance Triad and Point Defence

Space engagements combine three offensive mount classes with distinct flight profiles and counterplay:

```text
+tac/beam [<contact>]
+tac/torp [<contact>[=<count>]]
+tac/railgun [<contact>]
+tac/pdc <auto|manual|point|offense>
```

- `+tac/beam [<contact>]` fires offensive laser mounts at a detected hostile contact (speed-of-light hit within mount range, up to `160 km` on a corvette laser; draws capacitor charge and builds hull heat).
- `+tac/torp [<contact>[=<count>]]` launches a torpedo salvo (default `1`) at a detected hostile contact (`10–500 km` envelope, accelerating under 3D proportional navigation before ballistic coast).
- `+tac/railgun [<contact>]` fires spinal or turret railguns at a detected hostile contact along a world-frame lead solution (`10–250 km` envelope; countered by lateral jinking thrust during time-of-flight).
- `+tac/pdc <auto|manual|point|offense>` configures point-defence autocannons or dual-mode laser mounts to intercept incoming torpedoes automatically, stand in manual/point mode, or switch dual-mode mounts to offensive fire.

When fire disables a hull's drive or structure, the vessel remains in place as an open-epoch derelict with its flight recorder and cargo intact for salvage and towing.

---

## Asteroid Mining (`+mine`)

When your carrier arrives at an orbital resource cluster, use `+mine` to prospect asteroid pockets, stake or contest Freeport Registry claims, and extract ore into `stow.ore`.

```text
+mine/status
+mine/ping [<rock>]
+mine/stake <rock> [<family>]
+mine/contest <rock>
+mine/laser <rock> [<family>]
+mine/drones <rock> [<1..4>] [<family>]
```

- `+mine/status` (or bare `+mine`) shows the active orbital cluster, velocity-matched rock, extraction cycles, and `stow.ore` hold usage.
- `+mine/ping [<rock>]` pings asteroid pockets (`A`, `B`, `C`) for spectral class (`C-type`, `S-type`, `M-type`), ore reserves (`metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`), and claim tenure.
- `+mine/stake <rock> [<family>]` stakes a 168-day Freeport Registry claim on an open pinged asteroid block (requires a live Registry prospecting licence).
- `+mine/contest <rock>` contests an existing player claim within the 30-day Registry contest window.
- `+mine/laser <rock> [<family>]` starts a timed 60 s mining-laser extraction cycle on rock `A`, `B`, or `C` into the hull's `stow.ore` hold.
- `+mine/drones <rock> [<1..4>] [<family>]` launches up to 4 collector-drone sorties (`ExcursionId`) against rock `A`, `B`, or `C` into `stow.ore`.
