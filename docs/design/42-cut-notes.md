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
