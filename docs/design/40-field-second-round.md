# 40. The field, second round — surveyed

Draft 0.2, 2026-11-03, docs only. Surveyed against `origin/main` at `c83c2eff` (SF.R14 and SF.R15
merged as PRs #338 and #339; doc 36a's lane 5 as #346; doc 38 Draft 0.2 as #345; lane 3 still open
as PR #340 at `claude/relaxed-fermat-60b65b`, read here for the shape its held-out walls take on
the live record). Stacked on [doc 39](39-industry-production-chain.md)'s PR #347 so its README row
lands after doc 39's. Every tree fact below was re-read at `c83c2eff`; where doc 38 §4's anchor
has moved by a few lines (the join loop, `arena_task.rs:1879`, was `:1869` there) the new line is
given. Draft 0.1, the same day, was the survey. **Draft 0.2 records the owner's rulings of
2026-11-03 on every question: every default accepted as written** ("all Q-40s accepted as their
proposed defaults"). No question was amended, so each stands with its proposed text as the ruling,
and §6's cut list is now the implementation order.

This is a survey in the form doc 38 used: what the tree has, what the rulings require, and the
questions each area raised, each with a proposed default, an alternative and a reason.
"Provisional" marks a number to be tuned later under the 2026-10-24 rule. **(expensive to
reverse)** keeps doc 38 Q-38-10's meaning — an answer a significant refactor would be needed to
undo — and stands on three questions only (Q-40-3, Q-40-7, Q-40-22), each now ruled and built
exactly as written.

## 1. Why this document exists

Doc 38 §6 proposes this document for four of its categories: **G** command and reinforcement,
**H** relations (stances, standing, the void), **I** kernel depth (one rebless), and **C**'s kernel
half (the coupled tow), plus doc 36's Q-36-48 seam (`SpacePartyId` to the arena's `PartyId`). Its
rulings of 2026-11-03 bind every default here (§2.1). Doc 38 §7 places the stance table beside
doc 39's survey now, the rebless "planned through 1–2 and built once, when its four items are
ready", and doc 38 §8 asks this document for one thing before any kernel file is touched: **the
hash-pinned harness runs the rebless re-blesses, listed** (§4a).

Doc 37 §7 had the same work spread over items 4 (stances), 8 (the second rebless) and 9
(reinforcement launches); doc 38 gathers them because each of the three reaches into the field
host's join path and its books, and two reblesses for one round would break doc 36 §7's first
rule.

## 2. What binds this survey, and what it leaves alone

### 2.1 The rulings

