# 27b. Detachments in field combat — implementation plan

Draft 0.1, 2026-10-30. Planned against `origin/main` at `2b2930c9`.
This document plans work; it does not claim the live combat integration has shipped.

Policy comes from [doc 27](27-ic-field-operations-design-and-survey.md), especially
FO-1–FO-3, FO-10–FO-12, Q-H1–Q-H12, Q-F4 and Q-R3. The owner accepted the
seven-step goal below on 2026-10-30. [Doc 27a](27a-field-unit-survival-and-towing.md)
catalogues existing field-operations foundations; this plan connects the relevant
parts to the production detachment, surface, cockpit and persistence paths.

## 1. Goal and stopping point

**Take a real detachment into a frontier skirmish and bring back the survivors,
with the losses still there.**

A captain and a hired NPC wingman deploy actual armed units, encounter an opposing
force, fight through ordinary controls and delivered orders, and withdraw with
real damage and ammunition expenditure. Disabled units remain at their actual
field positions. Their occupants have a defined way back to play. Restart does
not heal, replace or delete anything.

The first player-controlled field duel is an intermediate playable checkpoint.
The milestone is complete only when the hired-NPC, casualty, crew-survival and
restart paths also satisfy the seven-step story. A helper demonstration or an OOC
arena match is not completion.

The scenario should be useful as the combat part of a later mining escort or
recovery expedition. Concurrent escort economics and a full mission generator
are not prerequisites for this first skirmish.

## 2. Seven-step success story

1. **Prepare.** At port, the captain obtains armed units through a supported
   purchase, rental or lending path and hires a real NPC whose contract permits
   combat. Both board normally, acquire their actual stations and operating
   permissions, accept/ready the player assignment, and activate the roster.
   Current carrier containment establishes personnel eligibility at preparation
   and launch. Sail to a designated frontier site and launch through DT.3.
2. **Encounter.** Another physically deployed force occupies the same ground.
   Sensors produce observer-specific contacts; arriving with a different faction
   alone does not start a match. A hostile actionable encounter or attack admits
   combat at the participants' actual positions on the existing terrain. There
   is no lobby, stock-unit substitution or teleport to starting pads.
3. **Command.** The captain moves, observes, targets and fires through the existing
   cockpit controls. An `ENGAGE` order reaches the NPC wingman through the normal
   IC communications/ECM path. The wingman resolves targets using its own sensors
   and permission, attempts the order through its actual helm/weapons, and sends
   its reply through the same transport. Lost contact remains uncertainty. A
   player recipient retains control regardless of an order or WILCO response.
4. **Take losses.** Real shots consume ammunition and cause localized component
   damage. Exercise an engine mission kill and a cockpit/head defeat in separate
   variants. Surviving weapons, other sections and cargo remain identifiable;
   jammed, damaged, destroyed and missing components stay distinct. A disabled
   unit remains physical and targetable, and later hits can cause further damage.
5. **Withdraw.** A mobile survivor disengages through reachable ground, returns to
   the actual ramp and recovers into the carrier. Spent ammunition and damage
   survive recovery. A disabled unit remains where it fell. Carrier departure
   requires an explicit confirmation identifying units being left behind; that
   confirmation neither recalls them nor closes their hull epochs or changes title.
6. **Remain playable.** Occupants have an explicit injured/incapacitated, aboard
   or stranded state and a real location. A stranded player can request bounded
   NPC extraction to an eligible known safe port under Q-F4/P-5. There is no player
   permadeath, indefinite capture or forced timeout extraction of an unwilling
   pilot. A friendly rescue already physically under way may defer the request
   only within the published bound. Crew extraction leaves the unit, equipment
   and cargo in the field; full towing and hull recovery follow in H2.
7. **Persist.** Reconnect and restart during the encounter and after withdrawal.
   The same units, casualties, positions, component conditions, remaining rounds
   and cargo survive. Committed outcomes are neither replayed nor rolled back.
   The existing outage freeze/countdown applies, with no unseen shooting or
   mining catch-up. Revisit the casualty later and find the same damaged asset.

Run the main story with a player captain, one hired NPC wingman and an opposing
force. Use two ordinary player pilots for the first duel checkpoint and for the
advisory-player-order variant. Additional variants prove reinforcement, neutral
presence, individual withdrawal and command succession without changing slots.

