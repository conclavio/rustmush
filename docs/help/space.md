# Space Flight, Tactical Combat, and Asteroid Mining

While aboard a carrier hull, bridge crew operate the vessel through three station-gated command families:

- **`+helm`** — flight controls, attitude, propulsion, navigation computers, docking, and towing.
- **`+tac`** — passive and active sensors, target locks, the space weapon triad, and point defence.
- **`+mine`** — zero-G orbital cluster prospecting, claim beacons, mining lasers, and collector drones.

All three commands require you to be physically aboard a carrier hull and seated at a bridge station that grants the corresponding control channel (`helm`, `nav`, `sensors`, `fire`, `eng`, or `command`). On the graphical terminal and web client, press `v` on the chart screen to cycle between the bridge holotank (`space.holotank`), the pilot canopy (`space.cockpit`), and the docking approach view (`space.docking`).

---

## Bridge Stations and Flight Controls (`+helm`)

Seat yourself at a bridge console before issuing flight or tactical commands:

```text
+helm/station <station>
+helm/status
```

- `+helm/station <station>` claims a bridge console on your current hull (for example `pilot`, `tactical`, or `command`).
- `+helm/status` (or bare `+helm`) displays hull attitude, velocity, propulsion state, fuel reserves, active local bubble status, and the bridge visual frame.

### Attitude, Thrust, and Flight Computers

When a journey enters a local space bubble, your carrier transitions from long-range transit to 1 Hz 3D Newtonian flight:

```text
+helm/heading <yaw> <pitch>
+helm/roll <deg>
+helm/thrust <0..100>
+helm/burn <0..100>
+helm/guidance <grade>
+helm/decouple <on|off>
+helm/emcon <mode>
```

- `+helm/heading <yaw> <pitch>` commands nose attitude in degrees (`yaw` `0..360`, `pitch` `-90..+90`). Turn rate scales with hull moment of inertia and reaction control propellant availability.
- `+helm/roll <deg>` sets bank angle (`-180..+180` degrees).
- `+helm/thrust <0..100>` and `+helm/burn <0..100>` set main drive throttle percentage and commit propellant consumption for the burn.
- `+helm/guidance <grade>` selects flight-computer assistance (`manual`, `assisted`, or `autonomous`), up to the hull's installed guidance tier.
- `+helm/decouple <on|off>` toggles decoupled translation so the nose can slew independently of your velocity vector.
- `+helm/emcon <mode>` sets emissions control posture (`silent`, `nominal`, or `full`), trading sensor signature against active subsystem output.

### Navigation, Evasion, Docking, and Towing

```text
+helm/autopilot <destination>
+helm/approach <contact>
+helm/match <contact>
+helm/submit
+helm/run
+helm/dock
+helm/tow <contact|off>
```

- `+helm/autopilot <destination>` plots a course to an orbital cluster (`belt_main`, `trojan_l4`, `saturn_b_ring`) or outer body (`ceres`, `jupiter`, `saturn`, `titan`, `enceladus`, `uranus`, `neptune`, `triton`), checking delta-v and life-support endurance before departure.
- `+helm/approach <contact>` and `+helm/match <contact>` slave the flight computer to close range or match velocity with a detected bubble contact.
- `+helm/submit` throttles down and yields to a customs picket inspection during an interdiction; `+helm/run` triggers an emergency breakaway burn.
- `+helm/dock` opens an interactive 60 Hz six-degree-of-freedom docking approach (`space.docking`) once inside port approach range. Align lateral offset, closure rate, and roll inside the port corridor before the session timer expires.
- `+helm/tow <contact|off>` latches or releases a magnetic tow line on a neutralized derelict hull within 250 m and 5 m/s relative speed, adding the towed mass to your carrier's inertia and burn costs.

---

## Tactical Sensors and Combat (`+tac`)

Tactical commands require a bridge station with the `sensors` channel (`+tac/status`, `+tac/scan`, `+tac/lock`) or the `fire` channel (`+tac/beam`, `+tac/torp`, `+tac/railgun`, `+tac/pdc`).

### Sensors and Target Locks

Vacuum detection scales with target thermal signature, main-drive plume bloom, emissions posture, observer sensor aperture, and inverse-square range. Undetected contacts do not appear on scans or bridge displays and cannot be locked.

```text
+tac/status
+tac/scan
+tac/lock <contact|off>
```

- `+tac/status` (or bare `+tac`) displays current sensor locks, weapon mounts, capacitor charge, magazine stores, and point-defence posture.
- `+tac/scan` sweeps the active space bubble and lists all detected contacts with bearing, range, relative closure speed, and classification.
- `+tac/lock <contact|off>` acquires or drops a fire-control lock on a detected hull or asteroid contact.

### Space Weapon Triad and Point Defence

Space engagements combine three weapon classes with distinct range and counterplay profiles:

```text
+tac/beam [contact]
+tac/torp [contact]
+tac/railgun [contact]
+tac/pdc <auto|conserve|off>
```

- `+tac/beam [contact]` fires directed-energy lasers. Beams resolve at speed of light inside short range (`<= 1,500 m`), draw from the weapon capacitor, and build hull thermal load.
- `+tac/torp [contact]` launches a guided torpedo from the ordnance magazine. Torpedoes track across long range (`> 5,000 m`) with an intercept countdown, giving the defender time to maneuver or intercept with point defence.
- `+tac/railgun [contact]` fires a hypervelocity kinetic slug along a lead solution at medium range (`1,500..5,000 m`), checking target lateral acceleration and time-to-intercept.
- `+tac/pdc <auto|conserve|off>` configures point-defence cannons to engage incoming torpedoes automatically, conserve ammunition for close threats, or stand down.

When weapons fire cripples a hostile carrier's reactor, drive, or hull structure, the vessel is neutralized in place as a derelict rather than vanishing, preserving its black-box flight recorder and cargo for boarding, salvage, or towing.

---

## Asteroid Mining (`+mine`)

When your carrier arrives at an orbital resource cluster, use `+mine` to prospect asteroids, register or contest claims, fracture rock with mining lasers, and sweep ore fragments into the cargo hold.

```text
+mine/status
+mine/ping
+mine/stake <rock>
+mine/contest <rock>
+mine/laser <rock> [power]
+mine/drones <launch|recall> [rock]
```

- `+mine/status` (or bare `+mine`) summarizes your active orbital cluster, registered claims, drone wings, laser heat, and refined ore stowed aboard.
- `+mine/ping` triggers an active spectrographic pulse across the cluster, revealing asteroid composition (`C-type` carbonaceous/volatiles, `S-type` silicate/industrial metals, `M-type` heavy/precious metals) and richness while temporarily raising your hull's electromagnetic signature.
- `+mine/stake <rock>` deploys a cryptographic claim beacon on an unclaimed asteroid in the cluster.
- `+mine/contest <rock>` challenges an existing claim beacon, broadcasting a piracy alert to the cluster picket roster.
- `+mine/laser <rock> [power]` fires the hull mining laser at a target asteroid (`1..100` power). Staying inside the rock's fracture band cleaves harvestable ore fragments into space; overheating past the thermal threshold triggers a volatile blowout that damages nearby equipment and cuts yield.
- `+mine/drones <launch|recall> [rock]` launches collector drones to haul cleaved ore fragments into your carrier's hold and refine them into resource units, or recalls your drone wing before departure.
