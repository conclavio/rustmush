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
would do in a decade). The circular numbers of ceres, mars and
pallas are **unchanged** from lane v3 — the Kepler set is exact at the epoch, so the engine's sky and
the slice's sky agree there in longitude and distance; they differ by the **ecliptic latitude** the
slice flattened (Ceres at i ≈ 10.6°, Pallas at 34.8°), which moves the pinned chords and the legs'
seconds once, measured in the cut note.

**The moons — D-19-17 (cut A.7, 2026-10-19; Q-19-9 ruled by the owner: "the moons are really where
the commerce is going to happen").** A `body` may name a **`parent`**: a body of the same pack about
the Sun (one level). Its four circular numbers and its element table are then **planetocentric** —
Horizons ELEMENTS with `CENTER` the planet, the ecliptic J2000 frame — and the sky places it at its
parent's position plus its own (`Sky::position_km`, both models; `Sky::distance_km` measures a moon
between composed positions, never by the coplanar law). Lane **v7** carries **luna** (about earth),
**phobos** and **deimos** (about mars) and **titan** (about saturn), and **Stickney Yards stands on
phobos** — the crater is Phobos's — so the Kirkwood–Stickney leg is 27 s shorter than the
Kirkwood–Mars chord was. A moon's rows are blended **in element space** (`kepler::blended_elements_position`:
scalars and the node interpolated linearly in time, the mean longitude ω + M interpolated along the
shorter arc with each row's M first advanced to the instant, the blended row propagated over zero
days), because a moon's node and pericentre precess in months (Phobos's node by 158° a year) and the
planets' position blend puts the midpoint of two phases of a small orbit inside it — Phobos's radius
halved at the worst instant; the planets keep the position blend as measured and pinned. Measured
against Horizons' heliocentric positions every 91 days over 2340–2400: Luna within 1.50 ‰ (Earth's
own 1.01 ‰ and the Sun's pull on a two-body lunar row), Phobos 0.088 ‰, Deimos 0.024 ‰. **Titan has
no Horizons ephemeris past 2250-01-05**: its table is the last decade Horizons knows (nineteen rows,
2240–2249), the last row propagated alone through the window — a near-circular 15.9-day orbit whose
plane precesses in centuries, self-consistent rather than Horizons-true, with no golden rows. A moon's
authored distance is quantized to the micro-AU (75 km), so the element table's consistency rule has a
150 km floor (`ELEMENTS_DISTANCE_FLOOR_KM`). **A moon's season is its primary's**: `Sky::period_days`
answers the parent's year for a moon, so the reach premium's synodic scan (§6) over a port on Phobos
is Mars's and Ceres's, not a 7.65-hour one. `+chart` rows a moon under the sky with "moon of mars";
`+chart/body phobos` shows its planetocentric row. Ports on Luna, Deimos and Titan landed as cut A.8
(§3.8).

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

### 3.8 Ports on the moons (cut A.8) — D-19-19 to D-19-25

**As built (cut A.8, 2026-10-19).** The first ports on the A.7 moons, the commerce Q-19-9's ruling
was for. The setting is [doc 11](11-tacsim-setting.md)'s (the houses, the history rows, the polities)
and the stations are [18a](18a-trade-run-stations-and-ship.md)'s; this section holds what the engine
and the lane carry. Every passage time and burn below is a season's, read from the design's replica
of the planner or from the engine where marked, and none is an invariant (D-18-50).

**D-19-19 Three ports, lane v8, six direct routes.** Lane **v8** adds three `location`s and three port
entries, one `routes` key on `ceres_port` and two programme reference rows (D-19-20) — 24 + 3 + 6 + 3
= 36 entries:

- **Aitken Highport** (`aitken_highport`, venue `luna`, faction `concord`) — Concord Orbital Access's
  spun civil drum in low polar orbit, named for the South Pole–Aitken basin it crosses (Shackleton and
  Malapert swing under the hub once an orbit; no lunar orbit stands over a feature). Nothing in this
  trade lands: a Dray at a twentieth of a gravity cannot lift from Luna's sixth, so ships berth at the
  drum's non-rotating hub. Cabeus Works asks regolith alloy stock and bids Belt concentrate; the Lunar
  Habitat Board bids ice and Titan's carbon billet; Crisium Instruments forwards the basin fabs'
  instrument boards; the Highport Victualling Office sells bunkers and stores. Twelve rooms.
- **Voltaire Anchorage** (`voltaire_anchorage`, venue `deimos`, faction `republic`) — the Republic's
  bonded anchorage in Voltaire crater, a micro-g surface port on the Stickney pattern, where the
  Enclave arks lay in the roads from the 2150s to the 2200s; under it the **Republic Strategic
  Reserve**, the Concord's blockade depot of 2308 conveyed with the debt in the 2312 Settlement (doc
  11 §3.2's `propellant_reserve` made a place). It sells only bunkers (reaction mass at Kasei's 60,
  provisions at 10), bids Luna's light alloy and boards through Syrtis Bonded Imports and Titan's ice
  and billet through the Reserve, and exports nothing. Eleven rooms, the Stickney boxes.
