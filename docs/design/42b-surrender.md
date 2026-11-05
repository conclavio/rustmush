# 42b. Surrender — survey

**Draft 0.2, 2026-11-05: ruled.** The owner accepted every default as written ("Q-42b all accepted
at default, section 5 consequences of the defaults acknowledged"). Each of Q-42b-1 to Q-42b-20 now
ends with a decision line, and its proposed text is the ruling. §5 is acknowledged. §7's cuts
SU.0–SU.6 are **binding**. **SU.0 (the trait on the record) and SU.2 (the arena's reach count) may
start now**, side by side, since they share no file. SU.1 follows SU.0, and SU.3 waits on SU.1
and SU.2 (§7's order). Nothing else changed from Draft 0.1.

Draft 0.1, 2026-11-04, docs only (PR #470). Surveyed against `origin/main` at `bf0b5f24` (PR #468
merged; doc 42 Draft 0.5 with CR.0–CR.7 built). The first survey of the future that doc 42 §8 and
§7b record: *surrender by circumstance and personality*. It follows docs 38–43. Each question has
a proposed default, an alternative and a reason, and is grounded in the tree with file:line
references. "Provisional" marks a number to be tuned after first play (the 2026-10-24 rule).
**(expensive to reverse)** has doc 38 Q-38-10's meaning: undoing it would take a significant
refactor. It is used once, on Q-42b-6. The survey ends with the cuts SU.0–SU.6 (§7).

**The accepted defaults this builds on** (owner, 2026-11-04):

1. **Who surrenders.** Only NPC crews surrender, and only in a later version than CR. Players keep
   ejection (CR.4) and capture (CR.1).
2. **The trigger.** A morale check when the unit is outnumbered or has lost its carrier, weighted
   by a personality trait on the crew record.
3. **The outcome.** A surrendered unit is captured through CR.1's capture path, and CR.2's fee
   releases it.

**The standing rulings it must not break** (owner, 2026-11-04):

- A raider unit whose carrier is wrecked fights to the death. It has no life support to scatter or
  to wait for a pickup. This stays the default until surrender exists, and stays the outcome for
  any crew whose morale holds.
- A withdrawal counts only once the unit is aboard.
- Any wait on a player's response lasts at least **28 IC days**. Seven IC days is only about seven
  real hours.

## 1. Why this document exists

Doc 42 built custody for one case: a crew captured inside a casualty that a non-friend loads
(CR.1), or a pilot captured by an ejection into a held cell (CR.4). Every captive so far is a
*player*. NPC crews either have no record at all, or are hired pilots whose contract ends at a
capture (Q-42-26). Doc 40's raid ramp made the gap concrete. A director raid whose carrier is
wrecked leaves its hulls in the field with their WITHDRAW lifted (`field/raiders.rs:790–806`), and
they fight until they are destroyed. The owner ruled that this is correct for now, and named
surrender as the way out later. Doc 41 RS.1's note on `gc11` points at the same place.

This survey asks what "later" needs:

- what the check reads (§4 A);
- the trait and where it lives (§4 B);
- the moment of surrender, and who it is offered to (§4 C);
- what the field shows, and what players can do about it (§4 D);
- the seams with the kernel, the director and the stranded-raider ruling (§4 E);
- what the captive becomes through CR.1 and CR.2 (§4 F);
- the timers (§4 G).

## 2. Already decided; not asked again

| Ruling | Where | What this document does with it |
|---|---|---|
| Players are never surrendered; they keep ejection and capture | owner, 2026-11-04 | The trait is absent on every player record, and absent means "never" (Q-42b-5). |
| Stranded raiders fight to the death | owner, 2026-11-04; `raiders.rs:780–806` | Kept as the outcome whenever morale holds. Surrender is a draw against it, never a replacement (Q-42b-12). |
| A withdrawal counts only once the unit is aboard | owner, 2026-11-04; `exit.rs:21–29` | A surrender is not a withdrawal. It never counts in the director's withdrawal or lift readings as "out" (Q-42b-14). |
| Minimal custody: no interrogation, no ransom market | doc 38 Q-38-22 | No ransom, no bounty in credits, no prisoner exchange (Q-42b-17). |
| Every non-friend load captures; the captor is the carrier's custodian | doc 42 Q-42-4, Q-42-16; `tow.rs:3230–3256` | The surrendered crew is taken by that same load (Q-42b-15). |
| A character cannot leave a hull on foot in a field | doc 32 Q-CF-39 | A surrendered crew stays in its hull until a load moves it. |
| One kernel rebless a programme; a cut that touches a hash-pinned file dispatches the harness kind | doc 36 Q-36-56 (l.778); doc 40 §4a; doc 43 rule 13 (l.140) | v1 needs no hash change (Q-42b-11). The kernel's "yielded" state waits for a programme's batch. |
| Any player-facing wait is at least 28 IC days | owner, 2026-11-04 | Applies to the captor's window to collect (Q-42b-19). |
| Mechanics at the game level only, no targeting maths | house rule | Morale reads game-level facts the kernel already exposes: reach, losses, health per mille. |

**Out of scope.** Player surrender. Surrender of a carrier or of a ship in space. Parley or a
negotiated truce between player parties. Hired NPC pilots in a player's detachment (Q-42b-2 makes
them a later step). Boarding. Interrogation. Morale effects short of surrender, such as breaking
off or loss of accuracy.

## 3. What the tree has (checked at `bf0b5f24`)

**Raider hulls have no crew.** `seed` makes the leader as an NPC with no location
(`raiders.rs:454–457`). It makes every hull and the carrier as a GOD-owned `ObjKind::Thing` with a
unit spec and a `TACSIM.FIELD.RAIDER` mark (`make`, `raiders.rs:502–525`), and seats nobody. Each
hull flies its ROM seal (Standard Doctrine). The leader stands "behind" the party as
`PartyKey::Fixture { leader }` (`field/parties.rs:379`) and is never aboard anything. **There is no
character to capture.** A surrender built on CR.1 must first give the raider crews to capture
(Q-42b-1).

**The crew record is player-only.** `CrewRecord` (`field/crew.rs:254–275`) holds a location, a
condition, an organizer, an extraction and a treatment. It has no trait field. `check_in` returns
early for anything that is not `ObjKind::Player` (`crew.rs:758–764`). `on_frames` writes a record
for any seated pilot whose control section fell (`crew.rs:1475–1525`), but it is fed only by the
seats the host tracks. The record is `deny_unknown_fields` (`crew.rs:253`). An added field with
`serde(default)` and `skip_serializing_if` decodes old records and leaves `CREW_FORMAT` 1
unchanged: the `Extraction::forced` precedent, `crew.rs:211–214`.

**Capture at a load.** `load_crew` (`surface/tow.rs:3230–3256`):

1. A Friend's load sets the crew aboard.
2. A non-friend's load runs `npc_exit_loaded`, then `captured_loaded`. `npc_exit_loaded`
   (`crew.rs:1340–1408`) returns a seated `ObjKind::Npc` with a crew contract to its hire pool;
   an NPC with no contract returns `None` and is left alone. `captured_loaded`
   (`crew.rs:1005–1056`) captures every record `held_in` the casualty, meaning `Unit` or
   `Stranded` in it (`crew.rs:950–961`). It vacates the seat, steps the captive into the carrier's
   bay room, and writes `Captured { carrier, captor, since_ic }`.
3. `capture_standing` (`crew.rs:1177`) costs the captor `STANDING_CAPTURE` −5 only when the captive
   was no Hostile of theirs (`held_hostile`, `crew.rs:1130`). The captor is the carrier's custodian
   (`custodian_of`, `tow.rs:1048–1051`). The claim's `UnlawfulKind::Capture` evidence row is
   written beside the load (`surface/claim.rs:691–704`).

**The handover and the fee.** `hand_over` (`crew.rs:2504`) runs at docking. It writes
`Processing { port, release_due_ic, charge: None }`. `levy_release` (`crew.rs:2604`) then creates
one sanction obligation debiting *the captive's own account* under the port clerk's
`port.release_fee` grant. The release pass (`crew.rs:2695–2712`) sets the captive down once
`release_due_ic` passes. An NPC has no account to levy (Q-42b-16).

**The held case.** `captive_bound_s` 604,800 (seven IC days, Q-42-8) lets a *player* captive ask
`+sortie/extract` once it passes (`may_extract`, `crew.rs:413`). An NPC never asks. The captor's
`+ship/release` (`release_cmd`, `crew.rs:2362`) sets a captive down at a port's arrival room, or
aboard as a passenger (`set_down`, `crew.rs:2434`).

**The raider's field life.**

- *Leaving.* The leader's WITHDRAW is a host-side order under the raider's fence: `order_home`
  (`raiders.rs:694–706`), Withdraw with HOLD FIRE, sent by `offer_order` (`raiders.rs:717–760`),
  sequence 1. `strand` (`raiders.rs:790–806`) lifts it at sequence 2, with no directive, when the
  kernel refuses the release `CarrierLost` (`arena/withdraw.rs:52–54`, `:126–133`;
  `exit.rs:367–373`). From then the hull's doctrine "fights as it would, to the death".
- *Losing the carrier.* `wreck_aboard` (`raiders.rs:814–850`) wrecks the hulls still aboard a
  wrecked carrier.
- *The lift check.* `lift_check` (`raiders.rs:912–1050`, paced at `LIFT_CHECK_MS` 10 s,
  `raiders.rs:873`) decodes the last snapshot once. It reads each hull's condition as demotion
  would write it (`close::condition_now`, `field/close.rs:218`), and counts `launched`, `exited`
  (`:958`) and `lost`. It asks the director's `withdrawal_due` (`surface/raid.rs:471–508`), whose
  triggers are a lost share (`raid_lost_permille` 500), field time (`raid_field_s` 1,200) and idle
  (`idle_demote_s` 600). It lifts the carrier once every launched hull is down or out. A pending
  eject capture holds the carrier (`:946`). A wrecked carrier never lifts (`:983–989`).

**What the kernel already answers, as reads.** None of these is hashed:

- `Arena::party_holds_armed_hostile_in_reach(party)` (`arena/state.rs:5749–5766`), a boolean;
- `Arena::health_permille(id)` (`arena/state.rs:5783`), armour plus internals per mille of what
  was authored;
- the eject read (`arena/eject.rs`, "a read, never a step").

The host's seeded draw is FNV-1a over the site tag, the field ordinal, the unit and the arena
millisecond (`field/eject.rs:255`). Its holder rule (`eject.rs:220`) names the Hostile party that
holds a cell and has a carrier on the site.

**The no-ramp exit.** `Arena::withdraw_to(unit, None, _)` takes any unit out at once, with no ramp
and no drive test (`arena/withdraw.rs:116–123`, `take_out` `:163–171`). It is doc 32 F-6's carrier
lift, and the host has used it since CF.7. A unit taken out this way is checked in by
`on_withdrawn` (`exit.rs:345`) through the IC condition plan. `left_condition` (`close.rs:199–214`)
writes `Casualty` for an immobile, unarmed or disabled state and `Wreck` for a wreck. Otherwise it
writes nothing, so a sound hull is no casualty and cannot be towed (`LeftCondition`,
`surface/site_record.rs:152–157`, "towable (27a H2)").

**Tows on a promoted site.** Surface acts wait while a field runs (Q-CF-30, `tow.rs:44–56`). The
exception is FR.5's coupled tow of a kernel unit, which loads only once both tug and casualty have
exited at the ramp (`pair_out`, `tow.rs:683–690`).

**No trait anywhere.** Nothing in `crates/tacsim/src` or `content/` authors a personality, morale
or temper. (A grep for those words finds only the `temperate` biome.)

### 3a. Contradictions found

- `strand`'s doc comment says the hull fights "to the death". That is true only because nothing
  else can end it. Once SU lands, the comment and doc 40's boarding-window note must read "to the
  death unless its crew yields".
- `lift_check` counts an `exited` hull as launched-and-out (`raiders.rs:958–964`). A hull that
  left by any exit other than the ramp would read as boarded, against the "aboard only" ruling.
  Today nothing else exits a raider hull, so it is latent. SU.3 must not trip it (Q-42b-14).

## 4. Owner survey

**Ruled 2026-11-05.** Every default below is accepted as written; no question is amended, so each
proposed default is its decision.

### A. What morale reads

**Q-42b-1 — Who the raider's crew are.** **(the prerequisite)**
- **Proposed default:** the seed makes one NPC crew member for each armed hull. Each is an
  `ObjKind::Npc` named in the content, or "<hull> crew" by default. The crew member is seated in
  the hull's station as a hired NPC is seated (`parties::seated`, `field/parties.rs:238–245`), and
  given a crew record `Unit { unit }` at once with the raider as organizer. The carrier gets none
  (Q-42b-3). A re-seed makes fresh crew. A hull's crew go where the hull goes: aboard, launched,
  wrecked. Nothing about the kernel's doctrine changes. The hull still flies its ROM seal, and the
  seat is host bookkeeping only, as a DT NPC pilot's is.
- **Alternative:** no characters. The hull's raider mark carries a crew count, and a surrendered
  hull's load creates its captives on the spot.
- **Why:** every custody reader is keyed on a character's record: `captured_loaded`,
  `+ship/captives`, the handover, the bound, the captor's wreck (CR.3's `wrecked_captive`). Creating
  captives at the load would need a second capture path. Characters made at the seed also give the
  NPC knowledge model (memory `npc-knowledge-model`) a body to hang memory on later. The cost is a
  few NPC objects per raider.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-2 — Whose crews may surrender in this version.**
- **Proposed default:** fixture and director raider crews only (Q-42b-1's records). Hired NPC
  pilots in a player's detachment never surrender in this version, and their contract and Q-42-26
  exit stand. Doc 40 NPC escorts and miners never surrender either.
- **Alternative:** any NPC with a crew record, hired pilots included, with the trait authored in
  the hire pool.
- **Why:** a hired pilot who yields hands away a hull a *player* owns. That is a player-facing loss
  needing its own rulings: the employer's consent, the contract's terms, who gets the fee, and the
  28-day rule for the employer's answer. Raiders are the case the owner named.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-3 — Does the carrier ever surrender?**
- **Proposed default:** no. The fixture carrier is unarmed and has no crew under Q-42b-1. It lifts
  (GC.0) or it is wrecked. Taking a carrier is boarding, which doc 32 §5 keeps out.
- **Alternative:** a carrier with its hulls lost yields with whatever is aboard.
- **Why:** the ruling concerns units whose carrier is lost. A carrier yielding would mean a hull
  in a bay, a bay in a hull, and a lift that never comes. That is a later design, if ever.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-4 — The inputs, and how they combine.**
- **Proposed default:** a per-hull **pressure**, 0 to 1,000, computed by the host from the
  decoded last snapshot `lift_check` already holds. Each term is provisional:

  | Term | Read from | Adds |
  |---|---|---|
  | **Carrier lost** | `condition(carrier) == Wreck`, or the carrier is not in the field | 400 |
  | **Odds** | Hostile armed units within this hull's reach, minus the raider's own standing hulls; a new kernel read returning the *count* beside today's boolean (Q-42b-11) | 150 for each unit of excess, at most 450 |
  | **Losses** | the raid's `lost / launched`, as `withdrawal_due` reads it | that share × 300 |
  | **Damage** | `1000 − health_permille(hull)` | that × 3/10 |
  | **Leader absent** | `leader_of(hull)` is `None` (gone or GOING) | 100 |

  The sum is clamped to 1,000. The yield chance, per mille, is `pressure × (1000 − resolve) /
  1000`, where `resolve` is the trait (Q-42b-5). A hull under no pressure never yields. **Gate:**
  the check runs only when the "carrier lost" term or the odds term is non-zero. This is the
  accepted trigger, "outnumbered or its carrier is lost". Losses, damage and the leader only
  weight a check those two have opened.
- **Alternative:** a single threshold. The hull yields when pressure passes `resolve`, with no
  draw.
- **Why:**
  - Every term is a reading the host already makes, or one count beside a boolean the kernel
    already exposes. Nothing is targeting maths.
  - A threshold makes every crew of one resolve yield at the same instant, which reads as
    scripted.
  - The draw keeps personality as weight, not as a switch.
  - The gate keeps a crew that is merely damaged and winning from ever yielding.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
### B. The personality trait

**Q-42b-5 — The trait's shape, and where it lives.**
- **Proposed default:** one number, `resolve_permille: Option<u16>` (0 to 1,000) on `CrewRecord`,
  with `serde(default, skip_serializing_if = "Option::is_none")`. `CREW_FORMAT` stays 1, as the
  `forced` precedent did. `None` means never yields. That is every player record and every record
  written before SU, so the player rule is the data's default, not a branch. 1,000 is a fanatic who
  never yields. 0 yields at the first pressure. It is written once, at the seed. `check_in`,
  `capture` and the handover carry it unchanged.
- **Alternative:** a trait table, such as `{ resolve, loyalty, greed }`, or the trait on the
  hull's raider mark rather than on the record.
- **Why:**
  - The accepted default says "on the crew record".
  - One axis is all the trigger reads.
  - The hull mark would lose the trait at a capture, when the person leaves the hull. A later
    trait for an NPC's talk or knowledge would want it on the person.
  - More axes can be added beside it the same way.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-6 — How it is authored.** **(expensive to reverse)**
- **Proposed default:** `raiders.json` schema 5 adds `resolve_permille` to each raider (the crew's
  default) and an optional `resolve_permille` and `crew` name on each unit. The schema-4 brief
  grows a sentence. The seed copies the value into the record. Shipped values, all provisional:
  Skulk raiders 400 for a syndicate of hired hands, and Reach Reavers 750. A missing value reads
  500. The raid director never varies it.
- **Alternative:** a per-organization value in the faction registry, which every raider of that
  organization inherits.
- **Why:**
  - Content is where every raider fact lives today (`raiders.rs:1–45`).
  - The pack is digest-pinned only through the lane and relay (CR.2's precedent), so a schema bump
    is a known cost.
  - It is expensive to reverse because the content schema, the seed, the stories and the record
    all carry it. Moving it later to organizations would touch all four.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-7 — Variation inside one crew.**
- **Proposed default:** none beyond what the author writes. Each hull's crew takes its unit's
  value or the raider's.
- **Alternative:** a seeded spread of ±150 around the raider's value at the seed.
- **Why:** an authored spread is visible and testable. A random one makes the stories pin a seed
  for no play value yet.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
### C. The moment, and who it is offered to

**Q-42b-8 — When the check runs.**
- **Proposed default:** in `lift_check` (`raiders.rs:912`), for each hull still in the field,
  after the decode and before the withdrawal reading. It runs at the existing 10 s pace
  (`LIFT_CHECK_MS`). A hull draws **once for each 100-per-mille step its pressure reaches**. The
  highest step drawn is kept on the hull's raider mark as `morale_step`, so a restart never
  redraws a step it has already drawn. The draw is `eject::draw`'s FNV-1a with the step folded
  into the seed: `(site, ordinal, hull, step)`, not the arena millisecond.
- **Alternative:** a draw at every paced pass, with a lower chance per pass.
- **Why:**
  - A draw every 10 s compounds with time, so a long field would always yield. Steps make yielding
    follow events: a loss, the carrier's wreck, a new hull in reach.
  - The mark is durable, so a restart behaves.
  - Seeding on the step keeps a resumed field's draw identical.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-9 — Who it is offered to, and who accepts.**
- **Proposed default:** nobody accepts. A yield is unilateral, and the field announces it. The line
  names the party that holds the hull's cell, by eject's holder rule (`eject.rs:220`): Hostile to
  the raider, a live armed unit with the hull in its reach, and a carrier on the site. With no
  holder, the hull *does not yield*. The check's gate also requires a holder: without a carrier on
  the site to take them, there is no pickup, and the owner's ruling (no life support to wait)
  stands. Any non-friend's load then captures the crew (Q-42-16), whoever was named.
- **Alternative:** an explicit `+field/accept <hull>` by the named party's leader, with a timeout.
- **Why:**
  - An acceptance verb inside a running field is a wait on a player's response. The 28-IC-day
    rule makes any timeout on it absurd at field tempo.
  - Unilateral needs no wait.
  - The holder rule is the CR.4 reading the owner already accepted for "who holds the cell".

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-10 — Whole party or one hull.**
- **Proposed default:** one hull at a time, each on its own draw. When every hull of the raider
  standing in the field has yielded, the leader's standing Hostile marks on the site are lifted
  through the ordinary door (`field::mark_from_order`, the director's own declare path,
  `raid.rs:299`). Then the field can demote by disengagement.
- **Alternative:** the whole party yields on the first hull's break.
- **Why:** per-hull morale is what the trait weights. Lifting the marks only at the last yield
  keeps a half-broken raid hostile, which is honest, and lets the field end once nobody is left
  fighting.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
### D. What the field shows, and what players can do

**Q-42b-11 — What a yield is in the kernel.**
- **Proposed default:** **v1 adds nothing to the kernel's step.** The yield takes the hull out of
  the field at once by the no-ramp exit, `withdraw_to(hull, None, None)` (`arena/withdraw.rs:121`;
  F-6's lift), asked by the host as the leader with a new `exit::Then::Yield`. `on_withdrawn`
  checks the hull in where it stands. Out of the field, it cannot be fired on, it fires on
  nothing, and it no longer holds the field engaged. The one kernel addition is a **read**: the
  count of Hostile armed units in a hull's reach (Q-42b-4), written beside
  `party_holds_armed_hostile_in_reach` (`arena/state.rs:5749`). No hash moves. Because it edits
  `arena/state.rs`, doc 43 rule 13 still dispatches the harness kind before the cut is marked
  ready, and every pin in doc 40 §4a must come back **unmoved**.
- **Alternative:** a kernel `yielded` unit state. The hull stays in the field, holding fire;
  doctrine stops targeting it; it counts as unarmed for the quiet test. It is one rebless,
  batched into the next programme's (Q-36-56), with every §4a pin re-read.
- **Why:**
  - Doc 40's rebless was this programme's (FR.5). A second one now would break Q-36-56.
  - A hull holding fire inside the kernel would still be fired on by every NPC Friend's doctrine,
    because relations are per party, not per unit (`arena/state.rs:5758`). It would die "yielded".
  - The no-ramp exit is built and used: CF.7, GC.0.
  - The cost is that the yielded hull leaves the tactical picture (Q-42b-13). If a later
    programme wants yielded hulls inside the fight, it reblesses for that.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-12 — The check-in condition of a yielded hull.**
- **Proposed default:** a yielded hull is checked in as a **casualty** on the site record, whatever
  its cells say ("its crew powered it down"). `left_condition` (`close.rs:199`) gains one input
  for a `Then::Yield` row. Every tow rule for a casualty then applies unchanged: attach, tow and
  load by anyone (Q-36-52), and the claim's `Recovery` row. Its crew record is written by the
  widened `check_in` (SU.1) as `Unit { unit }` if the hull is sound, or `Stranded` otherwise.
  Either is `held_in` the hull, so CR.1's load captures it.
- **Alternative:** a new `LeftCondition::Yielded`, which every tow, claim and demotion reader
  learns.
- **Why:** a new site-record condition touches every reader of `LeftCondition` and its word. The
  hull really is out of action: the crew gave it up. A wreck is never yielded: a wrecked hull's
  crew is CR.3's stranded case.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-13 — How players see it.**
- **Proposed default:** three things.
  1. **A line** to every character in the field and on the site, in the house voice: "The {hull}
     stands down: its crew yield to {party}. It is out of the fight, and anyone may take it in."
  2. **The site's casualty listing.** The hull stands at its field pose, listed with the site's
     casualties (the record's ordinary display), marked *yielded*, from a `yielded` key on its
     raider mark.
  3. **The captor's +ship/captives** after the load, as for any captive.

  The cockpit frame draws nothing new: a hull out of the field is no contact. When the field
  demotes, a yielded casualty is ground clutter like any other.
- **Alternative:** a YIELD badge on the cockpit's contact row and in the TUI, which needs the hull
  to stay in the field (Q-42b-11's alternative).
- **Why:** the default needs no frame or encoder change (doc 21's one Frame and three encoders). A
  badge is the alternative's natural partner, not the default's.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-14 — What players can do with it.**
- **Proposed default:**
  - **Take it in.** After the field demotes (Q-CF-30: surface acts wait on a promoted site;
    `tow.rs:44–56`), any party attaches, tows and loads the yielded casualty. The load captures
    its crew through CR.1's branch. While the field still runs, the hull lies outside it, and
    FR.5's coupled tow does not apply, because that needs a kernel unit.
  - **Leave it.** It waits for Q-42b-19's window, then its crew leave play.
  - **Fire on it.** Not possible: it is out of the field.

  No verb is added.
- **Alternative:** let a party attach to a yielded casualty while the site is promoted, by
  widening `pair_out` (`tow.rs:683`) to pass an exited casualty.
- **Why:** taking prisoners once the fight is over is the shape the tree already has. Widening
  `pair_out` reopens FR.5's rule that a load runs only when both pieces are out at the ramp.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
### E. The seams

**Q-42b-15 — The fight-to-the-death ruling.**
- **Proposed default:** the ruling stands as the outcome whenever morale holds. Surrender is
  reached only:
  - through the check's gate (outnumbered, or carrier lost);
  - with a holder present that has a carrier on the site (Q-42b-9);
  - on a draw that the trait weights.

  `strand` (`raiders.rs:790`) is unchanged. It lifts the WITHDRAW, and the doctrine fights. Each
  later pass may break that crew by Q-42b-8's steps. With no holder, or with `resolve` 1,000,
  nothing changes from today. `strand`'s comment and doc 40's note gain "unless its crew yields"
  (§3a).
- **Alternative:** a stranded hull yields at once to any holder, with no draw.
- **Why:** the ruling's reason, no life support to wait, is met by the holder's carrier being on
  the site. The personality is the accepted default's weight.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-16 — The director (doc 40, GC.11) and the lift.**
- **Proposed default:**
  - A yielded hull counts as **lost** in the `RaidReading`, never as exited, so a yield can
    trigger the director's "lost" withdrawal for the rest of the raid.
  - `lift_check`'s `exited` count (`raiders.rs:958`) excludes yielded hulls.
  - Its `lost` counts them. The carrier, if it still stands, lifts once the others are aboard or
    down, leaving the yielded hull behind.
  - A hull whose WITHDRAW is under way yields only if refused `NotUnderOwnPower`, or if stranded.
    Otherwise it keeps going home: withdrawal first, while home is reachable.
  - The director marks nothing new. "Losses are real" (`raid.rs:15`): a yielded hull is a hull the
    raider no longer has.
- **Alternative:** the morale check replaces the withdrawal triggers for director raids.
- **Why:**
  - This keeps the "aboard only" ruling exact: a yield is never a withdrawal.
  - A raid with a way home goes home; one without might yield.
  - The withdrawal triggers stay the GC.11 rulings.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-17 — Standing and evidence.**
- **Proposed default:** no standing cost for capturing a yielded raider. It is Hostile to its
  captor, so `capture_standing` already writes nothing (`crew.rs:1177`, `held_hostile`). No
  evidence row beyond the claim's own, because a lawful party's load of a yielded Hostile is
  lawful salvage. No bounty and no ransom (Q-38-22). The handover is the reward, as Q-42b-18 says.
- **Alternative:** a standing gain with the port authority's organization for each raider handed
  over, through a mush-faction receipt.
- **Why:** the alternative is cheap and probably wanted. But it is the first *positive* standing
  written by custody, and the stance table (doc 40 H) is the place to rule it. Flagged for after
  first play.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
### F. What the captive becomes

**Q-42b-18 — Through CR.1 and CR.2.**
- **Proposed default:**
  1. **CR.1.** The non-friend load runs `npc_exit_loaded` first. It returns `None` for a raider
     crew member, who has no contract (`crew.rs:1349–1355`). Then `captured_loaded` captures the
     crew member by the ordinary three steps. The record keeps `resolve_permille`.
  2. **CR.2.** At docking, `hand_over` writes `Processing` as for anyone. `levy_release` (`:2604`)
     **skips an NPC captive.** An NPC has no account, so `charge` stays `None` and no obligation
     is created. The release fee is a player's debt, never an NPC's.
  3. **Release.** At `release_due_ic`, an NPC captive **leaves play**. The record is removed, and
     the object is marked GOING and moved out of the world, as the seed's leader would be if staff
     removed it. The handover line to the captor reads "{authority} take {who} into custody", not
     "released".
  4. **A player captive** in the same carrier is untouched.
- **Alternative:** a released NPC goes back to its organization's pool and crews the next seed.
- **Why:**
  - A raider re-crewed from its own released prisoners is a revolving door with no rule behind it.
  - "Losses are real" applies to people too.
  - The fee is ruled to be the captive's own debt, and an NPC has none to owe.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
**Q-42b-19 — Held, released at will, or the captor's wreck.**
- **Proposed default:**
  - **Held.** An NPC captive held past `captive_bound_s` stays held. Nobody asks for its
    extraction, and holding one costs nothing.
  - **Released at will.** `+ship/release` at a port sets the NPC down, and it leaves play at the
    next crew tick. On a site, it is refused for an NPC with "{who} has no way off this ground;
    hand them over at a port", because a crew set down on open ground has no life support.
  - **The captor's wreck.** The NPC is stranded in it (CR.3's `wrecked_captive`), and leaves play
    once the site's field demotes.
- **Alternative:** a bound for NPCs too, after which the authority claims the captive off-screen.
- **Why:** each branch reuses CR.3's code and keeps the life-support ruling literal. A hoarding
  captor gains nothing, so no bound is needed yet.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
- **Build note (SU.4, doc 38a review):** release under way was unspecified. As built,
  `+ship/release` refuses an NPC anywhere but a port, a carrier under way included, with the site
  line; at a port the NPC is set down and leaves play at the next crew tick.
### G. Timers

**Q-42b-20 — The captor's window, and every other timer.**
- **Proposed default:**
  - **The captor's window.** A yielded hull left on the site after demotion waits
    **`yield_hold_s` 2,419,200 (28 IC days)** for anyone to load it. This is the one wait on a
    player's response, so it is at the floor. `[tacsim.field].yield_hold_s` ranges from 2,419,200
    to 7,776,000, and the floor *is* the minimum. When it lapses, the crew leave play and the hull
    stays on the site record as a casualty for doc 41's salvage loop. Lore: a hull powered down to
    survival load lasts weeks. The life-support ruling concerns scattering on foot or waiting for
    a pickup by their own side.
  - **No other timer waits on a player.** The morale check runs at field tempo on events. The
    yield is unilateral. The handover's `release_s` (one IC hour) waits on nobody. No timer here
    gates a player's verb.
- **Alternative:** the crew leave play when the field demotes if nobody has attached by then.
  There is no window.
- **Why:** a captor who won the fight and logged off before towing deserves the full 28 days. The
  floor rule is the owner's. A range that starts at the floor keeps a config from undercutting it.

- **Decision (accepted default, 2026-11-05):** the proposed default, as written.
## 5. Consequences of the defaults

**Acknowledged by the owner, 2026-11-05.** SU.6 writes these amendments into the documents named.

- **Doc 42.** §8's future line points here. `Captured` and `Processing` gain no variant, but the
  record gains `resolve_permille`. `levy_release` and the release pass learn the NPC branch.
- **Doc 40.** The boarding-window note on `strand` reads "to the death unless its crew yields".
  §4a's inventory is re-run for the reach-count read (expected: every pin unmoved).
- **Doc 32 and doc 36.** The Q-CF-44 fixture raiders gain crew. Q-36-55's withdrawal triggers gain
  "a yielded hull is lost".
- **Content.** `raiders.json` moves to schema 5. The lane and relay are re-signed if the pack is in
  their content (CR.2's path).
- **Config.** `[tacsim.field].yield_hold_s` is added to `mush-config`'s table.
- **The contract guards.** The NPC count in census rows grows. No verb or switch is added, so
  `legacy-handlers.toml` is unchanged.

## 6. Acceptance sketch

Each story runs at both sizes (Q-38-29), with the bot typing the player's half over SSH, through
`custody_rig` (`tests/support/field/custody_rig.rs`, the CR.1 hand-over's rule).

- **(s1) The stranded raid yields.** A director raid lands. Two player DT flights and a carrier
  hold the site. The players wreck the raider's carrier. One hull, at `resolve` 0, yields at the
  next step: it is out of the field, a casualty, and its crew record reads `Unit`. The other, at
  1,000, fights to the death. The field demotes. A player tows and loads the yielded hull, and its
  crew member is `Captured` aboard the player's carrier. The carrier docks at Ceres: processing,
  no obligation, and the NPC leaves play at release.
- **(s2) Morale holds.** The same, with both hulls at 1,000. Nothing changes from today's
  fight-to-the-death story (the `gc11` gate's expectations are kept).
- **(s3) No holder, no yield.** The carrier is wrecked with no Hostile carrier on the site. The hull
  fights on whatever its trait.
- **(s4) The window.** A yielded hull nobody loads. A rate-knobbed IC clock passes 28 days, the
  crew leave play, and the hull stays as a casualty.
- **(s5) A restart** between two steps redraws nothing (`morale_step` durable).

## 7. Cuts SU.0–SU.6 (binding since the ruling of 2026-11-05)

Like doc 42's CR cuts: each runs through the production command paths, ships its gate at both
sizes, and is recorded as built in `42b-cut-notes.md`. Two cuts touching the same files never run
at once.

**SU.0 — The trait on the record.** Q-42b-5. `CrewRecord.resolve_permille`, decode tests (an old
record reads `None`), `status_line` untouched. Files: `field/crew.rs`. Gate: in-source tests. *May
start now.*

**SU.1 — Raider crews.** Q-42b-1, Q-42b-2, Q-42b-6, Q-42b-7. Schema 5 of `raiders.json`; the seed
makes and seats one NPC crew member per hull with a record; `check_in` and `on_frames` widened to
an NPC that already has a record (`crew.rs:758–764`); a re-seed's fresh crew. Files:
`field/raiders.rs`, `field/crew.rs`, `content/field/raiders.json`, the lane content if pinned.
Gate: the seed's tests, and the existing `gc11`/`field_gc*` gates green with crew aboard. *After
SU.0* (crew.rs).

**SU.2 — The reach count.** Q-42b-4's odds term, Q-42b-11's read. One read on `Arena`, beside
`party_holds_armed_hostile_in_reach`. Files: `arena/state.rs`. Gate: an in-source test and the
**harness kind dispatched** (doc 43 rule 13), every doc 40 §4a pin unmoved. *Independent: may run
beside SU.0/SU.1.*

**SU.3 — The check and the yield.** Q-42b-4, Q-42b-8, Q-42b-9, Q-42b-10, Q-42b-11, Q-42b-12,
Q-42b-15, Q-42b-16. In `lift_check`: pressure, the step draw, `morale_step` on the mark, the
holder; `exit::Then::Yield` through the no-ramp exit; the casualty check-in; the yielded hull
counted lost; the last yield lifting the leader's marks; the field and site line. Files:
`field/raiders.rs`, `field/exit.rs`, `field/close.rs`, `field/eject.rs` (the draw and holder made
shareable), `surface/raid.rs`. Stories (s2), (s3), (s5). *After SU.1 and SU.2.*

**SU.4 — Capture and the NPC handover.** Q-42b-17, Q-42b-18, Q-42b-19. The NPC branch in
`levy_release`, the release pass (leaves play), `+ship/release`'s site refusal, and the wreck
branch. Files: `field/crew.rs`. Story (s1). *After SU.3* (the story needs a yield) *and SU.1*
(crew.rs).

**SU.5 — The captor's window.** Q-42b-20. `[tacsim.field].yield_hold_s` with its floor, and the
lapse pass on the crew tick. Files: `field/crew.rs`, `mush-config`. Story (s4) with the rate knob.
*After SU.4* (crew.rs).

**SU.6 — Docs.** §5's amendments to docs 32, 36, 40 and 42; `42b-cut-notes.md`; the as-built
draft. *Rides each PR; closes after SU.5.*

**May start now (2026-11-05):** SU.0 and SU.2, in parallel. Before SU.1, check no doc 40 or
doc 41 cut is open on `field/raiders.rs` (the edge below).

**Order.** SU.0 → SU.1 → SU.3 → SU.4 → SU.5, one writer on `crew.rs` and `raiders.rs`. SU.2 runs
beside SU.0/SU.1 and must land before SU.3. SU.6 rides along.

**Edges.**

- SU.1 touches `raiders.rs`, a file that doc 40's director cuts and doc 41's RS cuts have both
  edited. Before SU.1 starts, check that no doc 40 or doc 41 cut is still open on it.
- SU.2 must not share a batch with any rebless (Q-36-56).
- The yielded-in-kernel alternative (Q-42b-11) is a candidate item for the next programme's
  rebless, not a cut here.

## 8. Defaults held with least confidence

Ruled as written; worth a second look after the first stranded raid is played.

- **Q-42b-11, out of the field rather than holding fire in it.** It is cheap and safe. The cost is
  that a yield vanishes from the cockpit, and players may read that as a despawn. The line
  (Q-42b-13) has to carry it.
- **Q-42b-4, the pressure numbers.** Every term is a guess, to be tuned after the first stranded
  raid is played.
- **Q-42b-18, NPC captives leave play at release.** Simple, but it closes the door on prisoner
  dialogue and on exchanges, which Q-38-22 rules out for now anyway.
- **Q-42b-17, no positive standing for a handover.** Probably wanted. It is held back only so the
  stance table rules it.
