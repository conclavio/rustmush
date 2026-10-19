# 19. The astronomy engine — a moving sky in three dimensions

**Draft 0.1 — 2026-10-18.** The post-slice cut doc 18 §4.8 names **P-A**, written as its own document
because it supersedes a ruling (doc 10 §22 T17, the frozen sky, as amended 2026-10-15) and reaches
every surface that reads a distance. The owner's direction (2026-10-15, re-stated 2026-10-18): the
frozen sky "wasn't as fun as a dynamic world would be"; belter-charts
(github.com/skylercole/belter-charts, belter-charts.pages.dev) is the reference — real ephemerides, a
3D heliocentric map, canon stations, an Epstein flip-and-burn planner with travel time, flip point,
peak velocity and light lag. What the owner wants from it, in their words: "an asteroid like Pallas
being so off-ecliptic means there are optimal times to trade there and times when it's prohibitively
expensive to reach it"; "an emergency on the station while they're at max-inclination would have
bonuses associated with it"; "a player can't just decide an arbitrary 3-station hop and go trade
between them like it's Trade Wars 2002 — there are times when any given rotation is optimal and times
when it's not worth the fuel costs to get there."

So the engine has to move three things with the sky: **time** (a passage takes longer in a bad
season), **fuel** (a passage costs more reaction mass in a bad season — which the slice's coast law,
a flat 105‰ of departure mass, cannot do), and **price** (a port that is awkward to reach pays more
for what it needs). Doc 18 R9 kept the slice from foreclosing any of it: one sky query, one planner
over the sky at departure, legs as body pairs, the ticket freezing the plan, the `Sky` enum and the
tagged `DriveLaw`. This document fills those seams.

Decisions are **D-19-n**; the owner's open questions **Q-19-n** (§10), each with the default the
build takes. Doc 18's Q-18-35 to Q-18-41 were left open "for P-A"; §2 takes each one's recorded
default as a lead ruling, and the owner overrides by data, not by a rewrite.

---

## 1. What stands, what is superseded

**Stands** (doc 10 §22 as amended): 1A's scales and compression; 1A's rule that transfer conditions
modify routes and never close a destination (no launch windows; every port reachable at a cost);
T1's freeze — a passage's per-leg distances, durations, burn and crew-days are computed once at
commitment and frozen into the ticket and the journey, nothing after the burn re-reads the sky;
T17's coherence — every pair sharing a body changes together, kept by the one query; the slice's
seams (`Sky::distance_km`, `Sky::position_km`, `Sky::revision`, `plan_passage`, `choose_route`,
`passage_burn`, `DriveLaw`, `PassagePlan` frozen whole in the ticket); mush-sim stays sky-free.

**Superseded:** "No body moves"; the circular, coplanar model as the final model (it stays the
e = i = 0 case and every fixture pack is still one); the coast law as the Dray's law (§4; Q-18-35);
Kirkwood's placeholder orbit (§3.6; Q-18-36); `rate = 0` as the shipped configuration (§7;
Q-18-37); the readiness line's "constant" wording; `@corridor/show`'s "constant sky at epoch".

---

## 2. The rulings taken from doc 18 §8 (owner may override, each by data)

| Question | Default taken (lead ruling) | Where it lands |
|---|---|---|
| **Q-18-35** R3 against R9 — the daily passage under a moving sky | The Dray's drive becomes a **flip law** (constant-thrust accelerate / flip / decelerate), tuned so a brimmed hull's worst season is about a day and its best about half (§4.3) | cut A.2, `dray.json` v3 |
| **Q-18-36** Kirkwood's orbit | **Ceres's own elements, trailing by 2° of mean anomaly** (about 0.097 AU; the brief said "a few degrees" — two keeps the hop under four hours at the Dray's thrust) | cut A.1, lane v4 |
| **Q-18-37** the sky rate under world rate 24 | **1000, real motion**: the sky's date is the IC date, which real data requires; the knob stays a permille so a slower sky remains one line of config | cut A.6 (templates); tests keep 0 |
| **Q-18-38** the engine's data | **Element tables** per body (osculating Keplerian elements from Horizons at semi-annual epochs over 2340–2400 plus the lane's epoch, propagated from the bracketing rows and blended), asserted against a committed golden table of Horizons positions every 91 days; packed daily vectors are not built | cut A.1 |
| **Q-18-39** the price policy's reach | A capped once-per-IC-day **reach premium** on demand-side bids plus **emergency Wanted bids** for life-support goods when reach crosses a threshold, funded from finite capital | cut A.4 |
| **Q-18-40** solar keep-out | **A cost, never a closure**: a leg whose chord passes inside 0.3 AU of the Sun is planned along the shortest path around that disc | cut A.2 |
| **Q-18-41** coast under the rocket equation | Moot for the Dray (it no longer coasts); the Coast law keeps the slice's doctrine burn for any hull that still flies it | — |

