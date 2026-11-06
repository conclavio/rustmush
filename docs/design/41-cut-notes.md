# 41. Cut notes — restoration and the salvage loop

One `## RS.n` heading per cut of [41-restoration-and-the-salvage-loop.md](41-restoration-and-the-salvage-loop.md)
§6, in the order they landed: what was built, the story that proves it, where the PR departed from the row,
and the cell's cost (doc 43 rule 2.3 #14).

## RS.1 — the tender (2026-11-03)

Row: §6 RS.1; Q-41-1..4, Q-41-27, Q-41-29, Q-38-16, Q-36-35/36. Lane v21 opened here (relay v12), on
TS.8's one strip chain.

**Built.**

- **The definition** (`definitions/logistics.rs`). `logistics.tender { stores, reach_cm }` (the stores
  one of the unit's compartments and accepting `stow.bulk`; reach 1..10,000 cm) and
  `logistics.recovery_profile: standard|recovery_optimized|heavy` (default `standard`, omitted). Both
  are read by tacsim's port and surface code only, never by `bind()`, so no `DefinitionRef` moves.
- **The cap on the profile** (mush-tactics `recovery.rs`, Q-41-27). `TowRigProfile::cap_permille`:
  `Standard` and the new `Heavy` 2×, `RecoveryOptimized` 3×; `tow_mass_ratio_permille_for(tug,
  casualty, profile)` beside the old 2× function, read by the speed factor and the inspection; `Heavy`
  takes the optimized speed. `attach_request` reads the tug's profile off its definition
  (`TugFacts.profile`, `tow::profile_of`) instead of hard-coding `Standard`.
- **The laden count** (`tow.rs` `attach_facts`, Q-41-29; doc 36's owed hopper change). The casualty
  counts dry mass plus `tow::carried_kg`: a rig's hopper or stores (`hopper_load_of`), a hull's
  payload as the passage planner weighs it (`voyage::basis`: hold, tank, locker and every rig in its
  bay, laden). What will not read counts nothing, logged. The refusal line now reads "at 13 t laden it
  is more than the 12 t a 6 t tug can tow".
- **`tacsim.unit.tv_tender`** (`content/units/tv_tender.json`, pack `tacsim.pack.tv_tender`,
  `EXTENSIONS_TENDER`, fielded last): the prospector's chassis, its hopper made a `stores` bin of
  2.7 t / 4.8 m3 `stow.bulk`, 7 t dry, unarmed, `recovery_optimized`, `tender { stores: "stores",
  reach_cm: 1500 }`. Listed LOOKLESS (the prospector has no look yet either).
- **Occator's lease row** (lane v21): `vehicles.designs` gains `tender`, pool 2, `OWP-TD-`,
  1,800 + 7,000 for 168 IC days, the licence gate, the battery. `strip_tender` heads TS.8's chain
  (`tests/support/lane_chain.rs`).
- **The stores and the bale** (`surface/tender.rs`; the `stores` op of `tacsim_surface_tow`).
  `RigRecord.bale` (IN.1's `Bale`, the twin of `BayHull.bale`). `+tv/<n> stores` shows the stores and
  the bale; `+tv/<n> stores load <qty> <good> [go]` moves goods from the carrier's hold into the
  stores, in the bay only, re-checked against the stores' limits in the prepare; `+tv/<n> stores open
  [go]` debits a tonne of alloys and a tonne of fittings into an unopened bale.
- **The `tend` op** (`tow.rs` dispatch, `tender.rs` body). `+tv/<n> unjam <unit>[=<part>]`,
  `stabilize <unit>[=<section>] [<points>|all]`, `rearm <unit>[=<part>]`, each `[go]`: the preview
  names the work, the draw and the time; `go` files one timed act on the site's record
  (`TowStage::Tend`), so the tender drives or tends, never both. Reach is the tow's cell. A target
  carried in tow is refused. Time is the tariff's base at `TENDER_TIME_FACTOR` 3 (unjam 6 IC h, a point
  108 s, a rearm 1.5 IC h). The draw: an unjam the library's 10 kg of fittings, a point the tariff's
  10 kg (the constant until IN.8; alloys none), both through `Bale::draw` from the stores; a rearm
  whole units of the mount's munitions good, what the stores hold, up to the magazine. No money moves
  (Q-41-4).
- **`register_tender`** (`tender.rs`), fed from the compartment: spare parts = the bale's fittings and
  the stores' whole tonnes, rounds = the stores' units of the mount's good, no charge. The finish runs
  mush-tactics' `field_unjam_component`, `field_stabilize_damaged_section` or
  `field_resupply_ammo_and_charge` on the target's unit, applies the change (`apply_plan`) in the tick,
  then submits `tended`, which debits the stores and writes the bale (the load's `place_in_bay`
  precedent; `TendApplied` in RAM keeps a resubmitted finish from landing twice on the unit).
- **The field** (Q-41-3, Q-CF-30). No tend starts on a promoted site. An act in flight when a field
  opens is paused (`paused`: its IC time left) by the tick and resumed on the tick that finds the site
  settled. FR.5's coupled withdraw (Q-40-25) is not built here.
- `+tv` takes the four words (`ACT_WORDS` 24); the bot's `salvage::in_vocabulary` admits them; `help
  field` gains the tender and the laden tow; doc 36's two owed lines are marked closed.

**Story.** `crates/tacsim/tests/shard_10/field_rs1.rs`, `rs1_ahuna_…` (256) and `rs1_urvara_…` (512),
one body: the captain leases the escort and the tender, stocks the stores from the hold, opens the
bale and launches both; the escort, jammed, 40 points down and its magazine at 10, stands beside the
tender; `+tv/2 unjam`, `stabilize … all` and `rearm` are each filed, paused by a stranger's
`+field/mark`, resumed when the field settles and landed: the cannon clear, the hull whole, the
magazine refilled by what the stores held, the bale at 590 kg of fittings. Then the stranger's
prospector, 7 t of ore in her hopper and no power: its own 6 t rig is refused on her 13 t laden (her
dry 6 t is within the 2×), and the tender's 3× coupling takes her. Local Windows debug: 73 s and
74 s. Unit tests: `surface::tender::tests::rs1_*` (the grammar, the time and the draw, the
registration), mush-tactics' `h2a_universal_2x_tonnage_cap_and_speed_scaling` (the profiles' caps),
mush-bot's `rs0_the_salvage_lines_are_closed` (the tender's lines). Re-run green: `field_gc4`,
`field_gc5`, `field_carrier`, and every lane, relay, content and census pin the bump moved.

**Divergences.**

- **`+tv/<n> stores load`** is not in the row: nothing else fills a rig's compartment from the hold,
  so the stores could not be stocked. It moves pools only (bulk goods; a part lot stays in the hold).
- **The stores hold 2.7 t, not 2 t** (provisional, under the economy rule): a tonne of each line and a
  magazine. The plan's 2,000 kg held the two tonnes and nothing else.
- **The bale is not returned to the stores** at the rig's return: kilograms cannot go back into a
  whole-tonne pool, so the bale rides the rig record, as the hull's bale rides the hull, and the next
  work draws it first. `stores open` refuses an open bale.
- **A rearm is the tariff's per-mount 1,800 s × 3**, not the plan's "ten minutes for 30 rounds": the
  tariff has no per-round loading time.
- **The work lands on the unit in the tick, before the finish's transaction** (the load's precedent):
  a stores shortfall at the finish draws what is there and lands `tended:short`.
- **A tend start creates the site record** when no field has written one yet, at the site's side
  (`TendFacts.side`; the tow never needed it, since a casualty always comes of a field).
- **The bay room check** now sees the casualty laden (the library's own load check reads total mass);
  the bay's contents are still counted dry, as before.
- The census pin (`mill::sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter`) is re-blessed for
  the tender's two `.debit(` calls, matter spent or held in the bale, never made.
- **The tender shows in Occator's readiness line**, as Q-41-1 means (Occator leases it): `shard_6`'s
  `vehicles::sm3a_the_apply_lays_up_occators_rigs_and_every_hulls_bay` pins "tender: 2 of 2 made; 2 in
  the shed, 0 leased.", and `sm3a_the_rig_pools_preflight_refuses_each_recorded_breach` the port map's
  12 rigs (8 + 2 escorts + 2 tenders).
- **`field_director::gc11_a_scheduled_raid_arrives_declares_and_leaves_with_its_losses`: two
  branches, by owner ruling.** On conclav.dev (three runs, identical) the guard's mind, which sorts
  targets by `(range, id)` (`arena/mind.rs`), took the raid's carrier first under this lane's
  object ids and wrecked it at 47 s; `raiders::lift_check` skips a wrecked carrier before
  `withdrawal_due`, so the raid never turned for home and its hulls fought to the end. The owner
  ruled on 2026-11-04: "a raider unit that loses its carrier would fight to the death since they
  wouldn't have enough life support to scatter or wait for a pickup .. surrender potentially
  depending on circumstances and personality .. fighting to death is easiest for now since we don't
  have surrender yet". `field/raiders.rs` is unchanged (FR.3's, and #400 does not touch it); the
  gate asserts whichever branch the record shows: (a) the carrier stands, the raid turns for home
  at half losses and lifts with what stands, its losses where they fell; (b) the carrier is a
  wreck on the site's record, no hull withdraws or boards, and every hull is a wreck there or stands
  on the ground when the field closes.
  Surrender by circumstance and personality is a future pointer in doc 42 §8. The earlier fixture
  change stands too: It wore one raider hull and asserted the guard took exactly that one. The
  field's seed mixes the world's instant and a process salt (`match_::seed_for`), so on #397's first
  run the guard's first rounds at the worn hull missed, it turned on the whole hull and fell, and the
  raid stayed. Both hulls now come home worn, and the gate asserts the raid declares, loses at least
  one hull, leaves with what stands (aboard) and leaves its losses where they fell. Nothing of RS.1
  reaches the raid's leaving (it boards by the field's withdraw and lifts on the bay gate, still
  counted dry); the extra boot objects only moved the draw.

**Cell cost (rule #14).** Pending the PR run's JUnit; filled in on the PR.

## RS.3 — the claim's close (2026-11-03)

Rulings: Q-41-20..23, Q-41-28, Q-38-19, Q-S13, Q-S14; round-2 items 41-1, 41-8, 41-10, 41-12a (§4b).

**Built.**

- **`claim_window_s` from config.** `TacsimRecoveryCfg.claim_window_s` (`mush-config`, default 172,800 real
  seconds) is mapped by `mush-server/src/config_ext.rs` and refused at boot outside one second .. a week; the
  stories keep setting `TacsimRecoveryConfig` directly.
- **Who answers for a title** (`surface/claim.rs` `answerers_of`). A character's title is theirs; an
  organization's is its live rig lessee first, then its active members and its live grantees, unfiltered by
  capability (Q-41-28: Occator's only grantee is its lessor agent, an Npc). The pin carries the list
  (`TenurePin.answerers`, written only for an org title, so a character's pin prints as before) and
  `answering` is its first. Filing tells everyone who answered at the attach and everyone who answers now,
  then the landholder; the notice is mailed one letter a recipient, so a post that refuses an agent costs no
  one else theirs.
- **The narrowed award.** `mush_economy::award_salvage_title` refuses an organization's hull only when the
  economy sees a lease or charter on it (an `assignment`, a `Lease` or `Collateral` encumbrance); tacsim's
  tick refuses an org hull under a live rig lease or a programme charter (`leased_or_chartered`). An org hull
  with neither is awarded (story: Occator's unleased spare goes to its salvager).
- **`+salvage/authorize`** (Q-41-21). `<hull>=<who> [until=<UTC seconds>] [go]` by whoever answers for the
  title submits Org's own `unitgrant_issue` for `unit.recover_load` on the hull (`runtime::submit_from`, the
  personal-title parent). `<site>/<block>=<who> [until=<IC seconds>] [go]` by the block's live claim holder
  writes a `LandGrant` on the site's record through the tow action's `claim` op (`ClaimOrder::AuthorizeBlock`,
  keyed by the block's place), bound to the claim's holder and registration instant so a new holding voids it.
  Both are read at the attach into the pin (`owner_grant`: the grant id and its grantor, found by Org's
  `Authorize` and `List` reads as the recoverer; `landholder_authorized`), and `open_claim` feeds them to
  `evaluate_recovery_entitlement`. The load's unlawful-recovery log honours either.
- **The contest kinds** (Q-41-23). `+salvage/contest <hull>[=recovery|procedure|right] [go]`; the bare form
  keeps the automatic choice (any answerer may contest on bare title). `procedure` is for whoever answered for
  the title and rests on `procedure_defect`: an answerer (or the landholder) not told, a pin missing or another
  tug's, the box's last `recovered_to_base` line naming another tug than `custody.recovered_by` or another base
  than `custody.carrier` — never the claimant; an empty box is no ground. `right` rests on `right_of_grantor`:
  the pin's `owner_grant`, filed only by its grantor and only when the recoverer is the claimant, carried as
  `SuperiorSpecificRight { contract_id: "grant/<id>", obligor: claimant }`. The prepare re-checks each ground
  against the record (the box defect is the command's reading of the box, the registry's). Notices say why the
  claim was vacated.
- **Repairs at risk** (Q-41-20, Q-S14). The `+bay` quote under a Salvor right prints
  "At your risk: the claim on her is open, and a failed claim repays nothing." first. The claims' tick turns a
  salvor's completed order (Done, `Right::Salvor`, the custodian's, closed since the recovery) into an
  `AtRiskRepairEntry` (job lines; fee plus goods at the goods table's reference; the close instant) and writes
  it as the custodian (`ClaimOrder::AtRisk`): onto the filed claim, or onto `SalvageCustody.at_risk`, which the
  filing carries into the claim. The award and vacate notices list the entries; nothing is repaid.
- **Hot parts** (§4.6, item 41-12a). `BayHull.claim_lots` lists a lot `+bay/strip` mints while the actor holds a
  filed custody on the hull (dormant: a salvor never strips on the ruled table). On a vacate the tick marks the
  claim's lots (`strip::claim_vacated`) and flags them one act at a time (`strip::flag_next_vacated` →
  `strip::vacate_claim`, which drops the lot from the list in its own write; a sweep over
  `TacsimStore::bay_vacated_lot_hulls` resumes the list each tick). The mush-tactics
  `mark_stripped_component_contraband_on_vacated_claim` is deleted.

**Story.** `crates/tacsim/tests/shard_2/salvage_close.rs`, `rs3_story_b_the_claims_close_on_urvara_reach` (512)
and `…_on_ahuna_flank` (256), one body: Brask Freight's hull `a` (T its grantee) off L's block without
authority — T's `procedure` finds no defect, T's `right` no right, T's `recovery` vacates; T's own hull `c`,
recovered under T's `+salvage/authorize` and L's block delegation (both on the pin) — lawful, L's `right` is no
ground, T's `right` vacates; Occator's spare `b` — the lessor agent is told, contests on bare title, the claim is
awarded to P. The at-risk line in the quote, the written entry and the vacate notice:
`shard_1/bay_36a.rs` `b36a_rs3_the_salvors_repair_is_at_risk_and_reported` (the bay works only docked at a
workshop port, so it rides 36a's salvor-at-Ceres fixture). Unit tests: `surface::claim::tests::rs3_*` (the
procedure reading, the right, the at-risk list, the land grant). Re-run unchanged and green: `field_gc5`,
`field_gc6`, `bay_gc8`, `review_36a`, `bay_36a`.

**Divergences.**

- The landholder's delegation lives on the site's record (`SiteRecord.land_grants`), not inside the block's
  `BlockClaim`: the tenure family is written by industry's tenure bundle (`port/claims.rs` `stage`), and the
  site record is the claim's own action's. It is bound to the claim instance the same way.
- The at-risk entry is written by the claims' tick from the completed order, not inside `complete`'s write: the
  bay record and the site record are written by two actions. The tick writes it before anything settles, once.
- `procedure`'s "the filing registry against the site's" has nothing to compare: every lane site files at the
  one lane registry.
- `right`'s retrieval-contract source is RS.5's (no retrieval contract exists yet); RS.3 builds the authorization
  source. The hull form of `+salvage/authorize` issues on a personal title only: an org hull's grant needs a
  delegable org grant parent, which `@org/grant` cannot make; the story's `right` variant uses T's own hull.
- Story (b)'s vacated hull's return, the port's berth "held for T", the fine and the quarantine are RS.4 and
  RS.7; the freeze-paused window stays proven by `field_gc6` at 512.
- Story parties: a second salvager P takes `b`, since O's Dray-P bay holds two casualties.

**Cell cost (rule #14).** shard_2 gains the two story tests (local Windows debug: 32.4 s at 512, 29.7 s at 256);
shard_1 gains `b36a_rs3_…` (bay_36a's 14 tests ran in 19.9 s). The PR run's JUnit deltas are recorded on the
PR.

## RS.0 — the vocabulary (2026-11-03)

Row: §6 RS.0; Q-38-29. PR [#385](https://github.com/conclavio/rustmush/pull/385), base `a2ef4075`.

**Built.** `crates/mush-bot/src/mine/salvage.rs`, mounted as `mine::lane::salvage` beside CR.0's `crew` (the
`#[path]` sibling under `lane`). The captain's (`field::in_vocabulary`) and the escort's
(`escort::in_vocabulary`) closed vocabularies both admit `salvage::in_vocabulary`, each line in closed form:

| Line | Form admitted | Whose verb |
|---|---|---|
| `+tv/<n> pickup`, `+tv/<n> scrap` | `[<object>] [go]`, `<n>` 1–99, `<object>` a decimal object number, no leading zero | doc 36 GC.5, GC.6 (shipped) |
| `+salvage`, `/claim`, `/contest`, `/inspect`, `/report`, `/authorize` | `+salvage/claim <hull> [go]`; `/contest <hull>[=recovery\|procedure\|right] [go]`; `/inspect <hull>`; `/report <hull>=<who> [go]`; `/authorize <hull>=<who> [until=<n>] [go]` and `/authorize <site>/<block>=<who> [until=<n>] [go]` | doc 36, RS.3 (shipped) |
| `+salvage/retrieve`, `/jobs`, `/accept` | `/retrieve <hull>[=<port>] [fee=<cr>] [go]`; `/jobs`; `/accept <job> [go]` | RS.5 (Q-41-9; "the bot learns the three") |
| `+salvage/report … registry` | `/report <site>[=<who>\|#<hull>] registry` | RS.7 (Q-41-24) |
| `+bay`, `/quote`, `/order`, `/fit`, `/strip`, `/scrap`, `/cancel`, `/berth`, `/stow` | `+bay <hull>`; `/order <hull>=<work>[,…]` (the seven kinds or a section or part word); `/fit <hull>=<slot> <lot>` (`lot#` and 16 hex digits); `/strip <hull>=<part>`; `/stow <hull>[=<carrier>]`; the rest `<hull>` | GC.8a/b (shipped); RS.4 the bay-to-bay `/stow` |

A `<hull>` is `#<id>` or a name (letters, digits, spaces, `-`, `'`, `.`, at most 48); a `<who>` `#<id>` or a
plain name; a site, a port, a slot, a section or a part a lowercase word. Nothing beginning `@`, and no line
over 160 bytes or with a line break.

`salvage::answer` reads the server's answers as one of fourteen `Answer`s: the claim's (not in custody,
previewed, filed, filed already), the contest's (previewed on any ground, filed, no ground for any of the five
reasons, window closed), the rig's scrap (previewed, done, no remains, not the title holder) and pickup
(previewed or queued, no crate). `rs0_the_salvage_answers_match_the_game` holds each to tacsim's own
`surface::claim` and `surface::salvage` constants through the development dependency (`lines`' precedent),
and `rs0_the_salvage_lines_are_closed` the forms both ways.

**Taken already, not repeated.** `+field/void` and the four-digit address parser (`lines::is_addr`) are doc
40 FR.0's; `+sortie/condition` is doc 42 CR.0's; `+tv/<n> tow|load|release` are GC.4's and FR.0's. Doc 39's
IN.11 row names `+bay/order replace` as its vocabulary: RS.0's `+bay/order` form admits `replace` with the
other six kinds, so IN.11 reads it rather than adds it.

**Story.** `crates/mush-bot/tests/mining_cohort.rs`, `rs0_the_bot_types_the_salvage_words_on_urvara` (512) and
`rs0_the_bot_types_the_salvage_words_on_ahuna` (256), one body (`salvage_smoke`), `#[ignore]`d and run by name
(`cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored rs0_ --nocapture --test-threads 1`): the
production root booted in the test binary, the bot's own lane over its own SSH session in line mode, the
canary's two phases around the rate lift. The lane kits up at Ceres, lifts to the site and launches rig 1; then
the bot types story (b)'s `+salvage/claim <hull>`, `+salvage/contest <hull>=recovery` and `+tv/1 scrap`, each
asserted in `field::in_vocabulary` before it goes, and `salvage::answer` classifies each answer. **Passed on
the owner's Windows box, 2026-11-03:** Urvara Reach launched at +321 s, the three answered at +330 s (no
salvage custody names the hull; no remains lie within a cell of rig 1); Ahuna Flank launched at +352 s,
answered at +356 s; 687 s for both.

**Divergences.**

- The row's acceptance reads the lines on a site with nothing recovered: the answers classified are the
  refusals (`NOT_IN_CUSTODY`, `NO_REMAINS`). The filed claim, the contest preview and the break-up are read
  by the same reader against the game's own sentences in the unit test; RS.3's `salvage_close` story and
  RS.9's whole stories play them on a real casualty.
- RS.5's and RS.7's forms are admitted before the game has them, at doc 41's ruled grammar, as FR.0 admitted
  `+faction/stance` before FR.1. `+bay/return` (doc 39 IN.8) has no ruled grammar yet and is left to IN.8;
  `+debts` and `+debts/pay` (RS.5) and the tender's `+tv/<n> stores open|unjam|stabilize|rearm` (RS.1) are
  outside the row and left to those cuts.
- No lane plays the lines yet; the stories grow them from RS.1 on.

**Cell cost (rule #14).** **The delta is two new tests of 0.02 s in all, in the `bot` cell; no other cell gained
or lost a test.** Measured from the JUnit of #385's CI run
[41760031062](https://github.com/conclavio/rustmush/actions/runs/41760031062) on head `3424a895`, against
main's last CI run [41754225803](https://github.com/conclavio/rustmush/actions/runs/41754225803) on `c0d123a9`.
Main since then (`a2ef4075`) differs from `c0d123a9` only in `40-cut-notes.md`, so the two runs differ only by
this cut.

- **New tests**: `mine::lane::salvage::tests::rs0_the_salvage_answers_match_the_game` (0.01 s) and
  `…::rs0_the_salvage_lines_are_closed` (0.008 s), both `bot`. The two SSH smokes are `#[ignore]` and run by
  name only.
- **Each cell's test count**: the same in both runs except `bot` (736 → 738).
- **Summed minutes per cell, main → PR**: `bot` 11.80 → 12.13; `bot-stories` 26.58 → 28.24; `rest-server`
  48.83 → 65.47; `rest-server-boundary` 20.90 → 36.84; `tacsim-integration-7` 48.51 → 27.53;
  `tacsim-integration-6` 45.24 → 24.80; total 637.12 → 673.04.
- **These movements are host noise, not this cut.** They run both ways, from −21.0 to +16.6 minutes, in cells
  whose tests are identical in both runs (`bot`'s +0.33 is 0.02 s of new tests and 19.8 s of noise).

No cell's wall moved because of RS.0, so nothing moves shard (rule #3).

## RS.2 — the bay away from port (2026-11-04)

Row: §6 RS.2; Q-41-5..7, Q-38-16, Q-36-35. **One marker: Q-41-7** (no fee away from port, expensive to
reverse). No lane rows: lane v22 (IN.6) stands, v23 stays free.

**Built** (`port/bay_work/{mod,command,record}.rs`, the one writer of `port/bay_work/*` this wave).

- **`facility_of`'s fourth case** (Q-41-5). A hull in a carrier's bay whose carrier is not docked at a port
  with a workshop operator (on a site, under way, at a port without one) stands in `FacilityClass::CarrierBay`
  with `Facility.port: None`, `operator: None`, the tariff at 1,000 ‰ and `site` the site she stands on
  (`site_of_carrier`: the site port she is docked at, else `hull_view_berth`'s). The docked-only refusal
  string is gone; "in no bay and no workshop" and "in no carrier's bay" are the refusals left.
  `Facility::away()` names the case.
- **`carrier_of_bay` reads the yards** (round-2 item 41-16). `bay_carriers` lists every port map's hull rows
  and its site row's yard hulls, the two sources `world_hulls` reads, so the Occator Yard's Dray-P bay is a
  facility for the house rigs. **The IN.9 seam** is `bay_carriers`' doc: `tacsim.owned_hulls` joins there.
- **The yard's rearm through the bay.** `leases` reads a house rig's view (`yard::rig_lease`, the one lessee
  reader) as well as the stored leases, so the dispatcher holds the lessee's right (repair and rearm) over its
  rigs in the yard hull's bay. Nothing orders that rearm yet: `rearm_ic_secs` retires when IN.9's dispatcher
  does (§4.2).
- **`source_carrier` without the port.** A carrier bay's hold is her carrier's, wherever she is; the workshop
  case still needs the berth's carrier docked at that port.
- **No fee contract away from port** (Q-41-7). `BayOrder.contract` is `Option<u64>` (an old record's number
  reads as `Some`); away from port `take` files nothing, `fee_minor` is 0, the payer account is not asked for,
  the cancel releases nothing and `complete` pays nobody. The payee of a filed fee is now read off the
  contract's organization party (`payee`), so an order taken docked is paid to that port's operator wherever
  the carrier has sailed since (the old `facility_operator` refused the fee once she undocked). The quote
  prints "no fee" in the price column and "away from port (at 2x the workshop's time, from her hold; no
  fee)"; the taken line says "No fee away from port: her crew does the work".
- **The "needs a workshop" line** names the workshops, nearest first by the lane (`workshop_refusal`): "Salt
  Moth's machine_gun is destroyed: a workshop must replace it — the <room> at Ceres Freeport, or the <room>
  at Stickney Yards." Nearest is the fewest route hops from her site (breadth first over every port's
  `routes`), then the lane's order; under way, the lane's order. The quote's elsewhere lines and the order's
  refusal both print it; docked, the old "berth the hull at the port's workshop" stands.
- **`+bay/fit` from the hold away from port** (Q-41-6): the `Replace` a fit makes passes the class check in a
  carrier bay away from port, at 2×, from her hold (the hold lot check is GC.8b's). Docked, fit stays the
  workshop's. A plain `replace` is refused away from port (IN.6's store does not reach a carrier).
- **Bay acts wait under a field** (Q-41-5, RS.1's tend precedent). No order is taken while a field is open
  on her site (the `FIELD_OPEN` line). `wait_under_field` runs before each open order's finish in the bay's
  tick: a field on the order's site pauses it (`BayOrder.paused`, its IC seconds left), and the fire that finds
  the site settled resumes it, due that long after. An order whose draw is already owed is past the work and
  is not paused. `+bay` shows "waiting under a field with …h left". Nothing of `field/crew.rs` or
  `surface/tow.rs` is written (CR.1's); the field is read through `tow::record_or_new`.
- `+bay`'s help names the bay away from port and the fit there.

**Story.** `crates/tacsim/tests/shard_10/field_rs2.rs`, `rs2_ahuna_…` (256) and `rs2_urvara_…` (512), one
body: the captain leases the escort and a prospector and sails to the site; the escort, in the bay, has a
damaged cannon, a hull 40 points down and a destroyed machine gun. `+bay` quotes the mend and the armour at
2× with no fee and the machine gun's workshop line (Ceres before Stickney); `+bay/order …=replace` is refused
with it; `+bay/order …=mend,armor` is taken with no contract, due at twice the tariff's seconds, drawing a
tonne of alloys and a tonne of fittings from her hold into the bale. A stranger's mark over the prospector
opens a field: the order pauses, `+bay` says so, the field settles and the order resumes later than first
due. The crew works on the site until half an hour before the work is due (the hop is 70 IC minutes, the work longer), the Dray-P sails for Ceres (`leave=2`), and the order finishes under way: the cannon intact, the hull
whole, the tonnes gone from the hold, nobody paid. Re-run green: `bay_gc8` (its fee contract read as
`Some`), `in6_part_lot`.

**Divergences.**

- **The bay works at a port without a workshop too**, not only on a site and under way: Q-41-5's "wherever
  she is not docked" read as "not docked at a workshop", since the hold is aboard and nothing supplies her
  there either.
- **Ordered from aboard.** Away from port an order is taken by anyone aboard her carrier, not only from
  inside the bay room (docked, the old rule stands).
- **"Nearest by the lane" is route hops**, not passage time; the lane has no distance table the bay could
  read cheaply. Every site routes to Ceres alone, so Ceres is first from any site.
- **`+bay/fit` away from port is built but not in the story**: a traced lot in a carrier's hold needs a
  workshop strip first (GC.8b's fixture), which the site story never visits. It is the `fit_job` path GC.8b's
  story covers, with the class check relaxed.
- **The yard's rearm is the right, not an act**: no production caller orders a house rig's rearm until IN.9.

**Cell cost (rule #14).** `tacsim-integration-10`: **+1.80 min** for the two new story tests (52.9 s at 256, 55.1 s at
512 on PR run 41819115615's JUnit, head `279d895a`). The cell's sum reads 58.17 → 60.08 min, main's run at the merge base
(41814093413, `3fb3e861`) against the PR's, 366 → 368 tests; the other 0.11 min is runner noise spread over unchanged
tests (`landing::sm2_rheasilvia…` 144.8 → 149.6 s), and `field_rs1` ran no longer (95.2 → 95.0 s, 100.3 → 99.6 s).
`bay_gc8` changed one `expect`, nothing timed. shard_10 stays the room for growth, so nothing moves shard (rule #3).

## RS.5 — the retrieval contract and the charge (2026-11-04)

Row: §6 RS.5; Q-41-8..14, Q-38-6, Q-38-7. No lane rows: Tycho Mutual's org row and Ceres's `recovery` row
(IN.4) are on lane v22 already; the slot-day is IN.4's cost model, not a lane constant. v23 stays IN.5's.

**Built.**

- **The contract** (`mush-economy` `contract_live.rs`, after RS.3's award): `RetrievalSpec`,
  `post_retrieval`, `retrieval_contract_hull` (the shape: one consideration from the issuer, no
  deliverables/split/outcomes, exactly `[TitleHeld{hull, by: issuer}, Fact{delivery}]`, a `WorldIc`
  deadline, `Public`, at most one party beside the issuer), `retrieval_acceptor`, `accept_retrieval` (the
  acceptor joins as the second party in one write, the remainder pointed at their account, the poster's
  acceptance re-bound to the joined consent — the shape is the standing offer), `cancel_retrieval` (the
  ordinary unwind, escrow back). The fee is escrowed at posting under the poster's own debit authority; a
  poster who cannot cover it posts nothing.
- **The debt** (`mush-economy` `api.rs`, beside `reinstate_obligation`): `create_debt` (one-term,
  WorldIc, due now, `Ruleset{namespace}`, the actor lock of every ruleset obligation), `debts_of`,
  `pay_debt` (one post, the record `Satisfied`; refuses an account that cannot cover it with nothing
  written), `reinstate_debt` (`reinstate_obligation`'s locks, then due *now*: re-anchoring a cadence-less
  schedule leaves it due never — a defect in the plain reinstate for one-shots, found here). CR.2's
  Q-42-7 debt is the same seam.
- **The record** (`tacsim` `surface/retrieval.rs`): `RetrievalOrder` at `tacsim:retrieval:{hull}` (store
  field, `put_retrieval`, an R8 loader arm) carrying the contract, the fact's `serial`, poster, destination,
  fee and floor, `accept_by_ic`, the acceptor and `perform_by_ic`, the delivery and the stage
  (Open/Accepted/House/Delivered/Closed). `RETRIEVAL_WINDOW_IC_DAYS` = 28 is
  `[tacsim.recovery].retrieval_window_ic_days` (mush-config, the server mapping, clamped 1..=365); the
  contract's deadline is twice it.
- **The delivery fact** `tacsim.retrieval.delivered` (`RetrievalFactProvider`, unscoped, in both
  compositions): holds once a handover at the destination is recorded for the record whose serial the
  reference names. The handovers: `+bay/berth` into the destination's workshop; a stow into the poster's
  own carrier docked there; the shed's hand-back of a lease rig when the acceptor's carrier docks at the
  lessor's port (`on_docked`, from `TacsimCarrierHooks::arrived`); a hull's own docking is the space seam's
  call of `retrieval::deliver` (RS.8).
- **The clerk** (`retrieval::tick`, on the port tick after the works arm): unaccepted past `accept_by_ic`,
  Tycho Mutual accepts the same contract (`PortPartyCaps` now admits the recovery operator); a PC past
  `perform_by_ic` is cancelled, a fresh contract posted with one window's deadline (serial + 1) and Tycho
  accepts it on the same tick, the PC told, nothing charged; a delivered job closes when its contract
  settles and the acceptor is told the fee; any other terminal contract closes the record.
- **The fee floor** (`floor_minor`): the recovery row's `retrieval_fee_minor(days)`; days = route hops
  from Tycho's port to where the hull lies and on to the destination, less the one the round trip
  shares, at least one, plus one ground day — a Ceres site to Ceres is 2 days. Nobody posts under it.
- **The verbs**: `+salvage/retrieve <hull>[=<port>] [fee=<cr>] [go]` (a title holder, a hull not aboard a
  carrier, the destination Tycho's port by default), `+salvage/jobs`, `+salvage/accept <job> [go]`;
  `+debts`, `+debts/pay <id>` (LegacyPostImage).
- **Retrieval custody** (`surface/tow.rs`): `LoadFacts.retrieval` (serde default) is the third case
  beside the friend rule — the custodian holds an active retrieval on the casualty: no salvage row, no
  `Recovery` log, the result word `retrieval`, its own preview and line; a written-off rig under one goes
  into the bay and is handed back at the destination, not at the load. A write-off handed back at the load
  by someone holding no retrieval withdraws Occator's job. `Right::Retrieval` (berth and stow only) is
  read from the record, not stored on `BayHull` (the record is the one source).
- **The charge** (`port/tv.rs`): the wear allowance in `restoration_charge` (a tenth of the unit's
  structural points free, off the points the quote repairs or rebuilds; components whole;
  `allowance_minor` beside a net `total_minor`); `debt_minor`; `give_back` opens the debt beyond the
  deposit from the lessee's own return; `repossess` repossesses a rig left idle on a site **where it
  lies** (only a rig mid-act still holds the hull), prices the restoration and Tycho's quote, part-captures
  the deposit, posts Occator's retrieval to the lessor's port and opens the debt (a wreck: deposit whole,
  the fee the debt); a rig in the bay still goes to the shed (the recall takes the hull home).
- **The desk**: `+tv/lease` and `+coop/enrol` refuse a delinquent debtor naming the amount and reinstate
  each debt (the refusal's writes pinned).
- **Q-38-7's second clock**: `TvLease.wreck_aboard_ic` (serde default), set the first tick a live lease's
  rig reads a wreck in its own bay, cleared if mended; 28 IC days on it is the write-off on the wreck's
  terms; the rig waits aboard and `on_docked` hands it back at the lessor's port.
- **The `capture: None` hole**: a write-off whose lessor account does not resolve now waits instead of
  releasing the deposit to the lessee.

**Story.** See the RS.5 test files (`mush-economy` `tests/e4c/retrieval.rs`: the contract's escrow, one
acceptor, payment on the fact, the refund, the debt's delinquency, reinstatement and payment; tacsim's
story (c) at 256 and 512).

**Divergences.**

- **The contract is in force at posting**, not `Offered`: with the poster its only party, the escrowing
  acceptance activates it; the acceptor joins an active contract. Nothing a stranger could accept by the
  generic verbs exists either way.
- **No propellant in the floor** yet: the lifter that burns it is RS.6's; the days are route hops, not the
  planner's legs (no basis to plan a hull that does not exist).
- **The fourth handover** (a hull's own docking with a derelict clamped) is the seam `deliver`, for RS.8.
- **Owed: a rig left idle on a site is on nobody's index.** `repossess` leaves it where it lies, but
  only a field's close writes `SiteRecord.units`; a rig a departure leaves (`+ship/undock … leave=`)
  is not indexed either (pre-existing), so `+tv/<n> tow` cannot find it and the write-off's clock never
  starts. The site-record write is the staff fixture, offered to staff only, and the harness holds that
  GOD never submits from a tick, so the repossession cannot index it without widening a staff verb.
  The story stands it in with GC.5's fixture. The fix belongs with the next writer of
  `surface/tow.rs`/`site_record.rs` (a system index op, or the departure indexing what it leaves).
- **A rig aboard the charter's own hull still goes to the shed** at a repossession away from port: the
  recall takes the hull back, so nothing would be left aboard a carrier anyone holds.

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.7 — port security (2026-11-04)

Row: §6 RS.7; Q-41-24, Q-41-25, Q-38-20, doc 27 Q-S4. No lane rows (v23 stays IN.5's). Built on RS.5's
branch (#415): the fine is RS.5's one-term debt.

**Built.**

- **The record** (`tacsim` `surface/security.rs`): `SecurityRecord` at `tacsim:security:{actor}` (store
  field, `put_security`, an R8 loader arm) holding `ProvenAct { site, act, proof, proven_ic, debt }`, at
  most 64 rows (a full record drops its oldest settled row). `Proof::Vacated` or
  `Proof::Reported { by }`. A row's identity is the site plus the act's kind, actor, subject and instant,
  so one act is proven once.
- **Proof at the vacate**: `claim::tick`, after a `Vacate` settlement whose vacating contest was
  `ProvenUnlawfulRecovery`, copies the site's `Recovery` row on that hull by the custodian (or, when the
  ring has dropped it, the row as the custody knew it) to the custodian's record.
- **Proof by report**: `+salvage/report <site>[=<who>|#<hull>] registry` (RS.0's ruled form; one
  command, no `go`, as the bot's line has it) reads the site's log directly. The reporter's evidence
  (Q-S4): a witness pinned on a live custody row over the act's hull (`evidence_of`), or the reporter's
  own `known` row on the site learned within one IC day after the act. Refusals: no such site, nothing
  logged, no evidence, already before the registry, oneself.
- **The fine at docking** (`security::on_docked`, from `TacsimCarrierHooks::arrived` beside
  `retrieval::on_docked`): at a Registry port — `SECURITY_VENUES` `ceres`, `stickney`, by venue — for the
  carrier's master (`helm_holder`) and operator (her title holder when a character), every proven unfined
  row is fined by kind (`Pickup` 500, `Scrap` 2,000, `Recovery` 5,000 cr; `Vacated` and `Capture` none)
  as a one-term debt to the Freeport Registry's account under `tacsim.debt`, paid in the same step when
  the account covers it (`pay_debt`), the debt's id written on the row (the receipt). Each still-delinquent
  fine is reinstated (`reinstate_debt`) at every docking there.
- **The quarantine**: `DepartRefusalKind::Quarantined` in `prepare_departure` while a fine stands owed
  against the master or operator, with the ruled line; the queued-departure arm reports the line.
  `+debts/pay` lifts it.

**Story.** `tacsim` `tests/shard_5/port_security_rs7.rs`, story (b) extended at 512 (Urvara) and 256
(Ahuna): O's two unlawful recoveries, one vacated on T's `recovery` contest (proven), one left bare; O
docks at Ceres short, is fined 5,000 cr for the proven one only, held, refused `+debts/pay` short, paid
once funded, released; T reports the bare one to the registry on T's sensor record, L cannot.

**Divergences.**

- **Proven rows are copied, not moved**: the site's ring keeps them. Moving them would be a site-record
  write, which only the tow action makes; the security record is what the docking reads and dedups by the
  row's identity, so a row left in the ring is never charged twice.
- **The whole fine is the debt** when the account cannot cover it, rather than the shortfall alone: the
  fine is paid in one post or not at all, as the customs fine is.
- **An organization operating a carrier** answers through its helm holder only: `UnlawfulAct.actor` is a
  character, so an organization has no rows of its own.
- **The bare-row case is a `Recovery`**: story (b)'s bare row is O's unreported recovery of `d`, not a
  pickup; a crate pickup would leave ore in O's hopper and hold the departure (`HopperLoaded`). The rule is
  kind-blind: no row is fined unless proven.
- **Security venues are a constant**, not a lane field, to take no lane version.

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.8 — the space seams (2026-11-04)

Row: §6 RS.8; Q-41-26, Q-36-46, Q-36-47. The owner moved it from the space track to the programme
session. The as-built is doc 35c §12.1. No lane rows, no config keys, no kernel hash.

**Built.** In the space tree (`space/custody.rs`, `commands.rs`, `tick.rs`, `bubble.rs`, `docking.rs`,
`picket.rs`, `mod.rs`), plus three changes outside it:
- `surface/claim.rs`:
  - `tow_files_salvage` (named clear of `shard_3`'s industry-seam tripwire, which forbids `file_claim(`) is `+salvage/claim`'s filing, and `+helm/tow` calls it.
  - `title_of` is now `pub(crate)`.
  - The space orders persist the custody document, and the RAM claim-book mirrors are gone.
- `composition/sim_routes.rs`: one call. A hull her retriever docks under her own power is
  delivered.
- `carrier_damage.rs`: the module doc names where space maps `ShipSectionId`.

The claim path:
- `+helm/tow` puts the derelict in the tug's custody, in `tacsim:space:custody:{domain}`, and files
  on the ground's one path.
- The window is `claim_window_s`. `DERELICT_WINDOW_S` and the derelict's second contest clock are
  retired.
- The pins and titles name real principals.
- The RAM claim registration and the 1,200 cr bounty are deleted.

The retrieval:
- A tug holding a derelict's retrieval tows her for the job.
- Docking delivers it, both for a towed derelict and for a hull under her own power.

**Story.** Both stories are in `shard_10/voyage.rs`.
- `sfr19_one_salvage_path_unifies_space_derelict_claims_with_gc6_and_economy` is rewritten as RS.8's
  acceptance. A derelict is clamped, towed and docked under the one window, survives a full restart
  in her domain's custody document, is contested by `+salvage/contest`, and is awarded by the claim
  tick.
- `rs8_a_space_retrieval_job_is_delivered_by_the_tug_that_docks_her` is new. A space retrieval job
  on `+salvage/jobs` is accepted, towed and delivered by the tug's docking, and the fee is paid.
- sfr2, sfr7, sfr8, sfr12 and sfr15 are re-read for the retired bounty and the retired window.

Rule #1's 256/512 is not applicable: a space bubble has no site scheme. The bot does not fly space
(no mush-bot space lane, doc 35c §9).

**Divergences.**
- **The dock is Stickney Yards, not Ceres.** The test lane's interdicted leg is `ceres_stickney`,
  so the tug's next port is Stickney.
- **The clamp is not timed by `attach_s`.** The 6-DOF docking session is the act's time. A second
  timer would be a new space mechanic, and the owner ruled against adding space exposure.
- **No carrier floor in space.** The tow keeps its 2× dry-mass cap. `TUG_SHARE_PERMILLE` is not
  applied, and no space hull is a carrier wreck yet.
- **The space retrieval's poster is a fixture.** A staff `@asset/transfer` gives the Raider to a
  player. Production has no way for a player to own a hull that lies disabled in space and is not
  their own carrier.
- **A derelict nobody held** is towed into custody with no claim, as the ground's `NO_TITLE` does.

**Cell cost (rule #14).** The stories run in `tacsim-integration-10` (shard_10's `voyage::`). Measured from the JUnit `<testcase time>` sums of #439's run 41898886949 against `main`'s pre-RS.8 run 41888309144.
- The new `rs8_…` story takes 7.6 s: **+0.13 summed min**.
- The rewritten `sfr19_…` reads 9.1 s against 13.5 s, on a baseline whose whole cell ran 1.64× slower (71.9 against 43.9 summed min; TS.3's optimised dependencies landed in between). Scaled, that is about +0.9 s.
- sfr2, sfr7, sfr8, sfr12 and sfr15 hold their scaled times.
- The cut's delta is about **+0.14 summed min**. The whole cell's sum is not comparable across that change.
## RS.4 — the vacated hull's return (2026-11-04)

Row: §6 RS.4; Q-41-18, Q-41-19, Q-38-19, round-2 item 41-15. No lane rows. Consumes IN.9's
`+bay/stow <hull>=<carrier>` as merged; built on its branch (#416). Moves no crew (doc 42's).

**Built.**

- **`held_for`** on the bay record (`BayHull.held_for: Option<PrincipalId>`, serde default, omitted while
  `None`): the claims' tick writes the hull's title holder at a `Vacate` settlement
  (`bay_work::hold_for`), right after the site record's settle lands.
- **Bay to bay** (`bay_work::stow`'s new source, `stow_held`): `+bay/stow <hull>=<carrier>` on a hull
  that is not berthed and is held moves her out of whatever bay holds her into a free slot of the named
  carrier, for the holder (the character, or one acting for the organization) or a `maintain_refit`
  grantee, who has the run of that carrier, stands aboard her, and has her docked where the holding
  carrier is (one port, or one site's pad). The custodian cannot refuse it and has no right to move her.
  The slot check is IN.9's `free_bay_slot`.
- **The port holds her** (`bay_work::berth_held`, from `TacsimCarrierHooks::arrived`): a carrier docking at a
  port with a workshop has every held hull in her bay berthed into the workshop room with no carrier
  (`BerthRow.carrier: None`, as IN.9's yard berth), her slot freed; a lease rig is handed back to the
  shed instead. **Outside the stand count**: `berth`'s count of the workshop's berths skips held hulls.
  Her owner's carrier stows her from the floor through IN.9's existing named-carrier path, which clears
  the hold.
- `+bay <hull>` says "… is held for <name>: a carrier of theirs takes her".
- `surface::act::where_is` is `pub` (the story reads it).
- The fix thread's defect note (§2.2, the vacated hull's missing return verb) is closed by this cut.

**Story.** `tacsim` `tests/shard_7/vacated_return_rs4.rs`, story (b) extended at 512 (Urvara) and 256
(Ahuna): both of O's unlawful recoveries vacated on T's `recovery` contests and held for T; O cannot move
them; T aboard T's own carrier on the same pad takes one bay to bay; O docks at Ceres with the other, which
the port berths in the workshop held for T; T's carrier docks and T stows her from the floor.

**Divergences.**

- **A held hull never counts against the workshop's berths**, whatever her owner does: she waits there
  until a carrier of theirs takes her (no fee, no expiry). A storage charge or a lapse to Tycho's sweep is
  not ruled; it is RS.6's to consider.
- **The return carries no crew**, and a hull vacated while crewed keeps whoever is aboard where they are
  (doc 42 owns it).

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.6a — Tycho Mutual's designs and house (2026-11-04)

Row: §6 RS.6, its first slice (RS.6a/b/c, sliced with the lead's OK); Q-41-15, Q-41-17, Q-41-29. **Lane
v28 / relay v19** (`0b68e811…`, `876dcae5…`) over IN.9's v27; `lane_chain::strip_tycho_house` heads the
chain. RS.6b (the house arm flying a retrieval, story d) and RS.6c (the sweep, story e) follow.

**Built.**

- **The designs** (`content/units/tycho_crawler.json`, `tycho_lifter.json`, `EXTENSIONS_TYCHO`, fielded
  last, LOOKLESS): the crawler 130 t dry, tracked, unarmed, a crew of two, `recovery_profile: heavy`
  (2× laden), a 3 t tool locker; the lifter the Dray-P's hull grown to 600 t, a 10 t hold, a 200 t tank,
  one recovery bay of two places at 430 t / 1,600 m3, the rail 641,000 kg.
- **The bills** (doc 39's `design_bill`, pinned in `port/bill.rs`): the crawler 100 t of alloys, 34 t of
  fittings, 7 yard days; the lifter 441 t, 147 t, 30 days; hull kits in `goods.json` and recipe rows in
  `industry/hulls-v1.json`, so the Stickney yard builds either for anyone at its bill (Q-41-15).
- **The house** (`port/house.rs`, the port tick before the commission arm; `PortMap.house`): lane v28's
  `recovery.house { lifter_mark, crawler_mark }` (`TMA-LF-01`, `TMA-CR-01`) is laid up as Tycho's
  `DARK` dispatcher in the port's host room and two **yard commissions** titled to Tycho Mutual, seeded
  at `Laying` (no kit lot, no work order), which IN.9's commission arm carries to the register: the
  lifter docked at Ceres with a hold, the crawler on the workshop floor; once both are done the crawler
  is stowed into the lifter's bay (`bay_work::stow_house`). `house::hulls` and `house::dispatcher` read it.
- **The capital** (Q-41-17): Tycho's `capital_major` stays 50,000 and the row gains
  `"raises": [ 450000 ]` — 500,000 in all.

**Story.** `tacsim` `tests/shard_7/tycho_house_rs6a.rs`: the composed lane lays the house up on the port
tick — both hulls Tycho's, the lifter docked at Ceres, the crawler in her bay, the dispatcher in the host
room — and a minute more of fires seeds nothing again.

**Divergences.**

- **One bay of two places**, not two slots: the engine reads one bay a hull (`InteriorDef::bay`), so the
  crawler's 130 t / 400 m3 and the load's 300 t / 1,200 m3 share one 430 t / 1,600 m3 bay.
- **The capital as a raise**: a port update never re-capitalizes an org on a kept world (the applier
  refuses a changed `capital_major`), so Q-41-17's 500,000 is the 50,000 base and one 450,000 raise.
- **No storefront party** yet: Tycho already acts as a contract party through `PortPartyCaps` (RS.5); a
  storefront needs goods to sell, which the hull-sale offer of RS.6c brings.
- **No looks**: the kit has none for either (doc 41 §7); `+ship` prints the lifter's class as a refit
  hull's, since the class word reads only the Dray family. RS.6b gives it a word when the arm flies it.

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.6b — Tycho Mutual's house arm flies a retrieval (2026-11-04)

Row: §6 RS.6, its second slice; §4.4 "How it reaches a site", Q-41-11, Q-41-15. No lane rows. Stacked on
RS.6a (#455); carries the production fix #464 (a plan's fill before a site departure buys the leg home).

**Built.**

- **The dispatcher acts for the house** (house step 1b): grants on Tycho's root scope of
  `economy.account.post` (so `acts_for`, `helm_hulls`, the title views and `retrieval::held_by` answer it) and
  `vault.withdraw` (the org-owned hold takes a plan's buys: `may_take_from_vault` reads that capability for an
  organization's vault). **Its float** (step 1c): its own `tacsim.credit` account, which a sailing tops up to
  20,000 cr from Tycho's (`house_arm::FLOAT_MINOR`, provisional), since a plan's buy pays from its author's own.
- **Aboard and at the helm**: `voyage::aboard` and `coop::helm_holder` gain a house-dispatcher branch beside the
  Occator Yard's (`house::is_dispatcher_of`, `house::house_of`); a house hull flown by its dispatcher carries it as
  her one occupant (`prepare_departure`'s count; the dispatcher is a Thing `souls_aboard` does not count).
- **The arm** (`port/house_arm.rs`, the port tick's `Arm::House`, agent and principal the dispatcher): one act a
  fire through each production path, inside `Fire::as_principal` — the sail (the dispatcher boards; a sailing
  plan: `buy propellant fill`, `buy provisions fill`, `depart <site>`, run by the order arm as its author), the
  crawler's launch, the drive beside her, the tow, the drive to the ramp, the load, the recover, the voyage home
  (`+ship/lift`), and the berth into the home workshop (`bay_work::berth`, which posts the delivery; a lease rig
  goes to its shed and the delivery is posted). `next_act` re-derives the next act every fire from where the
  casualty, the crawler and the lifter stand: nothing about a job's progress is stored.

**Story.** `tacsim` `tests/shard_7/house_arm_rs6b.rs` at 256 (Ahuna) and 512 (Urvara): Occator's dead rig on the
ground, its retrieval to Ceres unaccepted for the window and handed to Tycho, flown by the arm end to end and
berthed in Ceres's workshop, the delivery posted.

**Divergences.**

- **Story (d) is owed in this PR**: the carrier wreck at 256 t laden with her two wrecked rigs, taken by the
  crawler and lifted. The no-acceptor story proves the arm's whole path on a rig.
- **The dispatcher moves into the workshop room** for the berth and back out: `berth` reads the berther's
  location, and the dispatcher has no walk.
- **The float**: a house's sailing buys through its dispatcher's own account, topped from the house's; the house
  account itself never buys (the plan's buy pays from its author's own account by design).

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.6c-1 — Tycho Mutual's sweep, the scrap half (2026-11-05)

Row: §6 RS.6, its third slice (RS.6c sliced -1/-2 with the owner's "accept all" of 2026-11-04); §4.4 "On its
own account", Q-41-16, round-2 item 41-7. No lane rows (IN.11 holds v30; this cut takes none). Stacked on RS.6b.

**Rulings (the owner, 2026-11-04, "Accept all").**

1. **An ownerless wreck** — one with no title on the economy, which every raid's hulls are (`field/raiders.rs`
   creates them with none), so the one claim path refuses them (`NO_TITLE`) — is titled by the sweep to the
   house's dispatcher under the registry issuer, as salvage of the unowned, and no claim is filed. A titled wreck
   goes through the claim and the award as §4.4 rules.
2. **Owned ground**: Tycho sweeps open ground and Registry seeds only (`claim::tenure_at` reads `Unclaimed`), and
   leaves a wreck on a player's claimed block alone. **Future cut: the landholder ask** — §4.4's "asks for the
   landholder's authorization by the registry's notice and takes it as given after 7 IC days of silence" — is not
   built; a later cut builds it and the sweep then reaches claimed blocks.
3. RS.6c-1 here (the sweep, the claim and award or the ownerless title, the assessment, the scrap, story (e)'s
   scrap half and the census); RS.6c-2 next (the repair, the hull-sale offer contract, `+salvage/forsale`, story
   (e)'s resale half).

**Built.**

- **The sweep** (`port/house_sweep.rs`; the house arm's job when it holds no retrieval): the first wreck on a lane
  site's index past `SWEEP_IC_DAYS` (56) of `left_ic`, on open ground, with no custody row and no live retrieval,
  on a site no field holds (`target`). Ownerless: `HouseAct::Title` titles her to the dispatcher first. Then the
  arm's own path: sail, launch, drive, tow, load — into the house's custody (the dispatcher is the custodian) — and
  `HouseAct::Claim` files the house's salvage claim (`claim::tow_files_salvage`), then home and the berth.
- **The award**: the claims' tick awards to the dispatcher at the window's end (an Object claimant).
- **The assessment** (`assess`): the full restoration quote — the workshop's fee (`BayQuote::fee_minor`: labour at
  the operator's multiplier and every replaced part) plus the bale's alloys and fittings at their references
  (`good_reference_minor`) — against the design's reference price (`reference_price_minor`) less
  `RESALE_MARGIN_PERMILLE` (200, provisional). Under the line: **kept**, her title to the house
  (`HouseAct::Keep`, a `TransferTitle` under the dispatcher's new `asset.title.transfer` grant; RS.6c-2 repairs
  and offers her). Over it, past repair, or a design the yard does not bill: **scrapped** (`HouseAct::Scrap`,
  IN.8's `strip::scrap` as her title holder in the workshop room), and the credit, paid into the dispatcher's
  float, goes back to Tycho's account (`return_surplus`).

**Story.** `tacsim` `tests/shard_7/house_sweep_rs6c.rs` at 256 and 512: Occator's spare rig wrecked on open ground,
unclaimed for 57 IC days; Tycho sweeps it, files and wins the claim, berths it at Ceres, assesses it past its line
and scraps it; Tycho's account takes the credit; the site's index is clear of her.

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.6c-2 — Tycho Mutual's sweep, the resale half (2026-11-05)

Row: §6 RS.6, its last slice; §4.4 "On its own account", Q-41-16. No lane rows (IN.11 holds v30, the 28-IC-day
timers v31; this cut takes none). Stacked on main after RS.6c-1 (#469).

**Defaults (the owner kept both, 2026-11-04).**

1. **The supply leg**: the restoration's bale goods (whole tonnes of `structural_alloys` and `industrial_parts`,
   drawn from the carrier's hold at completion, GC.8b) are bought by the house's lifter on a sailing plan to
   Stickney (`house_arm::SUPPLY_VENUE`), then the repair.
2. **The sale is the buyout's contract shape** (D-18-21, `port/buyout.rs::buyout_spec`): `+salvage/forsale` lists
   the houses' kept hulls at the design's reference price, and a buy verb files the contract.

**Built.**

- **The restoration** (`house_arm::restore_act`, the berthed-and-kept branch of the sweep, while her title is still
  the dispatcher's — its right in the bay): `bay_work::house_repair` reads the next order from her quote where she
  stands — every repair job by target, no rearm (she is sold with her magazines as found), at most one part from
  the stores (`ONE_STORE_PART`), a part the stores do not stock now left out — with its goods and fee. Goods short
  in the lifter's hold: `HouseAct::Supply` sails her to Stickney (fill, depart), `HouseAct::Stock` buys the
  shortfall there (`buy <good> <n>`, then fill, depart home); the float is topped up to `FLOAT_MINOR` plus twice
  the goods' references (`top_up`, shared with RS.6b's sail). With the goods aboard and the lifter docked at Ceres,
  `HouseAct::Repair` tops the float up by the fee, steps the dispatcher into the workshop room and takes the order
  (`bay_work::take_order`). While it is open the arm waits and **a retrieval waits too** (the lifter must stay
  docked: the goods are drawn from her hold at completion). With nothing left to order, `HouseAct::Keep` passes her
  title to Tycho as RS.6c-1 built.
- **The sale** (`port/house_sale.rs`, stateless): `for_sale` reads every house port's workshop room for hulls
  titled to its operator, not scrapped and not under work, at `reference_price_minor`. `+salvage/forsale` lists
  them (less any with a live sale contract); `+salvage/buy <hull> [go]` previews, and `go` offers `sale_spec` — the
  buyer's escrow, an `Exact` payout to the house's account, `TitleHeld{hull, house}` and an hour's `Before`, one
  `TitleTransfer` house → buyer — then the house accepts through `PortPartyCaps` (`accept_contract_as`). The title
  passes at the next settlement; a second buy while one is live is refused (`pending`, by the contract's name).
  She stays berthed until the buyer stows her into a carrier of theirs.
- **Bot vocabulary**: `+salvage/forsale` and `+salvage/buy #<n> [go]` are in (`mush-bot` `mine/salvage.rs`).

**Story.** `tacsim` `tests/domain_economy/house_resale_rs6c2.rs` at 256 and 512: Occator's spare rig, her chassis
destroyed, unclaimed for 57 IC days; Tycho sweeps, claims and berths her, assesses her under her line, runs the
supply leg to Stickney, restores her and takes her title; the pilot finds her in `+salvage/forsale`, buys her, and
her title and the price change hands.

**Cell cost (rule #14).** Owed from the PR's CI JUnit.

## RS.10 — the landholder consent step (2026-11-05)

Added to the plan after RS.6c-2, one PR; recorded in doc 41 §6 as a row. It is RS.6c-1's named future cut: Tycho's
sweep reaches a player's claimed block. No lane rows. **The owner's rulings (2026-11-04):**

1. **The notice** is a registry notice delivered as an obligation; the landholder answers with `+salvage/authorize`
   (RS.3's, the delegation row on the block) or a new `+salvage/refuse`.
2. **Silence** counts as consent after **28 IC days** (§4.4 said 7). **The owner's standing rule: any wait on a
   player's response is at least 28 IC days** — 7 IC days is about 7 real hours, and no workflow here assumes a
   player is on constantly.
3. **A refusal** blocks the sweep (and the claim) on that block for 28 IC days.
4. **Who it covers**: the same notice serves a player salvor filing a claim on someone else's claimed block, not
   only Tycho.

**Built.**

- **The ask** (`surface/claim.rs`, `ConsentAsk` on `SiteRecord.consent_asks`, bounded as the delegations are): one
  row a block, landholder and asker, riding the block's holding (`registered_ic`). Written by two new orders on the
  tow action's `claim` op — `ClaimOrder::Ask` (the asker's own) and `ClaimOrder::Refuse` (the landholder's, which
  also voids any delegation it gave the asker there) — and read by `consent_at` / `consent_on`: open ground, the
  asker's own block, a live delegation or an ask unanswered past `CONSENT_IC_DAYS` (28) is `Consent::Given`; a
  refusal inside its 28 IC days is `Refused`; an ask inside its wait is `Asked`; else `NotAsked`.
- **Ruling 1, the notice "delivered as an obligation"**: read as a standing ask the landholder must answer, kept on
  the record until answered or lapsed, with the registry's line delivered through `mush_economy::deliver_notice`
  (straight to a connected landholder, queued durably for an absent one). No money obligation is opened — **flagged
  for the owner** in case a ledger obligation was meant.
- **Read at the attach** (Q-41-21): `pin_for`'s `landholder_authorized` is the delegation *or* consent by silence,
  so a recovery after 28 IC days of silence is lawful off owned ground; one before it is not.
- **The verbs**: `+salvage/ask <site>/<block> [go]` (ruling 4: any salvor); `+salvage/refuse <site>/<block>=<who>
  [go]` (the landholder; asked or not); `+salvage/authorize <site>/<block>=<who>` as RS.3 built it is the consent.
  `who_named` also answers a recovery house's dispatcher, so a landholder can answer Tycho by name or `#<id>`.
- **Ruling 3**: a standing refusal refuses a new ask (`claim_refused`) and refuses `+salvage/claim` on a hull pinned
  on that block (`CLAIM_REFUSED`); Tycho's sweep passes over an asked or refused block (`house_sweep::target`).
- **The sweep** (`house_sweep::target` with the dispatcher, `HouseAct::Ask`): on a block nobody has asked, the house
  asks first (`claim::ask_consent_at`), then sweeps other wrecks; once consent stands it recovers her.
- **The floors**: `MIN_CLAIM_WINDOW_IC_DAYS` (28) is now `[tacsim.recovery].claim_window_ic_days`'s lower bound (was
  1); `player_wait_floors` test 8 pins the claim window, that floor, the debt grace (RS.5's charge and RS.7's hold)
  and the consent wait.
- **Bot vocabulary**: `+salvage/ask <site>/<block> [go]` and `+salvage/refuse <site>/<block>=<who> [go]`.

**Tests.** `claim::tests::rs10_an_ask_is_consent_by_silence_and_a_refusal_blocks` (the ask's own writer, silence,
a new holding, the refusal voiding a delegation, a refused re-ask, the lapse); the story
`domain_economy/consent_rs10.rs` — the sweep asks on a player's block and leaves her, then recovers her lawfully after 28
IC days of silence (256 and 512), and a refusal holds the house off for 28 IC days and a fresh ask follows its lapse
(256).

## The 28-IC-day rule (2026-11-04)

**The owner's standing rule: any wait on a player's response is at least 28 IC days.** Seven IC days is about seven
real hours at rate 24, and no workflow here assumes a player is on constantly. The audit of the built cuts found
four waits under it; the owner ruled "Apply all three" for doc 41's (the Co-op's seven-real-day dormancy is doc
18's and unchanged):

1. **The salvage claim's contest window (RS.3) is counted in IC**: `[tacsim.recovery].claim_window_ic_days` = 48
   (the key `claim_window_s`, real seconds, is gone; pre-launch, no migration), `TacsimRecoveryCfg.claim_window_ic_s`
   in IC seconds. The claim contract's deadline is real time by its shape, so the window is filed in real seconds at
   the world's rate (`claim::claim_window_real_secs`): 48 real hours at rate 24, as before, and 48 real days at rate 1
   (was 2 IC days). `MAX_SALVAGE_CLAIM_WINDOW_SECS` rises from a week to a year. A rate change after filing does not
   re-time a filed window.
2. **One-term debts (RS.5) have a 28-IC-day grace**: `mush_economy::create_debt_due` opens a debt due at a given IC
   instant; tacsim's debts (the lease charge and the port fine) fall due `retrieval::DEBT_GRACE_IC_DAYS` (28) after
   they are opened, payable at any time before by `+debts/pay`. Only past it, unpaid, does a debt go delinquent — so
   the desk's refusal waits for it.
3. **RS.7's port fine** is still levied at docking; the carrier is held (`DepartRefusalKind::Quarantined`) only
   once the fine is past its grace unpaid (`security::holds`), and until then she may undock.

**Tests.** `salvage_close::rs3_the_claim_window_is_forty_eight_ic_days_at_any_rate`; RS.5's story (the debt not
delinquent within the grace, the desk refusing past it); RS.7's story (the carrier not held within the grace, held
past it). Each fails without its change.

## RS.9a — the stories' in-process gaps: story (d) and (e)'s census (2026-11-05)

Row: §6 RS.9, its first slice. RS.9 is sliced: **9a** the in-process gaps (story (d), never built; story (e) reading
the matter census); **9b** the bot over SSH on (e) at 256 and 512; **9c** the bot on (b) and (c); **9d** the bot on (a),
then this file closed. No lane rows. Stacked on the 38a/b fixes (#489).

**The owner's ruling (2026-11-05).** A carrier wrecked while docked at a site is **set down on her pad cell** — the
centre of the cell her ramp stands on — so the tow and the house arm read her like any casualty. (A docked hull's
surface placement is the port domain's origin, out of any tug's reach: story (d)'s crawler drove beside her and every
tow read "not a casualty or a wreck within a cell".)

**Built.**

- **The set-down** (`surface::tow::set_down_on_pad`): placed at her ramp cell's centre in the port's domain, so she is
  still docked there (`berth_of` reads the domain) and never lifts. The field's close (`field::close::demote_units`)
  sets a carrier checked in a wreck down and indexes her there; a staff fixture calls it by hand.
- **A carrier is cargo in a bay** (`voyage::bay_gate_leaving`): a ship's hull a tow loaded is no rig. The departure
  read her hopper and planned dark ("could not read a rig's hopper … no hopper vault").
- **A berthed carrier keeps her place** (`mush_sim::retire_finished_journey`, called by `bay_work::berth`): berthed
  out of a bay she is unplaced in the workshop room as a rig is, and the journey tick's loose-terminal backstop took
  her finished journey's root for a stopped ship and quarantined her into the recovery room (LIMBO) on the next fire.
  Retiring the finished journey makes her an object again; her next departure starts a journey of its own.
- **Story (e)'s census** (`port::census`): a workshop's scrap (IN.8's `strip::scrap`, Tycho's at Ceres) mints from a
  hull the census never held, but only a site's remains scrap counted as the `scrapped` source, so the census fell by
  the yield across the sweep (1,183 kg: a tonne into lots, the rest into the shop bale). `BayHull.scrapped_kg` records
  the yield and the stripped parts' lots; the census sums it.

**The 256 t figure.** A Dray-P fresh from the pad with her two rigs is 272 t laden — her tank and locker count
(Q-41-29) — over the 130 t crawler's twice-its-own 260 t. §4.9's 256 t is her dry hull and two rigs: the story's
carrier flies out on the leg's propellant and little more (`driving_rig::on_pad_loaded_at`). A fuelled carrier on her
pad is beyond the house's crawler.

**Stories.** `tacsim` `tests/domain_field/story_d_rs9.rs` at 256 and 512: the Dray-P's hull lost on her pad, her two
rigs wrecked with her, set down on her pad cell; her title holder's retrieval unaccepted, Tycho's crawler tows her
laden, the lifter flies her to Ceres and berths her in C14 (where she stays), the delivery settles, `+bay` quotes her
rebuild. "No shipped tug can take her" is the table's own check
(`field_carrier::every_carrier_is_past_twice_any_tugs_tonnage`). Story (e): `house_sweep_rs6c`'s scrap stories and
`house_resale_rs6c2`'s resale stories hold the census balance (`in11_matter::steady_balance`) across the sweep, the
scrap, the supply leg, the restoration and the sale, at 256 and 512.

## RS.9b — story (e) whole, the bot over SSH (2026-11-05)

Row: §6 RS.9, its second slice. Stacked on RS.9a (#503). No lane rows.

**The story** (`mush-bot` `tests/mining_cohort/rs9_story_e.rs`; `mining_cohort::rs9_story_e_the_bot_buys_a_swept_wreck_from_{ahuna,urvara}`,
`#[ignore]`d, run by name, TS.2 tier by-hand): God's fixture makes a raid's bare prospector hull (no title, no vaults)
on the site's open ground, its chassis destroyed by `@unit/damage`, on the index by `@tacsim/site leave`; God opens and
funds the bot's credit account and lifts the world to 3,600. The house sweeps her at 56 IC days, titles her to its
dispatcher (salvage of the unowned), tows and loads her, flies her home, restores her and lists her; the bot types
`+salvage/forsale` until she is on it, then `+salvage/buy <hull>` and `… go`, and reads her title as its own on
`+bay/quote`. IN.11's `@industry/census` balance is held across the whole story. Green by hand at `38989d7e` at 256 (Windows, 3,206 s), then at `ca0ad46f` at both sizes side by side on
conclav.dev (3,205 s each; the house took her at about 1,500 s and listed her at 3,173 s for 21,415.00cr).

**What the story found** (fixed here):

- **A swept bare hull is cargo** (`voyage::bay_gate_leaving`): the departure weighed every hull in the bay as one of
  the carrier's rigs, and a raid's hull has no hopper vault, so the lifter's passage home planned dark and she never
  lifted from the site. A hull aboard that is none of the carrier's live rigs and has no hopper is cargo. In-process:
  `house_sweep_rs6c::rs9b_a_bare_raid_hull_swept_comes_home` (fails without; the in-process (e) stories used a line
  tank, which the gate already read as a combat hull's cargo).
- **The supply leg round Kirkwood** (`house_arm::SUPPLY_LEGS`): the restoration's leg to Stickney flew the corridor,
  which Kirkwood's picket interdicts on main (doc 39's IN.5 note), and an NPC house answers no customs bubble. It now
  sails the lane's own picket-free way out, Ceres → Luna → Deimos → Stickney, and home direct, as IN.5's bot does.

**From the review of #503 and of this cut** (the RS.9c/d session, 2026-11-06):

- **The bay gate fails closed.** Only a hull aboard with no hopper vault at all is cargo
  (`port::vehicle::hopper_of` is `None`); a rig whose hopper exists but will not read still plans the departure dark,
  as before this cut.
- **Only an arrived journey retires** (`mush_sim::retire_finished_journey`): a `Failed` record keeps the anomaly a
  human must decide, a `Captured` one its taker; `bay_work::berth` retires it after its unplace and move succeed.

**Cell cost (rule #14).** The supply leg's two extra calls push RS.6c-2's resale stories past the CI profile's
twenty minutes on the hosted runner (`tacsim-integration-6` and `-9`; 1,105 s and 790 s on main before); a
`profile.ci` override in `.config/nextest.toml` gives those two tests forty. Locally (Windows debug, two at once)
they run 569 s.

**The harness.** `fr7_round`'s fixture helpers (`placed`, `staff_hand`, `cid`, `PROSPECTOR`, `WEST`) and
`in11_matter::census` are `pub(crate)`. A side-by-side run goes through the built test binary: a second `cargo test`
cannot relink the binary the first holds open (LNK1104 on Windows).

## RS.9c — stories (b) and (c) whole, the bot over SSH (2026-11-06)

Row: §6 RS.9, its third slice. Stacked on RS.9b (#505). No lane rows; one engine change, which the bot found.

**Story (b)** (`mush-bot` `tests/mining_cohort/rs9_story_b.rs`; `mining_cohort::rs9_story_b_the_claims_are_contested_on_{ahuna,urvara}`,
`#[ignore]`d, TS.2 tier by-hand): three players over their own sessions. T, Brask's officer (`@org/grant`), and O each
kit up and fly a Co-op Dray-P to the site; L holds the Registry's claims on two blocks. Brask's `a` (titled at Brask's
root scope) and T's own `c` lie a cell either side of the ramp, a block line between them. L may not authorize T's hull;
T authorizes O and L to recover `c`; O holds no block to delegate; L delegates `c`'s block to O. O's rig tows each
(`+tv/1 tow #<id> go`) and loads it; O files a claim on each; O's quote of `c` reads "At your risk". The contests: on
`a` T's `procedure` finds no defect and O may not contest, T's `right` finds nothing that bound O, T's `recovery` (T's
sensor record) vacates; on `c` L's `right` is no ground and T's `right` (T's authorization bound O) vacates. O sails home
leaving the tug (`+ship/lift ceres leave=1`); at docking the Registry fines O 5,000 cr for `a`'s unlawful recovery,
standing as a debt, and the port berths both hulls "held for" T in C14 (T has no carrier docked). Past the fine's
28 IC days O's `+ship/undock luna` is refused ("lifts when it is paid"); O lists and pays the debt (`+debts`,
`+debts/pay <n>`). T sails home and, as Brask's officer, `+bay/stow`s Brask's `a` out of the workshop into T's own carrier
(her one free slot beside T's rig); T's own `c` waits there, held for T.

**What the bot found** (fixed here): **an organization's held hull would not leave the workshop for its officer.** A
vacated hull berthed in a workshop held for an organization (Q-41-18) refused the officer's `+bay/stow` (`NO_RIGHT`): the
workshop path read `bay_work::right_of`, which knows no `held_for`; only the bay-to-bay path (`stow_held`) admitted
whoever answers for the holder. The workshop path now admits them too. In-process: `vacated_return_rs4`'s `d` is now
Brask's, T its officer, stowed from C14 (fails without).

**Story (c)'s retrieval half** (`tests/mining_cohort/rs9_story_c.rs`;
`mining_cohort::rs9_story_c_a_retrieval_is_accepted_and_paid_on_{ahuna,urvara}`): A posts a retrieval of each of two
damaged prospectors beside the ramp (`+salvage/retrieve <hull> go`); B flies out, reads `+salvage/jobs`, accepts the
first (`+salvage/accept #<job> go`), tows and loads her — `+bay` reads "Your right: retrieval" — sails home, berths her
in C14 (`+bay/berth`, the first handover) and is paid; A is told she was handed over. Nobody accepts the second: past
its 28 IC days the clerk hands it to Tycho Mutual and A is told.

**What a fixture is** (doc 36a §8's seams): the organization and its grant, the hulls placed with their power destroyed
(`@unit/damage`) and titled (`@asset/title`), the site's index (`@tacsim/site leave`), L's block claims (typed records),
the players' money (`@econ/mint`) and O's spend-down (`@econ/pay`, so the fine stands as a debt), T's and B's walk to
C14 (`@teleport`), and the world's rate.

**Not run over SSH** (in-process instead, named in each story's head): (b)'s third salvager's lawful claim awarded across
a W-clock freeze (`salvage_close`, `field_gc6`), the at-risk repair itself (`bay_36a`), the bay-to-bay `+bay/stow` with
T's carrier docked first (`vacated_return_rs4`), and the retrieval-contract `right` ground §4.9 names beside the
authorization — **not built**: `claim.rs` has no retrieval ground and a retrieval load writes no salvage row, so (b)
runs the authorization variant §4.9 allows. (c)'s charter repossession of a dormant lessee (`field_rs5`): the programme's
dormancy is counted in real days, at least one, which no bot run waits out (asked of the lead; no test knob was added).

**The vocabulary** (`mine::lane::salvage`): `+tv/<n> tow #<id> [go]` (GC.4's bare tow takes the nearest casualty),
`+tv/<n> load [go]`, RS.5's `+debts` and `+debts/pay <n>`. mush-bot's dev-dependencies gain mush-economy and mush-org
(the fixture's titles and block claims).

**What the harness found.** The Co-op's desk is open only while the readiness report reads Ready, whose rate line wants
the boot's 24: every player enrols before God lifts the rate. Two `@tacsim/site leave` writes in `prepare` share one
idempotency key (the record's serial has not moved), so the second was dropped: the index is written from God's live
session, one write at a time. An unscoped `@asset/title` to an organization is outside its scope tree's grants. Held
deposits are not available to `@econ/pay`. The fine's line wraps at the session's width: read it whole.

**Runs (by hand, conclav.dev, both sizes side by side):** (b) green at `24594391` (1,202 s at 256, 1,203 s at 512); (c) green at `df517c94`
(995 s each).
