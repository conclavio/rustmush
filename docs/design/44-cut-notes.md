# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Not built.

## MW.1

Not built.

## MW.2

Not built.

## MW.3

Not built.

## MW.4

Not built.

## MW.5

Not built.

## MW.6

The Registry's pace as a file key, and the two notice drains that stopped on one bad holding
(doc 44 §7.1, Q-44-41; second review, §4c rows 4 and 10). Built on Q-44-41's default, which is
open: the key exists with today's value as its default, and a dev box sets it low.

**Built.**

- **`[tacsim.minds].registry_pulse_ms`.** `mush_config::TacsimMindsCfg.bulk_maintenance_ms` was
  `serde(skip)`, so no file could set the pace of a Registry lease; only the acceptance stories'
  harness could, in code. It is now a file key under the name `registry_pulse_ms` (the Rust
  field keeps its name and is not a key). Absent, it is 1,000 ms, production's second, as doc
  43c records was decided. `MindsConfig::validate` (`crates/tacsim/src/config.rs`) refuses a
  value outside 10 to 60,000 by the key's name, ahead of the bulk model's own check. The key is
  documented, commented, in `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl`, and in
  the key table at the head of `crates/mush-config/src/config.rs`. `tacsim::MindsConfig` itself
  still skips the field in its own serde; the host's mapping (`mush_server::config_ext`) is the
  path a file takes.
- **Both notice drains skip one bad holding.** In `crates/mush-roll/src/commerce/registry_recall/notice.rs`
  and `registry_revocation/notice.rs`, `tick` returned the first error from `own` before the
  cursors were written back, so the next tick read the same page, met the same holding and
  stopped again: every notice behind it waited for ever. Now an error that is one holding's own
  (`one_holding`: `Missing`, `Mismatch`, `Invalid`) skips that holding, emits nothing for it,
  logs it once (`Runtime::note_skipped`, bounded like the remembered notices) and lets the
  cursor pass. An error that is the inventory's (`Unavailable`, `Stale`, `Capacity`) still stops
  the drain where it is, before anything is emitted, to be tried again. `page` had the same
  fault one level up: a row that failed `validate` failed the whole page. Such a row is now
  carried with no recipient, so the cursor passes it.

**Tests.** `mush-config`: `m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo` gains
the key (absent is 1,000; `registry_pulse_ms = 50` parses; `bulk_maintenance_ms` is an unknown
field). `mush-server`: `config_ext`'s refusal table gains 5 and 60,001, each refused by the key's
name, and the mapper carries 50 through to the layer's config. `mush-roll`: one test in each
notice module, `a_bad_holding_is_skipped_and_an_unavailable_inventory_is_not`. Local: `cargo test
-p mush-config` 141 + 14 passed; `cargo test -p mush-roll notice` 3 passed; `cargo test -p
mush-server --lib config_ext` 18 passed; `cargo test -p mush-contract-tests` green; clippy with
warnings denied on mush-roll, mush-config, tacsim and mush-server.

**Divergences.** The unit's row asks for "the measured time a rating takes at both paces". It
was not measured in this cut. The figures on file are `ci.yml`'s (about 1,070 leases a
certificate, about three hours a story at 1,000 ms) and the acceptance stories' (minutes at
their 50 ms). MW.1's bring-up is where a dev box's rating is first timed.

**Not built.** Registry pacing beyond the pulse: a lease still takes 3 to 7 s of work at the
stock pace, and nothing here shortens the work a lease does. The key is not set in
`deploy/dev1` or `deploy/dev2`; MW.1 sets it when it renders the launch.

**Not pinned by a story.** The drains end to end: no rig corrupts one holding and shows the
holder behind it getting their notice. The tests pin which errors skip and which stop, and that
the skipped keys stay bounded. A row that fails `validate` in `page` is not pinned at all.

## MW.7

Not built.

## MW.8

Not built.

## MW.9

Not built.

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

## MW.14a

The compiled NPC commander (doc 44 §7.1, §4 D, §4c rows 14 and 15; Q-44-20, Q-44-44; doc 27
FO-15, Q-D4, Q-F1). Built on Q-44-44's default: a host-side issuer, compiled and deterministic,
with no LLM and no change to the mind ABI.

**Built.**