---

## 3. The model (cut A.1)

### 3.1 One sky, two kinds of body — D-19-1

`Sky` gains a second variant, **`Sky::Kepler`**, chosen by `Sky::from_pack` whenever any `body` entry
of the pack carries an `elements` block. Inside it a body is either **Keplerian** (it has elements) or
**circular** (it has only the slice's four numbers); a circular body inside a Kepler sky keeps the
slice's integer arithmetic to the bit (`Ephemeris::phase_mdeg_at`, `separation_km` for a circular
pair), so a pack without elements is `Sky::Circular`, unchanged, digest-stable, and every existing
fixture test holds. The model word is `kepler`. Nothing outside `tacsim::sky` learns which variant it
has (D-18-43).

### 3.2 A body's element table — D-19-2

`BodyDef` keeps `orbit_au` (the heliocentric **distance** at `epoch_ic`), `period_days` and
`phase_deg_at_epoch` (the **true longitude** at `epoch_ic`) and gains an optional `elements` table,
skipped on the wire when empty — one row of osculating Keplerian elements per epoch:

```
"elements": [
  [epoch_ic, a_au, e, i_deg, node_deg, peri_deg, mean_anomaly_deg, mean_motion_deg_day],
  [73794240000, 2.767171314, 0.077508988, 10.532434, 75.530482, 82.026578, 308.827624, 0.214116435],
  …                       // January 1 and July 1 of every year 2340–2400, plus the lane's epoch: 122 rows
]
```

Heliocentric ecliptic J2000; `peri_deg` is the **argument** of perihelion ω (the rotation formula's
own angle), `node_deg` the longitude of the ascending node Ω. Held as integers of fixed scale
(`kepler::ElementRow`) so the pack stays `Eq` and the digest is over integers: nano-AU, nano (10⁻⁹)
for `e`, micro-degrees for angles, nano-degrees per day for the mean motion; the authored decimals
round-trip exactly. **At an instant between two rows** each row's two-body propagation is taken
(the mean anomaly advanced by that row's mean motion, Kepler's equation, the rotation) and the two
positions are **blended linearly in time**; at a row the position is the row's, exact; outside the
table the nearest end row propagates alone. The wire refuses a non-finite value, `e` outside
`[0, 0.99]`, an inclination outside `[0°, 180°]`, a non-positive mean motion, rows out of order, a
table on a body without `epoch_ic`, and — **the consistency rule** — a table whose position at
`epoch_ic` disagrees with the authored `orbit_au` by more than 0.5 % or with `phase_deg_at_epoch` by
more than 0.5°, so a pack cannot carry two skies that contradict each other; `period_days` must be
`360 / mean_motion` of the nearest row within 1 %.

**Why a table and not one element set.** Measured against Horizons every 91 days over 2340–2400
(`fetch_elements.py`, 2026-10-18): a single osculating set anchored at the epoch drifts 1.6–4.6 %
for the asteroids (Jupiter's periodic pull is baked into the anchor; Hygiea, near the 2:1 gap, is
the worst) and a single **fitted** set — intercepts and secular rates by least squares over the
window — still 0.6–5.6 %, because the perturbations are periodic, not secular. The table of
semi-annual osculating rows with the blend holds **every body within 0.07 ‰** (Earth within 1.1 ‰,
the Moon's wobble on its osculating elements) — two hundred times better, at 122 rows × 8 numbers
per body (about 12 KB each, 150 KB for the lane). Yearly rows would hold 0.25 ‰ (Earth 7.6 ‰);
semi-annual was chosen so the one tolerance covers the planets too. A later window is more rows
(Q-19-8).

### 3.3 The calendar — D-19-3

The game clock is `mush_core::CivilDate` (365-day years, year 0 at IC 0; doc 18 lead ruling R3).
A real date maps to the game calendar **by its label**: the sky a player sees on the game date
2351-05-13 is the real sky of 2351-05-13. So the engine propagates over **real days**: an IC
instant's `CivilDate` → the Julian Day of that Y-M-D label (the standard Gregorian formula; the game
calendar has no February 29, so every label is a valid Gregorian date) plus the fraction of the day;
`Δt = JD(t) − JD(epoch_ic)` in days. Across a real leap day two consecutive game days are two real
days apart and the sky moves twice as far — one day in four years, invisible, and faithful to the
label. The slice's `rate` keeps its meaning: the elapsed IC interval is scaled by `rate / 1000`
before the calendar is consulted, so `rate = 0` holds every Keplerian body at its epoch position in
three dimensions (the constant sky, now with latitude) and `1000` is real motion.

