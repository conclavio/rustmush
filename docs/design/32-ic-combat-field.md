# 32. The IC combat field — requirements survey

Draft 0.2, 2026-10-30, docs only. Surveyed against `origin/main` at `2b2930c9`. Draft 0.1, the
same day, was the survey; Draft 0.2 records the owner's rulings of 2026-10-30 on every question,
the consequences of the amended ones (§4a), the follow-ups they opened (§5a) and the proposed
cuts (§6).

## 1. Why this document exists

The owner pinned the Codex field-combat plan (doc 27b, [PR #195](https://github.com/conclavio/rustmush/pull/195),
not yet on main) on 2026-10-30 before it started. The plan reuses the arena engine as the host for IC combat,
and the arena is the OOC simulator: two teams, a lobby, elimination, stock units, practice
reset. An IC field is none of those things. It is a piece of ground that several independent
parties share: a mining roster and its escort, a raider, a bystander working the next block,
and whatever the last fight left lying there. The owner's direction:

> Build the IC combat field requirements and engine first, cleanly, without bleedover from the
> arena; when it is done, update the plan to use it.

This document is the survey that comes before that engine. It does three things: records what
earlier documents already decided so nobody re-asks it (§2); records what the tree has today in
the words that matter for the design (§3); and asks the owner the questions that are open, each
with a proposed default and an alternative, in the form doc 27 §7 used (§4). Draft 0.1 planned no cuts; with the rulings in, §6 proposes them.

**The one boundary to state up front.** "Without bleedover from the arena" is read here as a
clean *host and relation model*, not a second simulation. The ground, motion, sensors and
contacts, weapons, damage, heat, the deterministic step, the tape and snapshot, the cockpit
frame, the mind host and doctrine are the kernel 16b built and tested; the field runs on them.
What must not leak is the match wrapper: `Team { Red, Blue }`, the lobby, the entry zones, the
elimination end, stock units and the simulator centre, the practice reset, the `SIM` verbs.
Q-CF-1 asks the owner to confirm that reading, because every later answer depends on it.

## 2. Already decided; not asked again

| Decision | Where | What it fixes for the field |
|---|---|---|
| Detachment is a command group, not a team; relations and known hostility decide targeting; neutral crews and a third force can exist; reinforcement by physical entry; withdrawal through reachable exits; cross-boundary fire must be specified | doc 27 Q-H3 | The field is multi-party. This survey asks *how*, never *whether* |
| A hostile act promotes the affected field; arriving with a different faction does not force a lobby or match; core-crime and safe-storage rules stay | doc 27 Q-H2 | Promotion is triggered, not ambient (Q-CF-14 asks what the trigger is) |
| Demote only when impacts, hostile interactions and boundary handoffs are settled; individual withdrawal need not wait; disabled units remain obstacles and future salvage; an empty unattended field may sleep cheaply | doc 27 Q-H4 | The demotion rule's shape; salvage persists |
| Engaged motion runs on wall time; unengaged field motion and towing stay on the IC tick; the handoff for a unit mid-drive is the host's to define; mining on engaged units pauses | doc 27 Q-H1, Q-R3; doc 20 D-20-1 | The clock discontinuity is accepted; Q-CF-31 asks only the handoff |
| Link loss: no sealed mind means hold the last safe posture, no fire, no motion, after `[tacsim.match].reclaim_min_ms`'s shape | doc 27 Q-H5, P-2 | Not re-asked |
| Post-battle information is observer-bounded; no objective arena tape to players | doc 27 Q-H6 | Salvage knowledge and digests follow it (Q-CF-28) |
| A disabled unit can be attacked again; recoverable states keep the hull epoch open; only irrecoverable closes it destroyed | doc 27 Q-H8, Q-H9; 27a H1-A | Casualties are live targets; epochs are the unit's, not the field's |
| The carrier may leave units behind after an explicit confirmation that recalls nothing and closes nothing | doc 27 Q-H10 | Not re-asked |
| No player permadeath; crew status modelled separately; bounded NPC extraction to a known safe port | doc 27 Q-H12, FO-12, Q-F4, P-5 | Q-CF-38/39 ask only the minimal model |
| Outage: silent re-anchor under the threshold, else a five-minute frozen countdown; no catch-up | doc 27 FO-11, Q-H11, Q-R4; built as PR #90 (the kernel: `mush-core` `WorldTimeline`, the `mush-server` boot policy and world task) | The field resumes after unfreeze (Q-CF-33) |
| The arena is an authority that shares no state and no lock with the world; four messages each way | doc 16 D-16-1 | The field keeps the seam; one task per site |
| The arena window and the site are one grid of 10 m cells, 256 a side **today** (`ARENA_CELLS`, `SITE_CELLS`, `MAX_TILE_SIDE`, unchanged since 16b T.3 and SM.1); doc 30 Q-30-6 ruled the biggest authored map 512 a side with WG.6 (the frame window, unbuilt) to play it; the 10 km path is doc 16 D-16-5's recorded addition, not yet scheduled. Docs 16 and 20 do not yet carry the doc 30 amendment | doc 16 D-16-5, §11; doc 20 §3.1; doc 30 D-30-7, Q-30-6, WG.6 | One field per site (Q-CF-13); the field never names a side (Q-CF-51) |
| A match is a simulator concept, not an IC one; an IC epoch never shares the match id packing | doc 23 D-23-10 | The field is not a match and must not borrow its identifiers |
| Sprites, not models; bodies in team colour with friend or foe left to the markers; three team palette slots | doc 22 D-22-9, D-22-10 | Q-CF-12 asks how colour follows relation |
| The weapons-free gate defaults FREE in the simulator | doc 21 Q-21-9 (G.12) | Q-CF-41 asks the IC default |
| Scale target: 100+ units, 12 v 12 mostly AI, scale-tested early; a sim task per region; two-layer AI | owner direction 2026-09 (combat scale targets) | Q-CF-45 sets the first budget under it |
| Economy and content numbers are provisional; take defaults, tune later, flag the expensive-to-reverse ones | owner rule 2026-10-24 | Every number below is provisional unless marked |

## 3. What the tree has (checked at `2b2930c9`)

- **The kernel** (`crates/tacsim/src/arena/`): ground from the atlas window, motion, perception
  with per-viewer contact scopes (`ContactId` re-keyed per hull), weapons, section and
  component damage, heat, a deterministic step with a chunked ack-gated ledger (E0), a periodic
  snapshot and boot resume that rewinds to the last snapshot (16b AF.3), the stock mind and the
  Rhai mind host (17b M7), the DT.1b order adapter (`arena/detachment.rs`) that applies roster
  orders with contact re-keying. All of it is reusable as is.
- **The match wrapper, two-sided by type**: `arena/state.rs:636 enum Team { Red, Blue }`; the
  callsign letter is a `const fn` over it and its own comment says a third side is a compile
  error; `Classification::Classified { team }`; `EntryZone.team`;
  `EndReason::Elimination { winner: Option<Team> }`; hostile means `team != viewer.team` in
  perception, conditions, the stock mind, motor, frames, view, profile rules and
  `state::hostile_in_weapons_range`; friendly-fire checks in `damage.rs`; the DT.1b adapter
  fences by team; the mind API and mind card expose `team`. About 940 occurrences in 36 files.
- **The custody seam** (16b T.3): `@arena/start` checks real `unit:<obj>` records out through
  `mush_tactics::custody::check_out` (a `deployed` marker, placement frozen), `match_::custody_in`
  diffs what the arena changed back on check-in, a void restores the check-out snapshot. The
  IC field keeps the seam and changes the void rule.
- **The campaign surface** (`crates/tacsim/src/surface/`, doc 20): the site's geology and
  ground, the IC-clock drive at five IC seconds a cell, mining, the rig record and excursion,
  `+tv` and the DT.3 launch/recover flights onto it. It has no sensors, contacts or weapons.
- **The DT line, complete through DT.3 and DT.5** (PRs #149–#185): hire, board, plan,
  activate, preflight, seat, launch, field orders over real transport
  (`sortie/field_orders.rs FieldObjective`: Hold, Advance, Engage, Withdraw, Screen, Support,
  Mine), the NPC mining executor on the surface tick, IC detachment chat. The field-ops helper
  library (`tacsim/src/field_ops.rs`, `mush_tactics::field_ops::*`) has one live caller,
  `assess_tacsim_unit`; everything else in it is uncalled.
- **W-clock** is built once, in the kernel (PR #90: `mush_core::clock::WorldTimeline`, the
  `mush-server` boot policy in `timeline.rs` and the world task's countdown step, with the
  `[game]` knobs `outage_freeze_threshold_s` 30 / `outage_countdown_s` 300). The field-ops
  twin PR #137 carried (`mush_tactics::field_ops::clock`, test-only) was retired on
  2026-10-31 (doc 27a cut notes). The arena task and the match resume do not consult it;
  the field module's boot and tick do (CF.2).
- **No armed IC hull, no combat contract**: the only leasable rig is the unarmed prospector;
  the armed designs are arena stock; both shipped crew offers carry mining and recovery duties.
- **Doc 22's looks** carry three team palette slots and the unit type; doc 31's model
  catalogue keys bakes by model.
- **The ground's side is a constant in three places**: `match_/arena_def.rs ARENA_CELLS = 256`
  (unchanged since 16b T.3), `surface/ground.rs SITE_CELLS = 256` (since SM.1) and
  `mush-atlas/src/cells.rs MAX_TILE_SIDE = 256`; the surface address scheme
  (`CELLS_PER_BLOCK` 64) and the site ground cache are sized from it. The WG.1 ground writer
  already accepts drafts to 512 a side (`MAX_ARENA_TILES` 16 quadrants of 128), and doc 30's
  D-30-7 puts the `window` on the frame, not in the sim. Nothing in the tree plays a 512 map yet.

## 4. Owner survey

**Ruled 2026-10-30.** The owner amended Q-CF-7, Q-CF-14, Q-CF-17 and Q-CF-19; refined Q-CF-2,
Q-CF-4, Q-CF-5, Q-CF-8, Q-CF-11 and Q-CF-12; confirmed Q-CF-1, Q-CF-3 and Q-CF-6 in their own
words; and accepted every other default as written, through Q-CF-51. Each question keeps its
proposed text under the decision for rationale and history. Numbers stay provisional (§2, last
row); Q-CF-19 was the one default marked expensive to reverse, and the owner reversed it knowingly.

### A. The boundary with the simulator

**Q-CF-1 — One kernel or two?**
- **Decision (owner ruling, 2026-10-30):** one kernel shared between IC and OOC, with the OOC
  concepts (red team, blue team, matches) extracted from it and layered separately on top. The
  default below, in the owner's words.
- **Proposed default (as written):** one kernel. The field and the OOC range share `tacsim::arena`'s state,
  step, perception, weapons, damage, motion, heat, tape, snapshot, cockpit frame and mind host.
  The OOC range becomes a field with two parties that happen to be named red and blue. The match
  wrapper (lobby, elimination, stock units and the simulator centre, practice reset, the `SIM`
  verbs) stays OOC-only and the IC host never references it.
- **Alternative:** fork the kernel into a field engine and leave the arena untouched.
- **Why:** 16b's test estate, the deterministic replay gates, the TUI/web/pixel cockpits, the
  bot's pilot and the mind API all target the kernel. A fork doubles every future weapon, sensor
  and damage change and guarantees the two drift. The cost of one kernel is a single
  generalization pass (Q-CF-3) and one rebless of the practice hashes.

**Q-CF-2 — Names and module.**
- **Decision (owner ruling):** *field*, not *engagement*, because the engagement is optional: a
  field can exist with no engagement in it. *Party*, not *side*, because side reads as a hit
  location. The vocabulary this fixes for the rest of the document: a **site** is the ground and
  its persistent record (Q-CF-24); a **field** is the site while it is promoted, the wall-time
  host with every party on the ground admitted; an **engagement** is a Hostile pair actually
  fighting inside a field, and a field may hold none (a leader marks a party Hostile and nobody
  fires; the field demotes under Q-CF-20). A field is identified by its site and an ordinal
  (`FieldId { site, ordinal }`); *match* stays the OOC word and identifier (D-23-10).
- **Proposed default (as written):** *field* for a site's engagement (`tacsim::field`); *party*
  for an independent side in it; *relation* for one party's view of another.
- **Alternative:** *engagement* and *side*.
- **Why:** doc 27 already says field and party-like things; D-23-10 already separated match from
  IC; *side* collides with the kernel's left/right `Side`.

**Q-CF-3 — Which parts of the wrapper are kernel-shaped and get generalized rather than excluded?**
- **Decision (owner ruling):** agreed. Red and blue are two parties; the simulator happens to
  run with only two, on a field layer that supports any number.
- **Proposed default (as written):** generalize in place, in one cut, with one mind-API, snapshot-format and
  replay-hash bump: `Team` becomes a party id (`PartyId`, small integer, allocated per field);
  `Classification::Classified` carries the party; every `team != mine` becomes a relation test
  (Q-CF-5); `EntryZone` becomes a per-party entry (Q-CF-17); `Elimination` becomes an end
  condition the host supplies (the OOC host keeps elimination, the IC host uses Q-CF-20); the
  callsign letter comes from the party (Q-CF-11); the cockpit frame's team field becomes the
  viewer's relation to the unit; the mind API exposes relation, not party. OOC practice hashes
  are reblessed exactly once, at that cut.
- **Alternative:** keep `Team` in the kernel and have the IC host map parties onto two teams.
- **Why:** the alternative cannot express three parties, which Q-H3 requires. Doing the pass
  once, before any IC host code exists, is the owner's stated reason for pinning the plan.

### B. Parties and relations

**Q-CF-4 — What is a party?**
- **Decision (owner ruling):** a party is a detachment. Several parties may share an
  organization; that shapes their relation (Q-CF-5) and does not merge them. One reading is
  recorded for the owner to object to (F-5): a rig launched with no roster at all is admitted as
  a one-unit party, the degenerate detachment, so the designator and relation machinery has one
  shape. The carrier belongs to the party of the roster it launched (Q-CF-19 makes it a unit).
- **Proposed default (as written):** an accepted detachment roster (one activation) is a party;
  a unit on the site with no roster is a party of one; the carrier's organizer or crew is not a
  party by itself.
- **Alternative:** the organization or faction is the party.
- **Why:** rosters already carry identities, leadership, orders and the IC chat channel.
  Organization membership is an input to relations (Q-CF-5), not the identity of a side.

**Q-CF-5 — How is one party's relation to another derived?**
- **Decision (owner ruling):** the default stands, with follow-up F-1: relations between
  organizations, so that organizations can declare on each other and establish alliances, as a
  further input to the same function. Q-CF-7's marks sit above everything derived.
- **Proposed default (as written):** a per-viewer relation with four values: *Friend*, *Hostile*,
  *Neutral*, *Unknown*. Friend: the same roster, or two rosters under the same organizer, or
  rosters whose organizers share an organization, or an explicit alliance declared for the
  operation. Hostile: a declaration (Q-CF-7) by or against them, or they fired on me or on a
  Friend of mine in this field and I or a Friend observed it. Neutral: a classified party that is
  neither. Unknown: not yet classified. No `mush-faction` reputation lookup in the first version.
- **Alternative:** faction reputation sets the default relation; declarations and fire override.
- **Why:** everything in the default is derivable from facts the field already holds. Reputation
  is a later input to the same function, not a different function.

**Q-CF-6 — Is hostility a fact or a belief?**
- **Decision (owner ruling):** a belief.
- **Proposed default (as written):** a belief, held per party. A's declaration against B makes B Hostile in
  A's book. B learns of it when the declaration is open (Q-CF-7) or when A's fire on B or on B's
  Friends is observed. Two parties can hold different relations toward each other for a while.
- **Alternative:** one symmetric, global relation per pair.
- **Why:** ambush, hidden theft and mistaken identity are the interesting cases, and doc 27's
  Q-H6 and Q-S4 already treat battlefield knowledge as per-observer. This is the Epistemic Engine
  applied to one fact.

**Q-CF-7 — How does a party become hostile without firing first?**
- **Decision (owner ruling; supersedes the default):** a detachment leader can explicitly mark
  another party Friendly or Hostile. The mark is private: the other party is not told of the
  transition. A Friendly mark is also how a leader cancels an automatic hostility that friendly
  fire produced. Marks sit at the top of the party's own book: explicit mark, then fire-derived
  hostility, then roster and organization, then Neutral or Unknown. The promotion a Hostile mark
  causes (Q-CF-14) is a physical fact everyone on the site observes; who marked whom is not.
- **Proposed default (superseded):** an explicit declaration verb by any armed unit, open to the
  target party at once.
- **Alternative:** declarations are private and only the first shot reveals intent.
- **Why:** a declaration exists to admit combat deliberately (Q-H2) and to warn. Hiding it buys
  nothing an ambush does not already provide, and it would make promotion invisible to its victim.

**Q-CF-8 — Does hostility outlive the field?**
- **Decision (owner ruling):** for now hostility is a temporal function of the field and ends
  with it, roster and organization relations aside. Follow-up F-2: hostility should be able to
  affect faction relations later.
- **Proposed default (as written):** no, in the first version. Relations reset at demotion except Friend by
  roster or organization. A durable per-character and per-organization "known hostiles" memory is
  a later Epistemic Engine cut on top of the same relation function.
- **Alternative:** persist hostile relations per party pair indefinitely.
- **Why:** scope. The persistent half needs the NPC knowledge model and the canon ledger, which
  have their own owners.

**Q-CF-9 — Friendly fire, and a neutral under fire.**
- **Decision (accepted default):** physically always possible; the kernel never refuses a shot by relation.
  Doctrine and NPC pilots never target Friend, Neutral or Unknown unless a delivered order names
  a specific contact, and refuse that order for a Friend with an honest NEGATIVE. A Neutral that
  is fired on becomes Hostile to the shooter's party in its own book; the shooter's Friends do not
  automatically become Hostile to it.
- **Alternative:** fire on a neutral makes the shooter's whole alliance hostile to it.
- **Why:** per-pair relations are what make a bystander a bystander. Escalation by alliance is a
  later political rule, not an engine rule.

**Q-CF-10 — How does a viewer learn a contact's party?**
- **Decision (accepted default):** as perception works today: a contact starts Unknown and is classified by
  the sensor rules over time; the classification carries the party id; a classified contact's
  relation is then the viewer's (Q-CF-5). No transponders or IFF squawk in the first version.
- **Alternative:** an IFF transponder that friends read instantly and hostiles can spoof.
- **Why:** the perception model is built and tested, and the lore line already says ECM is what
  kills long-range certainty. IFF is a later unit-kit component, not a field rule.

**Q-CF-11 — Callsigns and designators under parties.**
- **Decision (owner ruling):** two letters, unique across the field: the party first, the unit
  second. Today the second letter is the slot in bijective base-26 (`arena/state.rs:2481`), which
  would make every leader `?A` and leak the slot; so the unit letter is drawn at random per unit
  per field from A–Z less the confusable I and O and the provisional X, and is stable for the
  field. The party letter comes from the same alphabet in admission order. A viewer learns
  nothing about slots from a designator.
- **Overflow (owner ruling, 2026-10-31):** a party past 23 units keeps CF.0's three-letter
  `Designator::Wide` (party letter and a drawn pair, e.g. `RKQ`), refused past 552 units. It
  matters only for the scale harnesses.
- **Proposed default (as written):** one letter per party from a fixed alphabet in admission
  order; the second letter as today; the Designator test extended to N letters.
- **Alternative:** relation-relative designators (own units always `A…`, hostiles `H…`).
- **Why:** two viewers must be able to talk about `CB` and mean the same hull; a relative scheme
  breaks the moment they are in different parties.

**Q-CF-12 — Colour and markers on the clients.**
- **Decision (owner ruling):** colour by relation, not by party; and draw the party letter beside
  the health bar so parties stay visually distinct inside one relation colour.
- **Proposed default (as written):** body colour by the viewer's relation to the unit: own, Friend, Hostile,
  Neutral/Unknown. Doc 22's three team palette slots become four relation slots (own may share
  Friend's palette). The pixel, TUI and web cockpits take the relation from the frame (Q-CF-3).
- **Alternative:** one palette per party.
- **Why:** D-22-10 already said friend or foe is a marker, not a body fact, and a per-party
  palette needs N palettes nobody can tell apart at 96 px.

### C. The field: extent, admission, lifecycle

**Q-CF-13 — The extent of a field.**
- **Decision (accepted default):** one field per site, the whole site window on the site's own ground,
  whatever the window's side is (Q-CF-51). A field never spans sites and a site never holds two
  fields.
- **Alternative:** a smaller engagement window inside the site around the participants.
- **Why:** the ground, the geology, the rig records and the claims are all per site; a sub-window
  would need its own edge rules (Q-CF-22) for nothing.

**Q-CF-14 — What promotes a site?**
- **Decision (owner ruling):** two doors. A detachment leader marking another party Hostile
  (Q-CF-7), or a unit firing at another party. Co-presence, launch, arrival and mining never
  promote. Consequence (§4a): a fire order given on the unpromoted surface is aimed at a
  presence-picture contact (Q-CF-15); it promotes the site, and the shot resolves in the field's
  first pulse under the normal lock rules, so it may well miss or never leave the barrel.
- **Proposed default (as written):** a declaration by a unit on the site as the one door, since
  weapons and contacts exist only in the field.
- **Alternative:** two non-Friend parties on one site promote it automatically (always-on fields).
- **Why:** Q-H2 ruled that arriving with a different faction does not start a match, and an
  always-on field costs an arena task for every shared site, most of which are peaceful.

**Q-CF-15 — What does a unit on the surface know about other flights on its site before promotion?**
- **Decision (accepted default):** a coarse presence picture published by the surface tick: for each other
  flight on the site, a bearing and a range band at cell resolution and, once within a short
  visual range, its hull class. No identity, no party, no damage. It is what a declaration can
  name. It is not a sensor contact and carries no `ContactId`.
- **Alternative:** nothing until promotion; declarations name a party blind.
- **Why:** a raider has to see the miners to decide to raid them, and the miners' escort has to
  see something coming. Without this the field can only start by surprise. This is the one new
  surface-side mechanism in the survey and the owner should rule it knowingly.

**Q-CF-16 — Who is admitted at promotion?**
- **Decision (accepted default):** every flight physically on the site, at its exact surface pose, rigs
  mid-drive and mid-mining included, whatever its party or relation. The carriers on the pads are
  present per Q-CF-19. Nobody on the ground is outside the field.
- **Alternative:** only the declaring and the declared parties; others stay on the campaign
  surface and cannot be targeted.
- **Why:** a bystander that cannot be shot is not in the world. Q-H3 asked for neutrals that
  exist.

**Q-CF-17 — Entry during a field (reinforcement).**
- **Decision (owner ruling; supersedes the default):** refuse launches while the site is
  promoted. The field runs at human speed and the world outside it at 24×, so a fight is over
  before anyone outside could reasonably reinforce. A reinforcement system is reserved for later
  (F-4). Consequences (§4a): entry zones leave the field layer entirely; promotion is the only
  entry; recovery at the ramp (Q-CF-18) still works; a carrier arriving by voyage mid-field is
  F-6.
- **Proposed default (superseded):** launches from any carrier on the site enter at its ramp
  pose; a carrier arriving mid-field settles and may launch.
- **Alternative:** refuse launches while the site is promoted.
- **Why:** reinforcement is in Q-H3, and refusing launches would let a raider lock a carrier's
  bay by declaring.

**Q-CF-18 — Leaving a field.**
- **Decision (accepted default):** recover at the own carrier's ramp (`+sortie/recover` as it stands),
  individually and at any time if mobile and the ramp is reachable. No edge exit in the first
  version. A unit left behind stays per Q-H10.
- **Alternative:** site-edge withdrawal that removes the unit to the campaign surface.
- **Why:** there is nowhere beyond the edge to go until cross-site surface travel exists.

**Q-CF-19 — The carrier on its pad.**
- **Decision (owner ruling; supersedes the default):** the carrier is attackable on the ground,
  because the carrier is the ship, the ship leaves the map to fly in space and can be attacked
  there, and what can be attacked in space can be attacked on a planet. It is admitted as a unit
  at its pad pose with its real sections (the Dray, Dray-P and Halcyon already have unit specs
  with sections and facet bands under `crates/tacsim/content/units/`). Its damage persists like
  any unit's. What a hit does to lift, sailing, the bay and the hold, the loss story, and parity
  with space combat are F-3. The owner took this knowing it was the survey's one
  expensive-to-reverse default.
- **Proposed default (superseded):** a static, non-targetable structure with its real footprint,
  its party's entry and exit; no damage or boarding in the first version.
- **Alternative:** the carrier is a unit with sections, targetable from the start.
- **Why:** carrier damage, boarding and loss are a design of their own (doc 20 D-20-7's lift and
  doc 23's ship passages touch it) and should not ride in on the first field.

**Q-CF-20 — The demotion rule.**
- **Decision (accepted default):** a field demotes when, for `[tacsim.field].disengage_s` (provisionally
  120), no Hostile pair has fired or held a lock, no projectile is in flight and no entry or exit
  is mid-handoff (Q-H4). Demotion writes every unit's pose and condition back to the surface and
  the site record (Q-CF-24), leaves disabled units at their poses, and ends the task. It never
  ends on a "winner". A field whose only remaining units are Friends of each other demotes after
  the same interval.
- **Alternative:** stay tactical while any two non-Friend parties remain on the site.
- **Why:** Q-H4's letter; and an escort standing guard over miners for an hour is not a fight.

**Q-CF-21 — Capacity and refusal.**
- **Decision (accepted default):** one cap, `[tacsim.field].max_units` (provisionally 24), enforced at the
  surface: the site's existing per-site flight cap (32 in the excursion rules) is lowered to the
  field cap, so a promotion can always admit everyone on the ground. A launch that would exceed it
  is refused at the ramp with the reason, during a field or not. No partial admission ever.
- **Alternative:** admit in party order until full; the rest stay on the surface untargetable.
- **Why:** Q-CF-16. A unit that is on the ground and not in the field is the one state the
  design forbids, and refusing at the ramp is the only place with a player to tell.

**Q-CF-22 — Edges: fire and pursuit at the site boundary.**
- **Decision (accepted default):** the site boundary is impassable ground inside the field, exactly as it
  is on the campaign surface. A shot that leaves the window is lost; nothing is beyond it to hit
  or to flee to; no immunity question arises.
- **Alternative:** open edges with units leaving to an adjacent site.
- **Why:** sites are islands; the day they are not, this answer changes with the surface's, not
  on its own.

**Q-CF-23 — Tempo when nobody is watching.**
- **Decision (accepted default):** the kernel's adaptive tempo; with no player connected to any unit in
  the field, the coarse floor; a field eligible to demote does so (Q-CF-20) rather than idling.
  An all-NPC field (raider against hired escorts after the players disconnect) runs to its own
  demotion.
- **Alternative:** pause a field with no players.
- **Why:** pausing would make disconnecting a shield. Q-H5 already says the hull stays at risk.

### D. Persistent ground state: salvage, remains, cargo

**Q-CF-24 — The site record.**
- **Decision (accepted default):** one durable record per site on the authoritative lane
  (`tacsim.field.<site>`): the field ordinal, the units left on the ground with their poses and
  the epoch they were in when left, the non-unit objects (Q-CF-25) with poses and manifests, and
  the last demotion's IC instant. Every promotion loads it into the kernel; every demotion writes
  it. It survives restart, the carriers' departure and any number of later fields. The unit
  records themselves stay where they are (`unit:<obj>` under tactics custody); the site record
  indexes them and owns only the non-unit objects.
- **Alternative:** no site record; rebuild the ground state at promotion by scanning every unit
  whose placement is on the site.
- **Why:** dropped cargo, ore and the remains of an irrecoverable hull have no unit record, and a
  promotion should be a read of one document, not a scan.

**Q-CF-25 — What counts as salvage on the ground, and what is each thing?**
- **Decision (accepted default):** four kinds. (a) *Casualties*: disabled, immobile or unarmed units with
  their full unit record and an open epoch; towable (H2). (b) *Wrecks*: units in the Wreck state,
  same record, open epoch. (c) *Remains*: the site-record object an irrecoverable unit leaves
  after its epoch closes destroyed: a footprint, a mass and a manifest of nothing in the first
  version (H3 decides whether anything strips). (d) *Dropped cargo*: ore and goods containers as
  site-record objects with a manifest, left by a destroyed hopper or an explicit dump. Crew are
  never a ground object; crew state lives on the character (Q-CF-38).
- **Alternative:** everything on the ground is a unit record, remains and cargo included.
- **Why:** unit records are heavy and owned by tactics custody; a crate is not a tank. Keeping
  non-units as site-record objects also keeps the field cap (Q-CF-21) about things that act.

**Q-CF-26 — How ground salvage behaves inside a field.**
- **Decision (accepted default):** casualties and wrecks are units in the kernel: targetable (Q-H8), they
  block movement and give cover by the existing section and line-of-sight rules, and they do
  not count against the field cap. Remains and cargo are static obstacles with cover by their
  footprint, and are sensor contacts, so a scavenger can find them.
- **Alternative:** ground salvage is decoration in the field; only H2's recovery sortie sees it.
- **Why:** a wreck you can hide behind and shoot through is the whole reason a field remembers.

**Q-CF-27 — Decay and limits.**
- **Decision (accepted default):** no decay in the first version. A per-site cap on non-unit objects
  (provisionally 64); when it is reached, the oldest remains merge into one object at the
  centroid under a published rule; cargo and units are never dropped by the cap.
- **Alternative:** time decay of remains and cargo after N IC days.
- **Why:** persistence is the point. The cap exists only for memory and for the M7 gate.

**Q-CF-28 — Who knows what lies on a site?**
- **Decision (accepted default):** per observer (Q-H6). A party knows the salvage it saw in a field it was
  in, and what a later surface scan or survey on the site reveals; the site record is objective
  but is never published whole. There is no global wreck map and no registry of casualties. The
  owner of a left-behind unit knows where they left it (Q-H10's confirmation names it).
- **Alternative:** a public list per site, since the ground is physical and anyone could look.
- **Why:** "anyone could look" is a survey action, which is exactly the mechanism the default
  names. Q-S4 already keeps hidden theft hidden until evidence exists.

**Q-CF-29 — Salvage on claimed ground.**
- **Decision (accepted default):** physical presence is unaffected by mining claims; a casualty on a claimed
  block blocks nothing but the cells it occupies; entitlement to recover it is H2-B's and is not
  decided here.
- **Alternative:** a claim holder may have salvage cleared or claims the salvage.
- **Why:** doc 27 §D settled recovery entitlement; the field only has to remember the thing.

**Q-CF-30 — Surface work during and after a field.**
- **Decision (accepted default):** every surface act on a promoted site is refused for the field's
  duration (Q-H1); a rig mid-act at promotion keeps the distance it completed and the ore it
  extracted, is charged for that, and the act's remainder is cancelled without duplicate yield.
  Work resumes on demotion by a fresh act. Fill plans (doc 20 SM.11) pause and resume.
- **Alternative:** an unengaged party inside a field may keep mining.
- **Why:** Q-H1's letter, and the clock discontinuity makes a mining cycle inside a wall-time
  field meaningless.

### E. Time

**Q-CF-31 — The handoff for a unit mid-drive at promotion and demotion.**
- **Decision (accepted default):** at promotion the drive is settled at the cell it completed: charge is
  spent for the completed cells, the remaining leg is dropped, the field pose is the cell pose.
  At demotion the surface pose is the field's exact pose (`FieldPose`, which the surface already
  accepts); any intent the unit had in the field is dropped; a new drive is a new act.
- **Alternative:** finish the leg instantly at promotion.
- **Why:** no duplicate motion and no free distance in either direction.

**Q-CF-32 — IC time inside a field.**
- **Decision (accepted default):** the field runs on wall time (Q-R3); the world's IC clock keeps its rate;
  the field stamps its black-box lines and notices with the world's IC instant at the pulse, so
  they sort with everything else; messages and arrivals from outside admit at pulse boundaries.
  The compression is accepted (Q-H1: a ten-minute fight is four campaign hours at rate 24).
- **Alternative:** hold the site's IC time still for the field.
- **Why:** Q-R3 is ruled; holding one site's IC time would desynchronize its claims and deadlines.

**Q-CF-33 — The outage freeze.**
- **Decision (accepted default):** a field in progress at an outage resumes from its last snapshot after
  the countdown ends, not at boot; a field whose resume is refused demotes from that snapshot
  (Q-CF-35). The snapshot cadence is the kernel's.
- **Alternative:** resume at boot and let the countdown pass unnoticed inside the field.
- **Why:** FO-11's "no unseen shooting during the countdown". This is the one W-clock integration.

### F. Custody and durability

**Q-CF-34 — What "committed" means.**
- **Decision (accepted default):** an outcome is committed when it is in the last snapshot the world
  acknowledged. A resume rewinds to it and every pilot reads the existing resume line.
- **Alternative:** per-chunk durability so no acknowledged shot is ever rewound.
- **Why:** D-16-1's seam and 16c Q-AF4 already chose the rewind; per-chunk durability means
  custody writes on the hot path.

**Q-CF-35 — The void rule for IC units.**
- **Decision (accepted default):** a voided field (panic, stall, ledger overflow, refused resume) demotes
  every unit from the last snapshot. It never restores the pre-field check-out snapshot, which is
  the OOC rule and stays the OOC rule.
- **Alternative:** restore, as the OOC range does.
- **Why:** FO-1: restart does not heal.

**Q-CF-36 — The check-in plan.**
- **Decision (accepted default):** the IC condition plan (27a H1-A: only the custody vocabulary, the blanket
  wreck stamp stripped from sections with internals left) through tactics custody; poses to the
  surface; epochs untouched (Q-H9); cargo and ammunition cells as the kernel left them.
- **Alternative:** the OOC diff as it stands.
- **Why:** the OOC diff can mark a mission-killed hull wrecked in every section.

**Q-CF-37 — Who may read a field's tape.**
- **Decision (accepted default):** staff, through the existing `@arena/report`; players and the bot get
  observer-bounded digests and their own black-box lines (Q-H6, doc 23); no replay is published.
- **Alternative:** a public replay per field.
- **Why:** Q-H6.

### G. Crew and control

**Q-CF-38 — The minimal crew model.**
- **Decision (accepted default):** two fields on the character: a location (aboard a unit, stranded on a
  site, aboard a carrier, at a port) and a condition (fit, injured, incapacitated), set at
  check-in and immediately on a control-section kill, so the player learns it when it happens.
  No treatment, no ejection, no capture in the first version.
- **Alternative:** a richer injury model with compartments and treatment.
- **Why:** FO-12 and Q-H12 want survival and extraction to exist, not medicine.

**Q-CF-39 — Dismounting and extraction.**
- **Decision (accepted default):** a character cannot leave a hull on foot inside a field (no infantry
  yet, Q-D1); a stranded or incapacitated character requests extraction (`+sortie/extract` or the
  cut's name) and after a configured IC delay is at the nearest known safe port under Q-F4/P-5,
  deferred only while a Friend's recovery flight is physically on the site within the published
  bound. The hull, its cargo and its epoch stay.
- **Alternative:** on-foot movement in the field and a rescue vehicle to reach.
- **Why:** the infantry group of Q-D1 is future work; extraction as a bounded teleport-with-delay
  is what P-5 accepted.

**Q-CF-40 — Link loss.** Ruled (Q-H5, P-2). Not asked.

**Q-CF-41 — The weapons-free gate in IC.**
- **Decision (accepted default):** HOLD at launch in IC fields; the pilot sets FREE; doctrine and NPC pilots
  obey a delivered HOLD FIRE over their own gate. The OOC range keeps FREE (Q-21-9).
- **Alternative:** FREE everywhere.
- **Why:** an accidental first shot in IC declares hostility (Q-CF-5) and has consequences; the
  gate is the core-crime guard Q-H2 kept.

### H. NPCs, minds and opposing forces

**Q-CF-42 — Hired human NPC pilots in a field.**
- **Decision (accepted default):** their units fly the kernel's doctrine under their delivered standing
  order through the DT.1b adapter, with relation-aware targeting (Q-CF-9); the reply travels back
  on the field-orders transport; the surface NPC executor stays surface-only. No new AI.
- **Alternative:** extend the surface executor to drive the unit from the campaign tick.
- **Why:** combat decisions happen at wall time inside the task; the surface tick cannot see
  contacts and should not try.

**Q-CF-43 — Licensed Rhai minds in IC fields.**
- **Decision (accepted default):** deferred to doc 23 S.4b and the M8 line; the mind-API bump that exposes
  relation lands with Q-CF-3 so S.4b inherits it and no second bump follows.
- **Alternative:** host minds in the first field.
- **Why:** the claim, pool and resume rules are their own programme.

**Q-CF-44 — Opposing forces, for test and for play.**
- **Decision (accepted default):** fixture-seeded NPC parties at named frontier sites: a raider roster of
  N armed hulls with a doctrine preset, present on the site or launched from a fixture carrier,
  authored as content like the Slack Line crew. No generator, no respawn, no patrol logic. The
  same fixture is the bot's opponent.
- **Alternative:** a population or mission generator.
- **Why:** repeatable tests first; a generator is a content system with its own survey.

### I. Scale

**Q-CF-45 — The first field budget.**
- **Decision (accepted default):** 24 units a field (two full rosters, a raider party of six, six
  bystanders), one task per site, the M7 memory gate extended with one full-field fixture, and a
  measured run at 24 before the cap is raised toward the 100-unit target. Ground salvage does not
  count against it (Q-CF-26).
- **Alternative:** 12, the two-roster minimum.
- **Why:** the escort flow needs three parties on one site from the first acceptance; 12 cannot
  hold it.

### J. Acceptance

**Q-CF-46 — The engine gate, on the OOC range.**
- **Decision (accepted default):** before any IC host code, a three-party practice on the simulator
  centre proves parties, per-viewer relations, N designators, relation colouring in all three
  cockpits, demotion in place of elimination, the mind-API bump and the single rebless of the
  practice hashes. The bot's arena pilot runs against it unchanged except for the frame field.
- **Alternative:** prove parties only in the IC field.
- **Why:** the kernel change is testable without any of the IC seams, and gating it there keeps
  the IC host's own gate about the IC host.

**Q-CF-47 — The field story that accepts the engine.**
- **Decision (accepted default):** at a named frontier site, a mining roster works a block with an
  escort of two armed hulls; a bystander rig from another captain works the next block; a raider
  party launches from a fixture carrier, is seen on the presence picture (Q-CF-15), declares,
  and the site promotes with everyone at their poses; the miners see the raiders as contacts on
  the quarry screen and the escort sees the worked block on its map (Q-CF-48); the escort
  fights under delivered orders while the miners hold; the bystander is fired on by a raider and
  becomes hostile to that party only; one escort is mission-killed by an engine hit and one raider by a control-section hit;
  the raiders withdraw to their ramp; the field demotes; the miners resume their fill plan; the
  casualty and the raider wreck are on the site record; the server restarts; the carrier
  departs with the leave-behind confirmation; a second visit finds both where they fell; a second
  field at the same site uses the wreck as cover. Run through production composition over SSH,
  then by the bot with the raider as the fixture opponent.
- **Alternative:** the two-player duel of doc 27b as the first acceptance.
- **Why:** the duel proves a two-party host; this story is the flow the owner named, and it
  exercises every area above once.

### K. The picture: one map for mining and fighting

The owner's addition on 2026-10-30: the mining view and the field view have to be one view.
Today they are two pictures of the same ground. `+tv/map` and the TUI quarry screen (`q`, doc
20 SM.9) draw the site as cells, blocks, pockets, claims and the rig's own state, on the
campaign clock; the cockpit (doc 21: the TUI, the web, the pixel compositor and its map window,
doc 30's tiles) draws contacts, locks and poses, on wall time. A miner cannot see a raider
coming and an escort cannot see which block it is guarding.

**Q-CF-48 — One picture of the site.**
- **Decision (accepted default):** one site map at one cell grid, with two layers. The *campaign layer*
  is always drawn: ground and biome, blocks and claims, the pockets the party has read, the
  salvage the party knows (Q-CF-28), the presence picture (Q-CF-15), the carriers and their
  ramps. The *field layer* is drawn while the site is promoted: contacts coloured by relation
  (Q-CF-12), locks, own and Friend units, fire. `+tv/map` and the quarry screen become the text
  renderings of that one map; the cockpit's map window is its graphic rendering; the web studio
  stays an authoring tool and not a play view. D-21-14 holds throughout: the picture draws what
  the frame says and invents nothing.
- **Alternative:** two views that share the ground data and nothing else.
- **Why:** the escort flow is the acceptance story (Q-CF-47); it needs the escort to see the
  worked block and the miner to see the threat. Two pictures of one world will diverge.

**Q-CF-49 — The rig's cab is a cockpit.**
- **Decision (accepted default):** a mining rig inside a field has the same cockpit frame as an armed
  hull: its visual sensor gives it contacts, it has no weapons, and the quarry screen's rig state
  (hopper, charge, plan, the drive view) becomes a panel on that frame rather than a separate
  screen. `+tv` status keeps answering for the rig's own state inside a field. The escort's
  cockpit shows the claims, blocks and ore markers its party knows.
- **Alternative:** rigs have no cockpit in a field; only `+tv` and the quarry screen.
- **Why:** the miner's one combat action is to see the raider and drive to the ramp, and the
  escort's one mining question is what it is standing over. Both are the same frame.

**Q-CF-50 — Frame plumbing.**
- **Decision (accepted default):** one frame family carries both layers. The cockpit frame gains the
  campaign layer as a slowly changing block (ground, claims, known salvage, presence) beside the
  existing per-pulse field block. On an unpromoted site the surface host sends the frame at the
  surface tick's cadence with an empty field layer; inside a field the task sends it at the
  arena's. Click-to-lock works on the field layer only. The overview, 2.5D and first-person views
  draw ground salvage as the kernel holds it (Q-CF-26). This rides the same format bump as Q-CF-3.
- **Alternative:** the surface sends a frame kind of its own and the clients grow a second
  renderer.
- **Why:** the three clients already draw one frame; a second kind is a second renderer in each
  of them, and the two would disagree about the same cell.

### L. The size of the ground

The owner asked on 2026-10-30 whether the field should wait for the window to grow. The facts
(§3): the sim's window and the site are 256 a side by constant and nothing in the tree has
changed that; doc 30 ruled 512-a-side *maps* and owes WG.6 to play them, with the window on the
frame; doc 16 D-16-5 records the 10 km path (a thousand cells, 1024 in its power-of-two form) as
an addition never scheduled; docs 16 and 20 were not amended when doc 30 ruled.

**Q-CF-51 — Does the field wait for the bigger ground, and what does it assume meanwhile?**
- **Decision (accepted default):** the field does not wait, and it never names a side. In the kernel pass
  of Q-CF-3 the three constants become one per-region value read from the region's window
  (the `ArenaDef`'s tiles already say how big it is), with 256 the shipped default; every number
  in this survey is per cell, per area or per unit, never per window; the presence picture's
  range bands (Q-CF-15), the per-site caps (Q-CF-21, Q-CF-27) and the field budget (Q-CF-45)
  are tuned at 256 and re-measured at 512. WG.6 (the frame window and the tiles) runs beside
  the field as the client cut it is; the field's acceptance story (Q-CF-47) runs once at 256
  and again at 512 when WG.6 lands, and the second run is WG.6's gate as much as the field's.
  The 10 km path stays a separate scale programme: it changes the line-of-sight index, path
  search, the snapshot and the memory gate, and it belongs with the 100-unit target, not here.
  As part of this ruling, doc 16 §11 and doc 20 §3.1 gain one amendment row each pointing at
  doc 30 Q-30-6 and WG.6, so the three documents agree on what "the window" means today.
- **Alternative:** land WG.6 and lift the sim to 512 first, then build the field on it.
- **Why:** the field model has no size dependence once the constants are parameters, and the
  owner's reason for pinning doc 27b (no assumptions baked in that have to be undone) is met by
  parameterizing, not by waiting. Serializing the two programmes buys nothing the second
  acceptance run does not, and costs the field the weeks WG.6 and the 10 km work take.

## 4a. Consequences of the amended rulings

- **Vocabulary (Q-CF-2).** *Site*, *field*, *engagement* as defined there. Promotion opens a
  field on a site; demotion closes it; the site record (Q-CF-24) outlives every field. Where
  this document said "field" before Draft 0.2 it meant the promoted host, which is still what it
  means.
- **Private marks, visible promotion (Q-CF-7, Q-CF-14).** A Hostile mark by a leader promotes
  the site. Everyone on the ground observes the promotion: their clock changes, contacts appear.
  Nobody is told who marked whom. A party that finds itself in a field with no shot fired knows
  only that someone on the site considers someone hostile.
- **Fire as a door (Q-CF-14).** The campaign surface gains exactly one weapon-shaped act: a fire
  order at a presence-picture contact. It is a promotion trigger, not a shot; the shot resolves in
  the field's first pulse under the normal lock rules. With HOLD at launch (Q-CF-41) the pilot has
  set FREE first. Doctrine runs only inside a field, so an NPC can never open one by itself; a
  delivered ENGAGE received on the surface is treated as its leader's Hostile mark (Q-CF-7) and
  promotes that way.
- **Who may mark.** A detachment leader (doc 26's slot-order leader, doc 27 Q-D4). The pilot of a
  one-unit party is its leader.
- **No launches while promoted (Q-CF-17).** Entry zones leave the field layer; the OOC layer keeps
  two for its two parties. `+sortie/launch` and `+tv/launch` refuse with the reason. A departure
  already on the ramp at promotion completes into the field at the ramp pose, because the hull is
  physically committed (Q-H4's handoffs settle). Rigs still in a bay are inside the hull and not
  in the field.
- **The carrier is a unit (Q-CF-19).** Admitted at its pad pose with its sections; everyone
  aboard carries crew state "aboard the carrier" (Q-CF-38); its damage persists through the same
  custody; it is beyond FO-13's twice-own-tonnage tow rule and so never towed. Whether it may depart during
  a field, under Q-H10's confirmation, and what a drive hit does to that, are F-3 and F-6.
- **Designators (Q-CF-11).** The Designator keeps its two-letter shape (16b Q.1); the OOC host
  names its two parties red and blue and so keeps `R` and `B`; the IC host assigns party letters in
  admission order and unit letters at random. The Designator test grows the alphabet and the
  no-slot-leak property.
- **Party = detachment (Q-CF-4).** The party id is the activation reference; relations are kept
  per activation; the organization reaches the relation through Q-CF-5 only.
- **The ground's side (Q-CF-51, accepted).** `ARENA_CELLS`, `SITE_CELLS` and `MAX_TILE_SIDE`
  become one per-region value in cut CF.0. The amendment rows for doc 16 D-16-5/§11 and doc 20
  §3.1/§11 ship with this draft.

## 5a. Follow-ups opened by the rulings

| # | Follow-up | Opened by |
|---|---|---|
| F-1 | Relations between organizations: declare on each other, establish alliances, as an input to the relation function | Q-CF-5 |
| F-2 | Hostility in a field affecting faction relations afterwards | Q-CF-8 |
| F-3 | The carrier under fire: what a hit does to lift, sailing, the bay and the hold; the loss story for a destroyed carrier with crew and rigs aboard; parity with attack in space | Q-CF-19 |
| F-4 | A reinforcement system for a promoted site | Q-CF-17 |
| F-5 | The one-unit party reading: a rig launched with no roster is admitted as the degenerate detachment | Q-CF-4 |
| F-6 | A carrier arriving by voyage during a field (admitted at its pad as a unit at the next pulse boundary, may not launch) and a carrier departing during one. **Departure ruled 2026-10-31, built in cut CF.7 (§12):** after the leave-behind confirmation (Q-H10, CF.5's `leave=`), a carrier departing a running field is removed from it outright and lifts; it is not a ramp withdrawal, and the units it leaves stay in the field. Arrival stays unruled. | Q-CF-17, Q-CF-19 |
| F-7 | Persistent known-hostile memory per character and organization, the Epistemic Engine cut | Q-CF-8 |

## 5. Out of scope for this survey

Towing, recovery operations and salvage entitlement (27a H2, doc 27 §D); workshop restoration
(H3); DT.4 subset delegation; licensed minds in the field (S.4b); infantry and battle armor
groups; carrier damage, boarding and loss; cross-site surface travel and open edges; facility
assault; faction reputation as a relation input; a mission or population generator; wages;
persistent hostility memory. Each is named above where a default touches it, and each keeps its
own owner.

## 6. Proposed cuts

`CF.*` names the field engine's cuts. Each is independently reviewable with its gate shown before
the next starts; the lanes below say what runs beside what. Ordering is the owner's to change.

| Cut | Content | Gate | Depends on |
|---|---|---|---|
| CF.0 — parties in the kernel | `PartyId` replaces `Team`; per-viewer relations and the leader's marks; unit letters at random, party letters in order; relation colour and the party letter on the health bar; the end condition supplied by the host; the window side as a per-region value; the mind API, snapshot, hash and frame bump taken once; the OOC range as a two-party field named red and blue | Q-CF-46: a three-party OOC practice; practice hashes reblessed once; the bot's arena pilot unchanged but for the frame field | nothing |
| CF.1 — the site record and the presence picture | the per-site record (units left, remains, cargo); the surface tick's presence picture; surface acts refused while promoted; the per-site flight cap lowered to the field cap | a staff fixture leaves a wreck and a crate on a site; restart; `+tv/map` shows presence bands; refusals name the fight | nothing; runs beside CF.0 |
| CF.2 — the field host | promotion by mark or fire order; admission of everyone at surface poses through custody; launch refusal; recovery as exit; demotion by disengagement; the IC void rule and resume after unfreeze; the IC check-in plan; the clock handoff mid-drive; the site record loaded and written | two players in fixture-seeded armed rigs fight over SSH; the loser stays where it fell; restart mid-fight; a bystander rig reads as an Unknown contact and is shootable | CF.0, CF.1 |
| CF.3 — the carrier as a unit | admitted at its pad with its sections; crew aboard; damage persists; departure during a field under Q-H10; F-3 scoped | a carrier takes a hit and the damage shows in port | CF.2 |
| CF.4 — one picture | the campaign and field layers in one frame; the quarry screen as a panel; the rig's cab as a cockpit; `+tv/map` and the map window from the same frame | Q-CF-48 and Q-CF-49's statements over the TUI, the web and the pixel cockpit | CF.1, CF.2 (the frame bump rides CF.0) |
| CF.5 — crew and exit | crew state; `+sortie/extract`; the leave-behind confirmation on departure | a stranded player is at port after the delay; departure with units out is refused without the confirmation | the verbs: nothing; the derivation at check-in: CF.2 |
| CF.6 — NPC pilots and raiders | hired NPC units on doctrine under delivered orders in the field (the DT.1b adapter on the IC host); relation-aware targeting; HOLD FIRE; ENGAGE on the surface as the leader's mark; the fixture raider party as content; the combat-duty offer; the armed rig lease (doc 27b FC.1's content, unchanged) | ENGAGE through jamming arrives late and the WILCO later; HOLD FIRE within a pulse; NEGATIVE on a lost contact; the raider party fights to its own demotion with no player connected | CF.0, CF.2 |
| CF.7 — the field story | Q-CF-47 over SSH, then the bot against the fixture raider | the story as written at 256 a side; again at 512 when WG.6 lands | all of the above |

```
day 0:   CF.0 parties ──────┐
         CF.1 site record ──┼─▶ CF.2 field host ─┬─▶ CF.3 carrier ────┐
         CF.5 verbs ────────┤                    ├─▶ CF.4 picture ────┼─▶ CF.7 story + bot ─▶ doc 27b revised
         CF.6 content ──────┘                    ├─▶ CF.5 derivation ─┤
                                                 └─▶ CF.6 execution ──┘
```

Four lanes open on day one: CF.0 and CF.1 are the two halves of the engine and touch disjoint
code; CF.5's verbs and CF.6's content (the offer, the lease, the raider pack) need no engine.
CF.2 is the spine and should get the strongest session. WG.6 (doc 30) runs beside all of it and
meets CF.7 at the 512 run.

When CF.7 passes, doc 27b is revised to sit on the engine: its seven steps stand, its FC.1
content work has already landed as CF.6's content, and its FC.2 through FC.5 become the field
story's slices. Doc 27 §5 gains one row for the field engine and marks W-clock built.

## 7. CF.2 as built (the field host)

Built 2026-10-30 on `main` at `a22770e4`; the module is `crates/tacsim/src/field/`. What the cut
had to decide where the rulings left it open, for the owner to object to:

- **How a fire order or a mark names a presence row.** By what the viewer reads: an octant and,
  when two flights share it, a band (`+field/fire NE`, `+field/mark E near=hostile`). The flight
  meant is the nearest match, seen from the nearest of the viewer's own units. The picture hands
  out no token; nothing about a row's identity is printed back.
- **Party letters in admission order** are the parties' keys in order, never the promoter
  first: an `A` for whoever marked would tell the ground who marked whom (Q-CF-7).
- **The class rule is the simulator's.** "No arena fields a vehicle" (doc 18 D-18-29) is the
  OOC roster rule; the IC host admits every unit on the ground whatever its class
  (`FieldHost::every_class`), so a prospector or a hauler is in its field (Q-CF-16, Q-CF-49).
- **A hull on IC ground is running.** The field opens every unit online; `STARTUP` and its
  ten seconds are the simulator's cold start.
- **An IC field closes no hull's epoch** (Q-H9): the kernel's `sortie_closed` lines (`match_end`,
  and a wreck's `destroyed`) are the OOC host's only.
- **The opening shot** waits up to ten arena seconds for the shooter's own sensors to hold a
  contact on the target's cell, then locks it and fires the first loaded weapon through the
  pilot's seat. No contact in the window, no shot.
- **Demotion** uses the kernel's own `Engaged` triggers (a shot, an impact due, hostiles within
  weapons range of each other) with no round in flight, for `disengage_s` of arena time. Two
  hostile parties parked in each other's weapons range therefore stay a field; a restart restarts
  the quiet interval. The kernel's duration bound stays as a four-hour backstop and demotes too.
- **Ramp handoffs settle first** (Q-H4): a launch, a recover from beside the ramp, a drive or a
  reading from the bay and an unload finish as they would, and admission waits for them. A
  mining run stops after the cycle in flight, which keeps its ore.
- **The field's task** is keyed in the shared arena registry by a RAM number taken from the top
  of the key space down; it is never persisted and never shown. The custody mark carries the
  field (`DeployedMark::field`) and match number `0`; the black box is its own log per segment
  (`field.<site>.<ordinal>.<segment>`), so a resume needs no restored ledger tail.
- **After a restart** each pilot takes the cockpit back with `+focus <unit>/<station>`, as the
  resume line says: focus never survives a boot (mush-sim D-E7-34).
- **The site cap** is `[tacsim.field].max_units` at the launch command; the start's prepare and
  the fill plan keep the record's ceiling (24) as their backstop.

Not built in CF.2, each for a reason stated in its PR: **recovery as exit** (Q-CF-18: the kernel
has no way for a unit to leave a running arena, and the surface recover is itself refused while
the site is promoted; built by CF.5, §8); the site record's **objects as sensor contacts** and cover beyond their
cells (they are impassable cells, Q-CF-26); **remains and crates dropped at demotion** (an
irrecoverable hull's remains wait for the epoch close-destroyed rule, 27a H3, and a hopper's
crate for the inventory transfer that empties it); a hull under **dedicated IC custody** (a
licensed mind's sortie) refuses its site's field until doc 23 S.4b; **organization**
friendships (F-1).

## 8. CF.3 as built (the carrier as a unit)

Built 2026-10-31 on `main` at `d4d81c89`; the module is `crates/tacsim/src/field/carrier.rs`.
What the cut had to decide where the rulings left it open, for the owner to object to:

- **The pad pose.** A settled hull's own placement is its domain's passive pose (the origin),
  not the pad the surface draws at the site's centre. The field stands each carrier on the pad's
  cell. Several hulls can be settled on one pad and the kernel stands one unit in a cell, so the
  second carrier takes the nearest free passable cell, nearest ring first and row-major within a
  ring, up to eight cells out (`PAD_SPREAD_CELLS`). The hull's surface placement is never moved.
  A carrier's spec has a top speed of nil, so it goes home where it was, and the site record
  keeps the pad pose it was fought at.
- **Its party.** The carrier is in the party of the first open DT flight on the site that names
  it as its carrier, by operation. Otherwise it is a party of one led by its commander
  (`helm_holder`: the live charter's tenant, else the title holder). One commander behind a
  carrier and its excursion rigs makes them Friends by the standing rule (Q-CF-5).
- **No pilot.** A carrier's helm is not a cockpit. It is admitted on its ROM seal, over its own
  hull epoch (doc 23 S.2).
- **Its crew** (Q-CF-38). Everyone physically aboard at admission (the carrier's bounded subtree,
  as departure counts souls) is listed on the carrier's row of the field's live record.
  `tacsim::field::crew_location` reads them as `AboardCarrier`. This marker is CF.3's until CF.5's
  crew state lands on the character.
- **Damage in port.** The IC check-in already writes the field's hits into the hull's own unit
  record (Q-CF-36). `+ship` now prints a `Damage:` line for every section below full structure.
  Until this cut nothing in port read a hull's sections.
- **Never towed** (FO-13). No tow verb exists yet. Every shipped carrier is already past twice the
  tonnage of every shipped mobile design, and a test holds that. `field::carrier::refuse_tow` is
  the named gate a tow verb calls first.
- **Departure while the field holds it.** `+ship/lift`, `+ship/undock` and a queued depart are
  refused with `InField`, naming the units on the ground. The kernel has no withdrawal for a
  unit in a running arena, so a confirmed departure has nothing to confirm yet. Q-H10's
  leave-behind confirmation is CF.5's row. A carrier leaving mid-field is F-6, and what a drive
  hit does to lift is F-3.

Not built in CF.3: the carrier on the **presence picture** (CF.1's book still leaves hulls out,
so a fire order or mark on the surface cannot name a carrier; inside the field it is an ordinary
contact; CF.4's picture); the carrier's **footprint** (it is one cell, as every unit is);
**F-3's consequences** of a hit beyond section damage (the spec's own consequences apply in the
field: a destroyed drive section disables motion, a destroyed hull section destroys the unit).

## 9. CF.4 as built (one picture)

Built 2026-10-31 on `main` at `02ccdffc`; the shared builder is
`crates/tacsim/src/port/site_map/picture.rs`, the field's half `crates/tacsim/src/field/known.rs`.
What the cut had to decide where the rulings left it open, for the owner to object to:

- **One builder, two renderings** (Q-CF-48, Q-CF-50). The campaign layer is built once
  (`picture::campaign_layer`: block letters, live claims, public pits, the viewer's own readings,
  the pad, the presence picture, the salvage the viewer sees or remembers). `site.map` maps it into
  its grid (so `+tv/map`, the TUI's `q` and the web's quarry screen draw it as before); the cockpit
  frame of a unit in an IC field carries it as a `campaign` block in site cells
  (`{ site, side, block_side, field, marks: [[x, y, glyph]], legend, claims, salvage, presence,
  presence_line }`), which the TUI's and the web's cockpit maps and the pixel cockpit's map window
  draw under the contacts, in `marks` order. The field layer is the kernel's frame; `site.map` draws
  it from the viewer's own held cockpit frame of the field open on that site (`site.field_layer`,
  the contacts in the cockpit's glyphs `x` / `+` / `?`, the viewer's own unit as `@`). The surface
  does not push a cockpit frame of its own on an unpromoted site: `site.map` is that frame there,
  pulled as before with the `tv.status` nudge, because a second push renderer is what Q-CF-50's
  alternative was. The block is added only to an IC field's frame, so no simulator frame changed and
  `FRAME_VERSION` stays 9 (a key added is not a version).
- **SM.4b's exact rig cells** (CF.1's open question). The campaign layer carries the presence
  picture, so `site.map` draws another flight at its cell (`o`) and names it in metres
  (`Other rigs:`) only within sight — `SIGHT_CELLS`, 200 m — of one of the viewer's rigs on the
  ground, the sight the presence picture already names a hull's class at and CF.1 shows the record's
  ground within. Beyond it the flight is its band; inside a field it is a sensor contact. Doc 20 §5's
  loud-sounding mark moved with it: a sounding heard beyond sight marks the presence row
  (`(a sounding)`, `presence[].sounding`).
- **Ground objects in a field** (Q-CF-26). A remains or a crate's footprint is a **wall** on the
  field's ground (impassable to every ground row at the class's own price, opaque, three metres):
  the kernel's one way to give cover by a cell nobody stands in, since a class's own cover is the
  cover of the unit standing in it. CF.2's tracked price of 0 on the old class stopped a tank and no
  biped. An object is a **sensor return** by the world's trace (`mush_atlas::trace_cells`), seen in
  line of sight within 200 m from the unit's sensor height; the frame carries the ones a unit holds
  as `ground` rows. It has no `ContactId`, no lock, no place in the mind API, the snapshot or the
  hash.
- **Who remembers what** (Q-CF-28). The site record gains `known`: per principal, the things they
  saw (`[what, x, y]`, at most 32 each, 24 principals a site). A field's parties note what their
  seated units' sensors hold every pulse (RAM), and at demotion each party also looks once from
  every unit's last pose at the objects and at the casualties and wrecks the field leaves (its own
  always). The close writes it for each of the party's principals — its leader, the pilots who sat
  in its units, its rigs' operators — and forgets a remembered thing in a cell they looked at again
  and did not find. A character is shown what they remember only where they do not see now, marked
  `(remembered)`. A restart mid-field loses the pulses' notes before it; the look at demotion runs.
- **The rig's cab** (Q-CF-49). A rig in a field flies the kernel's cockpit frame like any hull
  (CF.2 admits every class); its cockpit frame gains a `rig` panel, the same row `site.rigs`
  carries, which the TUI's and the web's status blocks print. `+tv` status answers inside a field
  as before. The escort's cockpit shows the claims, blocks and ore marks of the person in its seat —
  the chart is per principal (doc 20 §5), so "its party knows" is read as "its pilot knows".

Not built in CF.4, each for a reason: **a surface scan revealing salvage** (Q-CF-28's second
source) — a scan reads geology into the chart, and which objects a scan's reach reveals is a rule
no ruling states; **NPC scavengers** finding objects (objects are not in the mind API); the
**web's own pixel map window** draws the campaign layer through the same `mush-pixel` code as the
TUI's, so it needs no second change. The cockpit map's `#` for a claim's edge shares its glyph with
the arena map's walls, which an object's footprint now is; the legend names both.

## 10. CF.5 as built (crew and exit)

Built 2026-10-31 on `main` at `02ccdffc` (after CF.3). The kernel's exit is `crates/tacsim/src/arena/withdraw.rs`
and `ArenaMsg::Withdraw` / `ArenaOut::Withdrawn`; the host's half is `crates/tacsim/src/field/exit.rs`;
crew state and extraction are `crates/tacsim/src/field/crew.rs`; the leave-behind confirmation is
the bay gate's (`crate::port::voyage::bay_gate_leaving`). What the cut had to decide where the
rulings left it open, for the owner to object to:

- **Where a withdrawing unit must be** (Q-CF-18, doc 27 Q-H3/Q-H4). Beside its own carrier's
  ramp: its cell within one cell of the ramp's, Chebyshev — the surface recover's rule (doc 20
  SM.3b) and the DT recover's (`at_ramp`), so the field adds no third reach. The kernel decides it
  on its own live pose; a unit elsewhere is refused "drive it there first" and fights on. Q-CF-18's
  "if the ramp is reachable" is therefore the pilot's drive to the ramp inside the field, not a
  path search at the verb.
- **Under its own power** is the DT recover's own test (power, control, and locomotion where the
  hull has a mobility profile), read by the kernel on the live cells, plus the kernel's own wreck,
  motion-disabled and disabled flags; so the recover the host submits next cannot refuse what
  the kernel let go. A hull that cannot come in stays, and its crew's way home is extraction.
- **The exit is checked in when it happens**, through the same IC plan as a demotion (Q-CF-36:
  only the custody vocabulary, no blanket wreck stamp, never the check-out snapshot), and the unit
  stands at its field pose beside the ramp. A later void of the field does not rewind it: an IC
  field never restores (Q-CF-35), and the unit is in no later snapshot. A boot resuming from a
  snapshot taken before the exit drops the unit from the arena (the live record no longer holds it).
- **The record**: the open promotion names each unit that left (`Promotion::exits`, written by
  `tacsim_field op=exit`, the host's own write like `close`); the site's unit index never holds
  a unit that left, since a demotion indexes only the units still in the field. CF.1's refusal of
  surface work on a promoted site stands, save exactly one act: the recover of a unit the
  promotion names (`open_field_except`), so the verb that asked runs again as its player once the
  record shows the exit — the rig's recover from beside the ramp, or the DT recover.
- **Crew location** (Q-CF-38) is physical: a pilot inside the hull (the hull's location chain, or
  the spatial graph) is aboard that unit, or **stranded** on the site when the checked-in hull
  cannot return under its own power or is a wreck; a pilot at a remote console aboard a carrier is
  aboard that carrier; an extracted character is at the port. **Crew condition** reads the control
  sections the anatomy names (`head`, `cockpit`, `bridge`): one destroyed incapacitates, one
  damaged injures, else fit; a remote pilot is fit. A character the field engine never touched has
  no record. The record is `tacsim:crew:<character>` on the module's lane (a workflow document,
  like the field's live record).
- **CF.3's marker is replaced.** `tacsim::field::crew_location` now answers CF.5's `CrewLocation`:
  `Carrier { hull }` while a running field lists the character on a carrier's row (CF.3's
  `FieldUnit::aboard`), else the character's crew record. At the carrier's check-in each soul on
  that row still aboard is recorded aboard it, so the location outlives the field.
- **Immediately on a control-section kill** is the pilot's own frame: the pulse a frame shows a
  control section destroyed around a pilot inside the hull, the pilot is incapacitated and told.
- **Extraction** (Q-CF-39, P-5): `+sortie/extract`, by a stranded or incapacitated character
  still inside the hull; never automatic. The **known safe port** is the one the game records a
  character knowing — the IC-return stamp's port (where they last stood) — else **Stickney Yards
  on Phobos**, the fallback doc 27 names. The delay is `[tacsim.field].extract_s` of IC time
  (provisional 21,600: six IC hours); a W-clock freeze pauses it because no tick runs a frozen
  world. On landing the seat is vacated (and the pilot unbound from any field still running), the
  character stands in the port's arrival room, and the hull, its cargo and its epoch stay.
- **A Friend's recovery flight** (Q-CF-39's deferral) is an open DT flight on the site, of an
  activation on `recovery` duty, under the same organizer as the stranded character's own party
  (Q-CF-5's Friend: same roster or same organizer). It defers the landing at most
  `[tacsim.field].rescue_bound_s` past the due instant (provisional 7,200), the published bound.
- **The leave-behind confirmation** (Q-H10) is `leave=<units>` on a live `+ship/undock` or
  `+ship/lift`: rigs by bay slot, the detachment's units by `#number`. A rig out on the ground or
  logged out elsewhere, or a DT unit in the field, refuses the live departure with its own line and
  the confirmation listing every unit out; named, it is left — the filed voyage records it
  (`PendingVoyage::left_behind`), the pilot and each left unit's operator are told, and nothing is
  recalled, released or closed. A unit mid-act or on the ramp never passes; a queued departure
  keeps the old refusal (no confirmation is ever presumed, and an embarked crew cannot be presumed
  to consent for another carrier's operator). The bay gate now also counts the DT units out from
  the hull, which held no departure before.

No snapshot, hash or frame bump: the withdrawal removes a unit from the arena and adds no field to
any encoded state; the site record and the voyage document gain optional fields, empty by default.

Not built in CF.5: **towing** a casualty home (FO-13; 27a H2); **ejection, capture and treatment**
(Q-CF-38 rules them out of the first version); a carrier departing **during** a field (F-6; CF.3 refuses it, `InField`, and that refusal stands ahead of the bay gate).

## 11. CF.6 as built (NPC pilots and raiders)

Built 2026-10-31 on `main` at `d4d81c89`. What the cut had to decide where the rulings left it
open, for the owner to object to:

- **A hired NPC keeps its seat; its hull is the mind's.** At admission a unit whose seated
  occupant is an NPC opens `Controller::Mind`, flown by its ROM seal's doctrine, while the NPC
  stays bound to its station. The arena therefore builds the NPC's own frame, and the host reads
  every answer off what the NPC sees, never off the kernel's truth (Q-CF-42). A resume re-binds
  the seat and never hands it the controls.
- **The executor is the DT.1b adapter, fed from the roster's own book.** A delivered order whose
  unit stands in a field reaches the kernel as the adapter's two messages (the fence, then the
  command), applied at the head of the next pulse (`field::orders`). The fence epoch is derived
  from the roster's revision and command term, so a restart re-offers the same fence and the
  kernel refuses the duplicate. An IC deadline becomes an arena deadline at the world's rate.
- **When the NPC answers.** WILCO once its frame shows the order standing on its own order row;
  NEGATIVE when the order named a contact and the re-keyed objective holds none (a lost contact
  is uncertainty, never a guessed lock), and at once for what a field cannot execute: a mining
  run, a campaign-sensor contact, a presence row, a contact from another field, support for a
  unit outside the field or the party. A cancellation is answered WILCO once the kernel takes
  it. The surface executor stays surface-only.
- **How a leader names a target.** Inside a field `ENGAGE <designator>` names a contact of the
  leader's own cockpit (Q-CF-11's field-wide designator); the order carries the row key in the
  leader's scope and the kernel re-keys it into the recipient's classified rows. On open ground
  `ENGAGE <octant>` or `ENGAGE <octant>-<band>` names a presence row, resolved by the recipient
  from its own unit. A campaign contact's key prefix keeps working where the campaign sensors
  hold one. New optional `at` on `FieldObjective::Engage`; old orders read unchanged.
- **ENGAGE on open ground is the leader's mark** (§4a): the recipient resolves the row and calls
  `field::mark_from_order` as the order's issuer; the WILCO follows the mark. A row that finds no
  flight, or a mark the host refuses, is NEGATIVE.
- **HOLD at launch** (Q-CF-41): `FieldHost.weapons_hold`, true for the IC host, opens every seat
  at weapons hold; the simulator keeps FREE. The gate binds only a pilot's `FIRE … WHEN` rules,
  as G.12a built it; doctrine obeys a delivered `HOLD FIRE` (`Arena::holds_fire`) from the pulse
  the command lands.
- **Jamming reaches the roster's transport.** A detachment message and its reply settle only
  between two units at one place; a declared jamming level at that place now holds them there
  as it holds any envelope (`mush-propagation`'s pass, R4: delivery-affecting only, never lost).
  Before this cut `@prop/jam` could not touch a field order at all.
- **The raider** (Q-CF-44) is content, `content/field/raiders.json`: an NPC leader and its armed
  hulls at a named site, seeded by staff (`@field/raiders <id>`), refused while any of its hulls
  stands (no respawn). A staff-owned attribute on each hull keys them into one party
  (`PartyKey::Fixture { leader }`) whose leader holds a standing Hostile mark on every other
  party, so the raider fights whoever it finds with no player connected. Its hulls fly their ROM
  doctrine. Doctrine never opens a field: the leader marks through the ordinary door. The shipped
  raider is two line tanks at Ahuna Flank (provisional; Q-CF-45 budgets six).
- **The combat-duty offer** is a third Slack Line person, Rook Adeyemi, under a combat contract
  alone (`tacsim.crew.ceres` version 2). The surface host admits a combat roster beside a mining
  one, and the contract must name the roster's own duty; a combat roster never mines.

Not built in CF.6: **the armed rig lease** (doc 27b FC.1's acquisition path). Occator's lease
machinery is one pool of one design at one programme port; a second, armed design needs the
pool, the shed, the take, the return, the repossession and the status lines generalized, which
is its own cut. Until then the escort flies a staff- or fixture-made armed hull, as every DT
story before it did. An ordered WITHDRAW in a field is the kernel's Withdraw (break off, hold
fire); it does not yet drive the NPC to its ramp and take CF.5's exit, and the fixture raider
has no ramp of its own, so a raider still leaves a field by dying.

**Found while building, for CF.7:** with a carrier admitted (CF.3), the shipped raider never
fires. The stock doctrine's stand (`stock_mind.rs`) takes the single best-ranked Hostile and fires
only if it has a line to it, never falling back to the next one; a carrier on its pad ranks first
from where the raider stands, has no line, and holds every raider gun silent for the whole field.
The raider gate therefore takes the prospector's carrier off the ground before the raid. Ranking
only the hostiles with a line is a kernel change that can move the OOC practice hashes, so it is
left to its own cut rather than slipped in here; CF.7's story, where the miners' carrier stands
on the pad, needs it first.

## 12. CF.7 as built (F-6 departure; the field story up to the escort's fight)

Built 2026-10-31 on `main` after CF.6. The cut has two halves. F-6's departure is built.
Q-CF-47's story is written up to the escort's fight; the rest waits on the gaps below.

**F-6, the carrier's departure from a running field** (ruled 2026-10-31; §5a):

- **The rule.** After the leave-behind confirmation (Q-H10, CF.5's `leave=`), a carrier departing
  a running field is removed from the field outright and lifts. It is not a ramp withdrawal:
  CF.5's `Arena::withdraw` frees a unit only within one cell of its ramp, and a carrier stands on
  its pad about four cells from its own. The units it leaves stay in the field.
- **The kernel.** `ArenaMsg::Withdraw` takes its ramp as an option. With none, `Arena::withdraw`
  takes the unit out with no ramp test and no under-own-power test, since a lift is not a drive.
  What a hit does to lift is still F-3's, so a damaged drive does not yet hold a carrier on the
  pad.
- **The host** (`field::exit`, `Then::Lift`). A live `+ship/lift` or `+ship/undock` first runs
  every check a departure runs, CF.5's bay gate included. The bay gate demands `leave=` naming
  every rig and DT unit the carrier has out. Only then does the host ask the field to let the
  carrier go, and it files nothing yet. At the next pulse the carrier is checked in through the IC
  plan, with the souls listed aboard its row, its pose unmoved (CF.3). The record's open promotion
  names its exit (`tacsim_field op=exit`). The host then runs the same departure again as the
  pilot, which files the voyage with the units left behind on it.
- **Refused without it.** A live departure that does not name every unit out is refused with
  CF.5's line and its `leave=` hint. A queued departure keeps CF.3's `InField` refusal, because a
  confirmation is never presumed. A carrier with nothing of its own out has nothing to confirm,
  and a live lift takes it out of the field without `leave=`.
- **No bump.** The withdrawal message is RAM, never journalled or snapshotted, and no document
  gains a field.

**The field story** (`tacsim/tests/shard_1/field_story.rs`) runs through the production command
paths, as every CF gate does, and passes as far as it is written:

1. At Ahuna Flank a player's rig works a block by hand (`+tv/1 mine go`), with an escort of two
   armed hulls beside it. A bystander rig from another captain works the next block.
2. The raider arrives and declares (`+field/mark`). The site promotes with everyone at their
   poses, the miners' carrier on its pad among them, and the miner's quarry screen draws the
   raiders as contacts (Q-CF-48). **One clause was pinned at its gap, not passed:** the raider was
   not on `+tv/map`'s presence picture (Q-CF-15), and the story asserted that. The reason first
   given here, that the presence book holds rigs only, was wrong: the book has held every
   non-hull unit on the ground since CF.2, and the cockpit's half of the picture already read it.
   `+tv/map` drew its flights from the lane's rigs alone, so the assert passed only because of
   that bug. Fixed by §15 (the review's CF-2): the clause now passes, each raider hull a row of
   the miner's picture.
3. The escort fights while the miners hold. The raider opens fire with the carrier on its pad,
   the escort answers, and neither miner fires. The escort stands nearer the raider than the
   carrier, so the doctrine's best hostile has a line in this layout; G-2 did not bite here.

The story is not `#[ignore]`d: the gated-proof guard (`ci_gated_test_inventory`) requires every
ignored test to be run by a CI job, and an ignored test failing at its stop could never be.

**Gaps the story found** (none of them small, so none was fixed here):

- **G-1, the raider's carrier.** Q-CF-47's raider launches from a fixture carrier. The fixture
  raider (`content/field/raiders.json`) has no carrier; its hulls are seeded straight onto the
  ground, which is how the story stands it in. (The presence-picture step this gap was blamed
  for was `+tv/map`'s own bug, not the seeding; §15.)
- **G-2, the doctrine's line.** The stand ranks every Hostile row (`stock_mind.rs`, the stand's
  `best_hostile(.., |_| true)`) and fires only at the first. Admitting only rows with a line
  changes the doctrine and its reflex twin, and can move the practice hashes, which would be the
  second rebless after Q-CF-46's one. CF.6's gate met it with the carrier ranked first (§11);
  this story's layout ranks the escort first and does not.
- **G-3, the raiders' withdrawal to their ramp.** An ordered WITHDRAW in a field breaks off and
  holds fire. It does not drive an NPC to a ramp and take CF.5's exit, and the raider has no ramp,
  so a raider leaves a field only by dying.
- **G-4, the bot.** `mush-bot` has no IC-field support: it never marks or fires on the surface and
  never reads an IC cockpit frame. Q-CF-47's second run has no pilot to fly it. *(The captain's
  both halves are built, §16.)*
- **G-5, the armed rig lease** (CF.6's deferral). The escort flies fixture hulls with hired NPCs
  seated. The delivered orders it fights under are proven by CF.6's transport gate in the
  split-boundary harness, not by this story. **Built as cut CF.6c (§14):** `+tv/lease escort`
  leases Occator's escort line tank, which a player flies out, fights from the line cockpit and
  brings home.
- **G-6, the hits.** "One escort mission-killed by an engine hit and one raider by a
  control-section hit" needs fixture-tuned sections; no player command aims at a section.
  *(Passed by the story's finish with seeded sections, §17.)*
- **G-7, the escort's map.** Q-CF-48's "the escort sees the worked block on its map" is the chart
  of the person in its seat (CF.4). A hired NPC holds no chart, so an NPC escort's map shows no
  worked block. *(Built, §17: a field party's members see its principals' readings on the
  field map, for display only.)*

**Who owns them** (the owner, 2026-10-31): G-1, G-2 with its rebless, and G-3 are cut **CF.6b**,
its own cut. G-4 is a later **bot cut**. G-5 stays CF.6's deferral. G-6 and G-7 are left for the
story's finish. CF.7's story is finished, with its 256 and 512 a side runs and the bot's run,
once CF.6b and the bot cut land. CF.6b is §13; G-1's presence clause is closed by §15.

## 13. CF.6b as built (the raider's carrier, the doctrine's line, NPC withdrawal)

Built 2026-10-31 on `main` after CF.7, the owner's cut of 2026-10-31 for three of the gaps CF.7's
story found (§12's G-1, G-2 and G-3). What the cut had to decide where the rulings left it open, for
the owner to object to:

- **The raider's carrier is a fixture, not a programme hull** (Q-CF-44, Q-CF-47).
  `content/field/raiders.json` (schema 2) gives each raider a carrier: a Dray at an authored cell
  with its ramp an authored cell beside it, within four. Its top speed is nil, so it stands still.
  It carries the raider's staff attribute like its hulls, so the field keys it into the raider's
  own party and admits it at its pose like any unit on the ground. It is not CF.3's programme
  hull: it has no pad, no bay and no voyage, so F-6's departure does not apply to it and it stays
  on the site. `@field/raiders <id>` seeds the carrier on the ground and its hulls aboard (made,
  never placed). `@field/raiders/launch <id>` puts every hull aboard on the ground beside the ramp,
  on the nearest free cell it can stand on, nearest ring first. A launch is refused while the site
  holds a field, since a mid-field arrival is its own cut. The shipped raider's carrier is the
  Skulk Barge at (156, 126), ramp (159, 126).
- **The doctrine's line** (G-2). The stock doctrine's stands (`stand`, and `withdraw_stand` for a
  hull going home) traced the best-ranked Hostile alone and stood silent when it had no line. A
  carrier on its pad that ranked first therefore silenced every hurt raider. CF.6's probe shows
  it: two raiders in their own `Withdraw`, the carrier at 300 m showing its flank with no line,
  ranked over a guard at 250 m facing them with one. The ranking weighs a flank over a front, so
  a far blocked hull can outrank a near one in the open. A stand now falls back: when the best has
  no line, it fires on **the best of the hostiles nearer than it**, if that one has a line
  (`nearer_in_line`). A farther one is never the fallback, so a blocked near hostile is not passed
  over for a far one in the open. That is two traces at most (`STAND_TRIES`), Class I's `line_to`
  cap and the lowest a scripted Standard Doctrine runs under, since Class Zero always runs this
  native ROM. A think that stands traces nothing else. The Rhai twin (`standard.rhai`) asks the
  same with one more `best_hostile` call, its ceiling the best's range less a centimetre. It adds
  no function, since the script stands at 31 of the sandbox's 32.
- **The hostile edges are untouched.** A hull that pursues still takes the best and closes on it
  until it has a line, as it always did. A healthy raider whose best is a carrier with no line
  drives at it and fires once it has one; it is not silenced. The same fallback on the edges was
  built and measured: it held the lanes' costliest think past Gate A's 1,200-op STOP (1,214 on the
  pooled lane, 1,210 on the mixed), so it was left out.
- **The rebless, taken once.** The Standard source moved (`3837c345…`; CF.0's was `a37766b5…`),
  and every seal records it, so every pinned tape's hash moves with it. Nothing else did. The
  four practice fights of the cross-platform pin end at the same milliseconds (170 250, 192 500,
  170 000 and 900 000), and the float probe's 3,600 thinks and mask are unchanged. The shipped
  duel's 34 ledger lines and the (j) field's 50 are byte-equal to their pins once each `hash` is
  taken out. The API 4 identity controls, which project the seals back to their old identities,
  still land on their old pins. Gate A's twin lanes hold T1 ≡ T2 with no divergence and under its
  1,200-op STOP, and the whole-match hashes agree between the runners over the pool. The
  reblessed pins: `STANDARD_SHA256` and its two test twins, the nine default seal digests
  (`match_::seal`), the production encoding pins (`tests/minds.rs`), the (j) field's first and
  last substep hashes and its 50 lines (`tests/reflex.rs`), the shipped duel's hash and its 34
  lines (`tests/motor.rs`), and the four cross-platform rows (`tests/minds_rhai.rs`), each with
  its reason beside it. `tests/twin.rs` counts five `line_to` sites in the script, where it
  counted three.
- **Where an NPC goes home** (G-3; Q-CF-18). A field has no entry zones (Q-CF-17), so the
  doctrine's `Withdraw` had no goal and held where it stood. The kernel's ground gains **homes**
  (`ArenaDefinition::homes`), which `motor::entry_cell` reads for a party with no entry zone.
  Every authored arena has none, and the digest folds them only when there are any, so no OOC
  digest moves. The hunt and its fan never read a home: no party is sent to look at another's
  ramp, and an unpiloted rig on the doctrine does not drive off to one. A field party's home is the
  ramp of the carrier its units came from: a DT flight's carrier, a rig's hull, a raider's fixture
  carrier, the first in unit order. It must be admitted with the party. It is kept on the live
  record (`FieldLive::homes`), so a resume and the void rule decode on the same ground.
- **A delivered WITHDRAW takes an NPC out at the ramp.** A roster's WITHDRAW to a hired NPC
  (CF.6's executor) and a raider leader's (`@field/raiders/withdraw <id>`) both become the
  kernel's `Withdraw`: break off and drive home. Under the order's HOLD FIRE the hull fires on
  nothing on the way. Without it, it stands and fires when cornered, as the doctrine's own
  `Withdraw` does, and goes on once it is not. A raider leader's WITHDRAW always holds fire: a
  raider leaving runs for its ramp. The raider's goes through the DT.1b adapter under its own fence, its hull its own issuer, because the
  leader stands in no unit and the kernel takes an order only from a unit of the recipient's party.
  It is marked on the hull, so a restart offers it again. Once a hired NPC has answered WILCO, and
  for a raider's hull from the order on, the host asks the field once a world second to let the
  unit go at its ramp (`field::home`). The kernel decides on its own live pose and damage, as
  CF.5's exit does: refused on the way, let go once the unit stands beside the ramp. Out, it is
  checked in like any exit, and its recover runs: the DT recover into the bay, or aboard the
  raider's carrier again.
- **An NPC with no reachable ramp** (the rule, from Q-CF-18, doc 27 Q-H3 and Q-H4). Its party
  has no home (its carrier not on the site's ground, or the ramp's cell not ground it can stand on),
  or it cannot get there or come in under its own power. Its `Withdraw` still breaks it off: it
  holds where it stands, firing only when cornered, and stays in the field. Nothing takes it out
  another way, since Q-CF-18 has no edge exit and Q-H3 withdraws only through reachable exits. It is
  on the site record where it stood once the field demotes (Q-H4), and its crew's way home is
  extraction (Q-CF-39), never automatic. The host asks a homeless unit nothing.

**The gate** (`tacsim/tests/shard_1/field_raiders.rs`,
`cf6b_the_raider_launches_fires_past_a_carrier_on_its_pad_and_withdraws_up_its_ramp`): the raider
is seeded with its carrier on Ahuna Flank and launched beside the ramp. One hull is hurt (armour
stripped, internals whole), so it is home already and fights from a stand. The other is whole and
is stood seven cells east of the ramp. Its leader marks an escort, and the field opens with the
prospector's carrier standing on its pad and the raider's home at its ramp. The hurt raider fires
on the escort from its stand, past the carrier. The leader's WITHDRAW takes both hulls up the
ramp, the whole one by a drive, and out of the field, and the open promotion names each exit. CF.6's own gate runs on the new content (seed, launch, and the carrier
made fragile too, since it is Hostile to whoever the raider fights), and CF.7's story launches its
raider the same way. In-source, `cf6b_mirror_the_doctrine_falls_back_past_a_hostile_with_no_line`
holds the stand's fallback through both twins, and the hostile edges unchanged.

Not built in CF.6b: **the raider's carrier leaving** the site after its hulls come aboard. It is
not a programme hull, so F-6's lift does not take it. It stays on the ground, unarmed and in the
raider's party. Whoever the raider fought may fire on it, so a field holding it demotes once it
falls or nobody does. **The raider on the
presence picture** (CF.7's step 2): the launch places the hulls on the site's ground, which the
book publishes. Whether CF.7's pinned assert now flips is for CF.7's story to read. **Bot
IC-field support** (G-4), **the armed rig lease** (G-5), **fixture-tuned sections** (G-6) and
**the escort's map** (G-7) keep their owners.

## 14. CF.6c as built (the armed rig lease, G-5)

Built 2026-10-31 on `main` after CF.6b, as G-5 (§12). It is doc 27b's FC.1 acquisition path for
a player. What the cut had to decide where the rulings left it open, for the owner to object to:

- **One lessor, several designs.** Occator's pool is no longer one design. The lane's
  `vehicles` block keeps its own design (`+tv/lease` alone, the prospector) and gains `designs`:
  further designs leased from the same shed at the same counter, each with its own unit, rigs,
  registry prefix, terms, tools and gate (`+tv/lease <word>`). The port map's `VehiclesRow`
  gains `designs`, one row of rigs per design in authored order. Designs are only ever
  appended: the applier refuses a map whose laid-up designs differ from the authored ones. Every
  reader that walked the pool's rows now walks every design's: the take, the readiness line, the
  console boot, `rig_row`, the cap census, the registry-inspection hull set and the per-rig tool
  lookups (a rig carries its own design's tools). The return, the repossession, the term settle
  and the status lines read the lease record, which never needed the design.
- **The armed design** (`tacsim.unit.tv_escort`, pack `tacsim.pack.tv_escort`): the arena's
  stock line tank's combat profile verbatim (five sections, the 105 mm gun and the coaxial
  machine gun, the stock sight, the line core, tracked), so it fights exactly as the simulator's
  line tank and stands wherever a prospector drives. To be leasable it adds class `vehicle`
  (never offered by the simulator lobby), an 8 t dry mass so it rides a Dray-P bay beside one
  prospector, a one-seat cab and a small kit locker (never a hopper). Two of them, marks
  `OWP-ES-01`/`02`, at Ceres Freeport, whose routes reach Ahuna Flank. Lane v16, relay seed v7
  (re-signed to require it, entries unchanged).
- **One console.** An armed rig's pilot console carries every cockpit channel (helm, sensors,
  fire), so the seat `+tv/<n> board` takes drives, locks and fires; an unarmed rig's keeps the
  helm alone. `+tv/<n> board` now finds the console that carries the helm rather than one that
  carries only the helm.
- **The lessee works a rig from its own cab.** `+tv` acts took the operator aboard the
  carrier. A lessee physically in the rig's cab now works it too, as a crewed DT operator
  already could, so the escort launches, drives and comes home from inside.
- **The gate: the escort clearance** (`LeaseGate::Escort`, read before anything about the
  hull). The prospecting licence must be in force; its grace does not serve. No lease of the
  lessee's may be forfeit. One armed lease is live at a time. None may have been written off in
  the last 28 IC days. Every fact comes from records the lease machinery already keeps; no new
  licence family.
- **Terms (provisional).** 2,500cr a term of 168 IC days and a 15,000cr deposit. A new First
  Captain holds 22,500cr, so a fresh captain can lease an escort with fuel money left over.
- **The write-off rule** (new). A leased rig wrecked out of its carrier's bay never comes back,
  so it is written off: at the lessee's `+tv/<n> lease return go` (at the counter, wherever the
  hull is) or at the charter's repossession. The lease ends. The deposit is captured to Occator
  whatever the term. The record goes `Returned` with `written_off`. The wreck stays where it fell,
  Occator's title, never pulled back to the shed. Its open excursion stays on the site and no
  longer counts against any cap. A rig out on a site but not wrecked still holds the hull, as
  doc 20 decision 13 has it. A damaged rig that comes home goes back with its damage. There is
  no repair charge, which is provisional.
- **Unchanged** (the bot cut can pin these): the line cockpit's sentences (`CONTACTS`, `LOCK`,
  `FIRE`, `STATUS`, `DRIVE`); CF.2's promotion (`+field/fire`, `+field/mark`); CF.5's exit
  (`+tv/<n> recover go` beside the ramp); Q-CF-17's `FIELD_OPEN` launch refusal; CF.3's
  check-in. No doctrine, field-host promotion rule or bot file is touched.

**The gate** (`tacsim/tests/shard_1/escort_lease.rs`) runs through the production command paths:

1. A First Captain previews and signs `+tv/lease escort` at the Exchange, is refused a second
   armed lease, and requests the operational loan (`+tv/1/permissions go`).
2. They sail to Ahuna Flank, where a miner's prospector works. From the hold they enter the bay
   (`+tv/bay in`) and board the escort, then `+focus` and `@station/handoff`. They launch it down
   the ramp and drive it out beside the rig.
3. They fire on an intruder's armed hull (`+field/fire E`). The site promotes; from the line
   cockpit (`CONTACTS`, `STATUS`, `LOCK`, `FIRE`) their rounds land and the intruder's land on
   them.
4. They `DRIVE` to the ramp, `+tv/1 recover go` takes CF.5's exit and the recover into the bay,
   and `+tv/1 launch go` is refused while the site is promoted (Q-CF-17). The damage is in the
   hull's unit record (CF.3). The field demotes, they `+tv/1 leave`, lift for Ceres, walk ashore
   and return the lease, deposit released.

A second story wrecks the escort in a field. The lessee is extracted (`+sortie/extract`) and
writes the wreck off at the counter. The wreck stays where it fell, and the clearance bars the
next armed lease.

**One stand-in.** The operational loan is the lessor's Org unit-grant action. tacsim's world
harness cannot land it (a plain prospector's loan fails there the same way), and mush-server's
`dt_preparation_plans` lands it in production composition. The story therefore asserts the
loan's submission and the board's OperateCrew refusal, then makes the board's two writes by a
staff hand (the cab, the helm billet).

**Expensive to reverse:** the escort's registry marks (`OWP-ES-`, never re-issued once laid
up), its pool of two (rigs are never retired, so it can grow but never shrink), the design word
`escort` and its index (designs are only appended), and lane v16 with relay v7. The prices, the
deposit, the 28-day bar and the dry mass are lane or code data and cheap to change.

**Not built:** an armed hull for sale; repair of a returned hull's damage; Occator recovering a
written-off wreck; a mixed bay's payload headroom check at lease time (an escort beside a
prospector with a full hopper can exceed the Dray-P's ceiling, and the departure refuses as it
would for any overload).

## 15. The 2026-10-31 review's fixes (CF-1, CF-2, CF-3a, CF-5a, CF-5b, CF-6)

Built 2026-10-31 on `main` after CF.6c. An external review of `830f2265` raised 18 findings; the
coordinator checked each against the code, and these six were real. What each fix had to decide,
for the owner to object to:

- **CF-1, the crash windows.** The host's own writes to the site record (`tacsim_field
  op=close` and `op=exit`) run on the K3 lane after the legacy batch that checked the units in.
  The two lanes never share a turn, and what the host owed lived in RAM only. A crash between
  them lost the close: the live record and the snapshot were already deleted and the units
  checked in, but the record was still promoted. The tick then admitted the finished field again
  and dropped its casualties, wrecks and `learned`. A lost exit left the unit refused its recover
  until the field closed. Reconciling at boot cannot tell that case from "promoted, not admitted
  yet". **The fix makes each owed write durable on the legacy lane, in the batch that owes it.**
  - **A close** is the site's **closing tombstone**, `tacsim:field:closing:<site>`
    (`field::record::FieldClosing`). It carries the close (actor, ordinal, the units left, what
    was learned) and the exits still owed. `owe_close` writes it, so all three ways to close (a
    demotion, a refused admission, the boot's void) are covered, including those with no live
    record. It is a document of its own beside the live record rather than the live record
    rewritten, and is written in the same batch that deletes the live record and the snapshot,
    so it is equally atomic. The boot reads it, owes the close again and holds the site
    closing, so the tick never admits it again. It is deleted once the record shows the close
    landed (not promoted, or promoted under a later ordinal).
  - **An exit** is a row of the live record's `owed_exits`, written by the same `write_live`
    that drops the unit from the record, beside its check-in. The boot owes it again and the
    tick removes the row once the record names the exit.
  - What stays RAM: the exit's continuation (the recover the host submits as its player). After
    a restart the player runs their own `+tv/<n> recover go`, which passes because the record
    names the exit.
- **CF-2, the presence picture.** `+tv/map` passed the shared picture builder only the lane's
  rigs (`surface::act::world_rigs`). The presence book has held every non-hull unit on the
  ground since CF.2, and the cockpit half of the picture already reads it, so a raider's or an
  escort's hulls were missing from an unpromoted site's map. That broke §9's one picture. The
  map now adds every other flight the book holds, minus the viewer's own. The rigs keep the
  map's own placement at the frame's instant (`place_of`), so their bands still never lag the
  grid, and a rig the book has not caught yet is still drawn. §12's G-1 reason is corrected
  there, and the story's G-1 assert is flipped: each raider hull is a row of the miner's picture,
  one new row per hull.
- **CF-3a, the cap.** Admission compared everything on the ground with
  `[tacsim.field].max_units`. Q-CF-26 and Q-CF-45 say casualties and wrecks don't count, and
  Q-CF-21 says a promotion can always admit everyone. The count now leaves out every unit the
  site record's index holds as a casualty or a wreck (`field::parties::counted`), and admits them
  all the same. No ruling exempts carriers, so they still count.
- **CF-5a, the landing release.** An extraction landing a pilot out of a running field unbound
  the seat and cleared the cockpit locally, but never told the arena. The arena kept building
  frames for an absent pilot, and the hull's controller stayed Pilot rather than Mind. The
  landing now sends `Control { Released }` on every channel (`arena_bridge::control_all_channels`),
  which unbinds and clears through `send_control`, as an `EXIT` does. The DT flight record stays
  open (Q-CF-39, by design).
- **CF-6, the opening shot.** A fire-promoted field's pending fire order was RAM only, though the
  book's doc said the boot rebuilds it. **Carried, not re-documented:** the live record gains
  `fire`, refreshed at every snapshot to the order as it stands then. Q-CF-34's resume rewinds
  to that snapshot, so a shot that had not left by then is owed again on the resumed task, and a
  shot that left after it is rewound away and owed again too. The window (`FIRE_WINDOW_MS`) is in
  arena milliseconds, which the resume carries on from the snapshot.
- **CF-5b, a test only.** An ENGAGE on open ground becomes the leader's Hostile mark plus a
  WILCO. Inside the field `EngageAt::Row` is refused by design (§11): the doctrine fights Hostile
  rows. No test showed a hired NPC firing after the open-ground order. One does now.

**The gates.**
- `tacsim/tests/shard_1/field_crash.rs`. **CF-1's close window and CF-3a:** a wreck, the
  field's close dropped on its way to the K3 lane (the injected fault), a restart. The field is
  never admitted again, the close lands with the wreck where it fell, and with the cap one below
  everyone on the ground a second field admits them all, the wreck among them. **CF-1's exit
  window:** a prospector out at the ramp, its exit dropped, a restart. The field resumes, the
  exit lands, the live record forgets it, and the player's recover passes. **CF-5a:** a biped's
  head destroyed, its pilot incapacitated in the field's first frames and extracted while the
  field runs. The world's mirror of the task holds the hull as the mind's, and the pilot is no
  pilot of the task.
- `tacsim/tests/shard_1/field_story.rs`: G-1, flipped.
- `mush-server/tests/split_boundary/dt_preparation_plans.rs`
  (`cf_review_an_open_ground_engage_ends_in_the_hired_escorts_fire`), **CF-5b.** It reuses CF.6's
  transport fixture, with the escort's rig rebuilt by a staff hand as a line tank (the armed
  lease's loan does not land in that harness). The escort stands between its leader and a
  bystander's hull. `ENGAGE E` is answered WILCO and is the leader's mark, and inside the field
  the escort's own rounds leave the gun with no further order.

**Ruled by design and left alone** (the review's CF-4 and the rest): a live lift with nothing out
needs no `leave=` (F-6); the raider's seed is refused while a wreck stands (Q-CF-44); wrecks are
admitted to later fields (Q-CF-26); the unarmed barge holding a field open is a known unbuilt gap.

**Expensive to reverse:** the tombstone's key (`tacsim:field:closing:`) and its document, and the
live record's two new fields (`fire`, `owed_exits`, both omitted when empty, so a live record
written before this cut still decodes).

## 16. G-4 as built (the bot in an IC field)

Built 2026-10-31 and 2026-11-01 on `main` after CF.6c (§14) and the review's fixes (§15). The
owner's ruling of 2026-10-31 has the bot fly both roles: the mining captain, and an escort pilot in
an armed hull a player can lease, taken through CF.6c's player lease path and never a seat the
harness made. The bot stays independent of the game: it reads only its own line session, and the
game crates are dev-dependencies for its contract pins and its server under test. What the cut
had to decide where the rulings left it open, for the owner to object to:

- **What the line cockpit answers from an IC field seat.** It answers everything a player types.
  The cockpit's one context test (`cockpit::flying`) is a focus on a unit that carries a deployed
  mark, and `DeployedMark::field` is one. A probe on CF.6's transport gate typed `CONTACTS`,
  `STATUS`, `WEAPONSTATUS`, `VIEW BRIEF` and `+field` from a rig's cab boarded the player's way
  (`+tv/<n> board` in the bay, `+focus`, `@station/handoff`), and every one answered. Inside a
  field, `order` goes to the roster's `field_order` only when the typist holds the operation's
  reservation. A seat the harness made with no roster behind it cannot order anyone.
- **How a captain tells the raider.** It reads `CONTACTS` from the cab. A rig's sensors hold
  every other unit as `X1 unknown` until they have dwelt on it, about twenty seconds in the probe.
  Then the row classifies and its class word names the party: `BD  blue …`, `AK  a …`. That is
  `PartyId::as_str`, so R and B print `red` and `blue`. No relation word is printed. The captain
  takes the first classified party that is not its own (`+field`'s `Your party is A`) for the
  raider. That reading is sound only because the expedition is **one party**: both rigs fly on
  the roster. A third, solo rig would be a party of its own (`PartyKey::Unit`) led by the same
  captain, and line text cannot tell it from the raider. With an escort pilot on the ground there are
  several other parties (the escort's rig and its Dray-P are each a party of one), and the captain
  waits instead for its escort's word (below): a page `Raider: party B, contact BK.`, whose party it
  then takes.
- **The expedition's shape.** `--mine-field ahuna` is the hired-crew expedition (`--mine-detachment`)
  under a combat contract. Rook Adeyemi is hired at the Slack Line and flies rig 2 as slot 1, role
  `escort`. The captain rides rig 1's cab as slot 2, so the solo lane's rig-1 beats run unchanged:
  scan, sounding, drive to the best pocket of its own reading, run. A roster rig flown from its
  cab can scan, sound and mine, but `+claim/stake` is refused there (`You work the rigs from aboard
  the …`), so the captain works **open ground**, and an open block may be mined (only a claimed
  one refuses).
- **The field's beats** (`mine/field.rs`). The site's open notice (`field::FIELD_OPENED`, told to
  everyone on the site) drops the step in hand, then:
  1. `+field` for the party letter.
  2. `CONTACTS` until a row classifies.
  3. `+field/mark <party>=hostile`.
  4. `ORDER DT:… 1 ENGAGE <designator>`. The reply is read in `+sortie/orders` above the highest
     order seen before it went. A NEGATIVE sends the captain back to `CONTACTS` for another
     contact, at most five times.
  5. One `CONTACTS` read five seconds on: the escort here is unarmed (CF.6c), and a longer hold
     let the raider shoot it to a wreck before its WITHDRAW, which a wreck answers NEGATIVE.
  6. `ORDER DT:… 1 WITHDRAW` and its reply. A NEGATIVE (an escort that cannot take the order) is
     recorded, and the captain holds on, as a player with a lost escort would.
  7. `+field` until it answers `No field holds a party of yours.` or the settled notice arrives.
- **Rig 1 home after the field.** The roster's recover takes only a unit already beside the ramp
  (`Return unavailable. Bring the unit physically to the carrier's ramp.`). The captain's rig is
  out at its pocket and no line names the ramp's cell, so the captain types the rig's own
  `+tv/1 recover go`. It drives the rig home with the captain in its cab, and it closes the
  captain's flight: the operation reads the slot `last confirmed returned`. Then come the
  expedition's own steps: both slots returned, `+tv/1 leave`, the unseat, the walk to the
  airlock, `+sortie/end` and `+sortie/deactivate`.
- **The fill plan resumes on rig 1, launched again on its own** (the coordinator's ruling of
  2026-10-31, which named a third rig; a Prospector's bay holds two, `The … bay is full.`). With
  the roster closed and the field over, its own party confuses nothing: `+tv/1 launch go`, then a
  drive to the best pocket of the captain's own reading (rig 1's first, at most three), then one
  run.
- **`leave=`.** The lane types a plain `+ship/lift`. A refusal for a unit still out (`TV_OUT`,
  `UNIT_OUT`, CF.5's `LEAVE_HINT`) is read, and the lift is typed again once with
  `leave=<what the refusal named>`, as a player meets the bay gate. Here the refusal names rig 1,
  so it lifts with `leave=1`. Home, `+tv` reads rig 1 out of the bay with its run's ore (`not in
  the … bay; … hopper 12 of 12 t`), and the story's staff read (`@spatial`) still finds it on the
  site's ground. Then the sale of the ore rig 1 cut before the field.
- **The vocabulary.** `field::in_vocabulary` is the expedition's vocabulary plus `+field`,
  `CONTACTS`, `+tv/1 board`, `+tv/1 leave`, `+field/mark <party letter>=hostile` (no other mark),
  `ORDER DT:<n>:<n>:<n> 1 ENGAGE <designator>`, `ORDER DT:… 1 WITHDRAW`,
  `+sortie/activate <id>=combat` and `+ship/lift ceres leave=<slots and #units>`. No `@` staff
  verb, `FIRE` or `+field/fire` is admitted.
- **The sentences, pinned.** Every field sentence the captain reads is now a tacsim `pub const`,
  and `mine/lines.rs`'s test builds each from it:
  - `field::{FIELD_OPENED, FIELD_SETTLED, FIELD_STATUS, NO_FIELD, PARTY_MARKED}`
  - `detachment::sortie::{ORDER_PREFILL, ORDER_SENT}`
  - the existing `surface::site_record::FIELD_OPEN` and `port::voyage::LEAVE_HINT`
  - every party letter's `PartyId::as_str` read back to its letter
  - the probe's own `CONTACTS` rows

  Nothing the game prints changed.
- **The escort pilot** (`--field-escort ahuna --escort-for <captain>`, `mine/escort.rs`). The
  brief named the arena pilot (`arena/pilot.rs`); the field escort is built on the mining lane's
  step machine instead. A field escort is a player at Ceres first: enrol, refit, licence, a lease,
  bunkers, a hop, which are the mining lane's beats. What it shares with the arena pilot is the
  line cockpit, and it reads that with the arena reader's own parsers (`WEAPONSTATUS` rows) and the
  mining lane's field ones. The arena pilot's lobby, its red/blue sides and its tactician are a
  match's, and none of them fits a field.
  - **The lease**, CF.6c's player path: `+tv/lease escort` previews, `+tv/lease escort go` signs,
    `+tv/1/permissions go` asks the loan and `+tv/1/permissions` reads it back. The escort's
    Dray-P carries the armed rig alone. The prospector's own needles hold: the escort's preview is
    `LEASE_PREVIEW`, which ends `+tv/lease go signs it.` even for the escort (signing it takes
    `+tv/lease escort go`, the game's sentence names the wrong command; a finding below). *(§17:
    the sentence now names `+tv/lease escort go`, and the needle reads the lane's own lease line.)*
  - **Aboard and down.** The escort walks to the bay and boards the cab (`+tv/1 board`). The
    printed `+focus` itself gives control of a lessee's own cab, so no `@station/handoff` follows
    (the handoff then fails, `That did not go through.`). It launches (`+tv/1 launch go`) and
    reads `+tv/1`'s `on the ground at the stern ramp`.
  - **Off the ramp's cell.** Every hull on a pad launches its rigs onto the pad's one ramp cell,
    and a field holds one unit a cell: with Rook's rig and the escort's both there, admission
    failed (`units #570 and #549 were both handed over in cell (128, 124)`) and the field never
    opened. The escort therefore drives 30 m east (`+tv/1 drive e 30 go`, three cells), as CF.6c's
    own story drives out beside the rig. A launch that does not spread is a finding below.
  - **Its ramp cell.** No sentence names it, but the field keeps each carrier's ramp at its
    surface placement (`field::home`, `exit::rig_ramp`), so the escort's ramp is the cell it
    launched onto. At the field's open notice it reads its own cell (`VIEW BRIEF`'s
    `you at (x, y)`, read off the joined window because the header passes 120 columns and wraps)
    and takes the ramp three cells west.
  - **The raider.** Line text names a contact's party, never its relation or what it is: a
    `SCAN` prints a section only when it holds a band for it, and a fresh look printed the header
    alone. The escort sees the captain's party and its own Dray-P's besides the raider's. It sets
    parties aside: its own (`+field`'s `Your party is C`), its captain's (the captain pages it,
    `page <escort>=Captain: party A.`, `lines::CAPTAIN_REPORT`, once its own `+field` has named it),
    and every party whose contact its `LOCK` reads `(FRIENDLY)` (`Locked on BF (FRIENDLY), …`: its
    own Dray-P, a Friend by the standing rule of one commander, Q-CF-5). It tells nothing until
    the captain's page has come, since before it the captain's contacts are no different from the
    raider's. The one party left is the raider. It pages its captain
    (`page <captain>=Raider: party D, contact DQ.`, `lines::RAIDER_REPORT`) and marks the party
    Hostile.
  - **The fight.** `WEAPONSTATUS` names the main gun (the weapon with the fewest rounds, never the
    coaxial machine gun), `LOCK <designator>`, then `FIRE <gun>` every three seconds until a round
    lands (`Hit on BK: …`, never the victim's `Hit on the …`), at most twelve. HOLD at launch
    (Q-CF-41) binds only `FIRE … WHEN` rules, so a hand `FIRE` is the pilot's, and the vocabulary
    admits no `WHEN` rule.
  - **The withdrawal.** `DRIVE x,y` for the ramp's own cell (the motor stops short of a cell
    another hull's rig stands on; a drive for the cell beside it stopped two cells off once), `VIEW
    BRIEF` until it stands within one cell, then CF.5's exit,
    `+tv/1 recover go` (`#… makes for the ramp: it leaves field 1 …`), and `+tv/1` until the rig
    is `in the bay`. It waits out the field (`+field`), leaves the cab (`+tv/1 leave`,
    `+tv/bay out`), sails home and docks. It has nothing to sell, so docking home is its success.
- **The harness** (`tests/mining_cohort.rs`,
  `a_captain_and_its_escort_fight_the_raider_in_an_ic_field`, in the `bot-stories` cell). It runs
  two actual executables, the captain and the escort pilot, over public SSH on the production root
  (`mushd::boot::boot_all_in_one`), `disengage_s` 30, the rate lifted to 168, with no model. Each
  bot has its own config file: the arena rig's shared `bot/arena.toml` was overwritten by the
  second before the first child read it, and both logged in as the escort. Its setup is the
  fixture opponent's, never a bot's. It stands in only the fixture raider:
  - **Seeded and launched at boot**, on the booted engine before the world task spawns.
    `@field/raiders` is a staff post-image write. A live world holds one behind every
    authoritative action in flight, and in two runs the seed or the launch never answered while
    the bot worked (left for its own look, below).
  - **Worn**, as CF.6b's own gate wears one: each raider hull's armour at its floor, its internals
    whole, by staff's `@unit/damage`, also at boot. Two full-strength line tanks fire from the
    moment the field opens, and in three runs they had shot the escort's turret off
    (`This unit carries no weapon.`) before it had classified anything. A worn hull is past its
    doctrine's withdraw edge and fights from a stand at its own ramp.  - **Its leader's declaring mark** is typed as that NPC straight into the world task
    (`Rig::act_as`, `WorldHandle::inbound`), as tacsim's field stories type it, because no player
    path reaches an NPC's own act. It is tried on each octant until the bot hears the field open.
  - **Its withdrawal** is staff's `@field/raiders/withdraw`, once the captain has withdrawn its
    escort.

Not built here, each for a reason:
- **An armed Rook.** The captain could lease the escort design for him (one armed lease a
  lessee). But the escort pilot tells the raider by its turret, and an armed Rook would wear one
  too, so Rook flies the plain lease. The captain's story therefore proves its orders and their
  replies, and the escort pilot proves the shots.
- **The presence picture's raider** (G-1's clause). The captain reads the raider from its cab
  inside the field and never marks it on open ground.
- **A shot at a section** (G-6) and **the escort's map** (G-7) keep their owners.

**Found while building**, each for its own look:
- **A staff post-image command typed while a bot works an IC site may answer late or never.** In
  one run God's `@field/raiders/launch` had not answered a minute later. At teardown its legacy
  batch was still queued behind the store writer, which had logged nothing until then.
  *(Not a bug, §17: the harness's `pump` returns at the first newline, so the probe never waited;
  measured properly, the three staff lines answer in under 100 ms while the bots work the site.)*
- **Two hulls on one pad launch their rigs onto one cell.** A second rig launched down the ramp
  lands on the first's cell. The surface allows it, and the field then refuses to open at all
  ("occupancy is one unit per cell"). This harness never sees it now, because the escort drives
  off the ramp. A player who does not drive off it stalls every field on that site.
  *(Fixed, §17: admission parts two units on one cell.)*
- **The escort's lease preview names the wrong command.** `+tv/lease escort` prints `LEASE_PREVIEW`,
  whose tail is `+tv/lease go signs it.`, and that command leases a prospector.
  *(Fixed, §17: the tail names `+tv/lease escort go`.)*
- **Line text gives no relation.** A field of several parties gives a line-mode player no way to
  tell a friend's party from a raider's. The captain here relies on its escort's page, and the
  escort on its captain's page and a `LOCK`'s `(FRIENDLY)`.

## 17. CF.7 finished (the whole field story at 256 a side)

Built 2026-11-01 on `main` after the bot cut (§16). CF.7's story
(`tacsim/tests/shard_1/field_story.rs`, `cf7_the_field_story`) now runs the whole of Q-CF-47 at
256 a side, through the production command paths as every CF gate does. The 512 run waits on
doc 30's WG.6 and is not part of this cut. What the cut had to decide where the rulings left it
open, for the owner to object to:

- **G-7, the escort's map (built).** A chart stays per person (doc 20 §5), and nothing writes
  anyone's row. While a field runs on a site, a viewer who sits in one of its units sees, on that
  site's map, the readings of every principal of that unit's party, beside their own. The
  principals are Q-CF-28's: the party's leader, the pilots who sat in its units, and its rigs'
  operators (`field::known::chart_mates`, `party_principals`). This is display only: it is the
  campaign layer of `site.map` and of the cockpit frame's `campaign` block, and the viewer's own
  reading of a pocket or a field wins. When the field closes, the party's readings leave the map.
  A Friend party (Q-CF-5) is not included. On a DT roster the escort and the miner are one party,
  so a player escort who never sounded sees the roster's worked blocks. In the story the escort
  hull is the miner's leased rig, so the miner is its operator and the escort pilot sees the
  miner's sounded pocket. The bystander, another captain, does not.
- **G-6, the hits (passed with seeded sections).** No command aims at a section, so a staff hand
  seeds both hulls before the field opens. The rounds that land are the real damage pipeline's.
  - **The escort's engine.** Its hull's rear plate is at its floor and its engine one point above
    destroyed, so a round struck through the rear plate (the engine's `struck_through`) breaks
    it. The escort faces away from the raider.
  - **The raider's control section.** The raider's second hull is rebuilt as a biped
    (`@unit/remove`, `@unit/create`, CF-5b's idiom), the one shipped design with a control
    section (`head`). Its head's plates are at their floor with one point of internals. Every
    other section's internals are at their floor without the `destroyed` condition, so a round
    that finds them breaks nothing (damage's transition rule, `fresh`). Its legs are recorded
    `destroyed`, so it stands where it was put, and its magazines are empty. Whichever round
    first draws the head is the control-section hit.
  - **The raider's fire.** The tank's main gun is empty, so it fires its coaxial alone. That
    keeps a hull round from reaching the escort's 200 internals before an engine round does.
  - **What stays random.** The arena's facet draw still picks the burst that lands: 5 in 100 for
    the head and 30 in 100 for the engine from the rear. The seeding fixes the outcome, not the
    timing, and the story fires until the section falls. It took one burst for the engine and
    about thirty for the head in the runs here.
- **Two hulls on one pad (fixed at admission).** Every hull on a pad launches its rigs onto the
  pad's one ramp cell, and the surface lets two rigs share it. Admission then refused the whole
  field, because the kernel stands one unit in a cell. Admission now parts them first
  (`field::carrier::unstack`):
  - In unit order, the first unit keeps its cell.
  - Each later unit on a cell already held is stood on the nearest free cell it can stand on,
    nearest ring first, row-major, within `UNSTACK_CELLS` (4). Its pose moves by whole cells, so
    its heading and its place within the cell are kept.
  - The move is a surface placement made before the check-out, like the settled rigs' (Q-CF-31),
    so the unit is admitted there and comes home from there.
  - A surface with no shared cell is untouched.

  *Why at admission and not at the launch.* The first fix moved the launch itself one cell off a
  taken ramp cell. That changed what `+tv` prints for such a rig ("about 10 m from the stern
  ramp", not "at the stern ramp"), and the G-4 escort pilot waits for the latter. On CI's slower
  runners the escort sometimes launched while Rook's rig still stood on the ramp, and the
  two-bot story hung. Confirmed on conclav.dev with the spread forced: the escort's `+tv/1`
  answered "about 10 m from the stern ramp". Parting at admission keeps every surface sentence
  and every "at the ramp" test as it was.

  Gated by `cf7_two_rigs_on_one_ramp_cell_are_parted_at_admission_and_the_field_opens`.
- **The escort's lease preview (fixed).** `LEASE_PREVIEW`'s tail is now `{go} signs it.`, where
  `{go}` is `tv::lease_go`: `+tv/lease go`, or `+tv/lease escort go` for a design leased by its
  word. The prospector's sentence is unchanged. The bot's needle is now
  `lines::lease_previewed(<the lane's own lease line>)`, and its contract pin builds both tails.
- **The late staff command (not a bug).** Investigated on conclav.dev with per-iteration
  world-loop tracing, against a copy of the G-4 harness with nothing seeded at boot.
  - **The world answers.** Typed through `Client::answer`, all three God lines answered while the
    bots enrolled and while they worked Ahuna Flank: `@field/raiders`, then `/launch`, then the
    list, in 9–10 ms, 68–91 ms and 0–1 ms.
  - **The harness measured it wrong.** The original probe used `Client::pump(timeout)`, which
    returns at the first newline (the line's own echo) rather than waiting out its timeout. So
    "a minute later" was about 50 ms.
  - **The "queued behind the store writer" line** was `flush_dirty`'s
    `store writer gone; batch dropped (teardown)`: God's line reached the world after the writer
    had already exited at teardown.
  - **Statically**, staged K3 work cannot shut out legacy dirty work
    (`Engine::stage_authoritative_action` refuses while the world is dirty). A typed line meeting
    a pending commit is deferred and, past 250 ms, refused `CommitTimedOut` in words the player
    sees, never silently.
  - **Two latent world-task items** were noted but not observed and not the cause, each for its
    own look: lines queued behind a held lifecycle slot wait silently, and `Wake::DecisionResult`
    is not gated on a pending commit.

  §16's finding is retracted. Seeding the raider at boot in the G-4 harness stays, and live
  seeding works too.

**The story's steps, in Q-CF-47's order**:

1. **The ground.** At Ahuna Flank the miner leases a prospector and the escort line tank. A staff
   `@econ/pay` moves 7,000cr from the bystander's captain first, since a new captain's stake
   covers one or the other. The miner launches both, and they share the ramp's cell for a moment.
   The prospector is scanned, sounded and set to core an unread pocket (`+tv/1 mine go`). A
   player escort flies the line tank. A hired NPC flies a fixture hull on doctrine. A bystander
   rig from another captain mines the next block.
2. **The raider.** It launches from its fixture carrier (`@field/raiders/launch`). Each of its
   hulls is a new row of the miner's presence picture before it declares.
3. **Promotion.** It declares on the bystander (`+field/mark NW=hostile`). The site promotes with
   everyone admitted at their poses: both carriers, all four rigs and hulls, the barge and both
   raiders. The raider's leader clears its standing marks on everyone but the bystander.
4. **One picture.** The miner's quarry screen holds the raiders. The escort's cockpit carries
   every reading of the miner's that the bystander's does not (G-7).
5. **The bystander.** A raider fires on it, and its own frame holds the raider's party hostile and
   no other party.
6. **The fight and the hits.** The leader turns on the escort and the escort's pilot marks the
   raider Hostile. The raider's coaxial breaks the escort's engine through its rear plate, and
   the escort's frame reads `immobile`, not `wreck`. The leader then holds the escort Friendly.
   The escort's coaxial works on the walker until its head goes: the control section, and a
   wreck. The miners never fire.
7. **Withdrawal and demotion.** `@field/raiders/withdraw` takes the tank home up its ramp, and
   the open promotion names its exit. The field demotes once the escort's pilot stands down
   (below).
8. **The fill plan.** The miner's `+tv/1 mine go` runs again.
9. **The site record.** The escort is a `casualty` and the walker a `wreck`, each where it fell.
10. **Restart.** Both are still there after the reboot.
11. **Departure.** `+ship/lift ceres` is refused for the units out. With `leave=` naming them it
    files, and the voyage records the casualty left behind.
12. **The second visit.** Another captain sails in, and by then the miner's carrier has lifted.
    Both units are where they fell.
13. **The second field.** The raider launches again and declares on the escort's hull. Field 2
    admits the wreck, the casualty and the tank. The wreck-as-cover clause is pinned (G-8).

**Stand-ins and pinned gaps**:

- **The escort's delivered orders (stand-in).** tacsim's world harness lands no DT activation,
  for the same reason it cannot land CF.6c's operational loan. One escort hull is therefore flown
  by a player's own cockpit commands, and the other by a hired NPC on doctrine. The delivered
  orders an escort fights under are proven by CF.6's transport gate and CF-5b's open-ground
  ENGAGE, in the split-boundary harness.
- **G-8, a wreck as cover (pinned at its gap).** "A second field uses the wreck as cover" asks
  that a wreck hide what stands behind it. The kernel's fire and sensor traces read the terrain
  alone. A wreck occupies its cell as an obstacle to movement (doc 16 §3.4) but blocks no line,
  and only the site record's objects are laid as walls (CF.4). The story stands the wreck on the
  row between the escort and the raider, and asserts the escort's sensors still hold the raider
  behind it, so the assert flips when a wreck gives cover. Laying a wreck's cell as cover while
  its unit stands in it needs a kernel rule (a unit standing in a cell that blocks lines), and
  that rule reaches the line-of-sight index and the hashes. It is too large for this cut.
- **The unarmed barge holds a field engaged (§15's known gap, met here).** An armed unit holding
  a live Hostile contact within its reach keeps the tempo engaged
  (`Arena::hostile_in_weapons_range`). The escort, fired on, holds the raider's party Hostile, so
  the barge beside it keeps the field open after the tank has gone home. In the story the
  escort's pilot stands down with a Friendly mark on the raider's party, which outweighs the fire
  it took (CF.0). A player has to do the same today. Ending the raider's barge once its hulls
  are home is the gap's own cut.

**Defaults taken (provisional):** the G-7 rule's reach (party principals, not Friends); the
unstacking order (unit order; nearest ring, row-major, within 4 cells); the lease preview's
`{go}`. The story's 7,000cr transfer is a fixture.

## 18. 512 sites: S.0 as built (the plumbing)

The owner's three-cut plan for Q-CF-51 (the field story again at 512 a side): **S.0** makes a
site's side a per-site value, **S.1** scales the scheme that rides on it, **S.2** runs the CF.7
story at 512 and measures it. S.0 changes nothing a 256 site does or stores; every shipped
region is 2,560 m, so every shipped site is still 256.

**The rulings** (the owner, 2026-11-01, accepting the plan's seven defaults):

1. A composite cap separate from the tile cap.
2. The side is the region's extent. Ahuna Flank and the other shipped sites stay 256; a new 512
   Ceres site comes in S.1; the ground stays generated.
3. Addresses are a 4 × 4 grid of blocks lettered A–P, each side/4 a side, so they run to C1024
   at 512. The per-block shifts stay `[i8; 16]`, and the geology digest takes a new version.
4. The pad comes from the site's pad room, and the raider fixture's cells become offsets from
   the pad.
5. The site view stays 32 cells across, each view cell side/32 site cells.
6. The drive budget is w × h, `to`'s maximum is side² − 1, the `ADVANCE` bound is the site's
   side, and the `u8::MAX` sentinel becomes an `Option`.
7. The presence and sounder bands and the ground cache stay as they are, measured at 512 in S.2.

S.0 builds 1, 2 (without the new site) and 6. S.1 builds 3, 4, 5 and the 512 site.

**The composite cap.** `mush_atlas::window::composite` builds each surface with
`Cells::filled_composite` over `WindowFrame::grid`'s `Grid::over_composite`, both capped by
its own `MAX_COMPOSITE_SIDE` = 512, and decodes a
composite's cells with `cells::deserialize_composite`, so a 512 window crosses the async seam.
A tile keeps `MAX_TILE_SIDE` = 256 by both doors (`Cells::filled`, `Cells`' own `Deserialize`).
A compile-time assert pins `MAX_COMPOSITE_SIDE` to tacsim's `MAX_ARENA_SIDE`. `@window`'s
`window_max_cells` (65,536) is untouched. `MAX_COMPOSITE_BYTES` (8 MiB) holds a 512 site's
one ground surface: its worst case, every cell different, is about 2.8 MiB encoded.

**The side from the region.** `surface::ground::site_side_of_region` reads the region's extent
in 10 m cells. It refuses an extent that is not square, not whole cells, or over 512
(`MAX_SITE_CELLS`). `chart::site_frame` frames `side × side` from that. `site_composite` holds
the composite to it, with CF.7's refusal sentence, its side now the region's. A region missing
from the atlas frames at the shipped default, so its capture refuses it exactly as before.
`ground_side(composite)` and `chart::site_side(w, entry)` are how the rest reads a site's side.
`SITE_CELLS` is now only the shipped default.

**Threaded through:**

- **The site record** carries `side`. It is omitted at 256, so a 256 record's bytes are
  unchanged. Every pose and remembered cell is held to it. Writers with the world build the
  record at the site's side (`fresh_record`). The two provider prepares, which read no world,
  build it from the offer's fixed `sides`, which is present only when a site is not 256.
- **The drive.** `ActParams::Drive` carries the `side` it was planned over, also omitted at 256.
  Its `to` and its `to` word (`y × side + x`) are held to it. `target_cell` and the A* budget
  (`drive_path_budget_cells`, w × h) take the surface's grid. The start's `to` parameter
  reaches `side² − 1` of the widest lane site, which is 65,535 today.
- **The field orders.** `FieldPayload::validate` takes the side. At issue it is the side of the
  site the members' flights stand on. A stored order is held to the widest site.
- **The rest.** The site map's drive window clamps to the surface's side, and the picture's
  `side` is the composite's. The staff `@tacsim/site` cell words and maximum use the site's
  side. A raider's carrier is held to its own site when it is seeded, and the fixture file to
  the widest site.
- **The readings tie-break** keeps the field index as an `Option<u8>`, and a reading with none
  sorts after every field. It no longer uses `u8::MAX`, which would be field 255 once a site
  holds 256 fields.

**Still at 256: what S.1 builds, against the rulings.** Each is wrong on a wider site until
S.1 lands:

- **Ruling 3, the address scheme.** Blocks become side/4 (`CELLS_PER_BLOCK`, `SITE_BLOCKS`
  scale, `BLOCK_LETTERS` stays A–P). Fields, pockets and `field_index` follow (16 × 16 fields
  at 512, so `field_index` reaches 255), up to C1024. Built on it: the chart's row stands and
  caps, the scan, sounding and run stands an excursion record keeps (`stand_on_site`,
  `address::on_scheme`), the campaign layer's block letters, and the picture's `block_side`.
- **Ruling 3, the geology table.** Pockets cover the whole site; the per-block shifts stay
  `[i8; 16]`; the geology digest takes a new version. Today a 512 site's geology covers only its
  south-west quarter.
- **Ruling 4, the pad.** It comes from the site's pad room (`PAD_CENTRE_CM`, `PAD_CELL` retire),
  and `field/raiders.json`'s carrier and ramp cells become offsets from the pad. S.0 checks
  them against the site's side when a raider is seeded.
- **Ruling 5, the site view.** It stays 32 cells across, each view cell side/32 site cells:
  `SITE_VIEW_SPAN` and `SITE_VIEW_CELL_CM` follow the side.
- **Ruling 2, the 512 Ceres site.** A new frontier region of 5,120 m and its port entry.

Ruling 7 builds nothing: S.2 measures the bands and the ground cache at 512, alongside the
story itself.

**Defaults taken (provisional):** a stored record with no `side` reads 256, since there are no
migrations before launch. A stored drive's arrival cell, and a stored order's `ADVANCE`, are
held to the widest site rather than to their own, because neither record names its site's side.
