# 49. Combat content — the armaments, the rounds and the shot, surveyed

Draft 0.5, 2026-11-15, docs only. Surveyed against `origin/main` at `b1b4c57f6` by one read-only
lane, with the lead spot-checking the cycle figures, the cycle floor, the card presets, the slot limit
and the laser recharge. Anchors are file names with line numbers; line numbers drift. Draft
0.1, the same day, was the survey.

**Draft 0.2, the same day, records the owner's first rulings**, made from play-testing in the arena:
"the shots happen too fast".
- **Q-49-2:** the cycle floor is **5 s**.
- **Q-49-3:** the cannon cycles at **15 s**, the coaxial gun at **5 s**, the autocannon at **10 s**,
  and the missile rack stays at **30 s**.
- **Q-49-5:** GC.7's prices stay. In the owner's words: "scarcity comes from magazine size, we dont
  want to discourage fighting".
- **Q-49-7:** accepted at its default.

The 5 s floor sits above two figures §5 proposed: the ground pulse laser's 3 s and the space offence
laser's 4 s. They are raised as Q-49-19 (§6.5).

**Draft 0.3, the same day, records Q-49-19 at its default:** one 5 s floor for every mount a player
or mind fires, on both kernels. The pulse laser and the space offence laser cycle at 5 s.

**Draft 0.4, the same day, records that every other question is accepted at its default as
written.** These are Q-49-1, Q-49-4, Q-49-6 and Q-49-8 to Q-49-18. Every question in this survey is
now ruled. What the defaults settle:
- **The trigger:** a condition slot stays, but must carry a judgement term (Q-49-8). The Line card's
  floor is 35% with a 25% reserve.
- **Rounds are produced at the works** (Q-49-4). Read with Q-49-5's ruling, the recipes are sized so
  a produced round costs about its GC.7 price.
- **Lasers are the anti-drone fit on the ground** (Q-49-6), and space content moves to data (Q-49-11).
- **Space magazines persist** and are rearmed at a berth (Q-49-12).
- **One re-bless per kernel**, with tests reading content (Q-49-16), and the bot is held to
  discipline (Q-49-17).

**Draft 0.5, the same day, folds in an outside review** of the PR at `8ddf6ca9b` (Astra), triaged in
[doc 48a](48a-outside-review-triage-2026-11-15.md). Every finding held.
- **Corrected in place**, marked **Correction (outside review)**:
  - §0, §3.2 and §4: laser recharge costs labour, not nothing.
  - Q-49-8: a standing slot is standing authorisation, not a single decision.
  - Q-49-14: choosing a round is engine work, not content.
- **Follow-up questions**, Q-49-20 to Q-49-24 (§6.6), open:
  - standing or single-use slots;
  - the round choice narrowed;
  - magazines sized by endurance;
  - when a reserve is spent;
  - laser modes.
- **§7** gains a round-conservation requirement, and CC.7 a supply gate.

The rulings stand until their follow-ups are answered.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) §5 and
[doc 44](44-minds-wired-through-the-game.md) used. It sets out what exists, then asks the owner
questions (§6), each with a proposed default, an alternative and a reason. **Provisional** marks a
number to be tuned later, under the 2026-10-24 rule that economy and content defaults are taken and
tuned, never waited on. **(expensive to reverse)** marks an answer that would take a significant
refactor of the codebase to undo. §7 sketches units of work; they are cut only after the rulings.

**Wording.** Under the owner's rule of 2026-11-02, this document describes combat at the level of
game mechanics. It talks about mounts, rounds, cycles, magazines and the combat kernel. It does not
walk through how a hit is resolved. Content file paths keep their own names.

## 0. In brief

1. **The placeholders are still live.** The owner ruled on 2026-10-02 that the shipped armaments are
   placeholders. The coaxial gun still cycles every 1 s and the space offence laser every 1 s; point
   defence fires every 0.5 s. The engine's floor for a cycle is 1 s (`CYCLE_FLOOR_MS`,
   `crates/tacsim/src/definitions/weapon.rs:24`), so content can never ask for less, but the floor
   itself is the placeholder's.
2. **There are two combat kernels, and only one of them is data.**
   - The ground kernel serves the simulator and the IC field alike. Its content is in data packs:
     five mount packs and five round packs.
   - The space kernel is separate, and so is its content. Mounts are Rust structs with "standard"
     constructors, and which hull carries what is a Rust `match`.
3. **Ammunition is a good on the ground, but nothing makes it.**
   - GC.7 added four munitions goods. Rearm draws them from a hold and charges a per-round fee.
   - No recipe outputs a round; vendors hold finite seeds.
   - Laser recharge draws no good. It costs only labour and time: 0.25 cr a charge and 30 IC minutes
     (corrected after the outside review, doc 48a A-8).
   - Space has no munitions goods and no rearm, and its magazines appear to refill each time a ship
     is checked out into a bubble.
4. **The player's decision support is mostly built on the ground.** `CONTACTS`, the mount status
   rows, the TUI cockpit and the pixel cockpit show the odds, the aspect and the rounds left.
   `STATUS` does not show rounds. Space minds read placeholder constants.
5. **One question needs a ruling before content can be tuned: is a condition slot the decision or
   the trigger?**
   - `FIRE <mount> WHEN …` arms a slot that fires on its own at the mount's ready moment while its
     terms hold.
   - The stock mind arms one slot per mount at a 10% odds floor and keeps no reserve.
   - The external bot fires every gun at full rate.
   - Under the owner's "no auto-fire" direction, that is the question this survey cannot settle
     alone.
6. **The setting says lasers are anti-drone, but the content does not follow it.**
   - The lore rule: lasers are extremely effective against drones and weak against hard targets.
   - Yet the ground has a laser tank and a laser biped.
   - In space every ship, the starter haulers included, carries offence lasers.