| Ruling | What it fixes |
|---|---|
| Doc 38 Q-38-17 (expensive to reverse) | the towed casualty is admitted as a kernel unit coupled to its tug: it can be struck, a hit on either does not break the coupling (Q-36-20), the tug may release inside the field, an Irrecoverable tug drops it at the tug's last cell; one rebless with Q-38-27 |
| Doc 38 Q-38-23 | an organization-to-organization stance table in mush-faction (`data.rs:23–26` reserves it): Allied, Neutral, Hostile, declared by an organization's owner, read at admission and at a join, never mid-field; one rung above shared membership in `parties.rs`'s order and below a mark and fire; faction effects as GC.10 |
| Doc 38 Q-38-24 | the fire tally and the void offers ride the live field record (as PR #340's held-out walls do); the void window closes at the leader's next command, synchronously in the command path, not in the sweep; a director raid's leader voids its own party's friendly fire automatically |
| Doc 38 Q-38-25 | Q-36-5's alternative, bounded: a carrier on its pad (there at promotion, or arrived by F-6) may launch into the running field; the launch enters at its ramp at the next pulse, counts against the cap, and has a launch cycle (provisional 60 s wall); the clocks stay as they are |
| Doc 38 Q-38-26 | DT.4 sub-leaders and open-ground SCREEN/SUPPORT stay deferred (Q-36-43, Q-36-42) |
| Doc 38 Q-38-27 (expensive to reverse) | one in-match effect a component kind, authored in the content: Sensor — detection and lock range; Ecm and Eccm — their modifiers off; Power — heat capacity and charge; Relay — orders and comms delayed; Core — as the section's `DegradeCore`; jammed is off until unjammed, damaged is degraded; one rebless with Q-38-17, `line_to`'s wrecks and `note_fire`'s fix |
| Doc 38 Q-38-28 | `bytes_total` resets a segment on resume with the archive holding segments; `max_bytes` unchanged |
| Doc 38 Q-38-29 | every new cut's story runs at 256 and 512 and the bot types the player's half over SSH; the four-digit addresses and the vocabulary are the first cut of whichever document starts first |
| Doc 36 Q-36-56, §7's first rule | one kernel rebless a programme: every hash-moving change batched, the practice hashes flipped once |
| Doc 36 Q-36-4, Q-36-5, Q-36-10, Q-36-11, Q-36-20 | F-6 arrival joins at its pad; the idle bound; Friends by shared membership below a mark and fire; standing at demotion with the leader's void; the coupled tow in a field |
| Doc 32 Q-CF-7, Q-CF-11, Q-CF-16, Q-CF-21, Q-CF-26, Q-CF-30, Q-CF-32 | marks are private; letters by key order; everyone on the ground is in the field; one cap, refused at the ramp; casualties and wrecks never count; surface acts wait on a promoted site; the field runs on wall time and the world keeps its rate |

### 2.2 Left alone

Doc 38 §2.2's fix thread keeps its defects; this document only batches the kernel-side ones into
the rebless (§4.1): the think-batch await before joins (`arena_task.rs:1879` against `:1937`),
`note_fire`'s direction (`arena/party.rs:340–357`), and the retry of GC.1b's refused arrival
(`field/carrier.rs:562–592`), which reinforcement needs anyway (Q-40-10). `ConditionTable`'s
scratch, `carried_rig`, the held-out walls and `set_down` after an irrecoverable tug are PR #340's.
Field engineering under fire (the tender's `field_unjam_component`, `engineering.rs:237`, refuses
`site_in_combat`) is doc 41's, so inside a field a jam lasts the field. Capture is doc 42's. S.4b
is doc 23's. DT.4 and open-ground orders stay as ruled (Q-38-26). The 10 km ground, infantry, the
mission generator and facility assaults stay out by doc 38 §4's ruling.

## 3. The tree at `c83c2eff`, in one table

What each area of §4 starts from, each fact re-read.

| Fact | Where |
|---|---|
| A struck part is set jammed or damaged only when the round penetrated and the section stands (`struck.penetrated && !struck.fresh`); one draw, `component:<shot_seq>`, below `jam_permille` a jam, below the sum a damage, the part picked in component-id order among those the outcome can change | `arena/damage.rs:716–732`, `part_struck` `:1025–1065` |
| The only in-match effect of a struck part: a jammed mount leaves the fire table, its rounds kept | `damage.rs:782–789` |
| `part_works` filters mounts at check-out by the stored condition word; nothing else reads a part's condition | `arena/state.rs:1802–1810` |
| The kinds: the mount, Sensor, Ecm, Eccm, Power (states `off`/`starting`/`online`), Relay (`radius_km`, a room), Core | `definitions/unit.rs:463–548` |
| The chances are content on `UnitSpec::component_hits`, absent in every shipped unit; `COMPONENT_HITS` 50 and 100 per mille when a unit names none | `unit.rs:156–159`, `:951`; `arena/resolution.rs:266–271` |
| Which shipped units carry which kinds: every tank, biped and the escort a sensor, a plant and a core; `line_tank_ecm` a pod, `line_tank_eccm` a filter; only the Halcyon a relay; the Dray, Dray-P and corvette a plant alone | `content/units/*.json` |
| Section destruction carries authored consequences: mounts removed, slowed, no turn toward, motion disabled, the core degraded or destroyed, the unit destroyed | `arena/resolution.rs:153–205` |
| Heat: one cell, the ceiling is the cell's maximum, dissipation carried in thousandths; no damage input | `arena/heat.rs:1–60`, `heat_max` `:82` |
| The sensor row the kernel reads: `max_range_cells`, `share_range_cells`, `sees_wrecks`, the signature factors; the range test at `:1236`; a wreck's sensor is cleared | `arena/perception.rs:206–222`, `:1236`; `damage.rs:1427` |
| The pod and the filter ride `UnitState.jammer` and `.eccm`; the field's widest bubble is `jam_radius_cm` over the units | `state.rs:2185`, `:2215`, `:2656–2662` |
| The core's class is read live off the `core_class` cell; a class below the sealed one drops a mind to the ROM at its next landing | `arena/core.rs`, `damage.rs:1238` |
| `line_to` traces terrain only; the kernel's fire and sensor traces see wrecks | `arena/stock_mind.rs:634–648`; `arena/wreck_cover.rs`; `damage.rs:641–650` |
| `line_to` is a kernel of the unit-mind API at fee 20, capped a think; API version 7 since CF.0 | `minds/kernels.rs:26–27`, `:190`; `minds/mod.rs:191` |
| `note_fire`: the victim's book and every book that holds the victim a friend hold the shooter hostile, except the shooter's own; a friend of both **is** moved | `arena/party.rs:340–357` |
| The relation order: own, else the leader's mark, else fire produced, else a standing friend, else neutral; `befriend` is both ways; the host's `FieldHost` carries opening marks and friend pairs | `party.rs:266–281`, `:324–331`, `:389–446` |
| A join opens the newcomer's book and applies the host's friend pairs and marks that name it, in either book; a mark by an existing party on the newcomer lands in the existing party's book | `arena/join.rs:165–180` |
| Joins are buffered one a unit and applied at the pulse's head after the homes and before the step; the previous pulse's think batch is awaited after them, so a join may meet `Awaiting` | `arena_task.rs:1830`, `:1876–1880`, `:1937`; `snapshot.rs:641` |
| The ledger: `max_bytes` 64 MiB, `max_unflushed_chunks` 256; `bytes_total` cumulative; `LedgerFull` past the bound; `EndReason::Timeout`; `LedgerState` carries `bytes_total`, `seq` and both digests; `from_state` resumes them as they stood | `mush-config config.rs:1618–1628`; `arena/ledger.rs:844`, `:873–882`, `:967`, `:1033`; `step.rs:752–762` |
| The field resumes with `segment += 1` and no reset; the live record holds `segment` | `field/boot.rs:228`; `field/record.rs:159` |
| GC.12's measure: 7.4 MB in two minutes of non-stop fighting at 100 units, about eighteen engaged minutes to the bound | doc 36 "As built (GC.12)" l.1805 |
| The towed casualty rides as its tug's load, off the kernel's ground; admission filters carried units out; release inside a promoted field refuses `FIELD_OPEN_CODE`; at demotion the casualty stands at its tug's cell, and PR #340 stands it at its own pose when the tug fell irrecoverable | `surface/tow.rs:49–55`, `:1331–1337`, `:2408–2422`; `field/admit.rs:335–340`; `field/close.rs:367` |
| Every attach is `TowRigProfile::Standard`; the cap is twice the tug's mass | `tow.rs:686`; `mush-tactics field_ops/recovery.rs:149–176` |
| A launch onto a promoted site refuses `FIELD_LAUNCH`; the cap is checked at the prepare against `site_population` plus the slots; `LAUNCH_S` is 300 IC seconds | `detachment/sortie/prepare.rs:559–584`; `surface/site_record.rs:60`, `:1106–1110`; `surface/excursion.rs:72` |
| The only `ArenaMsg::Join` senders: F-6 arrivals (spread 8 cells at the pad) and the director's raids (spread 4 at the ramp); a refused arrival is not retried while its field runs; the director retries each shape at the next fire | `field/carrier.rs:74`, `:505–515`, `:562–592`; `field/raiders.rs:64`, `:987`, `:1075–1083`; doc 36 GC.11 as built |
| The field host's tick holds while the W-clock is frozen; the exits at the ramp and the arrivals ride it | `field/admit.rs:3`, `:65–73` |
| Friends from a shared owner or an openly visible organization, faction or principalship, read when an entrant is built (admission and join); the principals a standing receipt names are the leader and, for a roster, the organizer | `field/parties.rs:13–21`, `:76–96`, `:116–125`, `:480`, `:509` |
| mush-faction reserves "the future relationship matrix"; its edges are `ChapterOf`/`SubordinateTo`/`ContractedTo`/`FrontFor`; a faction has `officer_level`, `hidden` and `principals`; the staff verbs are `@faction/…`, the player verbs `+faction`, `+factions`, `+standing` | `mush-faction/src/data.rs:23–26`, `:161–167`, `:247–311`; `commands.rs:1678–1810` |
| mush-org's `Organization` has no owner field; authority is a capability granted by `@org/grant <obj>=<org> <cap>`; `PrincipalId` is an object, an organization or a system principal; membership has a status and a visibility | `mush-org/src/record.rs:30–50`, `:124–134`; `ids.rs:142–149`; `commands.rs:44` |
| `FireTally` and `VoidOffers` are `reg.state` RAM; `void()` checks only the IC deadline; the sweep closes on `session_stat(leader).last_input` once a second; `live()` accepts an NPC leader; the receipt API already carries `voidable_by`, `void_standing` and `standing_causes` | `field/standing.rs:68`, `:84`, `:141–145`, `:287–320`, `:322–345`; `field/mod.rs:294–295`; `mush-faction/src/reputation.rs:62–72`, `:268`, `:332`, `:394` |
| The void window: 600 IC seconds, which at rate 24 is **twenty-five wall seconds** | `standing.rs:45`; doc 32 Q-CF-32 |
| The live record is written whole by `record::write_live` in the batch that changes it, under `tacsim:field:live:<site>`, format 1, checked for its invariants first; PR #340 adds `held: Vec<HeldWall>` with `#[serde(default)]` and no format bump | `field/record.rs:29–33`, `:149–180`, `:269–294`; `field/carrier.rs:535–552` |
| The last snapshot document each running field's world holds is the void rule's source, taken every `summary_ms` (5 s) and kept under `tacsim:field:snapshot:<site>` | `field/mod.rs:143`, `:352`; `config.rs:1624` |
| `SpacePartyId(pub u32)` maps to the arena's letters by `to_party_id`/`from_party_id`; `SpaceRelation` is Friendly/Neutral/Inspecting/Hostile; 68 hits, 57 in code across `space/bubble.rs` (14), `space/mod.rs` (25), `custody.rs`, `picket.rs`, `tick.rs` and `tests/shard_10/voyage.rs` (7), the rest in docs 35a, 36, 37 and 38 | `space/bubble.rs:37–72`, `:1257` |
| The stories: one script on Ahuna Flank (256) and Urvara Reach (512); every GC story at one size | `tests/shard_9/field_story.rs:90`, `:113`, `:406`, `:414` |
| Doc 39's cut list takes none of the bot's four-digit addresses (its IN.10 adds `+yard`, `+market` and `+bay` words only) | doc 39 §6 |

## 4. The survey

### 4.1 The rebless batch (I, C's kernel half, and the fix thread's kernel items)

**What exists.** §3's first thirteen rows: a part can be jammed or damaged by a surviving hit and
nothing but a mount notices; the mind's `line_to` sees no wreck while the kernel's own traces do
(GC.2's as-built note measured it: the twin record's `line` and `no_line` divergence counters went
0 → 28 and 0 → 31 when wrecks became cover); `note_fire` moves a friend of both; the casualty in
tow is not a kernel unit, so a hit on it is not modelled and release waits; a join can meet a think
batch still in flight and be refused by name.

**What the rulings require.** Q-38-27's six effects, authored in the content, one rebless with
Q-38-17's coupled tow, `line_to`'s wrecks and `note_fire`'s fix; doc 37 §7 item 8 adds the
think-batch await. Q-36-56's rule makes them one cut, with the pins of §4a re-pinned once.

**The batch, in the order the cut builds it.** Each item is named so §6's FR.5 can list its files:

1. **The think-batch await before joins.** `await_thinks` (`arena_task.rs:1937`) moves ahead of
   the marks, withdrawals, homes and joins the pulse's head applies (`:1863–1880`), so a join is
   built on an arena whose answers have landed and `Snapshot::encode`'s `Awaiting` refusal
   (`snapshot.rs:641`) is unreachable from the join. No OOC match ever joins, so no practice pin
   moves; it is kernel-task code and the rule batches it. GC.11's retry stays as a guard.
2. **`note_fire`'s fix.** The told set (`party.rs:344–352`) skips every party whose book holds the
   shooter a Friend, as the doc comment already claims; Q-40-9 asks whether the victim itself is
   skipped when it holds the shooter a friend. The books fold into the hash (`arena/hash.rs:1421–1423`),
   and only a field of three or more parties with a friend of both is moved: the two-party practice
   corpus is not.
3. **`line_to` over wrecks.** `stock_mind::line_between` (`:655`) takes the arena's wreck overlay
   (`wreck_cover::WithWrecks`) beside the terrain, so a mind's `line_to(v, id)` answers what the
   fire trace will find. The twin record's `line`/`no_line` counters close; every content-backed
   pin moves. Whether this is an API bump is Q-40-8.
4. **The component effects.** One function, `Arena::apply_part_effects(unit)`, derives the unit's
   live rows from its parts' condition words: called once in `admit_unit` (so a condition the bay
   left is honoured at check-out for every kind, as `part_works` does for mounts today) and once
   after `part_struck` lands its op (`damage.rs:780`). The rows and their defaults are Q-40-3's
   table; the content schema is Q-40-2. The draw-label set is unchanged (no new draw: the effect
   is a function of the condition, not a second roll).