- **Ligeia Terminal** (`ligeia_terminal`, venue `titan`, factions `company` and `townships`,
  D-19-24) — the Titan Concession Company's unspun spar in low Titan orbit, named for the methane sea
  its lifters climb from; the spar crosses the sea for a run of orbits each Titan day and then not for
  a fortnight (no Titan-stationary orbit exists: the synchronous radius, about 75,600 km, lies outside
  Titan's Hill sphere). The Company sells carbon billet, the lifters' overburden ice, the system's
  cheapest reaction mass and its dearest stores; the Selk Townships Board bids alloy and boards at
  frontier prices. Twelve rooms. No relay half (doc 15 §10 q2 option A): 67–92 light-minutes from
  Earth and Luna.

Six station packs (a region and a neighborhood each), five ports in `ic_return::PORTS` (`+ic` lands at
Aitken's arrivals, Voltaire's gallery, Ligeia's arrivals), eleven new orgs (twenty in the lane), three
new clerks, 6,300,000 cr of new capital in 16 tranches (32 of the pack's 64), 45 new readiness lines
(85 in all, 63 not ready before the first sweep — the three bid-only houses have nothing to seed and read Ready at once). **The graph**: Ceres lists `luna` and
`stickney`, Stickney `ceres` (unchanged), Luna `deimos` and `titan`, Deimos `stickney`, Titan `deimos`
and `luna`. Every new route is direct, `coast_hauler`, with no controller and no hazard; each `base_s`
is the epoch figure `tests/sky.rs a1_lane_legs_at_the_epoch_are_printed` prints (widened to every
route), documentary for a cross-body leg:

| route | from → to | base_s at the epoch |
|---|---|---|
| `ceres_aitken` | Ceres Freeport → Aitken Highport | 1,609,856 |
| `aitken_voltaire` | Aitken Highport → Voltaire Anchorage | 728,633 |
| `aitken_ligeia` | Aitken Highport → Ligeia Terminal | 6,803,208 |
| `ligeia_aitken` | Ligeia Terminal → Aitken Highport | 6,803,208 |
| `ligeia_voltaire` | Ligeia Terminal → Voltaire Anchorage | 6,477,235 |
| `voltaire_stickney` | Voltaire Anchorage → Stickney Yards (the hop) | 131 |

Titan's three rest on the 2249 row propagated a century (§3.6). `lane_legs_derive_at_epoch` holds
them to its 100 s check like every route, but against their own propagated 2249 row: for Titan the
check is the pack's self-consistency with the engine's table, not accuracy against Horizons, until
an ephemeris past 2250 exists. **One one-way leg and two declared absences**, each a commercial or a
charter fact, never a treaty one: `voltaire_stickney` is an **empty hop** by construction — Deimos asks no hold good
Stickney bids, and the leg before it (Luna's or Titan's cargo sold at Deimos) and the leg after it
(the reference return from Stickney) pay for its tonne; **no Stickney → Deimos** — the Republic does
not bond its own yard stock (and with Syrtis bidding alloy over Kasei's 400 ask it would be a
three-hour arbitrage); **no Luna → Ceres** — nothing at Ceres buys what Luna sells (Ceres bids no
boards, and Kasei's recorded stock already fills Freeport Works' 7,000 t alloy intake exactly,
D-18-27), and a Ceres-bound Co-op hull calls at the depot by charter (§4.6), so it would be a
via-Kirkwood leg carrying nothing Ceres buys; the Belt's share of the boards moves by other carriers
and the importers' own agents. Also unlisted: Ceres ↔ Titan (D-19-22), and Deimos → Titan and
Stickney → Titan (Titan would share Stickney's recorded alloy and fittings stock, sized to Ceres's
intakes). `tests/moon_ports.rs` asserts the graph exactly and the two absences as invariants, so a
later cut cannot add them by accident.

**The ladder.** The Ceres–Stickney shuttle stays the programme's tutorial and every circuit ends on its
return, so the moons feed the reference loop rather than replace it. The **Concord ring** — Ceres →
Luna → Deimos → the hop → Stickney → Ceres — pays about 1.2–1.3× the reference per IC day over about
65 IC days and needs about 60,000 cr at Luna; the **Titan shuttle** Luna → Titan → Luna about 1.5× as
the median over a Saturn year, finite at 12 loads each way, flown offline by the queue. Neither is a
pin or every season's: at the 2351 epoch itself the reference sits in a good season and out-earns the
Titan shuttle per IC day. The test asserts the ordering ring > reference > an empty Luna-and-back at
the epoch only, never a multiplier. The pairs (a 100 t / 120,000 L hold; no code, fixture, NPC line or
test stores a vertex, D-18-39; `tests/capacity.rs` derives them):

| pair | the whole-tonne load | margin |
|---|---|---|
| Ceres → Luna | 80 t ice + 20 t concentrate | 9,000 |
| Luna → Deimos | 60 t alloy + 40 t boards | 11,600 |
| Luna → Titan | 60 t alloy + 40 t boards | 45,600 |
| Titan → Luna | 75 t ice + 25 t billet | 14,875 |
| Titan → Deimos | 75 t ice + 25 t billet | 14,500 |
| Deimos → Stickney | none (the hop, declared empty) | — |

**D-19-20 Goods v2.** `goods.json` **v2** adds two rows and no family: **`instrument_boards`**
("instrument boards, crated", family `electronics`, 1,000 kg, 2,550 L, reference 900) and
**`carbon_feedstock`** ("carbon billet, pressed", family `carbon_feedstock`, 1,000 kg, 600 L,
reference 250); the catalog is 8. **The integrality finding**: among the six shipped hold goods the
whole-tonne rule (a mixed load that beats each good alone and is unique over whole tonnes) admits only
{metals, volatiles} and {alloys, parts} — the reference loop's two directions — so a moon port of
shipped goods is a second Ceres or a second Stickney, or flies empty. Two rows suffice: boards at
2,550 L pair with the shipped alloy at a whole vertex, so Luna's export needs no exotic dense partner,
and billet at 600 L pairs with the shipped ice, so Titan's backhaul needs no ninth row. The id is not
`electronics`: [doc 17](17-unit-minds.md) §16 and 17f M7.9 keep **`tacsim.electronics`** for
quantity-one certified die lots graded `die.c1..c3`, and `goods.json` carries one `mass_kg` per good,
so a bulk row under that id would make a die lot weigh a tonne. Crated boards are Luna's high-end
manufacture shipped as tonnage; the die lots stay a different good. An eleventh family is still
refused. `ceres_port.programme.reference` gains two rows — `instrument_boards` →
`luna_port/crisium_instruments`, `carbon_feedstock` → `titan_port/titan_concession` — so an enrolled
sale of a new good shares against a named ask and never logs "a sale's reference listing is gone";
the applier's drift check never compares the programme block, so v8 applies over a kept v7 world
(`tests/commerce.rs`, `a8_a_world_on_lane_v7_takes_v8_by_the_shelf`).

**D-19-21 Sibling-moon legs in the parent's frame.** In `passage::leg_distance_km` only: when both
endpoints' bodies have the same parent `p`, the leg is `|(pos(b, t1) − pos(p, t1)) − (pos(a, t0) −
pos(p, t0))|`, with no solar keep-out; `Sky::distance_km`, `Sky::position_km`, light lag and
`scan_days` are untouched. In the Sun's frame the 29,512 km Phobos–Deimos hop plans, in the design's
replica, as about 40 IC hours and 5 t after five passes (the chord from one moon at departure to the
other at arrival is mostly Mars's own motion) and still diverges at twenty; in Mars's frame it is
3.0–4.5 IC hours over forty IC days of the moving sky (`tests/moon_ports.rs`), 1 t and one crew-day — doc 10 1A's "nearby
moons: IC hours". A same-instant chord is frame-invariant, so the hop's `base_s`, the lane derive and
every pinned leg stand (the story's 321,547 + 1,256,295 s and 1,195,643 + 306,023 s do not move).
**The convergence exception** (§4.4): in the design's replica Deimos → Phobos leaves up to about
360 s between pass five and pass twenty, because Phobos turns up to about 190° about Mars during the
hop (Phobos → Deimos converges under 2 s), so a sibling-moon leg is held to **600 s**, not 60.
**The 2,000-day scan stays** for the Phobos–Deimos pair (one primary, so the synodic never closes and
takes §6's cap): 400 plans of a flat one-tonne hop per read is cheap, and a sibling-synodic scan would
be a sky API change for nothing measurable.

**D-19-22 Titan by throttle.** Laden at full throttle every Titan leg burns more than the Dray's
80 t tank: the engine asserts it at the epoch (`a8_titan_fits_the_tank_only_throttled`), and the
design's replica reads 94–118 t in every season (the engine's Saturn-year sweep plans at 450 and
465 ‰ only). The help, the Terminal's lore and its dockmaster say one word for every
listed Titan leg: **`throttle=450`, one soul**. The
sweep over one Saturn year (24 evenly spaced departures plus a ±14-day window at one-day steps about
each replica worst departure, 53 a leg, the full 80 t tank aboard, at least 15 ‰ of margin) reads, in
the engine (`a8_titan_throttle_fits_a_saturn_year`, each ceiling bisected at the departure with the
largest burn at 465 ‰), the laden one-soul ceilings at 491 ‰ (Luna → Titan, day +7,537), 491 ‰
(Titan → Luna, day +7,096) and 466 ‰ (Titan → Deimos, day +7,816) — the design's replica read 493,
494 and 466. The unlisted Ceres → Titan, swept in that direction only over 48 sampled departures,
fits 465 ‰ at the epoch and fails it in a later season: the test prints the first failing sample
(day +1,604, ceiling 461 ‰), and an earlier run of the same sweep, bisecting every sample, read the
minimum at 429 ‰ (day +7,562). 450 keeps the rule
on Titan → Deimos by a permille, so **440 is the fallback word** if the engine's Titan → Deimos
ceiling falls under 465 (about two more IC days a leg). At 450 a Titan leg
runs 52.4–65.2 IC days over the sweep's departures (2.2–2.7 real days at rate 24), 53–66 crew-days a
soul; two souls need 106–132 crew-days against the 120-day locker, so a
Titan run is single-handed in a
bad year, two souls only when the plan reads 60 days or less. **The `Throttle:` line**: `+ship/plan`
prints one line after its `Stores` line, only when the plan at the asked throttle still fails with the
tank and the locker filled to capacity (the payload re-based to 80 t of reaction mass and 120
crew-days) — `Throttle: with the tank and locker full, throttle=N fits: burn B t, C crew-days, D IC
days.`, or `Throttle: no throttle from 250 permille fits this load even with the tank and locker
full.` — `N` found by bisection over `250..=1000` on burn against the tank, the locker then checked at
`N`. TANK_SHORT, STORES_SHORT and the queue's hold line are unchanged, and the line never begins
`Burn ` or `Stores `, so every line-reading pin stands; the shipped lane needs it too (§4.3).
**Ceres ↔ Titan is not listed**: its laden one-soul ceiling is about 430 in the design's sweep (the
engine's 429 ‰), under
any word worth advertising, and its scan would cost Ceres's chart 1,994 days.

**D-19-23 Supplier sets.** A supplier of a good to a port is another lane port whose vendors ask the
good and that lists a route to the port's venue (§6). **The old ports**: no bid gains a supplier (Luna
asks alloy but lists no `ceres`; Deimos asks nothing Stickney bids); Ceres's emergency block and its
volatiles reach `None` stand; Stickney's provisions and propellant readings take Deimos, the hop, as a
second supplier and flatten to a reach of a few IC hours, so its emergency can never post for those
two — a depot next door, accepted. Its `+chart` emergency line names the largest ratio over its three
readings, so it can move now that two are the hop's own, but the volatiles reading, supplied through
Ceres alone, still sets the maximum: at the chart fixture's instant the line keeps its text (1016 ‰ of
the median) and was not re-blessed. `tests/price.rs`'s `a8_the_old_ports_suppliers_move_as_designed`
holds the old ports' rows against a v7-shaped lane at rate 0 (Stickney's provisions and propellant
reach 11,718 s on v8 against 1,250,608 s on v7; its volatiles, Ceres's readings and the four old bids
unmoved). **The moon ports' own**: Ceres
and Titan supply Luna's provisions, volatiles and propellant; Luna and Titan supply Deimos's provisions
and propellant, and Titan alone its volatiles (Luna asks none); Luna supplies Titan's alloy, boards
and provisions. All
three carry `premium { 300, 500 }` and an `emergency` block at 1,500 ‰, 100 units — Luna and Deimos
for provisions, volatiles and propellant, Titan for provisions alone (a Wanted for ice or reaction
mass at a methane-and-ice world would be absurd) — posted by the house that neither asks nor bids
those goods (Crisium Instruments, Syrtis Bonded Imports, the Selk Townships Board). No moon pair's
reach peaks near 1.5× its median in the design's sweep (Luna → Deimos at 1.26 is the highest), so the
blocks are levers, as the shipped ones are. §6 has the table; the chart's and the hourly reads' cost
is owed a measurement (the A.8 cut note's Deferred), not assumed, and a pure-function supplier-set
memo is the first follow-up if a frame is over budget.

**D-19-24 Titan's polity.** A chartered company-state: the Concord's **Saturn Charter of 2291**, a
99-year grant recorded at the Registry (a notary, never the grantor) and lapsing in 2390, makes the
Titan Concession Company proprietor and government, headed by its Chief Factor, clearing through Luna,
months from anyone who could object. The Selk Townships are the opposition, petitioning Shackleton —
whose charters these are — for one of their own, the petitions lodged at the Registry. Doc 11's one
other chartered monopoly, the Mercury Solar Authority, is Concord-chartered, so the grantor pattern is
the Concord's. **Faction words**: `company` for the Company's two houses (the Titan Concession Company,
Concession Stores), naming Titan's polity beside `freeport`, `republic` and `concord`; `townships` for
the Selk Townships Board. The shipped label is an org's **allegiance**, not its port's polity — Tycho
Mutual at the Freeport carries `concord` — so the Company's declared opposition cannot carry the
Company's word. Both labels are new and read by nothing until the faction cut, whose ready front they
are: a lapsing charter and a townships' counter-petition.

**D-19-25 The moon stock basis.** Every moon seller's stock is the sum of its reachable buyers'
intakes at **40 loads per Concord pair and 12 per Titan pair**; a good with two sellers (ice to Luna,
from Ceres and from Titan) splits the buyer's intake by the basis, and nothing enforces the split.
Pure buyers are capitalised at their exposure plus about 5 % (the Freeport Works pattern) and covered
at their reach peaks; sellers and operators at 50,000; tranches of at most 800,000. The rows:

| port | asks (stock) | bids (intake) |
|---|---|---|
| Aitken Highport | alloy 400 (3,120 t: Deimos 2,400 + Titan 720); boards 900 (2,080 t: 1,600 + 480); reaction mass 60 (5,000 t); provisions 10 (2,600) | concentrate 270 (800 t); ice 195 (4,100 t: Ceres 3,200 + Titan 900); billet 380 (300 t) |
| Voltaire Anchorage | reaction mass 60 (5,000 t); provisions 10 (2,600) | alloy 460 (2,400 t); boards 1,100 (1,600 t); ice 190 (900 t); billet 380 (300 t) |
| Ligeia Terminal | billet 250 (600 t); ice 40 (1,800 t); reaction mass 30 (5,000 t, 62 full fills); provisions 20 (2,600) | alloy 760 (720 t); boards 1,500 (480 t) |

**Ceres's stock is not topped up** (a recorded row never changes, Q-18-19), so **D-18-27 is amended**:
its rule that a seller's stock covers its buyers' intakes no longer holds at Ceres. Occator's recorded
9,600 t of ice now faces **12,800 t** of intakes by the basis (Stickney 9,600 + Luna's 3,200 share)
and **13,700 t** by the authored rows (Luna's whole 4,100), a shortfall of 3,200–4,100 t; the Freeport
Exchange's 4,000 t of concentrate faces **4,800 t** (Stickney 4,000 + Luna 800), a shortfall of 800 t.
Honest SOLD OUT arrives earlier in a busy cohort; only a fresh-store resize (Occator 12,800 or 13,700,
the Exchange 4,800) restores the rule, and that is the owner's. **The Reserve's ice row is an authored
floor**: a Titan backhaul lands at Deimos with 75 t of ice and 25 t of billet, sells the billet and
hops on to Stickney for the reference return anyway, where Kasei's 200 beats the Reserve's 190 in
every season; so Titan's Deimos-side ice sells at Kasei, up to 900 t, inside the slack Luna's draw on
Occator leaves Kasei's 9,600 t intake, and the Reserve's 900 t row stands unfilled. No clean price
exists — at 200 or more Deimos would become the terminal for Ceres ice by way of Luna, past the
Board's 195 — so 190 stays. Syrtis's alloy bid at 460 under Freeport Works' 520 is not dominated: a
ring captain who kept Luna's alloy for Ceres would displace Kasei's alloy from the same hold at the
same +120 a tonne and forgo Deimos's +60, so Syrtis's intake fills and Kasei's stock still sells.

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

**The shipped lane's own tank-short seasons (cut A.8, 2026-10-19; Q-19-13).** Laden with 100 t at
full throttle and one soul under the moving sky, the reference legs themselves are TANK_SHORT in a bad
season: Ceres → Stickney on about 14 % of rate-1000
departures and Stickney → Ceres on about 33 %, the worst
burns in the design's replica 82–83 and 86–87 t against the 80 t tank; the worst-season fits are about 960 ‰ and 870 ‰ (the design's replica; the engine's figures are read when the auto-throttle cut is taken). The plan refuses as it always has and a queued
depart holds on the tank line; A.8's `Throttle:` line on `+ship/plan` (D-19-22) now names the throttle
that fits. An auto-throttle on a queued depart is a follow-up cut, once the engine's figures confirm
the design's (Q-19-13).

### 4.4 The intercept — D-19-11

The fixed-point intercept stays (`intercept_passes`): the chord from the origin body at `t0` to the
target at `t1`, the law's seconds, `t1 := t0 + secs`, repeated. Under a flip law the contraction is
weaker than under coast (seconds grow with √d), so the planner takes **five** passes
(`INTERCEPT_PASSES`), and the test that showed the contraction shows passes four and five within
60 s on every lane leg at the epoch and at the worst season.

**The exception (cut A.8, D-19-21): a sibling-moon hop converges to 600 s.** Measured in the parent's
frame, Deimos → Phobos leaves, in the design's replica, up to about 360 s between pass five and pass
twenty, because Phobos turns up to about 190° about Mars during the hop; Phobos → Deimos converges
under 2 s. `tests/passage.rs` holds
passes five and twenty within **600 s** both ways on the hop; every other lane leg keeps the 60 s
bound.

### 4.5 The solar keep-out — D-19-12

A leg's chord is tested against a disc of `KEEP_OUT_AU = 0.3` about the Sun in the plane of the two
endpoints: if the segment passes inside it, the leg's distance is the shortest path around the disc —
tangent, arc, tangent: `√(|A|² − R²) + √(|B|² − R²) + R·θ`, closed form, in `f64`, rounded to the
kilometre — and the plan's leg carries that distance. A cost, never a refusal (Q-18-40): no route
is ever closed by the Sun, so no validator rule about "one non-refusable route" is needed.

### 4.6 Choosing a route — D-19-13

`choose_route` takes a **policy**: `Fastest` (least `total_s`, the default and the queue's) or
`Cheapest` (least `burn_t`, ties to the faster). `via=<place>` is honoured as a filter over the
listed routes (those with a leg ending at that place); `ViaReserved` retires.

**As built.** No direct Ceres–Stickney route ever shipped: the draft put `ceres_stickney_direct` and
`stickney_ceres_direct` in lane v5, A.2 moved them to A.3, and A.3 computes the direct chord from the
sky for `+chart`'s comparison instead, so lane v5 (A.4's premium blocks) carried none. The **ports'
Ceres–Stickney lists are via Kirkwood only** and the lane holds no direct route between them; the
queue's behaviour is unchanged and `+chart` shows the captain what the direct chord would have cost
(Q-19-3). The rule is the Co-op's charter, and — the rationale as narrowed by cut A.8, 2026-10-19 —
it binds the depot call on **Ceres-bound** legs (the Belt's inbound, by the Freeport-licensed depot)
and on the **Ceres–Mars corridor** the Republic's 2347 pickets watch (doc 11
§2; the pickets and customs of P4 are what make that a rule rather than a preference) — not on a
Concord-bound leg out of Ceres. **A.8 ships the lane's first direct routes**, six of them, all
moon-bound or moon-born (D-19-19): `ceres_aitken` flies direct (in the design's sweep its worst season
burns 68 t direct against 77 t by Kirkwood), and no new route is Ceres-bound or runs the Ceres–Mars
corridor, so the charter never binds one.

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

**As built (cut A.3b, 2026-10-19; the chart panel — D-19-18).** One frame, three renders. The
`sky.chart` panel feed (`port/chart_panel.rs`, registered with the port verbs; pulled, never pushed)
answers a **grid-shaped** frame — the shape `atlas.map` and the arena map wear, so the one grid printer
(`mush_ansi::grid_lines`) and the web's grid decoder already read it — whose grid is the server's
**flat render** of the sky: 60 by 28, the Sun at the centre, every orbit a ring dotted every 5° at a
**logarithmic radial scale** (`ln(1 + r/0.3 AU) / ln(1 + r_max/0.3 AU)`; a square root packed the six
inner orbits into seven rows), the bodies by letter (nearest the Sun first keeps a shared cell; the
vantage's body drawn last over everything; a moon shares its parent's cell and draws no letter), the
listed routes' legs as `:` body to body, the vantage port as the viewer's `you`. Beside the grid a
`chart` block carries the vector data (each body's position, distance, longitude, latitude, period,
parent, light lag, raster cell; the routes' legs and passages now at this load, empty and the direct
chord; the window over the next IC year; the lines `+chart` and `+chart <port>` print). **Line mode**:
`+chart/map` prints the title, the legend, the whole grid north up with its blank edge rows trimmed,
and the scale. **The TUI**: the first **lettered screen** — the ten digits are spent, so a screen past
them is reached by `Alt+<letter>` when the active screen binds nothing to the letter (the cockpit's
`c` stays its contacts), its `ScreenId` the letter's byte, which the tab strip prints as the letter;
`c` is the chart, gated on the feed (a world without the lane never grows the tab), `on_enter` asking
for the frame; its Main pane windows the grid to the pane around `you` with the title above and the
legend below, its Right pane prints the server's lines; `r` asks again, `w` prefills `+chart `, `m`
runs `+chart/map`. **The web**: the same policy (`ScreenDef.digit: number | null` with `hotkey`,
`Alt+letter` → `screenKey` when unbound, a `FeedScreen` registered on a `.chart` feed) and a
`ChartScreen` that draws an SVG picture from the vector data on the same scale (orbits, bodies, the
routes' polylines, the vantage ringed), prints the tables from its own pure composers, and shows the
flat map under a toggle; `web/src/lib/fixtures/chart/sky_chart.json` is a frame the server made,
blessed by `tests/chart.rs` and decoded by the web tests. The panel plans the port's programme hull
empty; `+chart` from a docked helm plans the charter's load. `+chart` and `+chart/map` joined the
read-only verb allowlist (both copies and the fixture).

**The chart as a picture (doc 21 cut G.5, 2026-10-22).** `^G` on the TUI's chart screen draws the
frame as a picture (`mush_pixel::chart`, a scene on this section's log scale): orbits, the bodies
with the vantage ringed, the passages as dashed legs bent by the scale, light-lag rings at 3, 6, 12
and 24 light-minutes about the vantage body, fetched again every minute while open; the web's
`ChartScreen` draws the same rings on its SVG and its caption now names the scale it draws
(logarithmic, where it had said "square root"). A ship under way is still not on the chart: the
frame is built from a port; ships along their tickets' legs remain this section's to add.

**The moon ports on the chart (cut A.8, 2026-10-19).** Three ports now stand on moons — Aitken
Highport on Luna, Voltaire Anchorage on Deimos, Ligeia Terminal on Titan — and the chart still draws
each on its parent's cell, named in the legend and the table; Ceres's frame gains the Luna route, its
window and Luna's demand lines, so the fixture `sky_chart.json` is re-blessed for those lines;
Stickney's emergency line keeps its text, the volatiles reading through Ceres still its maximum
(D-19-23). The A.3b note deferred the zoom to a planet's system (Luna, the Martian
moons, Titan) until a port stood there. That trigger is met and A.8 does not build the zoom: it is
**re-deferred pending the owner's call** — a date for it, or a cut of its own (none scheduled as of
2026-10-19); until then a moon port draws on its parent's cell.

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
the port map's `BidRow.price_major` stays the authored figure. **Not built**: the per-boot median
cache (not needed at these costs).

**As built (cut A.4b, 2026-10-19; the emergency bids).** The port entry's block is
`"emergency": { "org", "ratio_permille", "goods", "intake" }` (lane **v6**: the Exchange at Ceres
and the Yards at Stickney, 1,500 ‰, provisions / volatiles / propellant, 100 units), bounded in
`PortDef::check` (a premium to take the cap from; the ratio in 1,000..=100,000 ‰; one to sixteen
goods of the goods table, none twice; an intake the market buys; an org that is one of the port's
vendors and neither asks nor bids any of the goods). **The stock clerk** reads the port once a real
hour (`EMERGENCY_CHECK_SECS`; one IC day at rate 24), before its stock steps: a good whose reach is
at or past the ratio of its synodic median (`emergency_crossed`) and has no standing emergency bid
gets one — the org's vendor posts a Wanted for the intake at the premium's **cap** over the good's
**reference** (the port's highest authored ask or bid for it, else the goods table's
`reference_major`; Stickney's volatiles 200 → 300, provisions 10 → 15, propellant 60 → 90), as the
clerk, signed as every bid is; a house that cannot fund one unit posts nothing and is warned once a
boot (`board::vendor_funds`); a bid gone before its recorded deadline (its intake taken) is not
posted again until the deadline passes. A good whose reach fell back under the ratio while its bid
stands has it withdrawn. One act a fire, authored order; the hour is spent only when nothing was
due. The rows live in `PortMap.emergencies` (good → `BidRow`, the reference as `price_major`); a
port retire withdraws them with the standing bids. A good no other port supplies has no reach and no
emergency (Ceres's volatiles: nobody at Stickney asks volatiles). **`+chart <port>`** gained the
destination's demand side: each standing bid's list price and what it posts at with the premium and
the reach against the median, then the emergency line (which goods are due at the cap, or none and
where the reach stands). The emergency bids carry no `boards` (no room board renders them;
`+market/list` does).

**The supplier table (cut A.8, 2026-10-19; D-19-23).** A supplier of a good to a port is another lane
port whose vendors ask the good and that lists a route to the port's venue. Under lane v8:

| port, reading | v7 suppliers | v8 suppliers | effect |
|---|---|---|---|
| Ceres: the alloy bid (Freeport Works), the fittings bid (the Co-op) | Stickney | Stickney | none — Luna asks alloy but lists no `ceres` |
| Ceres: emergency provisions, propellant | Stickney | Stickney | none |
| Ceres: emergency volatiles | none | none | `reach == None` stands |
| Stickney: the concentrate and ice bids (Kasei) | Ceres | Ceres | none — the chart's bid lines keep their text |
| Stickney: emergency volatiles | Ceres | Ceres | none |
| Stickney: emergency provisions, propellant | Ceres | Ceres, **Deimos** (the hop) | the reach flattens to the hop's few IC hours; the emergency can never post (a depot next door, accepted); the `+chart` emergency line keeps its text, its maximum the volatiles reading's |
| Luna: the concentrate, ice and billet bids | — | Ceres (concentrate); Ceres and Titan (ice); Titan (billet) | the premium over the Ceres → Luna and Titan → Luna reach |
| Luna: emergency provisions, volatiles, propellant | — | Ceres, Titan | a lever; no pair peaks near 1,500 ‰ |
| Deimos: the alloy and boards bids (Syrtis); the ice and billet bids (the Reserve) | — | Luna (alloy, boards); Titan (ice, billet) | the premium over the Luna → Deimos and Titan → Deimos reach |
| Deimos: emergency provisions, propellant | — | Luna, Titan | a lever |
| Deimos: emergency volatiles | — | Titan | a lever (Luna asks none) |
| Titan: the alloy and boards bids (the Townships) | — | Luna | the premium over the Luna → Titan reach |
| Titan: emergency provisions | — | Luna | a lever |

Stickney lists no route to `deimos`, so the hop is never Deimos's supplier. The scans: Stickney's
provisions and propellant readings take the 2,000-day cap (the minimum over the Ceres plan and the
hop at each sample, §3.8 D-19-21); Deimos's scan Luna's 778-day synodic and Titan's 734-day one;
Ceres's chart frame grows by its second destination's window. Each cost is owed a measurement before
any cache is built; none is measured yet (the A.8 cut note lists it under Deferred).
`tests/price.rs`'s `a8_the_old_ports_suppliers_move_as_designed` holds the old ports' rows of the
table, v8 against a v7-shaped lane (D-19-23).

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
| **A.3b** chart panel (**landed 2026-10-19**; the cut note "A.3b") | the `sky.chart` panel feed (`port/chart_panel.rs`: a grid-shaped frame — the server's flat render, 60 by 28, the Sun, dotted orbits on a logarithmic radial scale, the bodies by letter, the listed routes dotted, the vantage as `you` — with a `chart` block of vector data, the routes, the window and the lines `+chart` prints); `+chart/map` in line mode; the TUI's lettered screen `c` (the digitless policy: `Alt+<letter>` where the screen binds nothing to it; `ScreenId` = the letter's byte), its chart pane windowed to the pane and its tables pane of server lines; the web's `ChartScreen` (an SVG picture from the vector data on the same scale, the tables, the flat map under a toggle) on the letter hotkey `c`, `FeedScreen` on a `.chart` feed; the fixture `web/src/lib/fixtures/chart/sky_chart.json` blessed by the Rust side | M | `tests/chart.rs`: the frame's shape, letters, legend, routes, window, lines, the flat map's lines, the blessed fixture; `chart_panel` unit tests (scale, raster, line, letters); `mush-net-ssh/tests/chart_screen.rs`: eleven screens validate, Alt+c opens the chart where the feed is published and is inert elsewhere, the frame draws; `web`: `chart.test.ts` (decoder, scale, points, polylines, tables), the hotkey screen and Alt+letter tests, `svelte-check` clean |
| **A.4** prices (**landed 2026-10-18**; the cut note "A.4") | `port/price.rs`: reach (`choose_route`, the Dray empty, the least over the suppliers), the synodic median (`Sky::period_days`, `synodic_days`, `scan_days`, `median_of`; ≤ 400 samples; one under the constant sky), `premium_permille`, `priced`, `bid_price_major`; read at posting by the applier and the stock clerk; `PortDef.premium { k_permille, cap_permille }` with bounds; lane **v5** (both ports at 300 / 500); the trade help | M | `tests/price.rs`: both ports carry the block; rate 0 posts 280 / 520 (reach now = median); rate 1000 posts `priced(authored, premium(reach))` for every bid, never below authored, never above the cap; a port without the block posts authored prices; k past the bound refused; Ceres–Mars synodic ≈ 1,162 days, Ceres–Kirkwood capped |
| **A.7** moons (**landed 2026-10-19**; the cut note "A.7"; Q-19-9, D-19-17) | `BodyDef.parent` (wire, the pack's one-level rule `UnknownParent`), `Body.parent`, the sky composing a moon's position with its parent's on both models, `Sky::parent_of`, `period_days` as the primary's, `distance_km` between composed positions for a moon; `kepler::blended_elements_position` (the element blend for moons; `propagate_dt`); the 150 km distance floor; `fetch_elements.py --moons` (planet-centred ELEMENTS, heliocentric golden; Titan's last decade); lane **v7** (luna, phobos, deimos, titan; Stickney Yards on phobos; the Kirkwood–Stickney leg 942,997 s); `+chart` "moon of"; pins re-blessed (lane digest, legs, passage 321,547 + 1,256,295 / 1,195,643 + 306,023, the story's 326,136 + 1,274,226) | M | `tests/sky.rs`: sixteen bodies, the four moons' parents, a moon's period its primary's; Horizons tracking for fourteen bodies (Luna ≤ 1.60 ‰, Phobos ≤ 0.09 ‰, Deimos ≤ 0.03 ‰); a moon stands its own orbit from its parent at the epoch and a year either side (Luna 356,000–407,000 km, Phobos 9,200–9,600, Deimos 23,400–23,500, Titan 1,180,000–1,260,000); the epoch consistency planetocentric for moons; `tests/lane.rs` twenty-four entries, the chord Ceres–Phobos, 125 periods of phase; `tests/chart.rs` sixteen rows with "moon of mars" |
| **A.4b** emergency bids (**landed 2026-10-19**; the cut note "A.4b") | `PortDef.emergency { org, ratio_permille, goods, intake }` with bounds; the stock clerk's emergency step (once an hour; post at the cap over the reference when the reach crosses the ratio, withdraw when it falls back, one act a fire; funded or nothing); `PortMap.emergencies`; `emergency_readings` and the `+chart <port>` demand-side lines; lane **v6** (both ports at 1,500 ‰, provisions / volatiles / propellant, 100 units); the trade help | S | `tests/price.rs`: the block and its refusals; rate 0 posts none (reach = median); a revision at 1,000 ‰ posts a Wanted per supplied good (the org's vendor, 100 units, `priced(reference, 500)`), the capital rows unchanged; a revision at 100,000 ‰ after a reboot withdraws them; `tests/chart.rs`: the demand-side and emergency lines under both skies |
| **A.5** comms (**closed 2026-10-18 as a seam**) | `Sky::light_lag_s` shipped in A.1 and read by `+chart` in A.3; doc 15 §3.5 amended — the backbone (R1–R10) is not started, so there is no live lag table yet; R5 consumes the seam when built | S | `tests/sky.rs`: the lag is `distance / c` rounded up; `tests/chart.rs`: every body's lag from Ceres |
| **A.6** flip (**landed 2026-10-18**; the cut note "A.6") | templates at rate 1000 (real motion; the Rust default stays 0); the readiness Sky line names the motion and the model; the migration tests of IC9/IC14 stand; docs 10/18/README amended; a drive-by: the stale dev1 clock pin in `shipped_configs.rs` | S | the two voyage tests; `shipped_configs` parity (12) |
| **A.8** moon ports (**landed 2026-10-19**; the cut note "A.8"; §3.8, D-19-19..25) | the first ports on the moons: lane **v8** (Aitken Highport on Luna, Voltaire Anchorage on Deimos, Ligeia Terminal on Titan; three locations, six direct routes — `ceres_aitken`, `aitken_voltaire`, `aitken_ligeia`, `ligeia_aitken`, `ligeia_voltaire`, the empty hop `voltaire_stickney` — Ceres's `luna` key, two programme reference rows; 36 entries); goods **v2** (`instrument_boards`, `carbon_feedstock`; catalog 8); six station packs; `ic_return::PORTS` at five; two engine seams: sibling-moon legs in the parent's frame (`passage::leg_distance_km`, the 600 s convergence exception) and the `Throttle:` line in `+ship/plan`; the chart fixture re-blessed. The chart's zoom to a planet's system, deferred by A.3b until a port stood on a moon, is not built though that trigger is met: **re-deferred pending the owner's call**, a date or a cut of its own, unscheduled (§5) | L | `tests/moon_ports.rs` (pure, no world): `a8_the_lane_carries_five_ports_and_six_direct_routes` (the graph exactly; no Luna → Ceres, no Stickney → Deimos, no Ceres ↔ Titan, by route and by listing); `a8_the_hop_plans_in_hours_on_the_real_route` (under 6 IC hours and at most 2 t at rate 1000, every four IC days over forty); `a8_titan_fits_the_tank_only_throttled` (at the epoch: TANK_SHORT at full throttle, fitting at 450 with the 15 ‰ margin, two souls over the locker past sixty days); `a8_titan_throttle_fits_a_saturn_year` (53 departures a leg with the full tank: the tank and locker at 450 and the tank at 465, each leg's ceiling and IC-day range printed; Ceres → Titan fitting 465 at the epoch and failing it in a sampled season); `a8_the_ring_outranks_the_shuttle_by_ordering_at_the_epoch` (every load solved by `support/vertex.rs` and asserted the both-limits vertex; ring > reference > an empty Luna-and-back). `tests/commerce.rs`: `a8_the_moon_ports_stock_list_and_bid` (ten stock rows minted once, the three bid-only vendors Ready at 0 of 0, the reference rows live) and `a8_a_world_on_lane_v7_takes_v8_by_the_shelf` (v8 over a v7 world without drift). `tests/capacity.rs`'s `ic8_the_posted_prices_make_the_both_limits_load_optimal` (eight pairs from the listed routes: seven whole-tonne vertices, the hop with none). `tests/passage.rs`: `a8_sibling_moons_are_measured_in_the_parents_frame` (passes five and twenty within 600 s both ways) and `plan_passage_reproduces_the_slice_figures` (every route's `base_s` exactly, the hop's 131 s). `tests/lane.rs`'s `lane_legs_derive_at_epoch` (every route within 100 s, Titan against its own propagated row). `tests/price.rs`'s `a8_the_old_ports_suppliers_move_as_designed` (§6's old-port rows, v8 against v7). `tests/sale.rs`: `a8_the_programme_reference_names_the_moon_asks` and `a8_without_the_boards_reference_row_the_sale_stamps_the_baseline_memo`. `tests/voyage.rs`'s `a8_the_plan_names_the_throttle_that_fits` (a full-hold Ceres → Titan plan at the epoch on a seeded lane prints one `Throttle:` line, none at the throttle it names, none on Ceres → Stickney) and the bisection's unit test `a8_the_highest_fitting_throttle_is_found_by_bisection` (`src/port/voyage.rs`). `tests/stations.rs`: the ten packs, `ic1_the_seams_are_walled_on_the_map` over five stations, `ic1_every_cross_deck_portal_stands_inside_both_rooms_and_clear_of_restricted_ones`. The pinned Ceres–Stickney legs unmoved |

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
  **Note (cut A.8, 2026-10-19).** The default stands for Ceres–Stickney, and its rationale is
  narrowed in §4.6: the charter binds the depot call on Ceres-bound legs and on the Ceres–Mars
  corridor, not on a Concord-bound leg out of Ceres, so `ceres_aitken` flies direct (Q-19-14 holds
  the via-Kirkwood alternative).
- **Q-19-4 The throttle floor.** Default 250‰ (a passage at most twice its full-thrust length).
- **Q-19-5 The price policy's caps.** Default: premium `k = 300‰` per unit of reach ratio, cap 500‰
  (cut A.4, lane v5's `premium` block on both ports; a lane data edit); the emergency ratio 1,500 ‰,
  life-support goods = provisions, volatiles, propellant, 100 units deep, posted by the Exchange at
  Ceres and the Yards at Stickney (cut A.4b, lane v6's `emergency` block; a lane data edit).
- **Q-19-6 The keep-out radius.** Default 0.3 AU (Mercury's orbit is 0.39 AU).
- **Q-19-7 The chart panel.** Default: text first (A.3), the panel as A.3b after A.6.
- **Q-19-8 Re-fitting the elements.** Default: one set per body for 2340–2400; a second era is a lane
  data change when the campaign nears 2400 (about 2,000 real days at rate 24 from 2351).
- **Q-19-9 Moons as bodies.** Default: not yet (Stickney names `mars`); Luna and Titan with a port.
  **Ruled by the owner 2026-10-19**: Luna, Phobos, Deimos and Titan are bodies — "the moons are
  really where the commerce is going to happen" — built as cut A.7 (§3.6, D-19-17); Stickney stands
  on Phobos. **Closed by the owner's ruling and cut A.8 (2026-10-19)**: the ports stand — Aitken
  Highport on Luna, Voltaire Anchorage on Deimos, Ligeia Terminal on Titan (§3.8, D-19-19).

**Q-19-1 through Q-19-8: the defaults above were accepted as proposed by the owner on 2026-10-19.**

Opened by cut A.8 (2026-10-19), each default taken by the build. These five are the ones this
document carries; the full owner list for the cut is O1..O16 in the A.8 cut note
([18-cut-notes.md](18-cut-notes.md) "What the owner decides"), which also holds the Titan word and
its per-route alternative, the sibling frame, the eight-route graph, the `proceeds.rs` demotion,
Deimos's polity, the names, Ceres's stock resize and the Reserve's ice floor:

- **Q-19-10 The new goods.** Default two rows, `instrument_boards` and `carbon_feedstock` (D-19-20),
  the Luna good under `instrument_boards` so `tacsim.electronics` stays doc 17 §16's die lots.
  Alternatives: the same row under the id `electronics` (the die lots then take another id), or a
  third row `composites` (1,900 L, reference 800) for a richer Titan backhaul. A goods-table edit.
- **Q-19-11 The moon ports' prices.** Default Luna's bids a shade under Kasei's, concentrate 270 and
  ice 195 (Luna earns less a tonne than Stickney and more per IC day on about three departures in
  four; the shuttle lives on Luna's finite intake, the capital the ring needs and the reference
  return), and Titan's at frontier prices, alloy 760 and boards 1,500 (the Titan shuttle about 1.5×
  the reference per IC day over a Saturn year). Alternatives: Luna at 260 / 185 (it wins per IC day
  on about half the departures) or over Kasei's at 290 / 210 (it dominates the shuttle until its
  40-load intake fills); Titan richer at 850 / 1,700 (about 3×). Lane data.
- **Q-19-12 Titan's polity and faction words.** Default the Concord's Saturn Charter of 2291 with
  `company` for the Company's houses and `townships` for the Board (D-19-24). Alternatives: a
  Registry grant with the word `concession`, or the Board under `concord` (the charter it petitions
  for) if a fifth word is unwanted. Lane and doc 11 data; read by nothing until the faction cut.
- **Q-19-13 The shipped lane's tank-short seasons.** Default: the `Throttle:` line only (§4.3,
  D-19-22) — the plan names the throttle that fits and the captain files it. Alternative: an
  auto-throttle on a queued depart, the highest throttle that fits chosen when the asked one is
  TANK_SHORT, as a follow-up cut once the engine's figures confirm the design's.
- **Q-19-14 Ceres → Luna direct.** Default direct (§4.6's narrowed rationale). Alternative:
  `ceres_aitken` by `[kirkwood_depot, aitken_highport]` — 77 t in the worst season, inside the tank,
  about 0.7 IC days longer — with §4.6's earlier unqualified sentence kept, and Luna's reach and
  premium figures and the ring's circuit re-derived by about a percent. Lane data.

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
