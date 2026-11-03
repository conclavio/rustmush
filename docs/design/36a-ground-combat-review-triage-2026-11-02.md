# 36a — The external review of CF/GC: triage (2026-11-02)

**What an outside review of the IC field and ground-combat code found, verified claim by claim against
`d0cc7e91` (origin/main).** The review (received 2026-11-02, written against `29df04bc`, five docs-only commits
behind) claimed 32 production bugs — 10 P0, 13 P1, 9 P2 — and nine acceptance-test gaps across docs 32 and 36.
Method: five independent read-only lanes (bay work; salvage claims and tow; carrier, site and lease; world task,
kernel and the rest; the acceptance tests), each tracing every function the claim names and its callers, and
reading the cut's §7 line, as-built section and the Q-36/Q-CF rulings before judging. A claim is CONFIRMED
only on a traced path with a player reproduction; BY-DESIGN only where a ruling or as-built note chose the
behaviour, cited; REFUTED only against the line that defeats it. Line numbers are the tree's at `d0cc7e91`.

## 1. The verdict

**Two of the ten P0s stand as P0.** Of the 32 claims: 9 confirmed as written, 13 partly (a real defect inside an
overstated mechanism), 6 by design with the ruling cited, 4 refuted. The reviewer's severities were high across
the board — three "P0"s are rulings the docs already record, and the world-task P0 is the designed behaviour —
but the review earned its keep: **the lanes found five defects the reviewer missed**, two of them worse than
anything in the review (§5).

The real P0s are both exploits any player can run today:

- **P0-2** the salvor edits their own salvage-claim contract through `@contract/*` and shortens the ruled
  48-hour window to one second, taking title before anyone can contest.
- **P0-5** a non-logistics wreck loaded into a carrier's bay bricks every departure of that carrier for good —
  and the load can be pushed into a stranger's carrier.

Nothing of this is deployed (doc 37 §3.2). The fixes are grouped into four lanes in §10.

## 2. Score