### 3.4 Positions and distances — D-19-4

Kepler's equation is solved in `f64` with a fixed sixteen Newton steps (start `E = M`, or `π` for
`e ≥ 0.8`), the orbital-plane position rotated by ω, i, Ω into the ecliptic frame, scaled by
`AU_KM`, blended between the bracketing rows and **rounded to the kilometre** into
`PosKm { x, y, z }` (`i64`). The distance between two positions of which either is Keplerian is
`isqrt` over the `u128` sum of squares of the integer differences (`PosKm::distance_km`) —
integer, deterministic, no second rounding path; a pair of circular bodies keeps `separation_km`
bit for bit. Platform drift in `sin`/`cos` can move a position by a kilometre in the last place;
the circular model has had the same exposure since S7 and committed plans are frozen, so it is
accepted and noted. Outside the table (a test world at IC year 60, a campaign past 2400) the end
row propagates alone — a sane two-body sky, never a refusal — and the readiness report is the
place to say the window has run out (A.6).

### 3.5 The revision — D-19-5

`Sky::revision` under `tacsim.sky.v1` covers the model word, the configured epoch and rate, every
body's four slice numbers and, for a Keplerian body, its twelve element integers. A plan carries the
revision (D-18-46); a plan made under the circular lane and one made under lane v4 are never read as
the same sky.

### 3.6 The bodies — D-19-6

Lane v4 carries **mercury, venus, earth, mars, jupiter, saturn, ceres, pallas, juno, vesta, hygiea**
from Horizons at the lane's epoch 2351-05-13 (the date dev1's fresh store opened on, 2026-10-19), and
**kirkwood** on Ceres's elements trailing by 2° of mean anomaly (Q-18-36's default: the
Ceres–Kirkwood leg is a constant short hop, Kirkwood–Mars the seasonal chord, every Ceres-bound ship
passes it, and the depot cannot drift off the lane in a lifetime, which an independent 2.06 AU orbit
would do in a decade). Moons are not bodies yet: Stickney keeps naming `mars` (Phobos's 9,376 km is
nothing a passage reads), Luna and Titan wait for a port. The circular numbers of ceres, mars and
pallas are **unchanged** from lane v3 — the Kepler set is exact at the epoch, so the engine's sky and
the slice's sky agree there in longitude and distance; they differ by the **ecliptic latitude** the
slice flattened (Ceres at i ≈ 10.6°, Pallas at 34.8°), which moves the pinned chords and the legs'
seconds once, measured in the cut note.

### 3.7 The data pipeline — D-19-7