7. **Retuning the ground is cheap in authoring but not in re-blessing; space needs code.**
   - Ground numbers are JSON and staff can edit them in the Catalog Studio.
   - A retune moves the content signatures, the arena harness baseline, two corpus pins, roughly 15
     to 25 test files that read readouts literally, the bot's scripted rig, a help page and the price
     notes.
   - Space needs Rust edits, including two hard-coded maxima in the mind view.

## 1. Why this document exists

On 2026-10-02, after the bot cuts AJ to AO, the owner ruled that the armaments shipped with the
simulator are placeholders: "A machine gun you have to fire every 1 second is not fun." The
direction the owner set has three parts:

- typical fire cycles of several seconds;
- ammunition made more precious, so a player waits for the good shot instead of firing each time
  the timer lapses;
- no auto-fire and no fire-when-ready queue.

What the engine should support is the decision: the odds for a contact, the target's aspect and the
ammunition left. It should not support the trigger.

Since then the programmes have built on the placeholder content. These include the IC field (docs 27,
32, 36, 40), the minds (docs 17, 44, 45), space (doc 35) and the bay's rearm (GC.7). None of them
changed it. On 2026-11-15 the owner asked for this survey beside [doc 48](48-economy-and-production.md),
the economy and production survey. Rounds are goods and their supply is a production question, so
the two documents meet at the munitions recipes (Q-49-4, Q-48-23).

## 2. What is in scope

**In:**
- every mount and round shipped in the simulator, the IC field and space;
- ammunition as a good: its price, its supply and its resupply;
- what a player and a mind can see and decide before a shot;
- shot discipline in the stock mind, the card presets, the shipped Rhai doctrines and the bot;
- the content format and what a retune costs.

**Out:**
- how the kernel resolves a shot. Doc 16 Q33 stands: odds in per-mille shown as a percentage, fixed
  damage, face location, blocked shots cost like any other;
- new unit classes beyond what the content needs;
- perception (the Epistemic Engine);
- the 10 km ground (D-16-5).

## 3. What exists

### 3.1 Where the content lives

**Ground.**
- Mounts are in `crates/tacsim/content/weapons/*.json` (five packs) and rounds in
  `content/ammunition/*.json` (five packs). The schema is in `src/definitions/weapon.rs` and
  `src/definitions/ammunition.rs`.
- A mount pack carries its delivery, its section and face, two range bands, `cycle_ms`, burst size,
  flight speed and heat per shot.
- A unit fits a mount as a component with a mount role (`units/line_tank.json:90-108`; the format
  is [doc 22](22-unit-kit.md)).
- The mount *parts* are goods (`market/goods.json:221-273`) built by recipes
  (`industry/recipes-v2.json`).
- `industry/hulls-v2.json` carries no mount data.

**Space.**
- Mounts are Rust structs with standard constructors (`src/space/weapons.rs:57`, `:94`, `:148`,
  `:210`: twin tubes, turret laser, spinal gun, rotary point defence).
- The fit of each hull is a Rust `match` (`src/space/custody.rs:79-246`).
- `raider_heavy` has no unit file; it is a Rust overlay over the corvette (doc 35d).

**The shipped palette**, with cycle, magazine and the two range bands as content gives them:

| Unit | Where it fights | Mount | Cycle | Magazine | Bands | Content says it is for |
|---|---|---|---|---|---|---|
| `line_tank` and its `_ecm`, `_eccm`, `_thermal` variants | simulator; IC field (raiders, `field/raiders.json`) | 105 mm cannon | 6 s | 40 HE | 1.5 / 3 km | the main gun against armour |
| same | same | coaxial gun | **1 s**, bursts of 10 | 2,000 (200 bursts) | 0.6 / 1.2 km | infantry and drones; wears tracks down |
| `line_tank_laser`, `biped_mk2` | simulator | pulse laser + coaxial gun | 2 s, three shots then a heat wait | 500 charges | 1.2 / 2.4 km | the pack note: "what you wear a tank down with" |
| `scout_mk1` | simulator | 30 mm autocannon + coaxial gun | 3 s, bursts of 3 | 240 (80 bursts) | 1 / 2 km | the scout's main gun |
| `heavy_mk1` | simulator | cannon + coaxial gun + missile rack | rack 10 s | 6 | weak inside 0.6 km / 3 km | armour; slow flight, a mover can walk out from under it |
| `biped_mk1` | simulator | cannon + coaxial gun | as above | as above | as above | |
| `tv_escort` | IC field (sold on the lane) | cannon + coaxial gun | as above | as above | as above | the escort |
| `corvette`, `corvette_interdictor` | space | twin torpedo tubes | 8 s reload, two-tube salvo | 8 | 10–500 km | strike |
| same | space | laser, offence mode | 1 s, heat per shot | none (heat) | to 160 km (corvette) / 500 km | offence |
| same | space | laser, defence mode | automatic | none | 45 km | stops torpedoes |
| same | space | spinal gun | 3 s | 24 | to 250 km | point defence cannot stop it |
| same | space | two point-defence rotaries | **0.5 s, automatic** | 3,000 each | 15 km | stops torpedoes |
| `raider_heavy` | space | two offence lasers, tubes, spinal gun, two rotaries | as above | 8 / 24 / 3,000 | as above | |
| `halcyon` | space | two lasers, tubes, spinal gun, four rotaries | as above | 16 / 32 / 4,000 | | |
| `dray`, `dray_p` | space | two **offence** lasers, two rotaries | 1 s / 0.5 s | none / 2,000 | | the starter haulers (`custody.rs:204-244`) |

**Unarmed units.**
- Ground: `drone_mk1`, `target_mk1`, the Dray and Dray-P as carriers, the prospector, tender and
  wrecker, both Tycho units, `conduct_carrier`, `relay_station`.