| Claimed | Verdict | Re-rated | One line |
|---|---|---|---|
| P0-1 part duplicator | PARTLY / by design | P2 (owner) | the workshop is an uncapped part vendor at labour price (400 cr, not 70); no laundering, lots have no buyer |
| P0-2 `@contract/*` on a claim | **CONFIRMED, worse** | **P0** | `@contract/amend` + `/deadline` shortens the window to 1 s; the "wipe a contest" half is refuted. **Fixed** `53603c4f` |
| P0-3 Occator rig title | PARTLY | P1 | the award is refused while the lease encumbers the rig; it posts after the 28-day write-off. **Fixed** `30c2808b` |
| P0-4 `repossess` no charge | BY-DESIGN | P2 (owner) | doc 36 GC.9 "Not built here" names it; reached only via dormancy or `@coop/revoke` |
| P0-5 non-logistics hull bricks the bay | **CONFIRMED, worse** | **P0** | nothing filters the load; `carried_rig` fails every `basis`; the load can target a stranger's carrier |
| P0-6 lot lost after the draw | PARTLY | P1 | doomed orders (component in a destroyed section) and cancel-during-draw confirmed; hull-moved race by design; **fixed** lane 2 `42206731` |
| P0-7 side effects before commit | PARTLY | P2 | the Vacate→Withdrawn flip is refuted; a one-second award/contest boundary and a lost `Loaded` remain. **Fixed** `45148865` |
| P0-8 demotion erases casualties | PARTLY | P1 | parts 1–2 confirmed (unplace precedes `pose_of`; held-out rows wiped); the `wreck_bay_rigs` step refuted |
| P0-9 strip/scrap before the mint | PARTLY, **new P1** | P1 | the writes are by design; a given-up act desyncs `record.acts` and bricks the hull (§5); **fixed** lane 2 `cb4df20c` |
| P0-10 BehindSlot drains into a commit | PARTLY | P3 | the designed behaviour; only the BEHIND_SLOT_WAITING promise is broken |
| P1-1 bay wreck skips the 28-day bar | BY-DESIGN | P3 (owner) | doc 32:1321 and doc 36:693 — the lessee pays restoration up to the deposit |
| P1-2 owner acts on a hull in custody | CONFIRMED | P1 | `right_of` ranks title first; `berth` checks no carrier right; a vacated claim stows into the salvor's bay; **fixed** lane 2 `307b76ee` |
| P1-3 `TowOrders` clobber | CONFIRMED | P1 | `+salvage/inspect` spam kills a queued contest; eviction cap is 4,096 not 64. **Fixed** `f6930752` |
| P1-4 attach race drops the pin | CONFIRMED | P1 | a second tug's attach replaces the pin; the winner loads with none; no `Recovery` entry, claim refused. **Fixed** `062ac032` |
| P1-5 left-behind rigs not indexed | BY-DESIGN | — | an operational rig "stays where it stands, still yours" (CF.5); the write-off is for disabled rigs (Q-36-22) |
| P1-6 delegated crew blocked | PARTLY | P2 | the delegation gap is real and fails closed; the slots-5/6 half is refuted |
| P1-7 bay/refit blind; scrapped shell | PARTLY | P2 | both sides pin the revision; no resurrection; the scrapped shell is loose and any player can `get` it; **fixed** lane 2 `111e5d6f` |
| P1-8 `DecisionResult` guard | PARTLY | P2 | the retained-seal path latches an integrity freeze; the legacy-work path only drops the answer |
| P1-9 condition table dropped | PARTLY | P2 | real since M6 for mind hulls; pilot hulls by design; hash/power by design |
| P1-10 `Release` lacks `promoted` | CONFIRMED | P1 | a release draining after a promotion drops the casualty from the index. **Fixed** `06859c3f` |
| P1-11 raider barge lifts with no drive | PARTLY | P2 | `raiders::tick` is gated; `raid::visit → recall` is not and leaves a stale row |
| P1-12 scrap row off-by-one | REFUTED | — | `MAX_ACT_ROWS` is 32; `lot_id` has no bound and no such error |
| P1-13 bot 4-digit addresses | ALREADY OWED | P3 | doc 32 §19/§20 list it |
| P2-1 `+field/void` window | BY-DESIGN | P4 | the comment at `standing.rs:296` states it; the proposed fix would break the void |
| P2-2 pickup capacity, landholder | PARTLY, worse | P2 | there is no capacity check at all, so the hopper overfills; `landholder: None` confirmed. **Fixed** `ed7f0d24` (P2-4's half stays with lane 2) |
| P2-3 cockpit `heard`; rig panel leak | PARTLY | P3 | `heard` is empty on the cockpit; the panel is per seated pilot, no leak |
| P2-4 mint with no capacity | CONFIRMED | P2 | strip, scrap, the Mint and `@inv/dissolve` all skip capacity; GC.6's remains scrap too; **fixed** lane 2 `3803351a` (the bay and `@inv/dissolve`; `+tv/<n> scrap` of remains is still owed) |
| P2-5 berth/stow guard; wall clock | PARTLY | P3 | all true in the code, none has an effect; no `CancelFee` act exists; **fixed** lane 2 `93619a2f` |
| P2-6 `+ship/plan` gates | PARTLY | P3 | `lift_refusal`, `bay_over_line` and the queue's `departure_refusal` missing; `detachment_out` is present |
| P2-7 BehindSlot barriers | PARTLY | P4 | barriers answered `Busy` on shutdown/exit; one notice per line is by design |
| P2-8 medical quarantine | REFUTED | — | quarantine needs a work order; a medical room takes none; `workshop_at` does not exist |
| P2-9 help contradicts GC | CONFIRMED | P3 | four passages in `docs/help/field.md` (friends, cover, tow, idle demotion) |

## 3. Confirmed (with the reproduction and the fix)

### P0-2. The salvor rewrites their own claim (`mush-economy/src/contract_live.rs`)
`file_salvage_claim` writes `managed: None` with the claimant as the only party (6036–6118). `cmd_contract`'s
only kind check is `managed::at` (296–305). `amend_core`'s `author_only_live` (3373–3379) admits the Active
claim and turns it back into a Draft; `@contract/deadline` then sets any positive number of real seconds
(`commands.rs:4286`). `salvage_claim_hull` ignores the state (5906–5940), `claim::tick` awards at `now >= end`
(`claim.rs:1354–1360`), and `award_salvage_title`'s `_` arm accepts a Draft (6255–6262).
Repro: `+salvage/claim #H go` → `@contract/amend #N` → `@contract/deadline #N=1` → next tick, title to S.
Path B (`/dispute` then `/agree =refund`, unanimous alone) withdraws the claim, dropping bare-title contests.
Fix: refuse every non-staff mutating `@contract` switch when `salvage_claim_hull(c).is_some()`; make the award
require Active/Expired; have `claim::tick` use `max(end, file.deadline_real_s)`.
**As built** (`53603c4f`): `cmd_contract` refuses every non-draft switch on a salvage claim for a non-admin
(`SALVAGE_CLAIM_LOCKED`); `award_salvage_title` runs only on Active past its effective end or Expired, and
refuses an organization-titled hull; `claim::effective_window` is the economy's end or the filing's deadline,
whichever is later, used by the tick, `open_window` and the list. Tests
`review_36a::p0_2_the_salvor_cannot_amend_or_shorten_their_own_claim` and
`p0_2_the_salvor_cannot_dispute_and_agree_their_claim_away`.

### P0-5. A non-logistics hull in the bay (`port/vehicle.rs`, `surface/tow.rs`, `port/voyage.rs`)
`tow_cmd` takes any indexed unit in reach (tow.rs:1812–1839); `load_cmd`, the Load prepare (1317–1356) and
`complete_load` never check for a rig or for logistics; `place_in_bay` puts the hull in the bay room (2205).
`rigs_in_bay` then lists it (vehicle.rs:1033–1070), `carried_rig` needs `spec.logistics` (1200–1205) and every
combat unit has none, so `basis` → `dark()` on every departure, plan, `+chart` and the space tick
(voyage.rs:3355). No `+tv` slot can launch it out; `+bay/berth` needs a port the carrier cannot reach;
`repossess` errors "no readable live lease record". **Worse:** `load_cmd` loads into the nearest carrier in reach
when the tug's own ramp is not (tow.rs:1964–1970), and nothing checks who owns it.
Repro: a field leaves a `line_tank` wreck; `+tv/1 tow #W go`, drive to a ramp, `+tv/1 load go`, `+ship/lift`:
PLAN_DARK forever. Fix: `carried_rig` gives a logistics-less hull `dry_kg = dry_mass_kg_for_spec`, no hopper,
no goods (as relay cargo does, vehicle.rs:1158); and a load goes only into the tug's own carrier or a Friend's.

### P0-3 → P1. The Occator rig after the write-off (`port/tv.rs`, `surface/claim.rs`)
The lease is an economy lease with the rig as premises, so `TransferTitle` is refused while it stands
(`engine.rs:1309`); the award retries every tick. `write_off_left_out` (tv.rs:4719–4801) then ends the lease
and hands the rig to the shed but never touches the custody row, and `claim::tick` checks `written_off` only
at filing (claim.rs:1055). The next tick posts the title to the salvor: Q-36-29 broken, the rig gone from the
pool. Fix: a written-off hull in custody closes the contract and drops the custody row; refuse an award on an
organization-titled hull.
**As built** (`30c2808b`): `claim::tick` walks every custody row; one whose hull `tv::written_off` reports
closes its contract (Filed) or its owed seam and settles `HandedBack`, which drops the row; an Award on an
organization-titled hull is never attempted (and `award_salvage_title` refuses it). Test
`review_36a::p0_3_a_leased_rig_written_off_in_custody_goes_home_and_is_never_awarded`: title stays Occator's,
the rig is back in the shed, the contract is terminal.

### P0-6 → P1. Doomed bay orders and cancel-during-draw (`port/bay_work/`)
`take` never dry-runs `jobs::work` (mod.rs:686–810); a component in a destroyed section reads `Destroyed`
(condition.rs:503) so the quote offers `Replace` and a selector can pick it without `rebuild`
(jobs.rs:214–235). `finish` owes the Draw first (mod.rs:913–930), the Draw consumes the lot (act.rs:690–730),
`complete` fails `HostSectionNotFunctional` and `close_cancelled` refunds the fee only. `cancel_order` checks
neither `record.owed` nor `draw_act` (mod.rs:845–853), and any right holder can cancel. Fix: dry-run the work
in `take` and again in `finish`; `cancel_order` refuses while `owed.is_some()`.

### P0-8 → P1. Demotion drops carried casualties and held-out rows (`field/close.rs`, `surface/tow.rs`)
`demote_units` unplaces an Irrecoverable tug inside its loop (close.rs:296–305) before `set_down` (367);
`set_down` then finds no pose for the tug and `continue`s (tow.rs:2335–2337), so the carried casualty — held
out of the field at admission (admit.rs:337–341) — is in no list, and `SiteRecord::demote` replaces the index
(site_record.rs:732). The hull stands with no row: untowable, never written off, the lease stuck at RIG_OUT.
Any hull held out under IC custody with an index row is wiped the same way. Above 128 rows `demote` refuses and
the site stays promoted (staff `@tacsim/site demote` still works). Fix: `set_down` falls back to the casualty's
own pose, or runs before the remains unplace; `demote` keeps rows for hulls that were never in the field.

### P1-2. The prior owner acts on a hull in custody (`port/bay_work/mod.rs`)
`right_of` ranks title, grant and lessee before salvor (299–311) and custody leaves title untouched (doc 36
GC.5 "custody is not title"); `berth` checks no relation between actor and carrier (1181–1215). O at C14 runs
`+bay/berth #H` out of S's bay, then `+bay/strip`/`scrap #H` mints into S's hold (`source_carrier` reads
`berth.carrier`, 505–516) or `+bay/cancel` kills S's order. After a vacate, O's `+bay/stow` puts H back into
S's slot (1249–1265). `holds_title` (258–262) is weaker than `bay_hull_title_stands` (no scope, stolen,
lifecycle, encumbrance). Fix: live custody answers at most Salvor for everyone else; `berth` requires a right
over the carrier or the custody; rebind `berth.carrier` on vacate; use `bay_hull_title_stands`.