`crates/tacsim/content/sky/fetch_elements.py` (stdlib only, prints one JSON object, writes the golden
table only when asked, caches the raw Horizons rows under `--cache`) is the committed source of
every element table and of `crates/tacsim/content/sky/golden.json` (eleven bodies × 241 Horizons
positions, km, every 91 days over 2340-01-01..2400-01-01, each row keyed by the game calendar's IC
second of its label). Horizons answers at most about eighty `TLIST` epochs per query, so the script
asks in chunks of forty; the gas giants are fetched by barycentre (`5`, `6`), since the planet-centre
records end at 2200. The test `a1_sky_kepler_tracks_horizons` propagates every golden row through
`Sky::position_km` and asserts the error against the per-body tolerance the script measured
(recorded in the test in hundredths of a permille, so a looser table fails the cut). No test runs
the script; nothing in the build reads it. `fetch_horizons.py` (the slice's date picker) stays as it
is.

---

## 4. The laws (cut A.2)

### 4.1 The flip law — D-19-8

```rust
pub enum DriveLaw {
    Coast { kms: u32, burn_permille: u32 },                 // the slice's, unchanged
    Flip  { accel_mg: u32, exhaust_kms: u32 },              // A.2
}
```

`accel_mg` is the hull's acceleration in milli-g **at its rated maximum departure mass**
(`dry_mass_t + payload_ceiling_kg`), so it is the acceleration every legal load is guaranteed; the
drive is thrust-limited, and a lighter hull is faster: `a = accel_mg · g₀ · (dry + ceiling) / (dry +
payload)`, with the payload the planner already sums (cargo, reaction mass, stores, occupants; lead
ruling L12). Per leg, the brachistochrone: `t = 2√(d / a)`, flip at `t / 2`, peak velocity `√(d·a)`,
`Δv = a·t`. The burn is the rocket equation over the whole passage, `m₀ · (1 − e^(−ΣΔv / vₑ))`, in
**whole tonnes, rounded up** — the one `f64` step in the planner, which the frozen ticket makes
harmless. A Flip leg does **not** pass through mush-sim's laden step: its load-dependence is the
physics above (mush-sim stays sky-free and the Coast law keeps D-18-44's step).

Fuel now moves with the sky: `Δv ∝ √d`, so the worst season costs about 1.8× the best season's
reaction mass as well as 1.8× its hours, and a brimmed hull at the worst season needs most of its
tank — the owner's "not worth the fuel costs" is a number a captain can read on `+ship/plan`.

### 4.2 The throttle — D-19-9

A passage may be flown below the hull's thrust: `HullDrive.throttle_permille` (`250..=1000`, default
`1000`) scales `a`. Since `t ∝ 1/√a` and `Δv ∝ √a`, half thrust is 41 % longer and 29 % cheaper.
`+ship/plan throttle=<permille>` shows it; the depart order's `throttle=` files it; the ticket's plan
records the law word `flip` and the legs it was computed with. The floor keeps a passage from
becoming a week by accident (Q-19-4).

### 4.3 The Dray — D-19-10 (data; Q-19-1)

`tacsim.unit.dray` v3: `"law": { "flip": { "accel_mg": 50, "exhaust_kms": 6000 } }` — a twentieth of
a gravity at 421 t, exhaust at 2 % of c: doc 11 §7's "slow, efficient" against the Vigil's torch.
**As built (A.2), at the epoch sky:** the story's outbound load (100 t of cargo, 53 t in the tank,
19 crew-days, one soul: 153,176 kg) flies Ceres–Kirkwood–Stickney in 321,547 + 1,256,313 s (18.3 h
at rate 24) for 51–52 t of reaction mass and 19 crew-days — the 2° hop to Kirkwood is 3.7 days of
flight, since a brachistochrone's short legs are slow; the return at 106,176 kg in 1,195,660 +
306,023 s (17.4 h), 48 t, 18 crew-days. Under the coast law the same loads took 20.8 h / 41 t / 22
and 18.5 h / 37 t / 20: the flip Dray is faster and thirstier, and the tank the slice bunkered
(42 t) no longer covers an outbound, so the story bunkers 53. At the best season (Ceres–Mars ≈
1.24 AU) a brimmed hull runs about 14 h and 40 t; at the worst (≈ 4.3 AU, the chord inside the
keep-out) about 30 h and over 70 t against an 80 t tank. Lighter loads are faster and cheaper on
both axes. Every doc 18 figure that was the coast law's output is re-stated "as built (A.2)" and
none is an invariant (D-18-50).

### 4.4 The intercept — D-19-11

The fixed-point intercept stays (`intercept_passes`): the chord from the origin body at `t0` to the
target at `t1`, the law's seconds, `t1 := t0 + secs`, repeated. Under a flip law the contraction is
weaker than under coast (seconds grow with √d), so the planner takes **five** passes
(`INTERCEPT_PASSES`), and the test that showed the contraction shows passes four and five within
60 s on every lane leg at the epoch and at the worst season.

### 4.5 The solar keep-out — D-19-12

A leg's chord is tested against a disc of `KEEP_OUT_AU = 0.3` about the Sun in the plane of the two
endpoints: if the segment passes inside it, the leg's distance is the shortest path around the disc —
tangent, arc, tangent: `√(|A|² − R²) + √(|B|² − R²) + R·θ`, closed form, in `f64`, rounded to the
kilometre — and the plan's leg carries that distance. A cost, never a refusal (Q-18-40): no route
is ever closed by the Sun, so no validator rule about "one non-refusable route" is needed.

