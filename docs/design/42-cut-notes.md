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

**Gate.** Story (a′) to the load and its review twin are one script over both grounds, hoisted to
`tests/support/field/custody_rig.rs` (declared in every shard root), so each ground's run can sit in a
different shard.
- GC.5's gate rewritten, `custody_rig::non_friend_load_captures_the_crew`: shard_1
  `field_gc5::gc5_a_non_friend_load_takes_custody_and_captures_the_crew` (256) and shard_8
  `crew_custody::cr1_a_non_friend_load_takes_custody_and_captures_the_crew_at_512`.
  - Custody and the claim are as before.
  - The captain is `Captured { the Red Moth, the stranger }` from the load, stands in the Red Moth's
    rooms and is out of the escort's seat.
  - `+sortie/extract` and `+tv/1 leave` are refused with the held line, `+sortie/condition` says "held
    aboard the Red Moth", and the stranger's `+ship/captives` names them.
- The twin, `custody_rig::captors_field_close_keeps_the_captive`: shard_1
  `field_gc5::gc5_review_the_captors_field_close_keeps_the_captive` (256) and shard_8
  `crew_custody::cr1_the_captors_field_close_keeps_the_captive_at_512`. The captive is among the souls on
  the Red Moth's row in the second field, `crew_location` gives custody, and the close leaves the record as
  it was.
- shard_8 `crew_custody::cr1_a_captured_hulls_hired_npc_goes_back_to_the_pool_and_off_the_roster` (256)
  and `…_at_512`: the hire by `+sortie/crew` and `+sortie/hire`, the load, the NPC in the Slack Line, the
  captain told, the action landed by the tick, `+sortie/contract` refused, the NPC on offer to a third
  captain, no `Capture` row. The hire and the action land on the composed path (`land_composed`), since
  both prepares read the world.
- shard_8 `crew_custody::cr1_the_captor_releases_a_captive_on_the_site` (256): `+ship/captives`, a refused
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

