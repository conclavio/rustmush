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

## RS.8 — the space seams (2026-11-04)

Row: §6 RS.8; Q-41-26, Q-36-46, Q-36-47. The owner moved it from the space track to the programme
session. The as-built is doc 35c §12.1. No lane rows, no config keys, no kernel hash.

**Built.** In the space tree (`space/custody.rs`, `commands.rs`, `tick.rs`, `bubble.rs`, `docking.rs`,
`picket.rs`, `mod.rs`), plus three changes outside it:
- `surface/claim.rs`:
  - `file_claim` is `+salvage/claim`'s filing, and `+helm/tow` calls it.
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

**Cell cost (rule #14).** @@CELL@@