### 4.6 Choosing a route — D-19-13

`choose_route` takes a **policy**: `Fastest` (least `total_s`, the default and the queue's) or
`Cheapest` (least `burn_t`, ties to the faster). `via=<place>` is honoured as a filter over the
listed routes (those with a leg ending at that place); `ViaReserved` retires. Lane v5 adds the direct
routes (`ceres_stickney_direct`, `stickney_ceres_direct`) to the lane; the **ports' lists stay
via-Kirkwood only** (the Co-op's charter routes its hulls by the depot; the pickets and customs of P4
are what make that a rule rather than a preference) so the queue's behaviour is unchanged and
`+chart` can show the captain what the direct chord would have cost (Q-19-3).

---

## 5. The surfaces (cut A.3)

- **`+chart`** — the sky from where the hull stands: every body's heliocentric distance, longitude
  and latitude, its distance from here in AU and light-minutes, and for each port the passage now
  (this hull, this load) and empty. **`+chart <port>`** — the best departure in the next IC year: a
  scan of `plan_passage` at one-IC-day steps (≤ 365 evaluations, pure), the fastest and the cheapest
  day, and the direct chord beside the listed route. **`+chart/body <id>`** — one body's elements
  and where it is. Text first, at the owner's 120-column floor; the web and TUI panel (top-down XY
  with a z-exaggeration toggle, ports, ships under way interpolated along their tickets' frozen legs)
  is cut A.3b on the one OutputEvent/Panel protocol.
- **`+ship/plan`** gains a window line: how this departure compares to the best in the next thirty
  IC days ("+3 h and +4 t against 05-21").
- **`@corridor/show`** says "the sky at epoch <date>" and gives each leg's distance at the epoch in
  three dimensions; the readiness Sky line reads "sky: moving (rate 1000, real motion), kepler,
  epoch <date>, rev <digest>" or "constant (rate 0)" with the model word.

---

## 6. Prices (cut A.4) — D-19-14 (Q-19-5)

Once per IC day the `tacsim.port` tick's price arm computes **reach(port, good)** = the least
nominal-load passage seconds from the ports that supply the good, under the lane's hull class at
`throttle 1000`, at now; against each port's **synodic median** (the median of reach over the next
synodic period, scanned once per boot and cached per `sky_rev`) a demand-side bid takes a premium of
`min(cap, k · (reach / median − 1))` permille, re-written in place with the applier's authority and
the org's officer of record as signatory (11 §2: nothing automated signs; the schema refuses an
unsigned listing). When reach crosses `emergency_ratio` the port posts **emergency Wanted bids** for
its life-support goods (provisions, volatiles, propellant) at the premium's cap, funded from the
org's finite capital (doc 10 7A; nothing minted; a house that cannot fund a tonne posts nothing and
pages once, as P-B does). Quotes stay frozen; the programme's reference reads the moved ask
(D-18-24). The caps and the ratio are lane-pack data on the port entry.

**As built (cut A.4, 2026-10-18; `crates/tacsim/src/port/price.rs`).** The premium is read when a
bid is **posted** — by the applier at apply and by the stock clerk at every re-post of a lapsed bid
— rather than by a once-per-IC-day price arm re-writing listings in place: the posting path already
carries the officer's signature, and a bid's TTL is the cadence the board re-prices on (the season
turns over years). **Reach** is `choose_route` for the lane's Dray empty with one soul at full
throttle, `Fastest`, over each supplier's listed routes to this venue, the least over the suppliers
(a supplier = another lane port whose vendors ask the good); the **synodic median** is the lower
median of that reach sampled from now over the synodic period of the port's body and the supplier's
(`Sky::period_days` from the table row nearest now; `1 / |1/P₁ − 1/P₂|`, rounded up, capped at
2,000 days — a pair on the same orbit, Ceres–Kirkwood, never closes and takes the cap), at most 400
samples (Ceres–Mars's 1,162 days every three), computed at each posting rather than cached per
boot; under the constant sky (rate 0) one sample, so every bid posts at its authored price. The
port entry's block is `"premium": { "k_permille": 300, "cap_permille": 500 }`, each `0..=10,000`,
absent = authored prices (lane **v5**; both ports carry it). `MarketStore` holds the posted price;
the port map's `BidRow.price_major` stays the authored figure. **Not built**: the emergency Wanted
bids and `emergency_ratio` (cut **A.4b**), and the per-boot median cache (not needed at these
costs).

## 7. Comms (cut A.5) — D-19-15

`Sky::light_lag_s(a, ta, b, tb)` = `distance_km / 299 792.458`, rounded up. Doc 15 R5 reads it for a
port pair at now and for a ship under way from its ticket's frozen legs (position interpolated along
the leg); the lag table becomes a live render.