5. **The coupled tow as a kernel unit** (Q-40-7). The casualty is admitted beside its tug and
   trails it cell for cell; it is struck through its own sections; release is a field act; an
   Irrecoverable tug leaves it where it stands. IC fields only, so no practice pin moves.

**Why one cut.** Items 3 and 4 move every content-backed pin; items 1, 2 and 5 move none of the
practice pins but touch the same files (`arena_task.rs`, `party.rs`, `state.rs`, `join.rs`) and
would each need the twin corpus re-run to prove it. One PR, one re-run, one re-pin.

**Questions.** Q-40-1 to Q-40-10.

### 4.2 The stance table (H)

**What exists.** `parties.rs:13–21`: Friends by a shared owner or an openly visible organization,
faction or principalship, read when an entrant is built, at admission and at a join; "there are no
declared stances between organizations yet". A raider's party opens with its leader's standing
Hostile mark on every other party through `FieldHost.marks` (`raiders.rs`, `carrier::joining`),
which is the one precedent for a relation that is neither membership nor a leader's live mark.
mush-faction reserves the matrix and has no row for it; mush-org has no owner on an organization,
only capabilities granted per object. The kernel's `Book` has marks, fired and friends and nothing
between fired and friends (`party.rs:266–281`).

**What the rulings require.** Q-38-23: Allied, Neutral, Hostile between organizations, in
mush-faction, declared by an organization's owner, read at admission and at a join, never
mid-field, one rung above shared membership and below a mark and fire; faction effects as GC.10.
Doc 38 §7: this half needs no rebless and starts now, beside doc 39.

**How it is read without a kernel change.** The kernel's order is mark, then fire, then friend.
Every case the ruled order needs falls out of the two rows the host already hands the kernel:

- a declared **Hostile** becomes an opening Hostile mark in the declaring party's leader's book
  (`FieldHost.marks`, exactly as a raider's), so it beats shared membership (a mark beats a
  friend), and a leader's later Friendly mark replaces it (a mark is a replace), which is "below a
  mark"; fire on a declared-Hostile party changes nothing, both say Hostile;
- a declared **Allied** becomes a friend pair (`FieldHost.friends`), so it loses to a mark and to
  fire produced, which is "below a mark and fire", and adds nothing to membership where both hold;
- **Neutral** is the absence of a row, and clears one.

At a join, `Arena::join` applies the host's marks and pairs that name the newcomer in either book
(`join.rs:165–180`), so an existing party's declared Hostile on an arriving organization lands in
the existing party's book at the join, and the ruled "read at admission and at a join" needs no
new kernel path. A stance change while a field runs changes nothing in it (ruled); the next
admission or join reads the table as it then stands.

**Who declares, and where it lives.** The field's organizations are `Affiliation`s
(`parties.rs:64`): a mush-org or a faction. The table is keyed by them, lives in `FactionState`
beside the edges and persists under `mod:faction:stance:<from>:<to>`. "An organization's owner" has
no field to read: for a faction it is a principal or a member at `officer_level`; for a mush-org
it is a member holding a capability (Q-40-11). The player verb is `+faction/stance`, the staff
verb `@faction/stance`, and `+faction <name>` shows the rows (Q-40-11).

**Faction effects.** GC.10's standing is unchanged by a stance: fire costs the shooter standing with
the victim's factions whatever was declared, and the void's friendly-fire test (`parties::friends_of`)
counts a mutual Allied pair as it counts shared membership (Q-40-14).

**Questions.** Q-40-11 to Q-40-14.

### 4.3 Durable standing: the tally, the offer, the synchronous close, the director (H)

**What exists.** The tally is counted from the `impact` lines of each chunk as it passes
(`standing::note_chunk`), in RAM, so a restart mid-field forgets the rounds before it; the offer
opens at demotion in RAM, so a restart inside its window closes it and the change stands
(`standing.rs:28–31`); the "next command" close reads the session's input stamp once a second in
a sweep, whose own doc records that two inputs in one second can slip and a command in the
demotion's second is not seen (`:293–320`); an NPC leader has no session, so only the IC window
closes its offer and nothing voids on its behalf. The window is ten IC minutes — at rate 24,
twenty-five wall seconds. The receipt already names who may void (`voidable_by`), records a void
beside the grant and lists causes by prefix (`reputation.rs:268`, `:332`, `:394`). PR #340's
`FieldLive::held` is the pattern the ruling names: rows on the live record, `#[serde(default)]`,
written by the batch that makes them, laid again by every rebuild.

**What the rulings require.** Q-38-24: both ride the live field record; the close is synchronous in
the command path; a director raid's leader voids its own friendly fire automatically.

**The tally on the record.** `FieldLive` gains `fire: Vec<FireRow { shooter, victim, rounds }>`,
counted as today and **written in the snapshot's batch** (`field/mod.rs:352`), not per chunk: a
field resumes from that snapshot and re-fights the pulses after it, whose rounds are counted
again, so a tally written with the snapshot is exactly the resumed field's (Q-40-15). A per-chunk
write would be a store write a pulse for nothing the resume could use.

**The offer.** At demotion the live record is removed and a closing tombstone written
(`record.rs:35`, `FieldClosing`), so an offer that opens at demotion has no live record to ride.
The one durable thing that outlives the field and already names the voider is the grant itself:
the receipt gains the window (`opened_at`, `until_ic`) beside `voidable_by`, written once with the
grant, and `VoidOffers` goes (Q-40-16). `+field/void` then voids every open grant whose voider is
the actor (`standing_causes(w, "field:")`), and `+standing <faction>` already shows a void.