### P1-3. `TowOrders` clobber (`surface/tow.rs`, `surface/claim.rs`)
One `(actor, order)` per key (tow.rs:520, 1748); claim orders are keyed by hull (claim.rs:951–962); the
offer is projected only at drain, after other commands ran; `put_order` runs before the engine's back-pressure
refusal. A salvor aboard the carrier spams `+salvage/inspect #H` near the deadline and the challenger's queued
contest dies Unauthorized after CONTEST_FILED was printed. Fix: key by `(actor, key)` or the idempotency key;
remove the row in prepare; evict oldest, not lowest id.
**As built** (`f6930752`): `TowOrders` is keyed by `(actor, rig)` with a write stamp; past `MAX_POINTERS` the
oldest stamp goes, never the lowest id. The row cannot be removed in the prepare (it has no World); the
per-actor key makes that unnecessary. Test
`review_36a::p1_3_the_salvors_inspect_spam_does_not_kill_a_queued_contest`.

### P1-4. The attach race drops the pin (`surface/tow.rs`, `surface/claim.rs`)
The Attach prepare probes only active tows (1227); the pin is written at the start (1248); `claim::pin` keeps an
earlier pin only for the same tug (claim.rs:210–216). Tug B's attach during A's 900 IC-s act replaces A's pin;
A lands first and loads with `pin == None`: no `UnlawfulKind::Recovery`, no witnesses, `+salvage/claim`
answers NO_TITLE for good. Fix: write the pin when the attach lands (`Attached`, 1272), or refuse a second
attach start while one is in flight.
**As built** (`062ac032`): the second path. `Tows::attaching_elsewhere` refuses a second tug's attach start
while another tug's attach act stands on the casualty, at the command (`ATTACHING_ELSEWHERE`) and at the
prepare (`tow_taken`). Test `review_36a::p1_4_a_second_tugs_attach_waits_and_the_first_tugs_pin_stands`:
the first tug's pin stands, the foreign load logs `UnlawfulKind::Recovery`, `+salvage/claim` is not NO_TITLE.