- **The rule, in one file** (`crates/tacsim/src/detachment/commander.rs`). `View` is all a
  commander decides on: what its own frame holds (`Sight`: the live contacts its party's book
  holds Hostile), the state of its roster (`Strength`: hulls, and those lost in this field) and
  the detachment's work point. `decide` is the whole rule, first match wins: **withdraw** once
  the roster has lost a hull (`WITHDRAW_AT_LOSSES`, 1), **hold** while a Hostile is in sight,
  **advance** to the work point otherwise. `Posture` is what it may ask for. The types are plain
  data in and plain data out, with no handle on the world, the clock or the dice, which is the
  shape MW.14b's doctrine and a House's policy doctrine (Q-44-32) take.
- **The issuer** (`crates/tacsim/src/detachment/sortie/commander.rs`, called from the NPC walk
  in `sortie/npc.rs`). When a roster's field command names an NPC its leader and that NPC's unit
  stands in a running field, `step` builds the view, takes the posture and gives it as a roster
  order to the first hull that does not yet have it. It goes through a player leader's delivery
  and nothing beside it: submitted as the NPC, fenced by the roster's revision and term
  (`field_orders::Act::Issue`); Org's mission rights read for the NPC over each unit; a
  `mush_propagation` envelope to the recipient, which interference holds; the recipient's own
  executor and the kernel's own checks (`field/orders.rs`, `arena/detachment.rs`, both
  unchanged). The order book is the commander's only memory, so a restart derives the same next
  order. The one thing kept in RAM is scheduling: an order just submitted to a slot is not
  submitted again for `RESEND_AFTER_S` (60 IC seconds), so one refused at its final take is
  retried at that pace and the commander's own hull flies its order meanwhile.
- **What the commander sees** (`crates/tacsim/src/field/orders.rs`): `sight` reads the Hostile
  contacts off the last frame the NPC's own seat was sent; `lost` says a unit is lost in its
  field when its operator's own frame shows a wreck or the field's kernel has named it
  destroyed.
- **A lost hull stops leading at once** (`sortie/field.rs`, `reconcile`): the roster's command
  facts read `lost` beside the world's unit record, which is checked out and stale while a field
  runs. Before this a commander destroyed in a field stayed the roster's leader until the
  field's close, and the kernel refused every order it gave (`NoCommander`). The fact is the
  roster's, so it holds for a player leader too: a player whose hull is destroyed in a field
  stops leading, and the next eligible player or the NPC commander leads.
- **The successor** (`crates/mush-tactics/src/detachment/roster.rs`, `reconcile`, and
  `roster/checkpoint.rs`, `restore`): while the nominated commander's own unit is lost or
  disabled, the first other NPC in slot order that is healthy and authorized leads under a new
  term, and the commander leads again once its unit is not. A commander who is only absent
  (still aboard in ramp transit, out of its seat) or only lacks the command right is not passed
  over; a roster that nominated no commander never elects an NPC; an active player outranks
  both.
- **Yielding to a returning player** needed no code: the roster's own rule (doc 27 Q-D4, the
  return hysteresis) makes the player the leader, `step` finds the NPC is not, and what the NPC
  had in flight under its term is discarded by the poll as any deposed leader's is.
- `docs/help/detachments.md` says what an NPC leader does. Doc 27's Q-D4 carries the successor
  as an amendment.
- **`roster_rig`** grew what the stories needed: `fly` takes a roster of two hired NPCs
  (below); `grant_command` issues the mission-role grants `+sortie/command-grants` prints, each
  under the organiser's own command loan; `orders_of` reads the roster's order book.

**Tests.** In-source: the rule's three arms and their order, and the same view giving the same
posture (`detachment::commander`); each posture as the roster order a player types
(`detachment::sortie::commander`); the successor, its restore, and a player outranking it
(`mush_tactics::detachment::roster`, two tests). Stories in `domain_field`
(`crates/tacsim/tests/domain_field/field_mw14a.rs`), each passing alone on Windows:

