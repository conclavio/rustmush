# 42. Cut notes — crew custody and capture

One `## CR.n` heading per cut of [42-crew-custody-and-capture.md](42-crew-custody-and-capture.md) §7
(`CR.0`–`CR.7`), in the order they landed: what was built, the gate that proves it, where the PR departed
from §7's row, and the cell's summed-minutes delta ([doc 43](43-testing-strategy.md) rule 2.3 #14).

## CR.0 — the record, the gate and the vocabulary

Q-42-1, Q-42-2, Q-42-3, Q-42-23, with Draft 0.3's 42-2, 42-5 (B) and 42-6. No behaviour a player can
reach changes yet: nothing writes the new locations or fields until CR.1 (the load), CR.2 (the handover)
and CR.4 (the eject). This cut is the shape every later cut writes into.

**The record** (`crates/tacsim/src/field/crew.rs`).
- `CrewLocation::Captured { carrier, captor, since_ic }` and `CrewLocation::Processing { port,
  release_due_ic, charge: Option<mush_economy::ObligationId> }` under the existing `at` tag;
  `charge` omitted while `None`. `CREW_FORMAT` stays 1: an old record decodes and re-encodes unchanged
  (pinned by the decode test).
- `Extraction::forced` (omitted when false) and `Extraction::capture: Option<EjectCapture { party,
  carrier, captor }>` (omitted when `None`), `party` the field's `PartyId` (wire word, e.g. `"c"`).
- Readers: `CrewRecord::in_custody`, `awaiting_pickup`, `hold() -> Option<Hold>`; `extractable` is `None`
  for both new locations (CR.3 opens a captive's after the bound).

**The rules.**
- `check_in` leaves a record alone (`leaves_alone`) when it is in custody, or its extraction is
  abandoned (GC.5) or forced. The prior condition is kept in every case — a check-in never heals (lane 5's
  rule, as on `main`).
- `crew_location` returns a record in custody ahead of a running field's carrier row.
- Every load rule (`loaded_aboard`, `abandoned_loaded`, both through the new `held_in`) skips an ejectee
  whose forced extraction is pending and never matches a record in custody.
- `on_frames` never incapacitates a record in custody or awaiting pickup.
- `tick`'s location match gains the two arms (no seat to vacate); the captured landing and the bound are
  CR.3's and CR.4's.
- `status_line`: "held aboard the {carrier}", "in processing at {port}, released in about {h} IC h[, the
  release fee charged]", and "pickup after your ejection, to {port}" for a forced extraction.

**The gate.** `crew::gate(ctx, Gate) -> Result<(), CmdError>` over `refusal(w, who, Gate)`, with
`held(w, who) -> Option<ObjId>` as Q-42-2's `crew::held`. Kinds: `Move`, `Operate`, `Extract`, `Chat`.
A captive is refused every kind with "You are held aboard the {carrier}."; an ejectee awaiting pickup
`Move`/`Operate` with "You are waiting for your pickup."; a character in processing `Move` only, with
"You are being processed at {port}; about {h} IC h.". Read at the head of:
- `go` and exits: `crew::HeldGate`, a `MoveAttempt` hook registered in `field::register` (in processing,
  a destination inside the same port passes);
- `+tv` (`port/tv.rs`): `/bay` and `board|leave` as `Move`, `/permissions` and every act but the status
  read as `Operate`;
- `+ship` (`port/ship.rs`): `/board`, `/leave` as `Move`, every other switch but the `/orders` read as
  `Operate`;
- `+helm`, `+tac` (`space/commands.rs`): every switch but `/status`;
- `+field` (`field/commands.rs`): `/mark`, `/fire`, `/void`;
- `+bay` (`port/bay_work/command.rs`): every switch but `/quote`;
- `+salvage` (`surface/claim.rs`): every switch;
- `+sortie` (`detachment/commands.rs`, `sortie_gate`): `/seat` `Move`; `/activate`, `/hire`, `/board`,
  `/contract`, `/launch`, `/recover`, `/cancel-launch` `Operate`; `/extract` `Extract` (also at the head
  of `crew::extract_cmd`); `/chat` `Chat`;
- `WILCO`/`NEGATIVE` (`detachment/commands.rs::respond`) and every cockpit keyword
  (`match_/cockpit.rs::cockpit_command`, which carries `ORDER DT:…`);
- `@station` (every switch but `/show`) and `+focus` (every form but the bare read) in mush-sim, through a
  new defaulted `CarrierHooks::restrains_operator(w, actor)` method (`mush-sim/src/journey.rs`), which
  `TacsimCarrierHooks` answers with the `Operate` refusal.

**Vocabulary** (`crates/mush-bot/src/mine/crew.rs`, a file of its own so FR.0's `lines.rs` change does not
collide): `+sortie/condition`, `+sortie/eject`, `+sortie/extract`, `+ship/captives`, `+ship/release
<name|#n>`, accepted by the field and escort lanes' `in_vocabulary`; `custody(text)` reads the three
refusal lines, contract-checked against `tacsim::field::crew`'s constants.

**Docs and census.** `docs/help/field.md`'s Crew section: the five-plus-one locations, the held rules and
the processing line ("There is no ejection or capture" struck). `crew.rs`'s head: Q-CF-38's "no ejection
or capture" and "until capture is built" replaced. `legacy-handlers.toml`: a `REFRESHED by doc 42 cut
CR.0` lead on `+sortie`, `+ship`, `+tv`, `+field`, `+bay`, `+salvage`, `+helm`, `+tac`, `@station`,
`+focus`, `wilco`, `negative` (a read, no write, no switch, no family; `PINNED_BASELINE` does not move).

**Gate.** In-source tests in `crew.rs`: `cr0_the_custody_record_decodes_under_the_same_format`,
`cr0_check_in_and_the_loads_leave_custody_alone`, `cr0_the_held_gate_refuses_by_hold`; in `mush-bot`
`the_crew_lines_are_closed` and `the_custody_lines_match_the_game`. No story (§7's row).

**Divergences from §7's row.**
- §5's `LOAD_PREVIEW_FOREIGN` ("is abandoned to extraction" → "is in your custody") and `ABANDONED`'s line
  are left as they are: they describe what the load still does until CR.1 changes it, and GC.5's gate
  asserts the preview. CR.1 changes both with the behaviour.
- The gate reading "+ship/\*", "+bay/\*", "+helm/\*", "+tac/\*", "@station/\*" and "+focus" lets each verb's
  pure reads pass (bare `+ship`, `/orders`, `/quote`, `/status`, `/show`, bare `+focus`), by 42-6's rule
  for `+sortie/orders` (a read comes off the list). `+salvage` and every cockpit keyword are gated whole.
- Q-42-2 names "detachment chat" only for a captive; an ejectee and a character in processing keep it.
- `@station` and `+focus` live in mush-sim, which cannot see tacsim's crew record; the gate crosses by the
  existing composition seam (`CarrierHooks`) rather than a new one.

**Cell delta (rule #14).** No shard cell changes: the five tests are in-source unit tests in the tacsim
and mush-bot library test binaries, each well under a second; no story file, no `mod` line.

## CR.1 — capture at the load

Q-42-4, Q-42-5, Q-42-16, Q-42-21, Q-42-22, Q-42-26 (Draft 0.4), with Draft 0.3's 42-4, 42-8 and 42-9. A
completed load into a carrier that is not the casualty's crew's party's or a Friend's now takes the crew
captive instead of abandoning them. GC.5's abandonment is gone; `Extraction::abandoned` still decodes on
old records.

**The capture** (`crates/tacsim/src/field/crew.rs`, `captured_loaded` over a `LoadCapture`).
- Every record `held_in` the casualty: the seat in the casualty vacated (no arena: a load runs only
  outside a field), the body stepped into the carrier's bay room by `mush_sim::place_interior` (a plain
  move if that refuses), then the record `Captured { carrier, captor, since_ic: now }`, the condition kept,
  any extraction or treatment dropped. Body first, record second, so `HeldGate` never sees the move.
- The captor (Q-42-4) is the hull's custodian as the load reads it (`tow::custodian_of`: the carrier's
  entrant owner, else its leader, else its helm), else the load's operator: the character the
  `SalvageCustody` row names.
- Lines: the captive hears `CAPTURED`; the captor hears `CAPTOR_TOLD` once a load.
- `LOAD_PREVIEW_FOREIGN` now ends "anyone still inside it is held in your custody aboard the {carrier}";
  `ABANDONED`, `abandoned_loaded` and `abandon` are deleted.

**The standing** (Q-42-21). Unless the captor held the captive Hostile, one `mush_faction::record_standing`
grant against the captor: `STANDING_CAPTURE` −5 with each faction the captive openly stands in
(`open_factions`, at most eight), cause `capture:<site tag>:<ordinal>:<captive>` (`capture_cause`: GC.10's
FNV site tag and the site's last field ordinal), not voidable; the captor is told. "Held Hostile" is read
from what survives a field's end (`held_hostile`): a Hostile stance either way between the two characters'
organizations (FR.1's table), or a GC.10 receipt of the site's last field naming either of them (fire).

**The NPC exit** (Q-42-5, Q-42-26). `npc_exit_loaded`: an NPC seated in the casualty under a crew contract
leaves the seat and goes back to the pool room — the NPC's `home` (the room the content created them in,
Ceres's Slack Line), else the contract's hire port's arrival room — and the organizer (the contract's
employer) hears `NPC_RETURNED`. The roster's half is owed in one bounded record, `tacsim:crew_exit:<npc>`
(`NpcExit { format, npc, organizer, recovered_ic }`, read at boot beside the crew records into the RAM
`NpcExits`). `npc_exits`, at the end of the tow tick, forgets an exit once the ledger shows neither the
contract nor the reservation held for that organizer, and otherwise submits `tacsim_crew_captured` as the
organizer through `Fire::as_principal` (a fire of its own, the NPC the agent). Until it lands the contract
stands, so `offers()` keeps the NPC off offer.

**The action** (`crates/tacsim/src/detachment/crew/runtime.rs`). `tacsim_crew_captured` (outcome
`tacsim_crew_captured_result`), the tacsim module's, declaring `"families":["tacsim.crew_contracts",
"tacsim.detachment_reservations"]`; registered by `register_captured` from `register_preparation` after the
activation, since it needs both handles. Its offer fixes the organizer's owed exits. Its prepare re-reads
the owed exit in the world (organizer and instant), removes the contract when the organizer employs the
NPC and the reservation when it is the organizer's, and refuses when it removes neither.
`detachment/activation/runtime.rs`: `RESERVATIONS` is `pub(crate)`, and a deactivation now passes over an
entity whose reservation is already gone (the captured NPC's) instead of refusing "Reservation changed.",
so the organizer can still stand the roster down.

**The evidence** (Q-42-22; `surface/claim.rs`). `UnlawfulKind::Capture` and `log_captures`: one row a
captive (`actor` the captor, `subject` the captive's object number, `landholder`), written in the load's
finish under the `Recovery` row's condition. `LoadFacts::captives` (omitted while empty) carries the
captives the physical step took, from a RAM `TowCaptives` (tug → casualty, captives) dropped when the act
is told.

**The verbs** (Q-42-9; dispatched from `port/ship.rs`, bodies in `crew.rs`, switches in `commands.rs`).
`+ship/captives` lists the captives the actor holds or that ride the hull they command, with the carrier,
the condition and the IC hours held. `+ship/release <name|#n>`: docked at an IC port (`ic_return::PORTS`)
the captive is set down in its arrival room, stamped, `Port { port }`; anywhere else they stay in the bay
room as `Carrier { hull: carrier }`; both told. The bot already had both lines (CR.0's vocabulary), so
none were added.

**The seam for RS.5 and FR.5** (`surface/tow.rs`). `place_in_bay` takes `(tug, operator)` and returns the
captives. The crew branch is one function, `load_crew(ctx, site, (casualty, carrier, room), friend,
operator)`, which every completed load should call wherever it runs. A written-off lease rig's crew step
into the bay room before `tv::hand_back` (42-8): captive for a non-friend loader, aboard for a Friend's.

**Gate.**
- `field_gc5::gc5_a_non_friend_load_takes_custody_and_captures_the_crew` (256) and `…_at_512`: GC.5's
  gate rewritten over `grounds_rig`'s two grounds, story (a′) to the load. Custody and the claim are as
  before. The captain is `Captured { the Red Moth, the stranger }` from the load, stands in the Red Moth's
  rooms and is out of the escort's seat. `+sortie/extract` and `+tv/1 leave` are refused with the held
  line, `+sortie/condition` says "held aboard the Red Moth", and the stranger's `+ship/captives` names
  them.
- `field_gc5::gc5_review_the_captors_field_close_keeps_the_captive` (256) and `…_at_512`: the twin. The
  captive is among the souls on the Red Moth's row in the second field, `crew_location` gives custody, and
  the close leaves the record as it was.
- `crew_custody::cr1_a_captured_hulls_hired_npc_goes_back_to_the_pool_and_off_the_roster` (256) and
  `…_at_512`: the hire by `+sortie/crew` and `+sortie/hire`, the load, the NPC in the Slack Line, the
  captain told, the action landed by the tick, `+sortie/contract` refused, the NPC on offer to a third
  captain, no `Capture` row.
- `crew_custody::cr1_the_captor_releases_a_captive_on_the_site` (256): `+ship/captives`, a refused
  release, a captive's own `+ship` refused, the release to `Carrier`, the list empty.
- In-source: `crew.rs` `cr1_a_non_friend_load_captures_the_crew` (replacing GC.5's abandonment test);
  `claim.rs` `cr1_a_capture_off_owned_ground_is_evidence_naming_captor_and_captive`.

**Divergences from §7's row.**
- The resubmission key is `tacsim.crew/<npc>/captured/<recovered_ic>/<try>`: §7a's key plus this boot's
  try ordinal, as the bay's owed act carries its tries (`tacsim.bay.act/<hull>/<act>/<tries>`), so a
  refused submission never blocks the next.
- A field's private marks end with it (Q-CF-8) and leave nothing after demotion, so "held Hostile" at a
  load reads stances and the last field's fire receipts only. A mark never followed by fire reads as no
  hostility, and the capture costs standing. A durable record of a field's marks is doc 40's to give.
- `TowCaptives` is RAM: a restart between the load's physical step and its finish loses that finish's
  `Capture` rows. The captives stay held; only the evidence is lost.
- The NPC story seats the NPC by staff hand (`seat_npc`, the billet an activation writes) and puts the
  escort on the index by staff hand (GC.5's later-visit precedent). There is no DT activation and no
  field, so no reservation exists and the action ends the contract only. The deactivation tolerance is
  covered by review, not by a story.
- The captain is seated as a DT flight seats a pilot (the station, and the body in the hull), never by
  `+tv/<n> board`: §3a's cab bug stays the fix thread's.
- `+ship/release`'s two landings, and a written-off Friend's load's crew stepping aboard, are built here;
  the `extractable` widening for a released passenger stays CR.3's.

**Cell delta (rule #14).** All of it lands in shard_1. `crew_custody` is pinned there by its `field_gc5` and `field_gc4` imports (doc 43 §10.2 asks a cut to say so). The two rewritten GC.5 tests became four, one per ground, and `crew_custody` adds three. Local Windows wall times (`-j 8`):
- GC.5's gate: 39.1 s at 256, 39.6 s at 512;
- the twin: 54.8 s at 256, 55.7 s at 512;
- the NPC exit: 21.6 s at 256, 21.8 s at 512;
- the release: 43.4 s.

That is about 4.6 summed minutes against the two 256-only tests it replaces, which were not timed here. **Owed:** the figure from PR #408's JUnit (shard_1's `field_gc5::` and `crew_custody::` testcases against main's last green run), to replace this estimate.