## 3. Scope

| Area | Included in this milestone |
|---|---|
| Playable setting | One designated frontier site and an authored path to suitable armed units and combat-qualified crew; preserve existing core-crime and safe-storage rules |
| Physical combat host | Production admission of real field units on the existing terrain, actual entry poses, exclusive motion authority, reinforcement and individual withdrawal |
| Player controls | Existing cockpit observation, movement, targeting and weapons connected to the field host |
| NPC execution | `ENGAGE`, existing movement/HOLD/WITHDRAW behavior and `HOLD FIRE`, through actual control and source action admission |
| Orders and information | Current mission grants, roster/leadership fences, delivered orders and returning replies; recipient-owned contacts and observer-bounded reports |
| Material outcomes | Persistent component damage, ammunition/charge expenditure, cargo, position, casualty identity and correct hull-epoch disposition |
| Crew and carrier exit | Minimal crew survival/extraction, disconnect posture, actual return/unseating, and explicit leave-behind departure |
| Time and restart | Surface/combat clock handoff, interruption of conflicting mining/work, durable combat recovery, and integration with the existing global outage freeze |
| Acceptance | Production-composition stories through ordinary player SSH, then an actual executable mush-bot combat expedition |

A detachment is a command group, not a universal faction. Use actual relations
and known hostility for targeting; neutral units and a third force are not
silently assigned to either of two teams. Bound participant admission using a
measured first-site budget and existing capacity mechanisms. Record that budget
and refusal behavior before enabling the host; capacity refusal must leave assets
and existing combat intact.

`SCREEN` and `SUPPORT` remain explicitly unsupported until a later tactical-order
slice. They must keep returning an honest NEGATIVE rather than a false WILCO.
Do not make licensed autonomous unit-mind hosting a prerequisite for a human NPC
pilot, and do not give a coreless unit an invented mind fallback.

## 4. Reuse and implementation boundaries

The following owners already exist and are the starting points for implementation:

| Existing surface | Reuse / required integration |
|---|---|
| `tacsim::detachment::{activation, crew, sortie}` | Contract, accepted assignments, current containment, launch flights, field orders, source receipts, leadership and return history |
| `tacsim::detachment::sortie::npc` | Existing bounded physical NPC executor; extend its supported objectives rather than introduce a second NPC controller framework |
| `mush_tactics::field_ops::{condition, engagement}` and `tacsim::field_ops` | Assessments, condition plans, engagement/crew helpers and arena reconciliation; verify semantics and connect the necessary helpers to actual host commits |
| `mush-tactics` unit records and dedicated IC custody | Stable asset identity, component state and exclusive physical/condition mutation; use existing coordinator ownership for new durable transitions |
| `tacsim::{surface, arena}` and cockpit adapters | Existing ground, physical placement, sensors, weapons, movement and terminal/client views; separate IC lifecycle from practice lifecycle |
| IC communications and mission unit grants | Delivery, ECM, replies and prospective authorization; leadership alone grants no helm, firing control or title |
| `mush_core::clock` and `mush-server::{timeline, world_task}` | Existing checkpoint/re-anchor, outage threshold, countdown, admission and resume; exercise combat through these paths |

The foundation catalogue in doc 27a is not evidence that a public command or a
live durable host already uses a helper. Audit real call sites before scheduling
new work. Its older personnel-manifest/embarkation-history wording does not
supersede doc 27's amended current-containment rule. Docs 26g–26j remain
superseded and must not be extended. Wages, if introduced later, follow contracts.