**Cell delta (rule #14).** Measured from the `<testcase time>` sums of the JUnit artifacts, against main's
last green run, 41814093413 (3fb3e861).
- **First PR run, 41820027413** (everything in shard_1). shard_1 went from 55.9 to 76.9 summed minutes and
  from an 18.7 to a 24.0 min wall, which set the run's wall (the next cell was 20.1).
  - `field_gc5::` went from 3.12 to 6.32 min. The two tests replaced had run 51.1 s and 67.2 s; their
    four successors ran 62.5, 67.4, 79.9 and 78.4 s.
  - `crew_custody::` added 3.01 min.
  - The unchanged rest of shard_1 also ran 28 % slower on that runner (52.8 → 67.6 min).
- **Moved.** By rule #14, the 512 runs and `crew_custody` moved to shard_8, the run's shortest
  integration cell (11.9 min), through `custody_rig`. shard_1 keeps the two 256 runs.
- **After the move, PR #412's run 41830026813:**
  - shard_1 is 56.7 summed minutes (+0.8 on main), with `field_gc5::` going from 3.12 to 3.32 min. Its
    wall is 18.3 min (main's was 18.7).
  - shard_8 is 44.1 summed minutes (+14.8 on main's 29.3), of which `crew_custody::` is 4.65 min; the
    rest of the shard's rise is that runner's speed. Its wall is 15.0 min (main's was 10.2).
  - No integration cell sets a new wall: the run's slowest was 18.3 min, against main's 20.2.

## CR.2 — the port handover and release

Q-42-6, Q-42-7, Q-42-19, with Draft 0.3's 42-2, 42-3 and 42-7. A captor's carrier that docks at an IC port
hands every captive aboard to the port's authority. After an IC hour of processing the captive is free at
that port, and the release fee is levied on their own account, paid or owed.

**Content: lane v23 / relay v14.** Each IC port entry gains:
- `authority`: Ceres `freeport_registry`, Stickney `stickney_yards`, Luna `orbital_access`, Deimos
  `voltaire_bond`, Titan `titan_concession`;
- `release_fee_cr`: 1,500.

`PortDef` validates both: the authority must be one of the entry's orgs, and a fee needs an authority and
is at least 1. The lane digest is `ed543059…` and the relay's `476bf7d8…`. The strip
`lane_chain::strip_release_authority` heads the chain (`strip_part_seeds` calls it first). The pins moved
are:
- tacsim: `cost_lane.rs`, `shard_4/lane.rs`, `shard_5/sites.rs`, `shard_10/stations.rs`,
  `shard_3/drones.rs`, `shard_9/mill.rs`;
- mush-server: `boot_parity.rs`, `relay_registry.rs`.

**Config.** `[tacsim.field].release_s`, 3,600, banded 60 to 86,400 (`FIELD_RELEASE_S`). It is documented in
mush-config's table and mapped in `mush-server/src/config_ext.rs`.

**The authority's grant** (`port/vendor.rs`, part (e).10). At apply, the port's stock clerk is granted
`port.release_fee` (`RELEASE_FEE_CAPABILITY`, added to the composition's capability vocabulary) on the
authority's root scope, once. The grant is recorded as `PortMap.release_grant`, skipped while absent, so
`PORT_MAP_SCHEMA` stays 1. The clerk is the levier, as the vendor's `vault.withdraw` grant already makes
it the vendor's.

**The handover** (`field/crew.rs`, `hand_over(ctx, hull)`).
- It runs from `composition/sim_routes.rs`'s `arrived` before the arrival's lines and hooks (42-3's
  persons-first rule), and again on the crew tick's `custody_pass`, which catches a lost callback.
- It selects only records still `Captured { carrier: hull }`, so a duplicate callback, a replay or a
  restart finds nobody twice.
- Each captive is set down in the port's medical room (`port::medical::room`), else its arrival room.
- The record is written `Processing { port, release_due_ic: now + release_s, charge: None }`.
- The captive hears `HANDED_OVER`; the captor hears `HANDED_OVER_CAPTOR`, with nothing paid.

**The fee** (`levy_release`). One `mush_economy::create_sanction_obligation` (the Registry fine's shape):
- one period due now, grace 1;
- the debit is the captive's own account, the credit the authority's account;
- `levied_by` is the clerk, under `port.release_fee` on the authority's root scope;
- the inspection key is `release:<captive>:<release_due_ic>`.

The record's `charge` is filled once it is created. An obligation already carrying the key is adopted, not
levied twice. A refusal leaves `charge` empty, and the crew tick tries again. The economy's sweep settles
it at once when the account covers it; otherwise it stands as an obligation, which RS.5's `+debts` lists
and `+debts/pay` settles.

**The release** (`release_processed`, on the crew tick). At `release_due_ic` the character:
- is set down in the port's arrival room, unless they already stand in one of its rooms;
- is stamped with the port as the one they know;
- becomes `Port { port }` and is told `PROCESSED`.

The release never waits on the fee (FO-12). Treatment (GC.13) begins at the handover, since the captive
stands in a port room. CR.0's gates already treat `Processing` as Q-42-2 asks.

**Gate.**
- `shard_8 crew_custody::cr2_the_captor_docks_at_ceres_and_the_captive_is_handed_over_and_released` (256,
  the fee paid) and `cr2_an_unpaid_release_fee_stands_and_the_captive_is_released_anyway_at_512` (512,
  the fee unpaid). Both use one script, `custody_rig::handover_at_ceres`, after story (a′)'s load:
  - the stranger lifts for Ceres (`+ship/lift ceres leave=1`) and docks;
  - the captain is in processing at Ceres, standing in a Ceres room, with the fee's obligation named;
  - `+sortie/condition` says "in processing at Ceres", and the captor's `+ship/captives` is empty;
  - paid: the obligation is `Satisfied` and 1,500 cr have left the account;
  - unpaid (a staff hand empties the available credit first): the obligation stands unsatisfied;
  - after the hour the captain is `Port { ceres }` and Ceres is the port they know.
- In-source: `crew.rs` `cr2_the_release_key_and_the_processing_band`.

**Divergences from §7's row.**
- The inspection key is `release:<captive>:<release_due_ic>`, not `…:<since_ic>`. `Processing` does not
  carry the capture's instant, and a retried levy must rebuild the key from the record. It is still one
  key per handover.
- The handover also runs on the crew tick, not only in the docking hook. The arrival hook is
  production's first chance, and the tick catches anything it missed.
- The bot does not type the captive's half in this PR's tests: they type it as the player on the
  production command path. The bot-over-SSH story is a dispatch-tier run, owed and named here.
- A space `PortBerth` docking (Q-42-6's second trigger) is CR.6's seam: it calls `hand_over` when the
  space track's crew state exists.
- RS.5's `+debts`, as #415 builds it, lists only `Ruleset`-cause one-term debts. The release fee is a
  `Sanction`-cause one-shot, so it is not a `+debts` row until `debts_of` and `pay_debt` take it.
  **Closed after #415 merged**, in RS.5's preferred shape:
  - mush-economy keeps `debts_of`, and adds `debts_of_with(w, debtor, namespaces, sanction_prefixes)`,
    `pay_sanction_debt` and `reinstate_sanction_debt`. Each takes `DebtCause::Sanction(prefix)` under
    the ruleset debt's own lock (the actor must be able to debit the debit account), and pays with the
    same single post and `Satisfied`.
  - `tacsim::surface::retrieval::DEBT_SANCTION_PREFIXES` is `["release:"]`, beside
    `DEBT_NAMESPACES`. `+debts`, `+debts/pay` and the desk's refusal (`desk_refusal`, with its
    reinstatement) read both.
  - The unpaid story now asserts that the fee is a `+debts` row and that `+debts/pay` settles it once
    the captain can cover it.

**Cell delta (rule #14).** Measured from the JUnit `<testcase time>` sums of PR #417's run
41846609530 against #412's run 41830026813, which first put `crew_custody` in shard_8.
- shard_8's `crew_custody::` goes from 4.65 to 6.56 summed minutes: +1.9 for the two CR.2 stories.
- shard_8's wall goes from 15.0 to 15.5 min. The run's slowest integration cell is 19.4 min (main's
  41827163605 had 19.3), so no cell sets a new wall.
- The lane-count edits to `sites::` (shard_5) and `vehicles::` (shard_6) move no time.

## CR.3 — the bound and the held case

Q-42-8, Q-42-9, Q-42-15. A captor who never docks can hold a captive only up to the bound. A captive the
captor releases away from a port can ask for extraction. A captor's carrier wrecked in a later field
strands its captives like the rest of its crew.

**Config.** `[tacsim.field].captive_bound_s`, 604,800 (seven IC days), banded 60 to 2,592,000
(`FIELD_CAPTIVE_BOUND_S`). It is documented in mush-config's table and mapped in
`mush-server/src/config_ext.rs`.

**The bound** (`field/crew.rs`).
- `refusal(w, who, Gate::Extract)` reads a `Captured` record's `since_ic` against the bound. Before it
  passes, the refusal is `HELD_BOUND` ("You are held aboard the {carrier}. In about {h} IC h…"). Every
  other gate keeps CR.0's `HELD`, so nothing else opens.
- The bound is IC time from the record, so a restart resumes it and a freeze pauses it.
- Past the bound, `extract_cmd` takes the request (`may_extract`), telling `EXTRACT_RELEASE`.
- The crew tick's existing `Captured => (None, None)` arm lands it as any extraction: no seat to vacate,
  the body moved to the safe port, `Port`.
- A handover at a port overtakes a pending extraction (`hand_over` clears it).

**The released passenger** (Q-42-9's widening). `may_extract` also takes `Carrier { hull }` when `hull` is
not the hull the character commands and they stand aboard it. `+ship/release`'s two landings were built in
CR.1.

**The captor's wreck** (Q-42-15). `check_in` asks `wrecked_captive` first:
- when a captive's carrier checks in and `aboard()` reads it stranded (GC.3's `CrewStranded`), the captive
  is `Stranded { site, unit: carrier }`, the condition kept, custody gone, told `STRANDED`;
- a sound carrier's check-in still leaves the captive alone (CR.0's `leaves_alone`).

**Gate.**
- `shard_8 crew_custody::cr3_a_captive_extracts_after_the_bound_and_a_restart_keeps_it` (256) and
  `…_at_512`, through `custody_rig::held_past_the_bound`, with the bound at 600 IC s:
  - after GC.5's load, `+sortie/extract` refuses with the time left;
  - a crash and reboot keeps `Captured` with the same `since_ic`;
  - past the bound, the request is taken;
  - after `extract_s` the captain is `Port`, and the captor's `+ship/captives` is empty.
- In-source: `cr3_a_captors_wreck_strands_its_captives`. CR.0's `cr0_the_held_gate_refuses_by_hold` now
  also pins the extract refusal's time and its opening past the bound.

**Divergences from §7's row.**
- Story (e)'s wreck half is an in-source gate on the check-in rule, not a fought loss. A second field that
  takes the captor's carrier's hull (GC.3's fixture, a third party firing on the Red Moth) would add a
  field run a ground for one branch of `check_in`. It is owed to the next story that fights over a
  captor's carrier.
- The extraction's landing line for a captive is the ordinary `EXTRACTED` ("Your hull is still on the
  ground"): the tick has no site for a carrier.

**Cell delta (rule #14).** Measured from the JUnit sums of PR #418's run 41851040446 against #417's run
41846609530.
- The two CR.3 stories ran 41.8 s and 42.1 s: +1.4 summed minutes in shard_8. On a faster runner,
  `crew_custody::` read 6.55 against 6.56.
- shard_8's wall is 12.5 min (15.5 before).
- The run's slowest cell was shard_1 at 23.2 min, which this cut does not touch. Every shard_1 module ran
  about 1.27× slower on that runner (`driving::` 14.0 → 18.7, `field_gc4::` 8.4 → 9.7), as doc 43 §5.8
  warns the hosted machines vary.

## CR.4 — Ejection

Q-42-10 to Q-42-14, with Draft 0.3's 42-1 and 42-5 (B). The seated pilot of a doomed unit leaves the
seat for a pickup. Under a Hostile party's hold that pickup may be the holder's, by a seeded draw.

**Config.** `[tacsim.field].eject_capture_permille`, 500, 0 to 1,000. It is documented in mush-config's
table and mapped in `mush-server/src/config_ext.rs`.

**The verb** (`field/eject.rs`, `+sortie/eject`, gated `Gate::Move`, so a captive or an ejectee
waiting for the pickup is refused with CR.0's lines).
- Who may use it: the seated pilot, from inside the unit. It is refused:
  - at a remote console (`EJECT_REMOTE`);
  - to a stranded pilot (`+sortie/extract` is theirs);
  - with an extraction already on its way;
  - while world time is frozen.
- An NPC types nothing.
- **In a running field** the host sends the arena `ArenaMsg::Eject { unit }` and keeps the asker in
  the RAM `EjectBook`. A restart before the answer loses only the request.
- **After a field** (the unit standing on the site's ground, no field running), the world's cells
  decide and nobody holds the cell.

**The arena's read** (`arena/eject.rs`, `Arena::eject_read`). It is taken at the next pulse's head,
before the withdrawals, and answered as `ArenaOut::Ejected` behind the ledger. **A read, never a step:
nothing moves, nothing is journalled or taped, nothing is hashed.**
- **Doomed** (Q-42-11) is any one of:
  - the kernel's own wreck, motion-disabled or disabled limits;
  - `field::crew::doomed` on the live cells: the check-in's stranding test (no power, no control, no
    motion, a wreck or irrecoverable). A unit with no mobility profile is never doomed for want of
    motion: that is the withdraw's own-power rule;
  - a control section already damaged.
- **Holding the cell** (Q-42-12 with 42-1) needs all three of:
  - (a) the holder's book holds the pilot's party Hostile;
  - (b) a live unit of the holder that is armed at check-out or has fired has the doomed unit's pose
    inside its `fight_inner_cm`;
  - (c) no unit of the pilot's own party or a Friend's has the pose inside its reach. The ejecting
    unit, wrecks and unarmed hulls are excluded.
- The answer lists every holder, nearest unit first, together with the condition the live sections
  give (Q-42-13) and the arena millisecond.

**The draw** (`field::eject::draw`).
- (d): the host takes the first holder that has a leader and a carrier in the field that has not
  fallen. That carrier is either one of its own units or the carrier of a party its leader leads.
- The draw is FNV-1a over GC.10's `cause_prefix(site, ordinal)`, the unit and the read's millisecond,
  taken mod 1,000. It captures below the knob, so 1,000 always captures and 0 never does.

**The eject** (`ejected`).
- `crew::release_seat`, factored out of the extraction's landing: every channel `Released`, the pilot
  unbound and the cockpit cleared, the station vacated.
- Then `Stranded { site, unit }` with an `Extraction { forced: true, capture }` due after `extract_s`.
  The condition is the worse of the record's and the cells' (Q-42-13).
- The body stays in the cockpit room. CR.0's gate and load filters already treat a forced record.

**The landings** (the crew tick).
- **Captured:** a forced extraction with a capture, whose carrier still stands on the site
  (`bay_work::site_of_carrier`) and has not fallen. The ejectee steps into its bay room and is
  `Captured` by the captor (`EJECT_CAPTURED`; the captor hears `EJECT_CAPTOR_TOLD`).
- **At a port:** otherwise, as any extraction.
- `raiders::lift_check` skips a carrier that a pending capture names (`crew::capture_pending`), so a
  raider's carrier waits for its captive.

**The deferral** (Q-42-14). `rescue_on_site` no longer keys on `CrewDuty::Recovery`. It defers when
either of these is of the character's own party or a Friend's (the character, their organizer, or a
character sharing an organization either openly stands in):
- any open DT flight in the field on the site;
- a live rig lease whose rig stands on the site's ground, other than the hull the character is
  stranded in.

It applies to asked-for and to uncaptured forced extractions alike, never to an abandoned or a
captured one.

**Gate.**
- `shard_8 crew_custody::cr4_an_ejection_under_a_hostile_hold_is_captured` (256) and `…_at_512`,
  through `custody_rig::eject_under_the_hold` with the knob at 1,000.
- `cr4_an_uncaptured_ejection_waits_for_the_wing_then_lands_at_a_port` (256) and `…_at_512`, with the
  knob at 0.
- In-source:
  - `arena::eject::tests::cr4_only_a_doomed_unit_ejects`;
  - `cr4_holding_the_cell` (the ejecting unit never contests its cell, a wing in reach does, a wrecked
    one does not, and a Neutral holds nothing);
  - `field::eject::tests::cr4_the_draw_is_seeded_and_bounded`.

**Divergences from §7's row.**
- Story (d) flies a leased escort rig, whose unit has no control section. The doomed arm it reaches is
  the powerless one, so the pilot lands Fit. The damaged-cockpit arm is pinned in-source, and the
  Injured landing at Stickney is owed to a biped story.
- The captor in the story is a player's carrier, not a director raider's. The raider lift's wait is
  in source (`lift_check`) and is owed to the next raider story.
- The "wing inside its own reach" variant is in-source (`cr4_holding_the_cell`). A field story would
  need an armed second hull of the pilot's party, which no rig lease gives.
- An eject capture writes no `capture:` standing and no `UnlawfulKind::Capture` row: the holder held
  the pilot Hostile by (a), so Q-42-21's charge does not apply, and nothing was loaded.

**Cell delta (rule #14).** Measured from the JUnit `<testcase time>` sums of PR #442's run
41891582950 against main's 41891314242 (a170600b), both shard_8:
- The four CR.4 stories add 3.45 summed minutes to `crew_custody::` (48.7 to 54.0 s each).
- Every other shard_8 test ran about 1.25× slower on that runner, CR.1–CR.3's stories included
  (`match_lifecycle::af3_…` 164 → 210 s, `unloading::sm8_…` 72 → 94 s). Doc 43 §5.8's runner
  variance, not this cut: the shard's summed minutes go 49.9 → 67.4, and 3.45 of that is CR.4's.
- shard_8's wall was 21.0 min on that run. The run's slowest cell was shard_1 at 23.9 min, which this
  cut does not touch.
## CR.5 — beds as a limit

Q-42-17 and Q-42-18. A medical room's beds limit the faster rate, never treatment itself. Q-42-18's
numbers stand: one IC day injured, three incapacitated, half at a medical room.

**The beds** (`port/medical.rs`). `medical_beds(w, port)` sums the slots of every facility holding the
`medical` capability in the port. Those slots are the authored beds: two at Ceres, four at Stickney.

**The treatment** (`field/crew.rs`).
- **Beginning** (GC.13's `treat`):
  - while the port has a bed free, a treatment begins at the medical rate, as before;
  - with every bed taken, it begins at the port's full time, told `TREATMENT_WAITING`.
  - A bed is taken while a treatment is under way at that port at the medical rate (`beds_taken`).
- **The beds pass** (`beds_pass`, on the crew tick after `treat`). At each port with beds free, the
  earliest treatments waiting at the full time take them:
  - in `started_ic` order, then by character (`bed_takers`);
  - the time left is rescaled to the medical rate (`rescaled_due`);
  - each is told `BED_FREED`.
- A bed is never taken from anyone.
- `+sortie/condition` adds "waiting for a bed" to a treatment at the full time at a port with a medical
  room.

**Gate.**
- `shard_8 crew_custody::cr5_a_third_casualty_waits_for_a_bed_at_ceres`, through
  `custody_rig::beds_at_ceres`:
  - three injured characters stand in Ceres's arrivals hall, their crew records read back at a boot
    (`driving_rig::reboot_seeding`);
  - two take the beds and the third waits, which the status line names;
  - once the first two are due, the third takes a freed bed, finishes sooner than the full time, and is
    fit again.
- GC.13's gates (`shard_1 crew_treatment`) are unchanged and pass.
- In-source: `cr5_the_earliest_waiters_take_the_free_beds`.

**Divergence from §7's row.** The third casualty's record is seeded as a stored document, not brought
home by a field. Three extractions to Ceres would add three fields' worth of run time to a cell, for a
rule that reads only the records and the port.

## CR.7 — docs

§5's amendments are now written into the documents they amend:
- **Doc 32:** Q-CF-38 and Q-CF-39 carry "amended by doc 42" notes, and §10's "Not built in CF.5:
  ejection, capture and treatment" is struck through.
- **Doc 36:** Q-36-52's non-friend half is marked superseded by CR.1 and its friendly half stands.
  Q-36-45 gains CR.5's beds. GC.5's "The abandoned crew" and lane 5's row 4 are noted as superseded.
  "GC.13: beds are authored but not a limit" is closed.
- **Doc 27:** Q-H12's rescue and capture route is marked built, and Q-F4's cap is `captive_bound_s`.
- **Doc 38:** §4 F and Q-38-22 point at doc 42, and Q-38-29's vocabulary gains the crew verbs.

Doc 42 is now Draft 0.5. It adds §7b, an as-built table with what stays owed, and records CR.6 as the
space window's (owner ruling, 2026-11-04). The help and code text items of §5 rode CR.1–CR.5.