- Space: the collector drones.

### 3.2 Ammunition

**Ground.**
- Each mount has an ammunition cell and a ready-at cell on the unit, so rounds persist between
  sorties (`src/match_/cockpit/weapons_fire.rs:1434-1437`).
- `+bay/order rearm` fills what is short (`src/port/bay_work/jobs.rs:80-91`). It can be done at a
  workshop, or in a carrier's bay anywhere at twice the time. It draws the rounds from the carrier's
  hold, all or nothing, and charges a per-round fee from the tariff.
- A laser's "recharge" is a cell, not a good (`jobs.rs:86`). **Correction (outside review, doc 48a
  A-8):** it is not free. It is charged at the tariff's `cost_per_ammo_round` (25 minor a charge) and
  takes `duration_ic_rearm_component` (1,800 IC s) like any rearm (`jobs.rs:430-433`). A full
  500-charge refill is 125 cr before the operator's multiplier.

**The goods.**
- GC.7 added four munitions goods (`market/goods.json:166-218`; doc 36): an AP round at 12 cr, a
  box of 100 ball at 25 cr, an HE round at 150 cr and a missile at 1,500 cr.
- They are carried as bulk cargo and stocked on two vendors' boards (lane `ceres_phobos.json`
  2803-2811 and 2895-2904).
- **No recipe makes any of them** (doc 48 §3.7 traces the production chain).

**Space.**
- Magazines only count down (`src/space/bubble.rs:2242`, `:2425`, `:2807`).
- The saved subsystem record keeps critical damage and stripped rotaries, but not magazines
  (`custody.rs:659-690`).
- A ship is built fresh from its Rust overlay at each check-out (`custody.rs:505-516`). The lane
  reads this as a refill of every magazine each time a ship enters a bubble. That reading is
  inferred from the code paths, not traced end to end.
- There are no space munitions goods and no space rearm.

**At zero.**
- In the simulator, a match where neither side can fire ends as a `draw` with reason `spent`
  (doc 16 §11; `arena/damage.rs:1693-1763`). That applies only under the Elimination end condition,
  so the IC field has no spent end.
- A dry stock mind withdraws (`arena/stock_mind.rs:434-437`).
- A turret that is lost keeps its rounds.

### 3.3 The player's shot

**Ground cockpit verbs** (doc 16 §3.7):
- `LOCK`.
- `FIRE [<mount>]`.
- `FIRE <mount> WHEN <terms>`: up to four terms over odds, aspect, range, band, rounds, reserve,
  identified and lock. It arms a condition slot.
- `FIRE <mount|ALL> OFF`; the `FIRE HOLD` / `FIRE FREE` master gate.
- `CONTACTS`, `RANGE`, `BEARING`, `SCAN`, and the mount status verb.
- Roster orders: `ORDER … ENGAGE` / `… HOLD FIRE`, and `FOCUS` (R-47-1).
- The postures `RETURN`, `GUARD` and `HOLD`, and `+ship/posture` in space.

**Acceptance.**
- A shot needs power, a lock, a ready mount and rounds.
- A blocked shot still spends its round and its cycle.
- A typed shot lands at the end of the pulse that accepts it. The bot measured landings on a 250 ms
  lattice (`crates/mush-bot/src/arena/pilot.rs:94-101`). The engaged pulse is 500 ms.
- The cadence tax: the lattice plus typing time is about half of a 1 s cycle and about a twentieth
  of a 6 s one. This is the mechanical reason behind the owner's ruling.

**What the screens show.**
- `CONTACTS` rows carry the target's aspect word and the main gun's chance as a percentage
  (`src/arena/view.rs:482-523`).
- The mount status rows show ready or cycling, rounds of the maximum, band and chance
  (`weapons_fire.rs:1392-1432`).
- The TUI cockpit shows the same with a colour ramp on rounds
  (`crates/mush-net-ssh/src/tui/cockpit_text.rs:620-662`), and so does the pixel cockpit
  (`crates/mush-pixel/src/cockpit.rs:1051-1064`).
- **`STATUS` shows no rounds** (`weapons_fire.rs:909-999`).

**Space verbs.**
- `+tac/lock`, `+tac/beam`, `+tac/torp [=count]`, `+tac/railgun`, and
  `+tac/pdc auto|manual|point|offense` (`src/space/commands.rs:130-134`).
- Automatic point defence and defence-mode lasers fire on their own (D-35-8).
- The lane did not check what the player's space cockpit shows before a shot.

### 3.4 The machines' shot

**The stock mind (Standard Doctrine).**
- Once it has a contact and passes its gates, it arms one condition slot per mount, up to the slots
  its class gives it (two at Class I) (`stock_mind.rs:2023-2060`).
- It plans a direct shot only beside a fresh lock. After that, the armed slot owns the shot and
  fires at each ready moment while its terms hold (doc 17 §5).
- A mount past the slots does not fire (`stock_mind.rs:1918-1928`).
- The Rhai port, `content/minds/standard.rhai`, matches it.

**The card presets** (`src/arena/card.rs:428-473`, checked):

| Card | Odds floor | Reserve | Other |
|---|---|---|---|
| **Line** (the default) | 10% | none | every aspect |
| Sentry | 55% | | screens, waits to open |
| Sniper | 65% | 30% | holds, stops when unseen |
| Skirmisher | 35% | | orbits, flank aspect only |
| Hunter | 30% | | hunts after 45 s |
| Rearguard | 50% | | breaks off at 70% |

Nothing may go under the compact floor of 10%. The card is where shot discipline lives today; the
default carries almost none.

**The Rhai mind ABI** gives `lock`, `fire(mount, slot)`, `arm`, `disarm`, `slots` and
`stock::fire_discipline` (`src/minds/mod.rs:1337-1353`).