PR [#194](https://github.com/conclavio/rustmush/pull/194) is open at this draft's
baseline. It proves the complete mining expedition and adds player bay access,
nested-console sailing/current crew counting, and exclusion of bay occupants
from vehicle inventory. Reuse those changes after they land rather than copy
or recreate them. Its mining pilot is the later bot's travel and logistics base.

Four seams need concrete behavior before their delivery slice is accepted:

- **State and custody.** Transfer the same unit and its exact condition/pose to
  the tactical writer and back. Preserve the existing source identity and epoch;
  add only the durable engagement/handoff state the live path needs. Retries must
  not repeat physical effects. Persist acknowledged damage and expenditure through
  the ordinary owning commit path; neither an end-of-match write-back nor an OOC
  checkout snapshot can erase acknowledged intermediate outcomes. Audit arena
  defeat consequences so a mission kill does not stamp every section as wrecked.
- **Time.** Q-R3 accepts campaign-paced unengaged surface motion and wall-time
  tactical motion. Define the actual boundary for a unit mid-drive, including
  completed distance, remaining intent and energy already spent. Stop conflicting
  mining/work without granting duplicate yield. Define the reverse handoff and
  ordered arrivals/messages; unrelated journeys continue during a local fight.
  Reuse configured rates and the shared clock rather than hard-code rate 24.
- **Lifecycle.** Hostiles can reinforce through real entry and a unit can withdraw
  independently. Specify edge fire/pursuit behavior; a map edge cannot silently
  cancel a shot or grant immunity. Demote only after pending impacts, hostile
  interactions and boundary handoffs settle. Defeat, empty fields, inactivity,
  disband and a match result cannot reset or remove IC units. A recoverable mission
  kill keeps its epoch open; recovery or genuine terminal destruction uses the
  existing appropriate close path.
- **Knowledge and control.** A commander's contact hint does not become the
  recipient's lock or knowledge. Specify ENGAGE target loss, HOLD FIRE and safe
  withdrawal behavior, with WILCO, attempted work and completion kept separate.
  Link loss uses an actually installed/sealed fallback where supported; otherwise
  the established no-motion/no-fire safe posture applies after the existing grace
  and handover policy. Reports and succession use actual crew/unit availability.

These are focused integrations, not a mandate to redesign ordinary movement,
all world observers, communications, the economy or the global clock. A genuine
shared-clock defect found by acceptance belongs in its owning module with a
specific regression; do not reopen W-clock as a new prerequisite programme.

## 5. Delivery slices

`FC.*` names the live integration cuts in this plan. They refine H1-A/B/C;
they do not rename the foundation APIs or imply that all of H1/H2/H3 is delivered.
Each cut should be independently reviewable, with its gate demonstrated before
proceeding. Early cuts are development checkpoints, not a release of incomplete
crew-survival or crash-consistency semantics into live play.

| Slice | Player-visible result and implementation focus | Completion gate |
|---|---|---|
| FC.1 — ready the real assets | Resolve the supported armed-unit acquisition and combat-crew contract path; bridge an actual DT flight to the field host using existing condition/custody owners | Ordinary preparation and launch produce the same units/operators on the correct site; missing authority, stale readiness and a mining-only contract refuse honestly; no stock replacement or duplicate flight |
| FC.2 — first field duel | Two players enter a live fight through actual controls; implement tactical admission, terrain/pose and clock handoff, durable damage/expenditure, reinforcement and withdrawal | Production SSH duel causes real spent ammunition and localized damage; survivors return with exact state; a disabled unit remains at its hit location; mid-fight restart preserves acknowledged outcomes |
| FC.3 — hired NPC wingman | Execute delivered ENGAGE and existing movement/HOLD/WITHDRAW orders through the NPC's real controls; preserve player autonomy, ECM and leader succession | Player/NPC encounter proves own-contact targeting, HOLD FIRE, lost-contact uncertainty, actual WILCO transport and withdrawal; obsolete/expired orders and revoked permissions produce no new unauthorized action |
| FC.4 — survive defeat and leave | Connect cockpit/engine defeat to crew location/survival and requested extraction; integrate real recovery/unseating, carrier leave-behind and safe combat demotion | Both loss variants leave the correct components/cargo; a stranded player returns to play without moving the hull; mobile survivors depart only with any required explicit leave-behind confirmation; no OOC reset path |
| FC.5 — complete expedition acceptance | Exercise all seven steps, ship player help and extend the independent bot after the human paths work | Actual bot executable completes a public-SSH combat expedition, with an opposing ordinary pilot/controller; separate loss/restart variants prove casualty persistence, extraction and no unseen outage work |

FC.1 must name the actual available armed design and public acquisition path, plus
its compatible combat contract and carrier capacity. Authored content and narrowly
missing player commands are part of that cut. Do not quietly make the full unit
designer/refit programme, a new procurement economy or free staff-spawned combat
assets prerequisites. Staff/test fixture setup may seed the world; it cannot
substitute for player launch, firing, damage, withdrawal or crew extraction.

Keep the existing six-slot bound. The first acceptance force need only use two
slots. Select a small opposing force suitable for repeatable tests, not a new
population/mission-spawning system. The actual names/syntax of any new encounter,
extraction or leave-behind commands belong in their owning cut's help and tests;
this draft does not present proposed commands as already available.

## 6. Acceptance and verification

For each slice, run focused owner/unit tests and the relevant production-composition
stories. Repeat broad gates when shared behavior changes. Follow existing Clippy,
memory, source-policy and deterministic replay requirements; do not weaken a gate
or rebless practice hashes merely to accommodate a new live host.

The final evidence must include:

- The seven-step public-SSH story, with actual asset references, contract,
  accepted roster/flight, cockpit observations, delivered orders/replies,
  ammunition expenditure, damage, return/leave-behind and later casualty revisit.
- A player-controlled duel and an NPC-wingman variant. Exercise a neutral and a
  reinforcement, individual withdrawal, player-recipient autonomy, stale roster
  or leadership terms, changed grants, and jamming/delayed messages.
- An engine mission kill retaining a surviving weapon and cargo, and a cockpit/
  head defeat retaining unrelated sections. Further fire can worsen a casualty;
  recovery/demotion/disband never heals it or turns disablement into terminal loss.
- Requested crew extraction and a bounded friendly-rescue deferral. Refusal or
  interruption leaves a defined crew location and does not strand play forever.
- Restart after an acknowledged shot/hit and during a physical handoff, then
  retry/lost-ack coverage: no duplicate shots, movement, units, returns or closes.
  Exact-state fixtures may supplement player observations to check hidden
  invariants; they must not supply hidden state to a player or bot controller.
- Short-restart re-anchor and configured longer-outage countdown, proving no
  unseen combat/work catch-up. A local fight does not freeze an unrelated journey;
  a global outage freezes both through the existing shared policy.
- OOC practice reset/void/replay still works within practice and cannot reach IC
  assets. The field ending publishes no objective enemy inventory or global replay.

The bot stays independent under [its architecture rules](../../crates/mush-bot/AGENTS.md):
no server normal/build dependencies or internal action/state access. Its acceptance
launches the actual executable over SSH, reuses travel/bunkering where practical,
and judges the fight through player-visible observations and outcomes. A stable
opposing fixture actor is acceptable; staff-applied damage is not a combat proof.

## 7. Next areas after the skirmish

| Order / area | Next player success story | Boundary |
|---|---|---|
| 1. H2 physical recovery | Return on a separate sortie, find the casualty, attach a suitable tug, tow it to the carrier and load the same damaged hull | Preserve components/cargo/identity and real capacity; include restart while attached/loading; recovery does not repair |
| 2. H2 salvage entitlement and evidence | Exercise lawful owner/landholder recovery and physically possible unlawful recovery, then the existing evidence and title-contest process | Custody is not title; hidden theft is not a global notification; full broader recovery release requires the legal path |
| 3. H3 restoration and doc 31 refit bay | Inspect the recovered damage, pay for real workshop work, repair or replace the appropriate parts, rearm and field the unit again | Resource/time/authority checks; stripping and scrapping retain provenance; no free restoration on return |
| 4. Richer tactical orders and escort play | SCREEN and SUPPORT work, an escort protects a mining expedition, and crews coordinate movement and fire under imperfect communications | Build on the proven field host; add measured behaviors rather than require a general strategy/mission framework |
| 5. Licensed autonomous unit-mind field host | A properly licensed/core-equipped unit joins the same expedition under its actual execution and control contract | Separate software mind from a hired human NPC; no fabricated licence, compute capacity or fallback |
| Separate owner decision: DT.4 | Delegate a subset of detachment command when actual play demonstrates the need | Still deferred; it is not required for any preceding milestone |

Facility assaults, broad territorial warfare, elaborate injury/treatment/prisoner
systems, wider maps and procedural enemy campaigns require their own later scope.
They are not implicit acceptance criteria for the first frontier skirmish.

## 8. Immediate next action

Start FC.1 by tracing one selected armed unit and combat-qualified NPC from public
port acquisition through the actual carrier, DT preparation and launch. Record
only the concrete gaps on that route and the owning files. Then implement the
smallest reviewed change that makes that route work, followed by the FC.2 duel.
No H2/H3 helper expansion or personnel-ledger work is needed to begin.