## 8. The flip (cut A.6) — D-19-16

Nothing is rewritten: in-flight journeys keep their embedded `RouteDef` and `leg_s`, burned tickets
their plan and `sky_rev`, orders their destinations and UTC times, quotes their prices. One
owner-approved boot takes lane v5 and the Dray v3; `route_in_world` and the reindex re-materialize
the chords under the new sky while v3-pinned consumers keep v3. The shipped templates
(`mushd.toml.example`, `deploy/instance/mushd.toml.tmpl`) set `[tacsim.time.ephemeris] rate = 1000`;
the Rust default stays 0 so every pinned figure in the tree is still the epoch's. Tests: a
mid-passage journey arrives at its frozen instant across the flip; a burned-unlaunched ticket
launches with its frozen `leg_s`; the golden "identical at the epoch" test (longitude and distance
within rounding, the latitude term measured) and the drift per IC year recorded in the cut note.
**Never deployed without the owner.**

---

## 9. Cuts

| Cut | What | Size | Acceptance |
|---|---|---|---|
| **A.1** data and model (**landed 2026-10-18**; the cut note in [18-cut-notes.md](18-cut-notes.md) "A.1") | `ElementRow` tables on `BodyDef` (wire, scales, bounds, the consistency rule); `kepler.rs` (`julian_day`, the solver, the blend); `Sky::Kepler` with `PosKm` in 3D, `isqrt` distances, `heliocentric`, `light_lag_s`, the revision; `fetch_elements.py` + `golden.json`; lane **v4** (eleven bodies + Kirkwood on Ceres's rows trailing 2°; the slice's circular numbers unchanged; routes re-timed to the 3D chords at the epoch); `@corridor/show` names the model; pins re-blessed (lane digest, passage figures) and the drift recorded | M | `a1_sky_kepler_tracks_horizons` within the recorded per-body tolerance; `a1_sky_kepler_is_the_circular_sky_at_the_epoch` (r within 10⁻⁶, λ within 0.001°, β the measured latitude); a fixture pack without tables is `Sky::Circular` byte for byte; every existing suite green |
| **A.2** laws (**landed 2026-10-18**; the cut note "A.2") | `DriveLaw::Flip`, the thrust-limited laden rule, the rocket-equation burn, the throttle (on the plan too), five intercept passes, the keep-out detour, `RoutePolicy`, `via=` as a waypoint filter; `PassageOptions` on `+ship/plan`, `+ship/undock` and the depart order; Dray **v3**; pins re-blessed; doc 18 §4.4 "as built (A.2)"; lane v5's direct routes moved to A.3 | M | the epoch figures of §4.3 within 2 %; a worst-season plan over 24 h and over 70 t; `Cheapest` never slower-and-costlier than `Fastest`; a keep-out leg longer than its chord and never refused; a route planned under Coast byte-identical to IC9 |
| **A.3** chart (**landed 2026-10-18**; the cut note "A.3") | `+chart` (the sky table, each port's passage now at this load and empty, the direct chord priced beside the listed route), `+chart <port>` (the window: fastest, cheapest, worst day of the next IC year against now), `+chart/body`, the `+ship/plan` window line; the direct comparison computed from the sky, so lane v5 stays uncut | S | `tests/chart.rs`: the scan ≤ 365 evaluations, the worst day no shorter than the fastest; 120 columns |
| **A.3b** chart panel | the web cockpit panel and the TUI screen on the OutputEvent/Panel protocol | M | later |
| **A.4** prices (**landed 2026-10-18**; the cut note "A.4") | `port/price.rs`: reach (`choose_route`, the Dray empty, the least over the suppliers), the synodic median (`Sky::period_days`, `synodic_days`, `scan_days`, `median_of`; ≤ 400 samples; one under the constant sky), `premium_permille`, `priced`, `bid_price_major`; read at posting by the applier and the stock clerk; `PortDef.premium { k_permille, cap_permille }` with bounds; lane **v5** (both ports at 300 / 500); the trade help | M | `tests/price.rs`: both ports carry the block; rate 0 posts 280 / 520 (reach now = median); rate 1000 posts `priced(authored, premium(reach))` for every bid, never below authored, never above the cap; a port without the block posts authored prices; k past the bound refused; Ceres–Mars synodic ≈ 1,162 days, Ceres–Kirkwood capped |
| **A.4b** emergency bids | the emergency Wanted bids for life-support goods at the cap when reach crosses `emergency_ratio`, funded from finite capital; `+chart <port>` naming the premium | S | later |
| **A.5** comms (**closed 2026-10-18 as a seam**) | `Sky::light_lag_s` shipped in A.1 and read by `+chart` in A.3; doc 15 §3.5 amended — the backbone (R1–R10) is not started, so there is no live lag table yet; R5 consumes the seam when built | S | `tests/sky.rs`: the lag is `distance / c` rounded up; `tests/chart.rs`: every body's lag from Ceres |
| **A.6** flip (**landed 2026-10-18**; the cut note "A.6") | templates at rate 1000 (real motion; the Rust default stays 0); the readiness Sky line names the motion and the model; the migration tests of IC9/IC14 stand; docs 10/18/README amended; a drive-by: the stale dev1 clock pin in `shipped_configs.rs` | S | the two voyage tests; `shipped_configs` parity (12) |

Each cut is one commit on main with the Fable trailer and a `## A.n` heading in
[18-cut-notes.md](18-cut-notes.md) (the astronomy engine is doc 18's cut; the notes stay in one
file), pushed when green. No deploy without the owner.

---

## 10. Open for the owner (defaults taken, none blocking)

- **Q-19-1 The Dray's numbers.** Default `accel_mg 50`, `exhaust_kms 6000` (§4.3). Alternatives: a
  faster Dray (70 mg: 12–13 h best, 22–24 h worst, the Draft 0.3 brief's figure) or a thriftier one
  (10,000 km/s exhaust: fuel seasonality halves). Both are a `dray.json` edit.
- **Q-19-2 Kirkwood's trail.** Default 2° of mean anomaly behind Ceres (a 3.4-hour hop brimmed).
  Alternatives: 1° (2.4 h), 3° (4.2 h). Lane data.
- **Q-19-3 Direct routes in the ports' lists.** Default **no**: the Co-op's charter routes by the
  depot; `+chart` shows the direct chord as information. Alternative: list them, and the queue flies
  direct every season (the depot becomes a stop a captain chooses) until P4's customs compel it.
- **Q-19-4 The throttle floor.** Default 250‰ (a passage at most twice its full-thrust length).
- **Q-19-5 The price policy's caps.** Default: premium `k = 300‰` per unit of reach ratio, cap 500‰
  (cut A.4, lane v5's `premium` block on both ports; a lane data edit); `emergency_ratio 1.5`,
  life-support goods = provisions, volatiles, propellant (cut A.4b, not built).
- **Q-19-6 The keep-out radius.** Default 0.3 AU (Mercury's orbit is 0.39 AU).
- **Q-19-7 The chart panel.** Default: text first (A.3), the panel as A.3b after A.6.
- **Q-19-8 Re-fitting the elements.** Default: one set per body for 2340–2400; a second era is a lane
  data change when the campaign nears 2400 (about 2,000 real days at rate 24 from 2351).
- **Q-19-9 Moons as bodies.** Default: not yet (Stickney names `mars`); Luna and Titan with a port.

---

## 11. Amendments owed (made by the cuts that land them)

- **Doc 10 §22** gains "Superseded by doc 19 (cut A.1 at `<hash>`)" at the amendment's end.
- **Doc 18 §4.8** points here for the engine; §5's P-A row names the A-cuts; §8's Q-18-35..41 read
  "ruled by doc 19 §2's defaults, owner may override by data".
- **README** rows for doc 19 and the superseded T17 item.
- **Doc 15 §3.5** — the lag table is live from A.5.
- **`mushd.toml.example` / `deploy/instance/mushd.toml.tmpl`** — rate 1000 from A.6 with the
  comment rewritten; `shipped_configs.rs` parity held.
- **Doc 18 §5's P-A row and §8's Q-18-35..41 banner, the README row, `docs/help/trade.md`** — made
  by A.4 (the premium paragraph in the help; every landed cut named in the row).