**The synchronous close.** The engine has no command hook (GC.10 as built). The smallest seam that
closes the window in the command path is one stamp: the session keeps the input before this one
(`prev_input`) beside `last_input`, and `+field/void` refuses when `prev_input > opened_at` — the
leader entered something since the offer opened, other than this void (Q-40-17). No sweep, no
tick; `tacsim.field.standing` retires. The one-second ambiguity stays where the stamps are
seconds, and is named.

**The director's auto-void.** A grant whose voider is a raider's leader under a director raid
(`RAIDER_ATTR`'s `raid` key, GC.11) is voided in the demotion's own batch by the host, as that
leader, with its own reason; a staff-seeded raider's leader is offered nothing, as today (Q-40-18).
`record_voidable_standing` refuses a voider that is no live character (`reputation.rs:276–278`);
an NPC leader is one, as `live()` already accepts.

**Questions.** Q-40-15 to Q-40-18.

### 4.4 Reinforcement (G)

**What exists.** `FIELD_LAUNCH` at the prepare (`prepare.rs:559–571`) with the cap's second refusal
after it (`:573–584`, `site_population` plus the slots against `max_units`); two `Join` senders,
each with its own spread and its own answer handler; the director's retry of refused joins at the
next fire (GC.11); GC.1b's refused arrival left outside until the field demotes; `LAUNCH_S` 300 IC
seconds for the launch act (12.5 wall seconds at rate 24); the ramp as a site position
(`exit::rig_ramp`, `:483`) and the kernel's one-cell rule at it (`withdraw.rs:17`); the field tick
holding while the W-clock is frozen (`admit.rs:65–73`), and the launch act on the IC clock holding
with it. Q-36-5's idle bound (`idle_demote_s` 600) stands.

**What the rulings require.** Q-38-25: a carrier on its pad — there at promotion or arrived by
F-6 — may launch into the running field; the launch enters at its ramp at the next pulse, counts
against the cap, and has a launch cycle (provisional 60 s wall) so a bay cannot pour; the clocks
stay as they are. Q-38-26: no sub-leaders, no open-ground SCREEN/SUPPORT.

**The launch rule** (Q-40-19). `+sortie/launch` and the rig ramp's launch run as today on the IC
clock; the prepare's `FIELD_LAUNCH` refusal becomes the cap check alone, and at the act's landing
each hull is checked out under the field's mark and offered as `ArenaMsg::Join` at the carrier's
ramp (`LAUNCH_SPREAD_CELLS` 4, the director's spread), landing at the next pulse through
`carrier::joining` and GC.1b's handler, told to the site with a line of its own. The cap is counted
twice as today, at the prepare and by the kernel's `PartyFull` (`join.rs:273`). The cycle is
`[tacsim.field].launch_cycle_s` (60, provisional, banded like `disengage_s`): one hull a carrier
a cycle, a launch act of several slots putting them down one a cycle in slot order, and a second
act inside the cycle refused with a line naming the field and the seconds left. A carrier that
arrived by F-6 launches under the same rule once it has joined, which closes the half state
Q-36-5 named. The W-clock freeze needs no new rule: the act and the join both ride ticks that
hold while it is frozen. The bay under fire: no hold beyond the cycle; GC.3's table already closes
a wrecked carrier's bay and a hull put down beside the ramp is a target from its first pulse.

**The refusal** (Q-40-20). A hull on the ground and not in the field is the one state the design
forbids (Q-CF-21), so a launch whose join the kernel refuses goes back aboard and is offered
again at the next tick, the director's pattern (`raiders::join_raid`), until taken or the field
demotes; GC.1b's refused arrival takes the same retry instead of waiting for demotion. After
§4.1's item 1 a refusal is rare; the retry is the guard.

**Its story.** Doc 32 Q-CF-47's escort story with a second escort launched from the pad after the
first shot, on Ahuna and on Urvara (§6, FR.3).

**Questions.** Q-40-19, Q-40-20.

### 4.5 The ledger per segment (I)

**What exists.** §3's ledger rows. `bytes_total` spans the field's life; the snapshot every 5 s
carries `LedgerState` whole; a resume takes it as it stood and bumps `FieldLive.segment`
(`boot.rs:228`); at 100 units the bound is about eighteen engaged minutes, and the field stops
with `Timeout` — the result's end, with nothing appended (`step.rs:752–762`). The egress keeps the
chunks the world acknowledged; `seq`, `digest` and `shipped` are the one chain `log:match.<id>`
can reproduce.

**What the rulings require.** Q-38-28: `bytes_total` resets a segment on resume, the archive
holding segments, so the bound is a segment's and the checkpoint already built carries the rest;
`max_bytes` unchanged.

**The roll.** The ruling's own reason — a reinforcement rule makes long fields the norm — is not
met by a reset that waits for a restart: a field that never restarts still stops at eighteen
minutes. So the segment rolls **in place** as well as on resume: at the snapshot cadence, once
`bytes_total` has passed a provisional three quarters of `max_bytes` (48 MiB), the task takes the
period's snapshot as the new segment's head, resets `bytes_total`, bumps `FieldLive.segment` in
the host's batch and appends one `segment` line; a resume does the same from the snapshot it
rewinds to (Q-40-21). `seq`, `digest` and `shipped` continue, so the archive stays one chain under
one match id with the segment boundaries on the tape; the world keeps the latest snapshot as
today, and the chunks are every segment's. OOC matches never roll (the field host is the only
caller), so no practice pin moves; it is kernel code (`Ledger`, the task) and §6 keeps it its own
cut, FR.4, built after FR.0's inventory proves the OOC pins stand.

**Questions.** Q-40-21.

### 4.6 The `SpacePartyId` seam (Q-36-48)

**What exists.** §3's space row: a `u32` newtype already mapped onto the arena's letters both
ways (`bubble.rs:44–57`), a four-valued `SpaceRelation` of its own, 57 code hits in five source
files and one test. Doc 38 §6 gives the seam to this document; doc 35b §8 owns the bubble's party
assembly. Doc 37 §7 item 1 flags the space track's fake ids as expensive once anything persists.

**What the rulings require.** Q-36-48 (doc 36, "space's own cut"): `SpacePartyId` to `PartyId`.
Nothing else is ruled.

**The seam** (Q-40-22). `SpacePartyId` is deleted and `PartyId` used in its five files and the
test; `SpaceRelation` folds onto the arena's `Relation` with its marks, and `Inspecting` — a
customs picket's screening, which the arena has no word for — stays a bubble-side flag on the
picket's party rather than a fifth relation; the bubble's `set_party_relation` takes a `Mark`.
The stance table of §4.2 then feeds a bubble's opening marks and friend pairs as it feeds
`FieldHost`, so one authoring verb serves both hosts. Types only: no hash, no content, no
persisted document moves at `c83c2eff` (the space track persists none yet, doc 37 §5.3).

**Questions.** Q-40-22.

### 4.7 Acceptance at both sizes, and the bot's first cut (K)

**What exists.** `field_story.rs` runs one script on Ahuna Flank and Urvara Reach; every GC story
runs at one size; `field_scale` is `#[ignore]` with one hand run (Engaged p99 6.375 ms at 100
against 40 ms, GC.12); the bot has tow, load and release and no word for `+field/void`, a launch
into a field, or `+faction/stance`; `is_addr` takes three digits (doc 38 §4 K). Doc 39 started
first (PR #347) and its cut list leaves the four-digit addresses untaken.

**What the rulings require.** Q-38-29: every new cut's story at 256 and 512, the bot typing the
player's half over SSH, and the four-digit addresses and the vocabulary as the first cut of
whichever document starts first.

**The rule applied.** FR.0 carries the four-digit parser and this document's words (`+field/void`,
`+sortie/launch` into a field, `+tv/<n> release` in a field, `+faction/stance`) and the pin
inventory of §4a; each later cut's story is one script over the two `Ground`s of
`field_story.rs:90–135`; FR.7 runs the round's story whole over SSH on conclav.dev with the bot
(Q-40-23).

**Questions.** Q-40-23.

## 4a. The hash-pinned harness runs the rebless re-blesses

Doc 38 §8's requirement, listed before any kernel file is touched. Each row is one pin or lane at
`c83c2eff`, with what moves it and what FR.5 expects of it. **Run first** means the row is run at
the batch's base commit and its output recorded in `40-cut-notes.md` before the first kernel
change; **by name** means an `#[ignore]`d test run as CI's `tacsim-minds` cell runs it. Every
cargo invocation on the owner's Windows box carries `-j 8`, one workspace build at a time.

| Pin or lane | Where | What moves it | Expected of FR.5 |
|---|---|---|---|
| `M4_5_POOLED_RECORD` (21 lines), `COVERAGE`, `POOLED_MATCHES_WITH_STAND_IN` | `tests/shard_2/twin.rs:1177`, `:1777`, `:1786` | content-backed hulls: the component effects (item 4) and `line_to` over wrecks (item 3) | **moves**; the `line`/`no_line` divergence counters return toward nought; every winner and `first_divergence_ms` re-read and explained |
| `M2_CROSS_PLATFORM_PINS` rows 0–3 | `tests/shard_2/minds_rhai.rs:3578`, `m2_the_twin_corpus_hashes_the_same_on_every_host` `:3692`, by name | the same two items | **moves** on rows 0–2; row 3 (the float probe) must not; Windows and Linux byte for byte |
| `DUEL_HASH_API6_OBSERVED`, `DUEL_HASH_AT_HEAD` | `tests/shard_10/motor.rs:68`, `:76`; `m1a1_the_shipped_duel_hashes_as_before` `:124`; `gc1b_an_ooc_match_offered_a_join_keeps_its_practice_hashes` `:161` | a hand-built duel with no chances and no wreck in a line; moves only if a new `UnitState` row folds when absent | **unmoved**: the batch folds `coupled_to` and the derived effect rows only when present |
| The (j) field's substep hashes and ledger lines | `tests/shard_2/reflex.rs:70`, `j_field` `:1199` | two reflex machines, no chances, no wreck in a line | **unmoved** |
| `M2_TWO_T1_MEMORY_DIGESTS`, `M2_UNARMED_MEMORY_DIGESTS` | `tests/shard_2/minds.rs:7262`, `:7326` | the production encoding of a think's memory | **unmoved** |
| The draw-label set (seven kinds, `component` last) | `arena/rng.rs`; `mush-contract-tests/tests/arena_library.rs:167` `the_draw_label_set_is_pinned`, `adding_a_label_shifts_no_existing_draw` | a new draw | **unmoved**: the batch adds no draw |
| `DIGESTS` (14 packs), `POWER_STATES_SCHEMA_DIGEST` | `tests/shard_2/content.rs:101`, `:2458` | a byte of a shipped pack | **unmoved** under Q-40-2's default (compiled defaults, nothing on the shipped units); moves once if the resolution pack carries the table |
| `STANDARD_SHA256`, `API_VERSION` 7, the default seal identities | `minds/content.rs:166`; `minds/mod.rs:191`; `match_/seal.rs:1392` `m5rules_default_seal_identities_are_pinned`; `shard_8/match_lifecycle.rs:8781`, `shard_2/content.rs:2260` | the doctrine source; a bump of the mind API | **unmoved** under Q-40-8's default; an API bump moves `MIND_ABI` and every `mind_sealed` digest |
| `SNAPSHOT_FORMAT` 19 | `arena/snapshot.rs:347` | a field that cannot decode by serde default | **unmoved**: `coupled_to`, the effect rows and the segment's byte count decode by default, GC.2's precedent |
| `FIELD_LIVE_FORMAT` 1 | `field/record.rs:29` | a row the boot cannot default | **unmoved**: `fire` rows default to empty, PR #340's `held` precedent |
| `ARENA_V4_DIGEST`, `OOC_V6_DIGEST`, `STAFF_LOBBY_V4_DIGEST`, `ATLAS_V1_DIGEST` | `tests/support/mod.rs:487–838` | a byte of those packs | **unmoved** |
| The field gates that assert kernel behaviour | `tests/shard_1/field_gc2.rs` (the four GC.2 gates), `field_gc4.rs:107` (story a: the tow's admission changes its asserts), `field_gc10.rs:197` (the void gate demotes on `idle_demote_s` today because of the friend-of-both; after item 2 it demotes on `disengage_s`), `tests/shard_9/field_story.rs:406`, `:414` (both sizes) | items 2, 4, 5 | re-read, their claims kept, their fixtures moved where the batch moves them, each noted |
| `field_scale` at 100 | `tests/shard_1/field_scale.rs`, `#[ignore]`, by hand in release, GC.12's command | the effect derivation and the trailing casualty in the pulse | re-measured once after the batch, as GC.12 re-measured after GC.2; the 40 ms budget stands |
| The contract guards | `mush-contract-tests`: the panic budget, the matter census (`.mint_origin(`), the restriction lints, `legacy-handlers.toml` | new verbs and switches (`+faction/stance`, `+field/void`'s change, the launch line) | green before every push, per the house rule |

## 5. Owner survey

**Ruled 2026-11-03.** The owner accepted every default as written ("all Q-40s accepted as their
proposed defaults"); nothing was amended, so no question carries a decision line of its own and the
proposed text of each is its ruling. Each question keeps its proposed default, alternative and
reason for the record; §6 is the plan built from them.

### 5.1 Order and shape

**Q-40-1 — The order of the cuts, and when the rebless lands.**
- **Proposed default:** §6's order: FR.0 (the inventory and the bot's words), FR.1 (stances), FR.2
  (standing), FR.6 (the seam) start now beside doc 39; FR.3 (reinforcement) after FR.1, since both
  edit `carrier.rs`'s host assembly; FR.4 (segments) after FR.0; FR.5 (the rebless) after FR.3 and
  FR.4, built once, last but for the story; FR.7 the story whole.
- **Alternative:** the rebless first, so reinforcement never needs the retry guard.
- **Why:** doc 38 §7 places the rebless "built once, when its four items are ready"; the host-side
  cuts are each small and independent, and the retry guard is the director's code already.

**Q-40-2 — Where the per-kind effects are authored.**
- **Proposed default:** a compiled table of defaults in `arena/resolution.rs` beside
  `COMPONENT_HITS` (`ComponentEffects`, one row a kind, the numbers of Q-40-3), overridable by an
  optional `component_effects` block on the resolution pack's def (`tacsim.resolution.stock`,
  `content/resolution.json`) and, per unit, by an optional `UnitSpec::component_effects` absent in
  every shipped unit — GC.2's `component_hits` precedent, so no `DefinitionRef` and none of the 14
  pack digests move.
- **Alternative:** the effect on each component row (the sensor row's `jamming` and `emission`
  blocks, the plant's states), re-signing the units that carry them.
- **Why:** the ruling says "authored in the content"; the resolution pack is content, it is
  already where the hit curve and the error terms live, and it is one digest to re-sign if it is
  ever used, against fourteen.

**Q-40-3 — The six effects, their numbers, and jammed against damaged.** **(expensive to reverse)**
- **Proposed default:** one function recomputes the unit's derived rows from its parts' condition
  words at admission and after every part change. Jammed is **off**, damaged is **degraded by a
  per-mille**, provisional:

  | Kind | Jammed (off until unjammed) | Damaged (degraded until repaired) | Where the kernel reads it |
  |---|---|---|---|
  | Sensor | the hull's sight is gone: `UnitState.sensor = None`, no detection, no lock, its contacts age out as a wreck's do | `max_range_cells` at 500 ‰ — detection and lock range; `share_range_cells` untouched | `perception.rs:1236`, `lock_range_step_m` `:2663` |
  | Ecm | the pod radiates nothing: `jammer = None`; the field's `jam_radius_cm` shrinks | the same (the ruling: "their modifiers off") | `state.rs:2185`, `:2656` |
  | Eccm | the filter is off: `eccm = None` | the same | `state.rs:2215` |
  | Power | the plant is `Power::Off`: motion and fire refuse (Q-40-4) | the heat ceiling and the dissipation at 500 ‰, read-side (`heat_max()` scaled, the cell untouched); on the surface the rig's recharge at 500 ‰ (host, doc 20 SM.4a's charge) | `heat.rs:82`, `:148`; `excursion.rs` |
  | Relay | the party's orders are held until the field ends or the part is unjammed (Q-40-5) | delivered a provisional 10 s wall late | `field/orders.rs`, host |
  | Core | `core_class` lowered by one while jammed, as `DegradeCore` | lowered by one until repaired | `arena/core.rs`, `damage.rs:1238` |
  | The mount | as GC.2 built: off the fire table, rounds kept | fires as before | `damage.rs:782–789` |

  A jam inside a field lasts the field (the tender refuses `site_in_combat`); a damage lasts to
  the bay. A jammed or damaged core that falls below a mind's sealed class drops the mind to the
  ROM at its next landing, as a destroyed section's `DegradeCore` does today, and it is not
  re-seated when the class returns: a mind comes back at the next check-out.
- **Alternative:** a mount-only kernel with the five others host-side at check-in (the degraded
  sensor range read into the next field's profile, no mid-match effect).
- **Why:** Q-38-27's words, kind by kind; the numbers are one knob each and provisional; the
  alternative is what GC.2 left and what both reviewers called out. Expensive because every row
  folds into the practice hashes once.

**Q-40-4 — A jammed plant.**
- **Proposed default:** the literal ruling: `Power::Off`, so the hull stands and does not fire
  until the field ends — a disabled hull in the kernel's own vocabulary, towable as a casualty,
  not a wreck. The chance is the unit's `jam_permille` (50) on a penetrating hit to the plant's
  section, and the draw picks among that section's parts, so it is rare.
- **Alternative:** a jammed plant takes the heat ceiling to nought (a laser hull cannot fire; the
  rest unchanged) and leaves motion alone.
- **Why:** "jammed is off until unjammed"; a plant that is off is a hull that is off. The
  alternative softens one kind against the ruling's rule for all six.

**Q-40-5 — The relay's effect is the host's.**
- **Proposed default:** only the Halcyon carries a relay (`halcyon.json`), so the effect is on the
  party whose carrier it is: `field/orders.rs` holds a delivered order while the relay is jammed
  and delays it 10 s (provisional) while damaged, before the DT.1b adapter offers it to the kernel;
  the comms side is the carrier's node in doc 15's relay registry marked offline or slow for the
  field's life. No kernel row, no hash; it lands in FR.5 as the same cut.
- **Alternative:** a kernel row, the delay applied to `DetachmentCommand`'s deadline.
- **Why:** orders travel on the IC transport and are applied at a pulse's head by the host; the
  kernel has no clock for "late" but the pulse, and the host already owns the delivery.

**Q-40-6 — What unjam and repair restore, and the check-out.**
- **Proposed default:** nothing new in the restorers: the tender's `field_unjam_component`
  (`engineering.rs:224–262`) restores `intact`, or `damaged` where the health cell is below its
  maximum, and the bay's repair restores `intact`; the kernel's effects are a function of the
  condition word, so a restored part works at the next check-out through the same derivation that
  applies them (`admit_unit`), and `part_works` (`state.rs:1802–1810`) is retired into it.
- **Alternative:** a stored "effect" row on the unit record, written by the bay and read by the
  kernel.
- **Why:** one source of truth, the condition word doc 36 Q-36-38 already made the record's; the
  bay and the tender need no new vocabulary.

**Q-40-7 — The coupled tow in the kernel.** **(expensive to reverse)**
- **Proposed default:** the casualty is admitted as a unit of its tug's party at the nearest free
  cell to the tug (the join's ring rule), with `UnitState.coupled_to = Some(tug)` folded into the
  hash only when present; each substep the tug moves, the casualty is set on the cell the tug left
  (a trailer, one unit a cell kept, `check_substep` honoured), and it never moves on its own; its
  limits are a casualty's (no motion, no fire, its sensors by their condition); it is struck through
  its own sections and facet bands, and never counts against the cap (Q-CF-26). The coupling ends
  only when the tug's hull is destroyed — the casualty then stands where it is, which is the tug's
  last cell — or when the tug releases. **Release inside a field is a field act**: `+tv/<n> release`
  on a promoted site sends `ArenaMsg::Release { tug }`, applied at the next pulse's head, answered
  behind the ledger, and the host retires the tow on the site record in the batch that takes the
  answer, so the surface and the kernel agree; a re-attach inside a field is not ruled and keeps
  waiting (Q-CF-30). At demotion `tow::set_down` reads the casualty's own check-in pose.
- **Alternative:** the casualty shares the tug's cell as a second occupant.
- **Why:** Q-38-17 and Q-36-20 as ruled; a trailer keeps every occupancy invariant the kernel
  has and lets a hit land on the casualty by its own geometry. Expensive because a unit that
  follows another is a new locomotion kind the step, the index, the snapshot and the hash all
  carry.

**Q-40-8 — `line_to` over wrecks: an API bump?**
- **Proposed default:** no bump. `line_to(v, id)`'s name, signature, fee and refusals are unchanged
  (`minds/kernels.rs:190`); only its answer changes, because the kernel now traces the ground the
  fire trace traces. `API_VERSION` stays 7, `MIND_ABI` and every seal digest stand.
- **Alternative:** API 8, as GC.2's note foresaw ("a mind API change with its own seal digests"),
  with the view handed wreck rows.
- **Why:** GC.2's note was about giving the view wreck positions; this does not, it answers the
  question the mind already asks. The practice pins move anyway (§4a row 1); a bump would move
  the seal pins too for no new capability.

**Q-40-9 — `note_fire`: the victim that holds the shooter a friend.**
- **Proposed default:** the victim is moved as today (fire on you is fire on you; the leader's
  Friendly mark cancels it, Q-CF-7); only the third parties whose books hold the shooter a Friend
  are skipped. That is the finding's own wording (doc 36 l.2785–2792).
- **Alternative:** skip the victim too, so friendly fire between standing friends never makes the
  victim hostile.
- **Why:** Q-CF-9 makes fire produce hostility in the victim's book; a standing friend who is
  fired on should see it and decide, and GC.10's void is the standing side of the same event.

**Q-40-10 — The think-batch await before joins, and the retry.**
- **Proposed default:** `await_thinks` moves ahead of the pulse-head applications (§4.1 item 1);
  GC.11's retry of a refused raider join stays as a guard; GC.1b's refused arrival and a refused
  launch join (Q-40-20) are retried at every tick while the field runs, not held to demotion.
- **Alternative:** keep the order and lean on the retry alone.
- **Why:** the refusal is by name, documented, and the common case for a raid's second pulse
  (GC.11); a join built on landed answers never meets it.

### 5.2 Stances

**Q-40-11 — The table's keys, its declarers, its verbs.**
- **Proposed default:** keyed by `Affiliation` (a mush-org or a faction) to `Affiliation`, one
  row a direction, `Stance::{Allied, Neutral, Hostile}`, in `FactionState` under
  `mod:faction:stance:<from>:<to>`, write-through like every faction mutation. Declared for a
  faction by one of its principals or a member at or above `officer_level`; for a mush-org by a
  member holding a new `org.stance.declare` capability granted through `@org/grant`; staff
  always. `+faction/stance <mine>=<other>/<allied|neutral|hostile>` for players,
  `@faction/stance` for staff, the rows shown by `+faction <name>` to anyone who may know the
  faction (hidden ones never listed). A declaration is a notice to the other side's officers.
- **Alternative:** staff-only declarations now (`@faction/stance`), the player verb later.
- **Why:** "declared by an organization's owner" has no owner field to read in either crate, and
  the two authority models the crates already have (officers and principals; capability grants)
  are the nearest thing. Keying by `Affiliation` lets one table serve a raider's faction and a
  player's co-op alike.

**Q-40-12 — Directional, or mutual.**
- **Proposed default:** Hostile is directional: the declaring side's parties open with a Hostile
  mark on the other's (the viewer's own belief, as every relation is per viewer, Q-CF-5). Allied
  counts only when both declare it: a mutual pair becomes a friend pair; a one-sided Allied has no
  field effect and `+faction` shows it "unreciprocated". Neutral clears the row.
- **Alternative:** Allied directional too, through a one-way `befriend` in the kernel (`party.rs:324`
  is both ways; a kernel file with no OOC hash move, since OOC fields have no friends).
- **Why:** `FieldHost.friends` is a symmetric pair and a one-sided friendship would make the
  undeclared side's colours lie; a Hostile mark is already private and one-sided by Q-CF-7.

**Q-40-13 — A stance never opens a field.**
- **Proposed default:** a Hostile stance promotes nothing by itself; a site with two such parties
  stays open ground until a leader marks or fires through Q-CF-14's doors, and admission then
  reads the stance. Doctrine never opens a field (`raiders.rs`'s rule).
- **Alternative:** two parties with a declared Hostile between them on one site promote it at the
  next tick.
- **Why:** a declaration is an organization's standing word, not a shot; opening fields by table
  would let a declaration lock bays across every site the two share, which Q-CF-17's reasoning
  refused once.

**Q-40-14 — Standing under a declared Hostile.**
- **Proposed default:** unchanged: fire costs the shooter standing with the victim's factions
  whatever was declared; the void's friendly-fire test counts a mutual Allied pair as it counts
  shared membership.
- **Alternative:** fire on a declared Hostile organization costs no standing.
- **Why:** standing is the victim's faction's view of the shooter (GC.10), and a declared enemy
  is not made fonder by being shot at; the Allied case is the one GC.10's void was written for.

### 5.3 Durable standing

**Q-40-15 — The tally on the live record, at the snapshot cadence.**
- **Proposed default:** `FieldLive.fire: Vec<FireRow>` (`#[serde(default)]`, format 1 kept),
  counted from the chunks as today and written in the snapshot's batch (`field/mod.rs:352`), so a
  resumed field's tally is exactly the rounds it has seen; `FireTally` goes.
- **Alternative:** written in every chunk's batch.
- **Why:** a resume rewinds to the snapshot and re-fights what followed, recounting it; a write a
  pulse would buy nothing the resume could keep.

**Q-40-16 — Where the offer rides.**
- **Proposed default:** on the grant: `record_voidable_standing` takes the window
  (`opened_at` wall, `until_ic`) with `voidable_by`, written once; `void_standing` refuses past it;
  `VoidOffers` and the `tacsim.field.standing` tick go; `+field/void` voids every open grant naming
  the actor (`standing_causes`, prefix `field:`). A later demotion writes its own grants with their
  own window rather than widening an old one.
- **Alternative:** the offer on the closing tombstone (`FieldClosing`), removed when the close lands.
- **Why:** the ruling's "live field record" fits the tally; an offer opens at demotion, when the
  live record is gone and the tombstone is seconds from going, and the grant is the one record that
  already names the voider and outlives both.

**Q-40-17 — The synchronous close, and the window's clock.**
- **Proposed default:** `SessionStat` keeps `prev_input` beside `last_input` (mush-core, one
  field); `+field/void` closes an offer when `prev_input > opened_at` — anything entered since the
  offer opened, other than this void. The window stays 600 IC seconds as ruled, with the
  observation that at rate 24 it is twenty-five wall seconds, and the owner may prefer a wall
  window.
- **Alternative:** a before-dispatch hook in mush-core's command path that `tacsim` registers to
  close offers; or a wall-clock window (60 s) in place of the IC one.
- **Why:** the ruling asks for the command path, not the sweep; a second stamp is the smallest
  seam and needs no hook; the one-second ambiguity the sweep's doc names stays wherever the stamps
  are seconds.

**Q-40-18 — The director's auto-void, and staff raiders.**
- **Proposed default:** at demotion the host voids, as the raider's leader and with its own
  reason, every grant whose voider is that leader, when the raider's carrier carries the director's
  `raid` key; a staff-seeded raider's leader is offered nothing, as today.
- **Alternative:** every NPC leader auto-voids.
- **Why:** Q-38-24 names the director's raids; a staff raid is a staff hand's, and nothing today
  decides for an NPC a hand is driving.

### 5.4 Reinforcement

**Q-40-19 — The launch rule's shape.**
- **Proposed default:** §4.4's rule: the act on the IC clock as today, the hull offered as a join
  at the ramp at the act's landing, one hull a carrier a `launch_cycle_s` (60 s wall, provisional,
  banded 10–600), a multi-slot act putting its hulls down one a cycle in slot order, a second act
  inside the cycle refused with the seconds left; the cap at the prepare and at the join; no hold
  for a bay under fire beyond GC.3's table.
- **Alternative:** one act a cycle with every slot down at once; or a "ramp under fire" hold for
  N seconds after a hit on the carrier.
- **Why:** "a bay cannot pour" is a rate on hulls, and one a minute into a ten-minute fight is the
  bound Q-36-5's alternative was rejected for lacking; a hold on fire would make the first shot at
  a carrier a bay lock, which Q-CF-17's amendment refused.

**Q-40-20 — A refused launch join.**
- **Proposed default:** the hull goes back aboard and is offered again at the next tick until
  taken or the field demotes (the director's pattern); GC.1b's refused arrival takes the same
  retry.
- **Alternative:** a refused launch is told and the player launches again.
- **Why:** Q-CF-21 forbids a hull on the ground and not in the field; a hull that is back aboard
  is on neither, and the retry is code the raid already runs.

### 5.5 Segments, the seam, acceptance

**Q-40-21 — The segment rolls in place, not only on resume.**
- **Proposed default:** at the snapshot cadence, once `bytes_total` ≥ three quarters of
  `max_bytes` (48 MiB, provisional), the task rolls: the period's snapshot is the segment's head,
  `bytes_total` → 0, `FieldLive.segment += 1`, one `segment` line; a resume rolls from the snapshot
  it rewinds to; `seq`, `digest` and `shipped` continue; `max_bytes` unchanged; the world keeps the
  latest snapshot and every chunk.
- **Alternative:** the ruling's literal: reset on resume only.
- **Why:** the ruling's reason is long fields, and a field that never restarts never resumes;
  without the live roll the eighteen minutes stand for exactly the fields reinforcement makes.

**Q-40-22 — The `SpacePartyId` seam.** **(expensive to reverse)**
- **Proposed default:** delete `SpacePartyId`; `PartyId` in its five files and the voyage test;
  `SpaceRelation` onto `Relation` and `Mark`, with `Inspecting` a bubble-side flag on the picket's
  party; the stance table feeds the bubble's opening marks as it feeds `FieldHost`.
- **Alternative:** keep `SpacePartyId` as a newtype over `PartyId` and map at the edges.
- **Why:** Q-36-48 as ruled; two party vocabularies would give the stance table two readers to
  keep in step. Expensive because 57 sites move and doc 35b's party assembly builds on the result.

**Q-40-23 — The bot's first cut is FR.0's.**
- **Proposed default:** FR.0 carries the four-digit parser (`mine/lines.rs:343–353`, `mine/lane.rs:320`)
  and this round's words (`+field/void`, a launch into a field, `+tv/<n> release` in a field,
  `+faction/stance`), since doc 39's list leaves the parser untaken; FR.7 runs the round's story
  over SSH with the bot on conclav.dev.
- **Alternative:** doc 39's IN.10 takes the parser, FR.0 only the words.
- **Why:** Q-38-29's rule is "the first cut of whichever document starts first", doc 39 started
  first and did not take it; a cut that exists is better than a rule that waits.

## 6. The plan: cuts FR.0–FR.7

The rulings of 2026-11-03 make this the implementation order. Each cut is one PR, tested through the production command paths
(`+faction/stance`, `+field/mark`, `+field/fire`, `+field/void`, `+sortie/launch`,
`+tv/<n> tow|release`, `+standing`), never a staff hand past the seams doc 36a §8 allows; each
cut with a ground in it runs its story at 256 (Ahuna Flank) and 512 (Urvara Reach) as one script
over `field_story.rs`'s two `Ground`s; each cut's note goes to `40-cut-notes.md`. One kernel
rebless, FR.5, and no other cut moves a practice hash (FR.4 proves it by FR.0's inventory).
**Dependencies:** FR.0 ∥ FR.1 ∥ FR.2 ∥ FR.6 may start now; FR.3 after FR.1; FR.4 after FR.0; FR.5
after FR.0, FR.3 and FR.4; FR.7 last. Two cuts carry the marker and are built exactly as ruled
(FR.5's component effects and coupled tow; FR.6's seam).

| Cut | Rulings | What | Acceptance (the story, at both sizes) |
|---|---|---|---|
| **FR.0** the inventory and the bot's words | Q-38-29, Q-40-23, doc 38 §8 | §4a's rows run at the base commit and recorded; the four-digit parser; the bot's `+field/void`, launch, `release` and `+faction/stance` words; `field_story.rs`'s two grounds hoisted to `tests/support/field` for every later story | the recorded inventory in `40-cut-notes.md`; the bot drives to a four-digit pocket on Urvara and types `+field/void` in a smoke |
| **FR.1** stances | Q-38-23, Q-40-11..14 | the table in `FactionState` and its key; `+faction/stance`, `@faction/stance`, `+faction`'s rows; the `org.stance.declare` capability; `parties.rs` reading it into `FieldHost.marks` and `.friends` at admission and in `carrier::joining` at a join; `friends_of` counting a mutual Allied pair | two co-ops with a declared Hostile meet on Ahuna and on Urvara; a `+field/mark` by one promotes; each presence picture colours the other hostile from the first pulse; a third co-op mutually Allied to the first is Friend and its one-sided Allied to the second is not; a stance changed mid-field changes nothing until the next field, which reads it; an arriving org joins under the declared Hostile in the existing party's book |
| **FR.2** durable standing | Q-38-24, Q-40-15..18 | `FieldLive.fire` at the snapshot cadence; the window on the grant, `VoidOffers` and the sweep retired; `prev_input` in mush-core and the synchronous close in `+field/void`; the director's auto-void at demotion | GC.10's void gate re-run at both sizes with a restart mid-field (the tally survives), a void after another command refused at once, a void with no command between accepted; a director raid's friendly fire on a Syndicate member voided with no hand |
| **FR.3** reinforcement | Q-38-25, Q-38-26, Q-40-19, Q-40-20 | `FIELD_LAUNCH` replaced by the cap check and the cycle; the launch's join at the ramp with the director's spread; `launch_cycle_s`; the refused join back aboard and retried; GC.1b's arrival retried | the escort story with a second escort launched by `+sortie/launch` after the first shot, standing beside the ramp at the next pulse and counted; a second launch inside the cycle refused with the seconds left; a launch past the cap refused at the prepare; an F-6 arrival that joins and then launches |
| **FR.4** segments | Q-38-28, Q-40-21 | `Ledger::roll_segment`, the task's threshold at the snapshot cadence, the resume's roll, the `segment` line, `FieldLive.segment` in the host's batch | a field driven past the threshold under a test knob (`max_bytes` lowered) rolls in its first minute, is restarted, resumes from the segment's head, and the world holds every chunk of both segments under one match id; the OOC pins of §4a unmoved |
| **FR.5** the rebless | Q-38-17, Q-38-27, Q-36-56, Q-40-2..10 | §4.1's five items in its order: the await, `note_fire`, `line_to`, the effects (the compiled table, the resolution pack's block, the derivation at admission and after a part change), the coupled tow (the trailer, `Release` as a field act, the drop); §4a's pins re-pinned once with reasons | the in-source kernel gates (one a row of Q-40-3's table; a trailer struck and released; a mind that no longer fires through a wreck); story (a) re-run at both sizes with the casualty struck while towed, released in the field by `+tv/<n> release`, and dropped where it stands by a tug that fell; `field_gc10`'s gate demoting on `disengage_s`; `field_scale` re-measured at 100 |
| **FR.6** the seam | Q-36-48, Q-40-22 | `SpacePartyId` deleted, `PartyId` in `space/*` and `shard_10/voyage.rs`, `SpaceRelation` onto `Relation`+`Mark` with the picket's flag | the voyage test's parties lettered by the arena; a compile with no `SpacePartyId` in the tree; no persisted document moved |
| **FR.7** the round's story | Q-38-29 | one script: a declared-Hostile co-op's raid on a mining field, reinforcement from the pad, a tow struck and released under fire, a jammed sensor, friendly fire voided at once, a segment rolled; at 256 and 512; the bot typing the player's half over SSH on conclav.dev | the script green at both sizes on gate.yml; the SSH run recorded |

## 7. What the first cuts need

Content and schema the cuts author as they land, listed once: the stance row's persisted shape
and its receipt-less write (FR.1); the `org.stance.declare` capability's name in mush-org's
built-in list (FR.1); `prev_input` on `SessionStat` (FR.2, mush-core); `launch_cycle_s` in
`[tacsim.field]` with its band (FR.3); the `segment` ledger line's kind (FR.4); `ComponentEffects`'
compiled table and the resolution pack's optional block (FR.5); `ArenaMsg::Release` and
`ArenaOut::Released` (FR.5); the two `Ground`s hoisted into `tests/support/field` (FR.0). For
doc 41: the tender's unjam between fields is what makes a jammed part worth carrying home, and
doc 39's part lot is what a damaged one costs; neither gates a cut here.

**The questions this draft had least confidence in**, now ruled as written: Q-40-4 (a jammed
plant is the whole hull off, the literal reading), Q-40-8 (a changed `line_to` answer is no API
bump), Q-40-11 (a faction's principals and officers, a mush-org's `org.stance.declare` holders),
Q-40-12 (Allied mutual, Hostile directional), Q-40-17 (the window stays 600 IC seconds, twenty-five
wall seconds at rate 24, closed at the next command), Q-40-21 (the live segment roll beside the
resume's), and Q-40-23 (the four-digit parser is FR.0's). Each is built as its default says, and a
cut that finds its default does not hold at the tree records the departure in `40-cut-notes.md`
for the owner, as the GC cuts did.