- `mw14a_an_npc_led_detachment_advances_holds_and_withdraws` (74 s) and `…_at_512` (65 s). A
  captain hires Sana Okonkwo and Ilya Mendes, names Okonkwo the commander and launches both
  from aboard the carrier; no player is in the roster's units. An escort and an intruder stand
  on the ground and the escort's fire order promotes the site. With nobody typing an order:
  Okonkwo orders both hulls to the work point and both drive for it; staff jam the roster's
  comms and the intruder fires on the roster's carrier, so her HOLD is admitted and sent and
  does not arrive, and both hulls keep their ADVANCE; the jam is cleared and both hulls stand;
  the intruder destroys Okonkwo's rig, and Mendes leads under a new term, orders the withdrawal
  from his own unit and drives for the ramp.
- `mw14a_a_returning_player_takes_the_lead` (27 s). A captain leads from rig 1 with Okonkwo,
  the plan's commander, in rig 2. While the captain leads she gives no order. The captain's
  session ends: she leads and orders the advance, to her own hull only. The captain returns:
  after the hysteresis the lead is theirs, their `ORDER DT:… 2 HOLD` is delivered and obeyed,
  and she gives no order under their term.
- `mw14a_the_same_run_gives_the_same_orders_twice` (57 s). The script to the HOLD on two fresh
  worlds delivers the same orders, with the same sequences, from the same slot.

**Divergences and defaults.** None needs a refactor to undo; each is one function or one filter.

- **The work point is a default.** A plan names no objective, so the commander advances to the
  site cell `WORK_POINT_CELLS` (6) out from the flight's ramp, straight away from the pad. A real
  objective is a field on the plan, which is a plan-format change and was not made.
- **An NPC-led roster holds only when it is fired on.** Only a roster's current leader may mark
  a party Hostile (`field/parties.rs`, `current_leader`), and an NPC marks nobody, so its book
  holds a party Hostile once that party's fire arrives on the roster or on a friend. The story's
  intruder fires on the roster's carrier for that.
- **"The next NPC in the roster's order"** is the first other NPC by slot number, not the next
  after the commander's slot. Its contract scope is not checked: Mendes, hired as a subordinate
  operator, succeeds Okonkwo. That amends doc 27 Q-F1's "explicitly assigned" for the one case
  of a commander who cannot command, as Q-44-44 ruled.
- **Succession is on the commander's unit, lost or disabled**, and nothing else. A commander
  who is absent for another reason is not succeeded and the roster has no NPC leader, as before;
  otherwise the successor would lead for the seconds the commander spends on the ramp at every
  launch. The lead comes back when the commander's unit is whole again.
- **One order a slot, never `ALL`.** An `ALL` is refused whole when any slot is unavailable, and
  a principal acts once a fire, so a posture reaches `n` hulls over `n` world seconds.
- **Only inside a running field.** On open ground an NPC leader gives nothing, and the surface
  executor goes on running whatever was delivered.
- **The link loss in the returning-player story is the session's facts set by hand**
  (`set_connected`, `set_live_conns`), as `mush-server`'s leadership story sets them. What the
  field does with an absent pilot's hull is MW.7's and is not exercised.
- **The jam is staff's** (`@prop/jam <place>=4` at the crew's place). No player-held jammer
  reaches the IC transport.
- **The stories run at 1:1**, the field rigs' rate, as every field story does. The one IC window
  in them is the roster's three-IC-second return hysteresis, waited out in full; at 24:1 it is
  an eighth of a second.
- **A Dray-P berths two, and the acceptance needs three people aboard.** The organiser and two
  hired NPCs fit only because a person in a rig's cab holds no berth: the organiser boards,
  takes rig 1's cab, the crew board, the plan is drafted and activated, slot 2's NPC is seated,
  the organiser steps out (`+tv/1 leave`) and slot 1's NPC is seated, all at Ceres before the
  hull sails. Every step is a player's command, and `roster_rig::fly` pins the sequence. **For
  the owner:** no help page says so, and if a cab should count against the berths then no
  shipped hull can fly a two-NPC detachment with its organiser aboard.
- **A lessee's mission-role grant needs its parent.** `+sortie/command-grants` prints a draft
  with `"parent": null`, which Org refuses from a lessee; the rig adds the lessee's own command
  loan. The help page already says a lender supplies its parent, and the printed line does not.

**Not built.**

- MW.14b, the Rhai commander.
- **The organiser cannot read what their NPC commander ordered.** `+sortie/orders` shows the
  reader's own outbox and an NPC's orders are in nobody's but its own, so the stories read the
  roster's order book as typed state.