### P1-10. `Release` has no `promoted` check (`surface/tow.rs:1280–1315`)
Every other player arm refuses under `promoted`; Release does not, and `field_gate` ran at command time. A
release draining after a hostile mark promotes the site retires the tow, the casualty is neither fielded nor
carried, `set_down` skips it, `demote` drops it. Fix: the one-line guard.
**As built** (`06859c3f`): the guard, first in the arm. Test
`review_36a::p1_10_a_release_draining_after_a_promotion_waits`: the release is refused FIELD_OPEN and the tow
stands.

### P2-2, P2-4. No capacity checks (`surface/salvage.rs`, `surface/tow.rs`, `port/bay_work/strip.rs`)
`pick_up` → `transfer_many` has no capacity rule, so the hopper overfills; PICKED_UP prints before the prepare;
`landholder: None` on `UnlawfulKind::Pickup`. Strip, scrap, the Mint and `@inv/dissolve` all skip
`PortCapacity`; GC.6's remains scrap too. Fix: check free capacity in command and prepare; carry
`landholder` in `PickupFacts`; refuse strip/scrap before the plan when capacity would refuse.
**As built, P2-2** (`ed7f0d24`): `pickup_cmd` merges the hopper's compartments (the mining plan's rule) and
refuses a crate that would overrun any limited measure (`NO_ROOM`, with the kg free and the crate's kg);
`PickupFacts` carries the merged limits, each accepted good's per-unit figures and the landholder; the prepare
re-sums the hopper's pools against the limits (`pickup_no_room`); `PICKED_UP` says "queued"; the evidence row
names the landholder. Test
`review_36a::p2_2_a_crate_the_hopper_cannot_take_is_refused_and_the_landholder_is_named`. P2-4 (strip, scrap,
the Mint, `@inv/dissolve`) stays with lane 2.
**As built, P2-4** (lane 2, `3803351a`): strip and scrap refuse before any plan when `PortCapacity` would refuse
the bulk lots; the Mint's prepare checks the hold plan its offer carries; `@inv/dissolve` consults the new
`mush_inventory::PoolCapacity` seam (tacsim's `PortCapacity`, wired in the composition). Tests
`bay_36a::b36a_5_a`..`5_c`. Still owed: `+tv/<n> scrap` of remains (`surface/claim.rs`) mints with no room check.

### P2-9. The help file (`docs/help/field.md`)
"Organizations and factions do not make friends yet" (93–94; GC.10), a wreck "gives no cover yet" (200–201;
GC.2), "there is no tow yet" (205–206; GC.3/4), and parked armed parties "keep the field open" (54–58,
98–101; GC.2's idle bound). Fix: rewrite the four passages.

## 4. Partly confirmed, smaller (the defect that survives)

- **P0-7**: a contest at `now <= end` and an award at `now >= end` share one second; the title posts, the
  contest lands, the next tick vacates — claim void, title with the salvor. Contest only while `now < end`,
  and drop the custody row in the award's bundle. A `Loaded` order lost to P1-3 while a foreign carrier lifts
  leaves the casualty in the departed bay, indexed and in tow.
  **As built** (`45148865`): `open_window` and the contest prepare admit a contest only while `now < end`, so
  the window's last second is the award's; the tick's `Loaded` facts fall back to the bay's own facts when the
  carrier is no longer berthed, so a lost finish still completes the load. Tests
  `review_36a::p0_7_the_windows_last_second_belongs_to_the_award_not_a_contest` and
  `p0_7_a_lost_loaded_finish_still_lands_after_the_carrier_has_gone` (the carrier's lift is a staff unplace, as
  the stand-in).
- **P0-10**: a line told BEHIND_SLOT_WAITING ("not refused") restarts its 250 ms in `defer` and can then hear
  CommitTimedOut. Gate `behind_ready` on no pending commit, or carry `told` into `defer`, or reword.
- **P1-6**: `+tv/board` lists only `tv_leases_of(actor)` (boarding.rs:188) and the detachment surface path
  refuses `lease.pilot != actor` (sortie/surface.rs:118), so a delegated `OperateCrew` grant can be held but
  never used. Accept any actor with a current grant, keeping `lease.pilot` as principal.
- **P1-7**: after scrap `berth = None` so `held()` is false and the shell is a plain `Thing` any player can
  `get`; `hull_named` matches it by name. Keep a scrapped hull held; skip scrapped hulls in `hull_named`.
- **P1-8**: `decision_result_armed` (mode.rs:547) lacks `retained.is_empty()`; a completion in the re-emit
  state stages, `flush_prepared_action` rejects, `abort_without_receipt` latches IntegrityFrozen. Add the
  guard; park the completion when legacy work is pending, as command submits do.
- **P1-9**: `join_in_place` (join.rs:143) and `new_with_minds` (state.rs:4717/4904) drop the installed
  `ConditionTable`; `adopt_mind` never restores it, so a mind hull's native profile rows are gone until a
  profile switch. Since M6 (`34a22ac1`), moved unchanged by GC.1b. Apply the profile rules for mind units
  after `admit_unit`; rebless if the hash folds conditions.
- **P1-11**: `raid::visit` stops only for a Wreck (raid.rs:124); a Casualty barge with a downed drive goes
  `under_way → recall → raiders::lift`, which unplaces it with no `carrier_loss` check and leaves its row.
  Breaks doc 36's "a barge whose drive is down stays on the ground".
- **P2-3**: `enrich_cockpit` passes an empty `heard` (picture.rs:592); the cockpit never shows "(a sounding)"
  though doc 32 §9 wants one picture. Build `heard` as `site.map` does.
- **P2-6**: `plan_block` and `plan_queue` skip `lift_refusal`, `bay_over_line` and (queue only)
  `departure_refusal`; the dry run reads green where the filing refuses.
- **P2-7**: both `behind.drain_all()` arms answer a waiting `Barrier` with `Busy` where `defer` settles it.

## 5. What the review missed (found by the lanes)

1. **Bay-act ordinal desync bricks the hull (P1).** The record bumps `acts` when it owes an act
   (mod.rs:913–914, strip.rs:81/171/218); the prepare accepts only `ledger.acts + 1` (act.rs:678–684); an
   act given up after `MAX_ACT_TRIES` leaves `record.acts` one ahead and nothing re-syncs it. Every later act
   on that hull is refused Conflict three times and dropped: every draw cancels its order, every strip or scrap
   destroys the part or hull and mints nothing. A player triggers it by selling an order's munitions before the
   due time (the Draw is refused "hold does not carry", act.rs:697). Fix: roll `record.acts` back to
   `landed()` on give-up, or accept any `act > ledger.acts`; apply strip/scrap plans only once the mint lands.
   **Fixed** lane 2 `cb4df20c` (the roll-back; `strip::land_mint`).
2. **A wreck can be loaded into a stranger's carrier** (P0-5's rider): `load_cmd` picks the nearest carrier
   in reach and no arm checks the carrier's owner.
3. **The one-second award/contest boundary** (§4, P0-7).
4. **`+bay`'s allowlist stanza is wrong** (P2): `legacy-handlers.toml:49–54` says the tick submits the owed
   act, but `/strip`, `/scrap` and staff `vacate_claim` call `settle → act::submit → submit_authoritative`
   inside the handler, against `submit_authoritative`'s "perform no world mutation" contract
   (mush-core module.rs:6438). The reviewer blamed `take` and `cancel_order`, which do not. **Fixed** lane 2
   `23d0afab`: the tick alone submits; the stanza now says so.
5. **GC.6's as-built overclaims**: doc 36:3153 says the rights table reads O's Salvor right before the award;
   the test asserts only Title after, and no `+bay` act by O exercises the right.

## 6. By design (the ruling)

- **P0-1** GC.8a as built: "the workshop supplies the part inside the replacement labour… a part price is a
  GC.8b or economy question"; GC.8b: "a replaced part is still the workshop's". The price is 400 cr × 1.2 at
  Ceres plus 4 IC h, not 70 cr; the stripped lot keeps its condition; no vendor buys `tacsim.part.*`. What is
  real: an uncapped part vendor at labour price — the owner's call under the provisional-economy rule.
- **P0-4** doc 36 GC.9 "Not built here": "A charter repossession… still releases the deposit within the term
  without a damage charge… an open question for the economy owner." Reached only by dormancy or
  `@coop/revoke`; a lessee can let a charter go dormant to recover a full deposit on a damaged rig.
- **P1-1** doc 32:1321 (the write-off is for a rig "wrecked out of its carrier's bay"), doc 36:693 (a
  recovered rig is returned and pays the damage charge). The bar is skipped; the deposit is not.
- **P1-5** CF.5's hint (voyage.rs:461) and doc 20 D-13; Q-36-22 writes off disabled rigs only.
- **P2-1** `standing.rs:296–299` states the gap; doc 36 GC.10 records it.
- **P0-10, P2-7 (notice)** defer_queue.rs:484–489 and doc 36 §8 l.1170–1176: each entry is "handled as a
  fresh message", "hears, once".
- **P1-9 (pilot hulls, hash, power)** conditions.rs:20–33; doc 36 §11 l.1601: "the task puts it online, as
  `spawn_field` puts every admitted unit".

## 7. Refuted

- **P1-12**: `MAX_ACT_ROWS = 32` (act.rs:62); `lot_id` only hashes `{hull, act, row}` (279–290).
- **P2-8**: quarantine means a live work order holds a slot on a facility with no capacity book
  (mush-industry store.rs:455); a medical room takes no work orders; `workshop_at` does not exist.
- **P0-7's flip**: `has_vacating_contest()` is tested before the Cancelled branch (claim.rs:1354) and
  `close_salvage_claim` is idempotent on a terminal contract (contract_live.rs:6175).
- **P0-2's contest wipe**: the same line; a vacating contest still settles `Vacate` and is logged.
- **P0-8's `wreck_bay_rigs` step**: it applies wreck plans to bay rigs and adds no rows (close.rs:383–414).
- **P1-6's slots 5–6**: `flight.slot` goes into the flight's own epoch (`Flight::validate` 1..=6); surface
  acts use `selected.lease.slot`; `RigRecord::check` never sees it.
- **P1-7's resurrection and concurrency**: both sides pin the unit revision (mod.rs:868;
  bay_refit/authority.rs:202/253); `scrap_hull` leaves nothing a refit can rebuild.
- **P1-11's `raiders::tick`**: it reads `RefusesLift` from the snapshot and skips the lift (raiders.rs:910–921).
- **P0-10's double count**: `Wake::Behind` never calls `command_deferred()`.
- **P2-3's leak**: frames are per seated pilot (arena_bridge.rs:3189–3244); there is no focus path.
- **P2-6's `detachment_out`**: `bay_gate_with` runs `bay_gate_leaving`, which runs it (voyage.rs:2737, 2873).
- **P0-9's undock and back-pressure triggers**: the Mint reads no docking state; `settle` returns early while
  `authoritative_pending`.
- **P1-3's cap**: `MAX_POINTERS = 4,096` (tv.rs:2215), not 64.

## 8. The acceptance tests

The house rule (doc 36 §7 l.820): every cut runs through the production command paths; a seed named in the
cut's as-built section is allowed when the thing the cut claims is still reached by player commands. Harness
facts: `launched()` lands on the plain path; `land_composed` has **exactly one caller** in the tree
(escort_lease.rs:537).

| Test | Allowed | Gap or dead |
|---|---|---|
| field_story (CF.7/GC.0) | funding (7,000 cr, not 700,000), the bystander rebuild, seeded sections (G-6), `@field/raiders/withdraw` | G-7 under a DT roster (doc 32:1753 records the stand-in); "the miners hold" (907–912) cannot fail — the prospector has no armament and the bystander's magazines are seeded empty |
| field_gc3 | `set_section` for the drive and the loss table | **DEAD**: `launched()` is plain, so `PrepareCtx::world()` is `Err`, `black_box::append` is `Ok(false)`, `unrecorded` is always true and the `else` at 235–243 asserting `IcCloseKind::Destroyed` never runs; the header's claim at :7 is unproven, as doc 36:2515 admits |
| field_gc4 | `powerless` | the `RecoveredToBase` box line (same root cause) |
| field_gc5 | site seeding, `seed_pool`, `move_to` into the casualty | a pilot who boarded by command being stranded |
| field_gc6 | `@asset/transfer`, `seed_typed` tenure, staff remains | `emit_world_event` Frozen/Resumed fires the economy's events while the clock keeps ticking — `@tacsim/time freeze/resume` exists and bay_gc8 uses it; `right_of` called directly, and doc 36:3153 overclaims the Salvor read |
| field_director (GC.11) | the site seeding | **the header is wrong**: "no staff hand anywhere" (:5) against `seed_pool`, `armed`, `seat_npc`, `rationed`, `fragile` at :170 (a staff wear on the raider during the raid), `raiders::seed`, `unplace`; and :12–14's "goes home once it has lost half" is false — the guard has 0 rounds and the raid leaves on `raid_field_s` |
| bay_gc8 | `@econ/mint`, `seed_hold`, `damage`, `put` | `vacate_claim` has **no production caller** (strip.rs:278 "only a staff fixture calls it"): hot parts are an unwired seam, not a test shortcut; story (c) does not chain from (a) |
| crew_treatment | the control section, `move_to` | `IndustryStore` mutated by hand instead of closing the facility through its act |
| escort_lease | — | `lease_escort` drains and discards the grant submission (:90–99); `board_escort` stands in with `place_interior` + `assign_station`; `gc0_the_harness_lands_the_org_unit_grant` asserts neither the record nor a re-run of `+tv/1 board`; doc 36:1154 "GC.1's gate is the first to fly through it" is false |

## 9. Owner questions raised

1. **A part price** (P0-1): keep the workshop as an uncapped vendor at 400 cr, or price `Replace{lot: None}`.
   Lane 2 (2026-11-02) changed no code here. The question, with today's numbers: a `replace` with no lot
   costs 400 cr × 1.2 at Ceres (480 cr) and 4 IC h, and the workshop supplies the part inside that labour
   without limit — keep that as the one source of parts, or price the part?
2. **Repossession damage charge** (P0-4): already doc 37 §6.3; the dormancy path makes it a leak.
3. **Does the 28-day bar follow a rig that came home wrecked** (P1-1)?
4. **Delegated crew** (P1-6): fail-closed today; is a delegate meant to board?
5. **The scrap yield against the hold** (surfaced by P2-4's gate, lane 2): `scrap_hull` yields
   `SCRAP_KG_PER_STRUCTURAL_POINT` (100 kg) a surviving structural point, so a whole escort (2,990 points)
   is 299 t of `structural_alloys`, against a Dray-P hold rated 86 t by mass — and a bay slot that takes the
   same escort at 20 t. With the gate, `+bay/scrap` of a whole escort off a Dray-P is always refused; a worn
   one (a quarter of its points) fits. Is the yield 15× too high (a mush-tactics constant), or should a
   workshop scrap spill to the port's stock rather than the carrier's hold?

## 10. Fix lanes

Each is one PR against main, tested through the production command paths, with a regression test per item
that reproduces the review's steps as a player would type them.

- **Lane 1, salvage claims and tow** (`surface/claim.rs`, `surface/tow.rs`, `surface/salvage.rs`,
  `mush-economy/src/contract_live.rs`): P0-2, P0-3, P0-7's boundary and lost-`Loaded`, P1-3, P1-4, P1-10,
  P2-2. **Built 2026-11-02**, one commit per item (`53603c4f`, `30c2808b`, `45148865`, `f6930752`, `062ac032`,
  `06859c3f`, `ed7f0d24`), each with its test in `crates/tacsim/tests/shard_1/review_36a.rs`.
- **Lane 2, the bay** (`port/bay_work/*`, `legacy-handlers.toml`): the ordinal desync (§5.1), P0-6, P1-2,
  P1-7's shell, P2-4, P2-5's `at_ic` and `PendingSalvageClaim`, the stanza; a note closing P0-1's question
  one way or the other. **Built 2026-11-02**, one commit an item: `cb4df20c` §5.1, `42206731` P0-6,
  `307b76ee` P1-2, `111e5d6f` P1-7, `3803351a` P2-4, `93619a2f` P2-5, `23d0afab` the stanza; P0-1 stays
  the owner's (§9.1). Tests: `crates/tacsim/tests/shard_1/bay_36a.rs`. As built: doc 36, "36a lane 2, the
  bay, as built".
- **Lane 3, carrier, site, kernel, world task** (`port/vehicle.rs`, `surface/tow.rs` load/set_down,
  `field/close.rs`, `surface/raid.rs`, `port/boarding.rs`, `port/voyage.rs`, `arena/join.rs`,
  `mush-server/src/world_task*`): P0-5 with the stranger's-carrier rider, P0-8, P1-11, P1-6's delegation,
  P2-6, P1-8, P1-9 (rebless only if the hash moves), P2-7's barriers, P0-10's wording.
- **Lane 4, tests, help and docs**: field_gc3's dead branch (a `launched_composed`), field_director's header,
  escort_lease through `land_composed`, field_gc6 under `@tacsim/time freeze`, field_story's "the miners hold",
  `docs/help/field.md`'s four passages, P2-3's `heard`, doc 36's GC.6 and GC.1 overclaims, and this doc's
  §9 answers once ruled.