**The space doctrines.**
- The belt raider opens with every ready mount and two torpedoes, then presses with everything
  except torpedoes (`content/minds/belt_raider.rhai:49-70`).
- The interdictor fires its laser every think and torpedoes only from a full magazine.
- The picket fires its laser whenever it is ready.

**Absent units.**
- `arena::absent::permits` is the one gate on the ground and in space. An absent unit never fires
  first, and answers only a unit whose fire reached it; `GUARD` widens that to fire on the same
  player's other units.
- A standing hold-fire order still binds (docs 45 and 47).

**The external bot** fires every gun at full rate on its own clock, and its tests assert that it
does (`crates/mush-bot/tests/arena_lane.rs:2296-2360`).

### 3.5 Damage, at the level of the game

**Ground.**
- A ground unit is made of sections, each with armour per face and internals. The face a round
  arrives on picks a section.
- Damage wears the armour, then the internals. When a section's internals are gone its consequence
  applies: the unit is wrecked, the section's mounts are lost, the unit slows or cannot turn or stop,
  or the core is degraded.
- A hit that leaves its section standing may jam or damage a part. Doc 40 FR.5's effects apply
  (`arena/part_effects.rs`):
  - a blind sight or a shorter-sighted one;
  - a pod that goes off;
  - a plant that goes off or runs cooler;
  - a core that thinks at a lower class;
  - a mount that leaves the fire table but keeps its rounds.
- The chances of these are per unit in content, with a compiled default that no shipped unit
  overrides.
- Each round does one fixed amount of damage. Its penetration class is carried and read by nothing
  (`definitions/ammunition.rs:4-8`).

**Space.** It applies damage by its own proportional splitter, with hard thresholds for point-defence
and sensor loss (doc 35d).

### 3.6 What is pinned to the placeholder numbers

- **Content signatures.** `DIGESTS` pins 15 packs (`tacsim/tests/domain_arena/content.rs:105`).
  Each unit pack names the versions it needs, so a round-pack edit re-signs the pack, every unit that
  needs it, and the pin.
- **The arena harness baseline** (`tests/harness_baseline/arena.linux.json`) has `rounds_spent`,
  `shots` and `cond_fires` leaves.
- **Corpus pins.** `M4_5_POOLED_RECORD` and `M2_CROSS_PLATFORM_PINS` move when content-backed hulls
  move. `DUEL_HASH_AT_HEAD` is hand-built and probably does not.
- **Readouts read literally.**
  - "40 of 40" and "2000 of 2000" appear in `crates/mushd/tests/sim_lenses/` and the cockpit tests.
  - 29 files mention "cycling (".
- **The bot.**
  - Its scripted server hard-codes the cannon at 6,000 ms and everything else at 1,000 ms
    (`arena_lane.rs:605-607`).
  - Cuts AL, AN and AO left full-rate and refusal-rate assertions and the lattice constants
    (`mush-bot/src/arena/picture.rs:179-277`).
- **Elsewhere.**
  - The goods' prices are sized to load sizes (the notes in `goods.json`).
  - Rearm is tested in `domain_field/bay_gc8.rs`, `bay_36a.rs`, `domain_economy/orders.rs`,
    `yard_in9.rs` and the composed IC slice loop.
  - The help page `docs/help/cockpit.md` and the web cockpit fixtures quote the numbers.
- **Naming.** About 45 files outside the content directory name a mount or round id, and about
  1,125 `"cannon"` literals across the crates, mostly tests using it as a component id. Renaming
  mounts would ripple far further than retuning them.

### 3.7 Ground and space

The ground's simulator and IC field share one arena kernel, one task per site through the same seam.
Space has its own kernel. Its fire paths are in `space/bubble.rs` (around `:2194`, `:2283` and
`:2379`), its damage in `SpaceShipState::apply_damage`, and its mounts in Rust.

What they share:
- the Slow consequence;
- the disable test;
- the face bands;
- the sections, drawn from the tactical unit at check-out;
- the absent gate.

## 4. The gaps against the owner's direction

1. **The cycles.** Several mounts cycle at 1 s or 0.5 s, and the floor allows 1 s.
2. **The trigger.**
   - Condition slots fire on their own once armed.
   - Doc 17 §5 argues a slot is a written decision, not doc 16c Q45's fire-when-ready trigger (which
     was not built).
   - But with the default card's 10% floor and no reserve, a slot behaves like one. Doc 17 itself
     warns that slots "will out-shoot typed `FIRE`" under the placeholder cycles.
3. **Scarcity.**
   - No round is produced.
   - Laser recharge costs labour only (125 cr for 500 charges), and no good.
   - Space magazines appear to refill.
   - The default card keeps no reserve.
   - Rounds are cheap against a sortie's earnings: a full HE load for the cannon is 6,000 cr.
4. **The lore.** Lasers are used against armour on the ground and as offence on every ship.
5. **Variety.**
   - Each mount names exactly one round and offers no loadout choice.
   - Damage is one figure and penetration does nothing.
   - Every ground mount sits on the turret's front face.
   - Five ground mounts; four space mount kinds with one constructor each.
6. **Decision support.**
   - Ground: odds, aspect and rounds are shown, except on `STATUS`.
   - Space minds: a constant chance, a fixed reach and a front aspect (`src/space/view.rs:335-336`,
     `:477-478`). The space mind view also hard-codes the maxima 8 and 24 (`view.rs:356`, `:365`).
7. **Authoring.** Only the ground has a designer-editable format. Space mounts and `raider_heavy`
   live in Rust.
8. **Mount count.** The scale target allowed 2 to 10 mounts a unit. Shipped ground units carry 2 or
   3, Class I gives two slots, and the stock mind cannot fire a mount past its slots.