- Any posture but the three: the commander gives no ENGAGE, SCREEN, SUPPORT or MINE, holds no
  fire, and marks no party.
- Damping. A Hostile that drifts in and out of the commander's sight flips the posture between
  HOLD and ADVANCE as often as once a world second.
- A commander's orders on open ground, and a plan objective.
- A second order to a hull that answered NEGATIVE: the delivered order is the posture, so it is
  not sent again.

**Not pinned by a story.** A restart with an NPC leading. The commander taking the lead back
from its successor (the roster's unit test has it). The ADVANCE returning once the Hostile is out
of sight. The withdrawing rig boarding its carrier (the story ends with it driving for the ramp).
A successor refused for want of the command right (unit test only). The commander's own orders
answered NEGATIVE.

## MW.14b

Not built.

## MW.15

The story doc 36 owed for SCREEN and SUPPORT flown as delivered orders (doc 44 §7.1; doc 36 cut
GC.0's "Not built"; Q-36-42). No engine code changed: the cut is a test rig and a story.

**Built.**

- **`roster_rig`** (`crates/tacsim/tests/support/field/roster_rig.rs`), the rig that flies a
  detachment's roster into a field in tacsim's own world harness. `fly` is `mush-server`'s
  `pair_field_fixture` on `driving_rig`'s composed world, every step the production command:
  the hire at the Slack Line (`+sortie/crew`, `+sortie/hire`), a leased prospector a slot with
  its operational loans (`+tv/lease go`, `+tv/<n>/permissions go`), the crew brought aboard
  (`+sortie/board`), the voyage to the site, the sub-loan to each NPC
  (`@unitgrant/issue … operate_crew parent=<loan>`), the leader's own boarding
  (`+tv/<n> board`, `+focus`), the plan (`+sortie/draft`, `+sortie/accept`, `@station/handoff`,
  `+sortie/ready`, `+sortie/activate`), each NPC's seat (`+sortie/seat`), the launch
  (`+sortie/launch`) and the refresh until the field command names a leader.
- Its parts are what MW.14a's story needs next: `Plan` takes any billets (a `Leader`, a `Hired`
  crew member by name) and a `commander` by name; `commit` lands whatever a line queued on the
  composed path whoever owns the action (tacsim's, Org's unit grants, Sim's helm hand-off, which
  `driving_rig::drain` drops); `apply_due` lands a due scheduled action on the composed scheduled
  path (Sim's `control_handoff_apply`, the lander doc 36 said such a story needs); `beat` is one
  world second of both; `fence` and `reply` read the order prefill and a sent order's status off
  `+sortie/orders`, as a player's client does.
- **The story** (`crates/tacsim/tests/domain_field/field_mw15.rs`), at 256 on Ahuna Flank and at
  512 on Urvara Reach. A captain leads a mining roster from rig 1 with Ilya Mendes hired into rig
  2. A second captain's armed hull escorts and a third party's armed hull stands off it; the
  escort's `+field/fire` promotes the site and three parties are admitted. Then, by the leader's
  own `ORDER DT:… ` lines:
  1. `2 SCREEN 90`: the order is delivered, stands on Mendes's own frame, the rig turns onto 90
     degrees and the WILCO comes back. The leader drives four cells off and the screen stays on
     its cell and its bearing;
  2. `2 SUPPORT 1`: the rig leaves the cell it screened from, closes to its station on the
     leader's rig, and answers WILCO;
  3. `2 SCREEN 270`: the rig turns onto the new bearing, so the bearing it holds is the order's;
  4. `2 SUPPORT 2` (its own slot): NEGATIVE, and the screen goes on standing.

**Tests.** `field_mw15::mw15_a_rosters_screen_and_support_are_delivered_and_obeyed` and
`…_at_512` in `domain_field`. Local, Windows: 85 s and 81 s, each passing alone. GC.0's two
in-source gates are untouched.

**Divergences.**

- **One NPC flies both orders, in turn.** A Dray-P's bay holds two rigs, so a roster with a
  player leader has one hired NPC (`fly` refuses a third billet). The row's "the screening unit"
  and "the supporting unit" are the same rig under two successive orders, with the screen's hold
  checked while the leader moves away and the supporter's close checked after it.
- **"Keeps to its charge" is the doctrine's station, with the motor's tolerance.** The supporter
  ends within `SUPPORT_STATION_CELLS` (5) of its own plot of the charge plus the kernel motor's
  arrival tolerance; the story allows two cells for that.
- **The escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on` and
  `seat`), not the content's raider on its doctrine, so nobody fires after the opening order and
  the story does not hang on the field's dice. Three parties stand in the field; only the roster
  is under test.
- **The rigs are stood on their cells by a staff hand** after the launch, as every field story
  places its units.

**Not built.** A SUPPORT whose charge is another NPC's rig (it needs a third bay), and SCREEN or
SUPPORT under fire with a Hostile in the screen's arc: the story is about delivery and obedience,
not the stand the doctrine takes. A restart with an order standing.

**Not pinned by a story.** The jammed and the damaged carrier relay holding these two orders back
(FR.5's stories pin that for the orders they fly). A SUPPORT refused for a charge in another party
or outside the field, which GC.0's in-source gate covers and this story covers only for the
recipient's own slot.

## MW.16

Not built.

## MW.17

The last decision in the staff view of an NPC (doc 44 §7.1, Q-44-30; first review, finding 8;
second review, §4c row 19).

**Built.** In `crates/mush-npc`:

- **A turn records what woke it.** `NpcHostState::begin_turn` (`runtime.rs`) drains the mailbox,
  so it is the one place that knows; it now writes a `LastDecision` on the NPC's runtime: the
  correlation, when it began, and for the first `LAST_DECISION_WOKE_MAX` (8) stimuli of the batch
  their kind (`Stimulus::kind_name`) and whom each was about. `fire_dialogue` (`module.rs`) adds
  how many public facts and recalled memories the snapshot showed the model
  (`note_decision_context`); guarded and secret facts never cross the async boundary, so the
  count is of public ones.
- **A completion records what the turn came to.** `NpcCompletion::on_complete` writes a
  `DecisionResult`: the model answered with these actions (their tool names), the model did
  nothing, the profile's fallback line with the staff-facing reason (capped at 160 characters),
  or the payload did not decode. A completion for another turn changes nothing
  (`finish_decision`).
- **`@agent/show` prints it**, in two lines after the runtime line (`LastDecision::lines`), and
  `docs/help/npc.md` describes them. The verb is Builder and up, as before.

The record holds kinds, ids and counts. It holds no prompt, no reply and no stimulus text, in
keeping with the audit's rule (`docs/help/llm.md`).

**Tests.** `runtime::tests::mw17_the_last_decision_records_what_woke_a_turn_and_how_it_ended`
(the record, the cap, another turn's completion, the two lines carrying no text) and
`mw17_staff_read_what_woke_a_turn_and_whether_the_model_or_the_fallback_spoke`
(`crates/mush-npc/tests/agents.rs`): through `@agent/show`, a turn in flight, the same turn
answered by the model, the next turn starved of budget and answered by the fallback, and a
player refused the verb. No test makes a provider call. Local: `cargo test -p mush-npc` 150 +
72 + 34 passed; clippy clean with warnings denied.

**Divergences.**

- **The NPC decision call still carries no NPC.** The row asks for `decision_llm.rs` to pass its
  NPC so the per-NPC budget and `@llm/kill <npc>` reach a decision. It cannot be done at that
  call site: the worker lease it is handed (`mush_core::decision::DecisionWorkerLease`) is
  public-safe by design, "no world or private witness reference can cross the async boundary",
  and names a controller and a decision, not an object. Carrying an audit attribution on the
  lease is a change to that contract in `mush-core` and belongs to its own cut.
- **"The knowledge tiers the NPC was allowed"** is reported as a count of public facts and of
  recalled memories. The snapshot carries no other tier, so there is nothing else to report.

**Not built.** The record does not survive a restart (it is runtime state, like the mailbox).
It keeps one turn, not a history. It does not cover a summarise turn or a scene-turn decision
(`npc_decision`), only dialogue turns.

**Not pinned by a story.** A completion whose payload does not decode (the `Unreadable`
result) through `@agent/show`.

## MW.18

Not built.

## MW.19

Not built.

## MW.20

Not built.

## MW.21

Not built.

## MW.22

Not built.

## MW.23

Not built.

## MW.24

Not built.

## MW.17b

Not built.