## 5. The shape proposed

The proposal in one paragraph:
- Settle the trigger question first.
- Then retune the ground as data in one re-bless, with longer cycles, smaller magazines, a higher
  floor for anything a player decides, lasers as the anti-drone fit, and rounds on `STATUS`.
- Make rounds a produced good with a real price; doc 48 places the recipe.
- Move space's mounts into the same content format, then give space the same treatment: magazines
  that persist, a rearm from munitions goods, and lasers per the lore.
- Retune the stock mind's default card and the bot to the new palette.
- Variety (a second round type for the main gun, a reading for penetration) comes after, as content.

**The ground palette.** The cycles are ruled (Q-49-3, 2026-11-15). The magazines are as proposed and
provisional:

| Mount | Cycle now → ruled | Magazine now → proposed | Note |
|---|---|---|---|
| 105 mm cannon | 6 s → **15 s** | 40 → **24** HE (six minutes of fire) | the main decision of a duel |
| coaxial gun | 1 s → **5 s**, bursts of 10 | 2,000 → **600** (60 bursts) | anti-drone and soft targets |
| 30 mm autocannon | 3 s → **10 s**, bursts of 3 | 240 → **120** (40 bursts) | the scout's main gun |
| missile rack | 10 s → **30 s** | 6 → **4** | rare, decisive |
| pulse laser | 2 s → **5 s** (Q-49-19, ruled) | charges as now | anti-drone only (Q-49-6) |

**A provisional space palette** (Q-49-12), once space is data:

| Mount | Proposed | Note |
|---|---|---|
| twin torpedo tubes | 8 s reload as now; magazine persists, rearmed at a berth | strike |
| offence laser | **5 s** (Q-49-19, ruled); short reach; weak against a hull | point defence and soft targets only (Q-49-7, ruled) |
| spinal gun | 6 s; 24 rounds persist | the hull-killer |
| point-defence rotaries | 0.5 s automatic as now; 3,000 rounds persist | automatic by ruling D-35-8 (Q-49-10) |

## 6. Owner survey

**First rulings 2026-11-15:**
- Q-49-2 and Q-49-3 are amended (the 5 s floor; cycles of 15, 5, 10 and 30 s).
- Q-49-5 takes the alternative (GC.7's prices stay).
- Q-49-7 is accepted at its default.

Each ruled question carries its decision first, with the proposed text kept under it for the record.
§6.5 holds Q-49-19, raised by the floor and accepted at its default the same day (Draft 0.3).

**Ruled 2026-11-15 (Draft 0.4):** every other question, Q-49-1, Q-49-4, Q-49-6 and Q-49-8 to Q-49-18,
is accepted at its default as written. Their proposed text below is the decision.

### 6.1 Order and the trigger

**Q-49-1 — Order of work.**
- **Proposed default:** settle Q-49-8 (the trigger) first. Then the ground retune as data in one
  re-bless. Then space's content format, then the space retune. Munitions production rides doc 48's
  production units.
- **Alternative:** retune both kernels together.
- **Why:** the ground is data today and the space format move is a refactor; one re-bless per kernel
  keeps each diff readable, and the trigger rule decides what the retune is for.

**Q-49-2 — The cycle floor.**
- **Decision (owner ruling, 2026-11-15):** raise the floor further, to **5 s** to start. From
  play-testing in the arena: "the shots happen too fast". My reading, not the owner's words: the
  automatic-defence class the default proposed stands, and the owner changed only the number. Point
  defence and defence-mode lasers keep their own floor (Q-49-10). At 5 s the cadence tax (the 250 ms
  lattice plus typing time) is about a twentieth of a cycle.
- **Proposed default (as written):** raise the engine floor for a mount a player or mind fires from 1 s to
  **3 s** (provisional). Add a content class for **automatic defence mounts** (point defence,
  defence-mode lasers), which keeps its own floor because nobody types them.
- **Alternative:** keep the 1 s floor and change only the content.
- **Why:** the floor is what stops a content edit from bringing back a 1 s gun. The cadence tax is
  the mechanical reason the owner gave, and at 3 s it is under a tenth of a cycle.

**Q-49-3 — The ground palette.**
- **Decision (owner ruling, 2026-11-15):** the cannon cycles at **15 s**, the coaxial gun at
  **5 s**, the autocannon at **10 s**, and the missile rack stays at **30 s**. My reading, not the
  owner's words: the magazines stay as proposed (24, 600, 120 and 4), since Q-49-5's ruling puts
  scarcity on magazine size. §5's table carries the ruled cycles. The pulse laser is Q-49-19's.
- **Proposed default (as written):** §5's table, provisional: cannon 8 s and 24 rounds, coaxial gun 4 s bursts
  and 600 rounds, autocannon 5 s and 120, missile rack 30 s and 4.
- **Alternative:** keep the cycles and cut only the magazines.
- **Why:** both halves of the direction (several-second cycles, precious rounds) are needed. The
  cycle alone keeps the old fight at a slower pace; the magazine alone keeps the cadence tax.

**Q-49-8 — Is a condition slot the decision or the trigger?**
- **Proposed default:** keep `FIRE <mount> WHEN …`, because it is the player writing the decision
  ahead, and the absent-player and mind work stand on it. But:
  - a slot must carry at least one judgement term beyond readiness (an odds floor or an aspect);
  - no slot may arm below the card's floor;
  - the Line card's floor rises from 10% to **35%** with a **25% reserve** (provisional).
  
  No bare fire-when-ready then exists for a player or a mind.
- **Alternative:** remove condition slots from the player's cockpit and keep them only for minds.
- **Why:**
  - A slot with a real threshold is "wait for the good shot" written down.
  - A slot with a 10% floor and no reserve is the auto-fire the owner ruled out.
  - Removing slots from players would make the stock mind out-shoot every human, which doc 17
    already warns of.
- **Correction (outside review, doc 48a A-7):** "no bare fire-when-ready" is true, but it is not "no
  auto-fire". A slot on a target that stays at 80% odds clears a 35% floor at every ready moment, and
  fires each time with no new decision. A threshold improves shot discipline; it does not remove the
  standing trigger. The ruling (Draft 0.4) stands. The honest choice, standing or single-use
  authorisation, is Q-49-20.

**Q-49-9 — The stock mind's shot discipline.**
- **Proposed default:** the default card keeps its reserve for answering fire, and the stock mind
  raises its odds floor as the magazine falls below half (one rule, readable in the card's text).
  The Rhai port follows.
- **Alternative:** discipline only through the presets, as today.
- **Why:** with scarce rounds, a mind that spends its magazine on 10% shots is dry at the first real
  contact. A rule the player can read keeps the mind's behaviour predictable.

### 6.2 Scarcity and supply

**Q-49-4 — Rounds are produced.**
- **Proposed default:** each munitions good gets a recipe at the works, from structural alloys,
  carbon and volatiles (provisional bills; doc 48 Q-48-23 places them in the chain). The vendor stocks
  stay as the starting seed.
- **Alternative:** keep rounds as a finite vendor good with a daily refresh.
- **Why:** a finite seed is either exhausted or refilled from nowhere. A recipe gives rounds a price
  that follows its inputs, and gives volatiles their first industrial buyer.

**Q-49-5 — How precious a round is.**
- **Decision (owner ruling, 2026-11-15):** **keep GC.7's prices**: AP 12 cr, a box of ball 25 cr,
  HE 150 cr, a missile 1,500 cr. "Scarcity comes from magazine size, we dont want to discourage
  fighting." A full cannon load of 24 HE is then 3,600 cr. Q-49-4 was accepted (Draft 0.4), so on my
  reading the munitions recipes are sized so that a produced round costs about its GC.7 price.
- **Proposed default (as written):** until the recipes set the price, double GC.7's prices (provisional): HE
  300 cr, AP 24 cr, a box of ball 50 cr, a missile 4,000 cr. A full main-gun load then costs about
  7,200 cr against the new 24-round magazine.
- **Alternative:** keep GC.7's prices; let the smaller magazines alone make rounds scarce.
- **Why:** a round should cost enough that a missed 10% shot is felt, without making a sortie lose
  money. The number is provisional and expected to move once doc 48's prices exist.

**Q-49-12 — Space magazines persist and are rearmed.**
- **Proposed default:**
  - Magazines are saved on the unit across check-outs, as on the ground.
  - A space rearm at a berth draws from new munitions goods: torpedoes, spinal rounds and
    point-defence boxes.
  - This needs room in the goods catalogue: `MAX_GOODS` is 48 with two slots left. Doc 48 raises it (Q-48-30).
- **Alternative:** keep magazines refilling at each check-out.
- **Why:** a free refill makes every space fight's ammunition infinite across fights. The direction
  is scarcity, and space is where strike rounds are most decisive.

**Q-49-10 — Automatic defence mounts.**
- **Proposed default:** point defence and defence-mode lasers stay automatic (D-35-8 stands). They
  are exempt from the no-auto-fire direction because they answer incoming rounds, not contacts.
  Their magazines become finite and rearmed under Q-49-12.
- **Alternative:** make point defence a player decision too.
- **Why:** nobody can type fast enough to stop a torpedo. The direction was about the player's
  shot.

### 6.3 The lore and variety

**Q-49-6 — Ground lasers are the anti-drone fit.**
- **Proposed default:** a laser does little against an armoured face and is very effective against
  drones and soft targets. Every ground unit may carry one; none relies on one for armour.
  `line_tank_laser` and `biped_mk2` keep a projectile main gun beside the laser. The pack note that
  calls the laser "what you wear a tank down with" goes.
- **Alternative:** keep the laser variants as they are, as a simulator-only curiosity.
- **Why:** the owner's lore rule (lasers are anti-drone, projectiles stay primary) is one of the
  world facts chosen to bound the engine. A tank that kills tanks with a laser undoes it.

**Q-49-7 — Space lasers.**
- **Decision (owner ruling, 2026-11-15):** the default, accepted as written.
- **Proposed default:** the same fact holds in space. An offence laser has a short reach and does
  little to a hull; it is for drones, collectors, torpedoes and soft targets. The starter haulers
  (Dray, Dray-P) carry defence mode only.
- **Alternative:** space is different (no atmosphere, long ranges), and offence lasers stay as built.
- **Why:**
  - One setting fact instead of two keeps both kernels' content in one rule.
  - The owner's stance that "there should be lots the Dray can't do, it's the starter ship" argues
    against a hauler fighting at 160 km.
- **(expensive to reverse)** only in the sense that every space doctrine is written around lasers.
  The shipped belt raider, interdictor and picket would be rewritten.

**Q-49-11 — Space content as data.**
- **Proposed default:** move space mounts and hull fits into the content pack format, with a schema
  beside the ground's, editable in the Catalog Studio. `raider_heavy` becomes a unit file. The space
  mind view reads its maxima from content instead of 8 and 24.
- **Alternative:** keep space in Rust and tune its constants.
- **Why:** the direction needs designer-editable content, and two kernels can share a content format
  without sharing a kernel. **(expensive to reverse)**

**Q-49-13 — Decision support.**
- **Proposed default:**
  - `STATUS` shows the rounds of each mount in one short field.
  - The player's space cockpit shows odds, aspect and rounds like the ground's (to be checked first).
  - Space minds read the real values instead of constants.
- **Alternative:** ground `STATUS` only.
- **Why:** waiting for the good shot needs the player to see the odds and what is left at a glance.
  `STATUS` is the line a player reads most. The 120-column floor holds.

**Q-49-14 — Round choice per mount.**
- **Proposed default:**
  - The cannon gets a second round type (AP beside HE); a mount holds one type at a time, chosen at
    rearm.
  - The kernel's unused penetration class stays unread. Doc 16 Q33's fixed damage stands, and the
    rounds differ in damage and in which sections they reach.
  - Other mounts keep one round.
- **Alternative:** read penetration now, as a kernel change.
- **Why:** a loadout choice is a decision made at the bay, before the fight, at no kernel cost.
  Penetration would re-bless the kernel for a variety the content does not need yet.
- **Correction (outside review, doc 48a A-6):** "no kernel cost" is wrong.
  - A mount names exactly one round (`definitions/weapon.rs:111`), so a selected round is new state
    on the mount, with its own compatibility and reload rules.
  - The section a round finds is chosen from the target's profile with no round in the call
    (`arena/damage.rs:895`). "Differ in which sections they reach" therefore needs a resolver change,
    and a re-bless of what the tape replays.

  Q-49-21 narrows the proposal.

**Q-49-15 — Mounts per unit.**
- **Proposed default:** shipped ground units keep two to four mounts. Slots stay bound to the core's
  class. The "2 to 10 mounts" figure stays a capacity bound for the harness, not a content target.
- **Alternative:** widen classes to fire every mount.
- **Why:** with long cycles and scarce rounds, more mounts per unit adds decisions faster than a
  player can make them in line mode.

### 6.4 Cost of the change

**Q-49-16 — One re-bless per kernel.**
- **Proposed default:** the ground retune lands as one PR that re-signs the packs, re-blesses the
  harness baseline and the corpus pins, and changes the readout tests. Where a test asserts a
  magazine or cycle literal, it reads it from content instead, so the next retune moves only data.
  Space follows in its own PR after Q-49-11.
- **Alternative:** re-bless literals as they are.
- **Why:** the owner's direction says tune later. A test that pins a placeholder literal makes every
  later tune a re-bless of 15 to 25 files.

**Q-49-17 — The bot.**
- **Proposed default:** the bot's scripted rig reads cycles from content. Its full-rate assertions
  become discipline assertions: it fires only above its floor, keeps its reserve, and never spends a
  magazine on shots under the floor.
- **Alternative:** retune the constants only.
- **Why:** the bot tuned to the 1 s gun measures the placeholder, not the game. Doc 17 and the
  owner's ruling both say so.

**Q-49-18 — A spent end in the IC field.**
- **Proposed default:** none. A dry unit withdraws (the stock mind already does), and the field
  closes by its existing rules. The simulator's `spent` draw stays.
- **Alternative:** add a spent end to the field.
- **Why:** an IC field has several parties and carriers to rearm at. A draw word fits a duel, not a
  field.

### 6.5 Raised after the rulings

**Q-49-19 — The floor's reach: the two lasers under 5 s.**
- **Decision (owner ruling, 2026-11-15):** the default, accepted as written. One 5 s floor binds every
  mount a player or a mind fires, on the ground and in space, and both lasers cycle at 5 s.
- **The problem:** §5 proposed the ground pulse laser at 3 s and the space offence laser at 4 s. Both
  are under the ruled 5 s floor. A player or a mind fires both, so neither is an automatic-defence
  mount, and the floor binds them.
- **Proposed default:** one 5 s floor for every mount a player or a mind fires, on both kernels. The
  pulse laser and the space offence laser cycle at 5 s. The spinal gun (6 s) and the torpedo tubes
  (8 s) already clear it. Only automatic defence (point defence, defence-mode lasers) keeps its own
  floor.
- **Alternative:** make the ground pulse laser an automatic anti-drone mount that fires at drones on
  its own, exempt from the floor, like point defence.
- **Why:**
  - The lore makes the laser the anti-drone fit everything carries, and at 5 s it still deals with a
    drone in a pass.
  - One floor across both kernels is one rule to read.
  - The alternative widens the automatic class from "answers incoming rounds" to "answers a
    contact", which is the auto-fire the owner ruled out.

### 6.6 Raised by the outside review (doc 48a)

Each question answers a finding that held. Until it is ruled, the ruling it qualifies stands, as
corrected above.

**Q-49-20 — Standing or single-use slots** (A-7).
- **The choice:** Q-49-8 kept condition slots with a judgement term. A slot still *stands*: once
  armed, it fires at every ready moment while its terms hold, until it is disarmed. That is standing
  authorisation. The alternative is single-use: a slot fires once and disarms.
- **Proposed default:** single-use for a seated player, standing for a mind.
  - A player's `FIRE <mount> WHEN …` fires once, when its terms first hold, then disarms. Re-arming
    is the same command again.
  - A mind keeps standing slots under its card and the absent gate. That covers a stock or Rhai mind
    flying for a player, and a unit left on `RETURN`, `GUARD` or `HOLD`.
- **Alternative:** standing slots for everyone, as Q-49-8 now reads.
- **Why:**
  - With cycles of 5 to 30 s, re-arming costs one command a cycle, and each shot is then a decision
    the player made, which is the owner's direction.
  - A mind stands in for an absent or delegating player. Its standing slots are bounded by its card
    and, for an absent player, by the gate that never fires first.

**Q-49-21 — The round choice, narrowed** (A-6).
- **Proposed default:** defer the round choice. CC.11 leaves this programme, and the cannon keeps one
  round.
- **Alternative:** build the selected-round state now, with rounds that differ in damage only (no
  resolver change).
- **Why:**
  - A damage-only pair trades one number for another and is not a real loadout decision.
  - The version worth building, rounds that reach different sections, needs the resolver and a
    replay re-bless. It should be scoped on its own when content asks for it.

**Q-49-22 — Magazines sized by endurance** (A-13).
- **The problem:** with the ruled cycles, the magazines §5 proposed *raise* continuous-fire endurance:
  - cannon 240 → 360 s;
  - coaxial gun 200 → 300 s;
  - autocannon 240 → 400 s;
  - missile rack 60 → 120 s.
- **Proposed default (provisional):** size each magazine to an endurance shorter than today's.

  | Mount | Rounds | Endurance |
  |---|---|---|
  | Cannon | 12 | 180 s |
  | Coaxial gun | 300 (30 bursts) | 150 s |
  | Autocannon | 54 (18 bursts) | 180 s |
  | Missile rack | 4 | 120 s |

  CC.2 measures fights at these sizes before it lands, and sets the final sizes from what it finds:
  fight length, hits per load, how often a unit withdraws dry, and human against mind. Prices stay
  GC.7's (Q-49-5), so a full cannon load is 1,800 cr.
- **Alternative:** keep §5's magazines (24, 600, 120, 4) and let the measurement decide.
- **Why:** Q-49-5's ruling puts scarcity on magazine size. What a fight feels is endurance (rounds ×
  cycle), and with longer cycles the old counts last longer.

**Q-49-23 — When a reserve is spent** (A-13).
- **The problem:** the Line card keeps a 25% reserve, but today "dry" means no round anywhere
  (`arena/stock_mind.rs:431-434`). A mind at its reserve therefore stops firing and never withdraws.
- **Proposed default:**
  - The reserve is spent only answering fire (a unit that fired on this one, as the absent gate
    counts it) and covering a withdrawal.
  - A unit at or under its reserve on every mount counts as dry for withdrawal.
  - The card's text says so.
- **Alternative:** the reserve is a soft floor that the mind may spend on a high-odds shot (65% and
  up).
- **Why:** a reserve exists to get the unit home. A mind that sits on it in the open is worse than one
  that runs.

**Q-49-24 — Laser modes** (A-12).
- **The problem:** a space laser switches between offence and defence on one cooldown. The 5 s floor
  (Q-49-19) and the automatic-defence exemption (Q-49-2, Q-49-10) are each per mount.
- **Proposed default:**
  - **The cycle belongs to the mode.** Offence cycles at 5 s, the floor. Defence keeps its own
    automatic cadence.
  - **A switch never fires early.** Switching mode starts the new mode's cycle from the switch.
    Switching back within one offence cycle keeps the offence cooldown running, so a switch is no
    way round the floor.
  - **The fit names the modes.** A hull's fit says which modes each mount may use. The Dray and
    Dray-P lasers are fitted defence-only, enforced by the fit, not by doctrine.
  - The same rule holds for a ground laser if it ever gains a defence mode.
- **Alternative:** one mode per mount, fixed at fit, with no switching.
- **Why:** the exemption has to follow what the mount is doing, not what it is, and a switch must
  never be a way round the floor.

## 7. Units of work (sketch, cut after the rulings)

One writer per file chain, as docs 44 to 47 ran. **CC** is unused as a unit prefix.

| Unit | What | Gate |
|---|---|---|
| CC.0 | Tests read cycles and magazines from content, not literals; the bot's scripted rig reads content | — |
| CC.1 | The trigger rule: a slot needs a judgement term (single-use for a seated player under Q-49-20); card floors; the Line card's reserve and when it is spent | Q-49-8, Q-49-9, Q-49-20, Q-49-23 |
| CC.2 | The ground palette (15, 5, 10, 30 s), magazines by endurance, and the 5 s cycle floor (with the automatic-defence class); measured fights before it lands; one re-bless | CC.0, CC.1, Q-49-2, Q-49-3 and Q-49-19 (ruled), Q-49-22 |
| CC.3 | Ground lasers as the anti-drone fit; the laser variants keep a main gun | CC.2, Q-49-6 |
| CC.4 | Rounds on `STATUS`; checked at 120 columns | Q-49-13 |
| CC.5 | Munitions prices, read from doc 48's PL.3 recipes | doc 48's PL.3, Q-49-4, Q-49-5 |
| CC.6 | Space mounts and hull fits as content packs, each mount's allowed modes in the fit; `raider_heavy` a unit file; mind-view maxima from content | Q-49-11, Q-49-24 |
| CC.7 | Space magazines persist; space munitions goods; rearm at a berth | CC.6, doc 48's PL.0 (`MAX_GOODS`) and **PL.3 producing the space munitions** (doc 48 Q-48-41), Q-49-12 |
| CC.8 | The space palette; lasers per the lore with the cycle per mode; the three space doctrines rewritten | CC.6, CC.7, Q-49-7 and Q-49-19 (ruled), Q-49-24 |
| CC.9 | Space decision support, for the player and the minds | CC.6, Q-49-13 |
| CC.10 | The bot's discipline assertions (and its re-arming, if slots are single-use) | CC.2, Q-49-17, Q-49-20 |
| CC.11 | ~~A second round for the cannon, chosen at rearm~~: deferred under Q-49-21's default (engine work: selected-round state, and the resolver for section effects) | Q-49-14, Q-49-21 |

**Round conservation (outside review, doc 48a A-14).** Every CC unit that moves rounds keeps them
counted. A reload, a type change, a crash and restart, a custody move, a refit and salvage each
conserve rounds, each with a test. The matter census counts munitions (doc 48 Q-48-31).

## 8. What this survey did not check

- What the player's space cockpit shows before a shot.
- Whether the server's substep is fixed at 250 ms or derived from the tempo configuration
  (`arena/clock.rs:669-679`).
- That space magazines refill at check-out, end to end (§3.2 reads it from the code paths).
- Whether simulator units always start a match with full magazines (tests assert it at the start;
  not traced).
