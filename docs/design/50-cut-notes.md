# 50. Law and the corridor — cut notes

One section a unit of [doc 50](50-law-and-the-corridor.md) §7. A unit's session replaces its own "Not built." line with
what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**, **Not built**.
Each session edits only its own section, so two units' notes never touch the same lines.

The order of work, and which unit waits on which, is the lead's map of 2026-11-19 (in brief):
- **Wave 1, no unit waits:** CR, KC.0a, OP.0.
- **KC:** KC.0a, then KC.0, KC.1, KC.2 (which also waits on CR), then KC.3 and KC.4.
- **LW:** LW.1 waits on KC.2's terms schedule; LW.2 on KC.0; LW.3 on LW.1, LW.2 and CR; LW.4 on
  LW.3; LW.5 on CR and LW.2.
- **OP:** OP.1 on KC.0; OP.2 on OP.1 and KC.0a; OP.3 on OP.2 and CR; OP.4 on OP.3 and KC.2; OP.5 on
  OP.0, OP.4 and LW.1; OP.6 and OP.7 on OP.5; OP.8 on everything.
- **Lane bumps go one at a time:** KC.0, KC.2, KC.4 and OP.1 each change the lane pack.

## CR

The cargo reading, and two live defects beside it from doc 50 §4.3: the picket and the hold's
fill check count a bulk good held as a lot; a seizure can take cargo under a haul's hold and fails
the haul; the 35 km breach records the pilot, not the hull, and writes a ruling.

**Survey** (at `ecb96979d`):
- **The reading.** `read_pool` (`tacsim/src/port_tick.rs:2626-2646`) adds `lot_tally` only for a
  part. Doc 48e had already added `read_held` beside it (`:2658-2675`), which counts lots for every
  good, and the planner, the gauges and the hull sale read through it. The picket's manifest
  (`space/picket.rs:688-704`) and the hold's fill check (`space/mining.rs:1301-1320`, read by the
  mining commands and the mining tick) still read `read_pool`. So a 51 t lot of strategic ore read
  as 0 kg and cleared the picket.
- **The seizure.** The ruling's prepare (`space/customs.rs:451-490`) took
  `min(seize_t, pool.available)` and debited it. `seize_t` is all the unlicensed ore aboard
  (`space/commands.rs:5001`). A haul's cargo is a pool hold in the hull's hold under the job's
  holder (`services/haul.rs:413-463`, `mush-inventory/src/pool/carriage.rs`), so it is not
  available. Ore held as a lot is not in the pool at all.
- **One write a key.** An `ActionTxn` writes a key once (`mush-core/src/action_state.rs`,
  `require_fresh_mutation`), so a `release` then a `debit` of one pool cannot share a bundle.
  Inventory had no call that releases a hold and debits in one write (`carry` is the precedent
  for one-write compound moves).
- **The breach.** `space/tick.rs:4639-4650` called `record_ran_the_picket_incident(ctx, hull,
  hull)` and set the outcome itself. `+helm/run` files a `Ran` ruling (`file_ruling`), and the
  incident follows the landed row, with `ruling.principal` as the pilot (`customs.rs:877-892`).

**Built:**
1. **The cargo reading: `crate::port::ship::hold_reading(ctx, hull, good)`.** It is
   `read_held` on the hull's hold vault, `Ok(0)` for a hull with no hold. The picket's manifest and
   `hull_hold_used_tonnes` read through it. KC.2's duty, LW.3's audit and OP.3's haul handling
   should call it too. `Lot` is unchanged.
2. **`InventoryFamilies::seize(txn, endpoint, commodity, amount, release)`**
   (`mush-inventory/src/pool/seizure.rs`). In one write of the pool's pair it releases the named
   live holds, in order and only while what is available falls short, then debits. It answers
   `Seized { taken, released }`. It credits nothing.
3. **The seizure takes lots and held cargo, and fails the haul.**
   - At the filing, `seizable` lists the live hauls whose strategic ore is held in that hold and
     the exact lots of it there, at most 16 of each. They ride on `PendingCustomsRuling` as
     `hauls` and `lots`, which are serde-defaulted, so the landed finding's shape is unchanged.
   - The prepare (`CustomsProvider::seize`) takes the unheld lots whole while they fit
     (`consume_products`). It then takes the pool through `seize`, naming each haul's
     reservation (`haul_reservation`).
   - For each released hold it writes `HaulLedger { acts: 2, last: LegKind::Seized }` in the same
     transaction. A later delivery or return then fails the ledger's replay guard. The customs
     provider gets the ledger handle from `haul::register`, which now returns it.
   - The haul arm sees `Seized` on the ledger. It calls `mush_economy::lapse_job`, which forfeits
     the bond to the shipper and refunds the fee, as a deadline does. Then it closes the record
     and tells the shipper and the hauler.
   - The seized ore still goes to nobody (LW.5).
4. **The breach is a run ruled as `+helm/run`'s.** The tick calls `customs::file_breach`, which
   files a `Ran` ruling with the hull's pilot as its officer. The pilot is
   `commands::helm_pilot_aboard`: the crew member aboard on a movement station, else the first of
   her crew aboard. The principal is resolved from that pilot. The incident, the corvette's turn
   and the outcome follow the landed row through `apply_landed`, as for `+helm/run`. The crew hear
   that the corvette is logging the run. A ruling already pending waits. A lapsed ruling is filed
   again on the next step, because she is still past the boundary.

**Tests:**
- `mush-inventory` `pool::seizure::tests::cr_a_seizure_releases_the_holds_it_needs_and_takes_held_cargo`:
  a debit cannot reach held cargo. `seize` releases only the named hold it needs and takes the
  rest. An unnamed hold stands, and no pool takes nothing.
- `tacsim` `domain_world` `voyage::cr_the_picket_reads_ore_held_as_a_lot_and_the_seizure_takes_the_lot`:
  51 t of strategic ore held as one lot, none pooled. The picket finds 51,000 kg, the ruling
  seizes 51 t and the lot is gone. Before the unit she was cleared with 0 kg.
- `voyage::cr_a_seizure_takes_cargo_under_a_hauls_hold_and_fails_the_haul`: a real `+haul/post`
  and `+haul/accept` of 51 t, all of it held. The ruling lands, the hold is (0, 0) and the
  ledger reads `(2, Seized)`. The arm closes the job, and the contract is `Expired`. Before the
  unit the ruling did not land.
- `voyage::cr_burning_past_the_picket_is_a_run_ruled_with_the_pilot_as_its_officer`: she burns
  flat out away from the corvette under her own drive (tests may not write the space book). Past
  35 km the tick submits one customs ruling, as the pilot. It lands
  as `Ran` with the pilot as actor and principal, the corvette is hostile and the front advances
  once. Before the unit no ruling was filed.
- Each tacsim story was mutation-checked (the fix taken out, the story fails): `read_held` back
  to `read_pool` in `hold_reading`; `seize` handed no holds; the tick's old direct record.
- Pinned sites moved with the change: the matter census (`domain_economy` `mill::sm12a_only_the_clerk_…`)
  now finds the seizure's `seize` and `consume_products` in place of its `.debit(`, and the bot's
  haul table has rows for `TOLD_SEIZED` and `TOLD_HAULER_SEIZED`.
- Re-run beside them: `voyage::sfr26c_…`, `voyage::sfr12_…`, `voyage::sfr6_…`,
  `domain_economy` `ex6_haul::`, and the tacsim lib's haul, customs and picket tests.

**Divergences:**
- **A new inventory call.** The brief kept CR to tacsim's cargo, customs and picket code. Taking
  held cargo in the ruling's one transaction needs a release and a debit in one write of the
  pair, and only `mush-inventory` can make that write. `seize` is additive, beside `carry`.
- **"Fail the job in the same transaction"** is the job's ledger row, written in the ruling's
  transaction. The contract's lapse follows on the haul arm's next fire. The economy's contracts
  are not action-state families, so no action can write them; the customs fine already follows
  this pattern. A contract that will not lapse then (a frozen economy) lapses at its own deadline.
- **Lots are taken whole.** A lot larger than what is left to take is passed over. Today
  `seize_t` is all the strategic ore aboard, so every unheld lot fits.
- **A breach with nobody of her crew aboard** has no officer to submit a ruling as. `filed_by`
  projects only to an actor aboard. The incident is then recorded at once, as before, but it
  names the hull's principal (`resolve_hull_customs_principal`) as its pilot, not the hull.

**Not built:**
- Where seized goods go (to the enforcing authority's vault, Q-50-44): LW.5.
- The full loss handling for wrecks and captures, and the canon record that advances the other
  side's clock: OP.3.
- **A held lot is not seized.** A lot under a lot hold (a listing, a dispute's evidence) is
  counted by the reading but skipped by the seizure. When all the unlicensed ore is in such a
  lot, the ruling still fails to land, as A-5 did. Releasing a lot hold in the ruling (ending
  the listing that placed it) is left to LW.3 or LW.5.
- **A haul whose hold has lapsed** but whose record is not closed is not failed by a seizure.
  Its ore is free and taken, so the return leg finds nothing and gives up after its tries.
- The fill check's switch to the reading has no story of its own. It calls the same function the
  picket's story mutation-checks.

**Fixes from doc 50b** (the outside review's CR finding; PR "50b customs P1").
- **A seizure takes the whole quantity before clearance.** One ruling takes at most
  `MAX_SEIZED_ROWS` (16) lots. A ruling that lands having taken less than it was filed for
  clears nothing: the rest is filed as a new ruling carrying `taken_before_t`, the ship stays
  uncleared, and the clearance names the whole total. Story:
  `kc2b_seventeen_lots_of_ore_are_all_seized_before_clearance` (seventeen 3 t lots end with 0 t
  aboard).
- **The seizure's lots are filtered inside the walk.** `works::unheld_lots_of` scans the lot
  family for unheld lots of the good, so lots of other goods, or held ones, no longer crowd the
  ore out of the 32 a walk returned.
- Still not built: a lot larger than what is left to take is skipped (no lot is split), and a
  held lot is still not taken.

**Fixes from doc 50b, the rest** (PR "50b customs P2").
- **The picket's run incident names its officer.** The canon incident names the officer who
  flew the run as `pilot`, as the standing loss does, and the principal who answers for the
  hull as its own field, `liable`, named among the entities when it is someone else.
- **Seized lots are offered.** `seized::offer_lots` lists each unheld lot of the good in the
  authority's vault as an exact-lot ask of its whole quantity at the seized price; the pool's
  ask never sold a lot.

## KC.0a

The faction engine that the Kirkwood chain and the Isidis operation stand on (Q-50-50, Q-50-51,
doc 50a A-1). All in `crates/mush-faction`; no caller outside it changed.

**Survey.** Each claim of §3.1 and A-1 held in code: `fire_consequence` retained every other
faction's record and pushed the new one, so a flip left two de facto holders; `since` was wall
time (`World::now`) and read by nothing; no public function wrote control; the engine's path
flipped control and never resolved the front, while `record_customs_assertion` and
`record_relay_discovery` resolved it and flipped nothing; `Clock::advance` returns at once once
completed, `@clock/set` took no count, and `clocks_watching` skips a resolved front. The canon
observer is member-gated save `WORLD_KINDS`. No crate outside `mush-faction` builds a
`ControlRecord` or a `Clock`, and nothing outside it reads `Front::resolved`.

**Built.**
- `src/control.rs`: the one control write. `ControlWrite { region, faction, kind, strength:
  Option<u8>, visibility: Option<Visibility> }` (with `ControlWrite::new(region, faction, kind)`),
  `write_control(ctx, w) -> Result<ControlChange, String>`, and `ControlChange { record, began,
  displaced: Vec<FactionId> }`. A de facto write moves the prior de facto holder out; de jure,
  covert and economic stay plural, one record a `(faction, kind)`. It persists the region's
  control document and writes no canon. A filled clock's `on_complete_control` (strength 3, the
  clock's visibility) and staff `@faction/control` both go through it; the staff verb now says
  who no longer holds the place de facto.
- `ControlRecord.since` became `since_ic`: IC seconds from `World::game_now_unix` (the world's
  clock seam; no OS-clock read), set when the hold begins. `ControlRecord::since_date()` gives
  `"March 1, 2350"`, and `+territory` prints `Holder (de facto since <date>)`.
  `FactionState::de_facto(region)` reads the one de facto record (truth, no visibility filter).
- One completion path: `advance_clock` (staff `@clock/advance`, the canon observer, the typed
  call below) marks the front resolved when its last clock fills.
- `src/lifecycle.rs` (Q-50-51): `reopen_front(ctx, FrontId) -> Result<Front, String>`;
  `set_clock(ctx, ClockId, filled: u8) -> Result<Clock, String>` (below the segments, and the
  clock is un-completed so it can fill and fire again); `advance_clock_by_cause(ctx, ClockId, by:
  u8, draft: CanonDraft) -> Result<ClockAdvance, String>`, where the draft's `source` is the
  cause id, `ClockAdvance::{Advanced { filled, completed }, AlreadyCounted}`. The call commits
  the draft, records the cause id on the clock (`Clock.causes`, newest `MAX_CLOCK_CAUSES` = 64;
  `MAX_CAUSE_LEN` = 96), then advances through `advance_clock`. Staff verbs: `@front/reopen <id>`
  and `@clock/set <id>=filled:<n>`. Help for `@front`, `@clock` and `+territory` says so.
- The canon observer is unchanged for what uses it (the Exchange board, the Red Dies, staff
  clocks, `mush-campaign`'s front flip).

**Tests.** `crates/mush-faction/tests/simulation.rs`, seven `kc0a_*` tests through the real
engine, with a test owner module that reaches the engine only by the public calls: a clock flip
moves the prior de facto holder out and keeps its de jure claim; a staff flip does too and says
so; a hold is dated from the IC clock, kept by a continuing holder, re-dated by a flip, and shown
on `+territory`; the public write seeds and flips and persists; the front resolves on its last
clock, not its first; a front reopens and a clock set back climbs and fires again (owner calls
and staff verbs); a polity clock moves by a typed cause, a retried cause writes nothing, canon
does not move it, a canon-bound clock refuses the call, and its fill flips control and resolves
the front. Four unit tests in `src/control.rs` (exclusive de facto, plural other kinds, a kept
date, an unknown name writes nothing). The dating test runs the world clock at 24:1 phased to
2350, so IC time is nowhere near wall time. Mutation-checked: dropping the de facto `retain`,
stamping `since_ic` with `World::now`, dropping `resolve_if_complete`, and dropping the cause-id
check each failed the matching test (the wall-clock mutation survived until the test set its own
world clock; the minimal world's IC time equals wall time). Each edit was undone by a reverse
edit, never by `git checkout`. Also run: every mush-faction test binary, and `mush-campaign`'s
`e7c9` (its front flip rides the canon observer).

**Divergences** (choices doc 50 does not make; the smallest taken).
- A write that finds the same faction already holding the same kind keeps its `since_ic` and
  takes the new strength and visibility: the hold goes on, it does not begin again.
- `set_clock` refuses a count at or above the segments: a clock fills only by advancing, so its
  consequence and flip have one path.
- `advance_clock_by_cause` refuses a clock that watches a canon kind, a completed clock, and a
  clock on a resolved front: a polity clock has one cause path, and a frozen front stays frozen
  until reopened. Staff `@clock/advance` still advances any clock, as before.
- `reopen_front` leaves the clocks' counts alone and `set_clock` leaves the front's flag alone;
  a repeat calls both.
- The field is renamed (`since` to `since_ic`), so stored control documents change shape; worlds
  are wiped pre-launch, and boot does not repair a stored region with two de facto records.
- Every engine-path clock now resolves its front when full, so the Red Dies, A Half Afloat and
  staff fronts close on completion where they stayed open before. Nothing reads the flag outside
  the crate.
- `write_control` writes no canon and takes no cause id; the caller that owes a line commits it.

**Not built.**
- The customs front stays on `record_customs_assertion`, which still resolves the front and flips
  nothing (KC.1). `record_relay_discovery` and `advance_relay_terms_front` are untouched.
- No public call opens a front whose clock carries `on_complete_control`, or opens one on a
  faction's board by `FactionId` with a control flip; KC.1 and OP.2 need one (the private
  `publish_board_clock` is the place).
- No public call creates a region or removes a control record; KC.0 seeds regions from content.
- TacSim's control read (KC.0); `de_facto` is a truth read with no visibility filter.

## KC.0

The six polities as factions from content, the table from lane organization to polity, the
lane's `regions` block, and the control read (Q-50-1 to Q-50-4, Q-50-52; doc 50a A-10a, C-3,
D-6). Stacked on KC.0a. Lane v45 and relay v36: **this unit's lane bump has landed on its
branch**; the next lane bump (KC.2, KC.4 or OP.1) starts from v45.

**Survey.**
- Doc 50a's corrections held. There are six tags on 24 lane organizations, in five ports.
- `launch-bringup.py` made one faction, the Republic, by `@faction/create`. The launch
  deployment then carried its id.
- `Faction.org` is one to one, so it cannot be the polity table.
- The lane had no regions and no control. Its one `region` entry is the atlas's (`sol`), so the
  new kind needed another word.
- Doc 11 §5 names organizations as holders, not only polities: the Co-op runs Kirkwood de facto,
  and Tycho, Occator and Ceres Clearing hold economically. Q-50-11 has the Co-op lose its hold.
  The Tether and OTS are no lane organizations.
- No public call made a region. `register_content_faction` needs an NPC principal.

**Built.**
- `mush-faction` (`src/control.rs`):
  - `ensure_region(ctx, name) -> Result<(RegionId, bool), String>` answers whether it made the
    region.
  - `ensure_faction(ctx, name) -> Result<FactionId, String>` makes a faction with no organization
    and no principal.
  - `contest_control(ctx, region, holder, kind, challenger) -> Result<bool, String>` adds a
    contest and never toggles it off.
- TacSim `src/polity.rs`:
  - The `regions` kind: `RegionsDef { rows }`, `RegionRow { id, name, places, holds }`,
    `HoldDef { kind: HoldKind, holder: Holder, contested_by }`, and
    `Holder::{Polity(tag), Org(lane org id)}`.
  - Pack-wide checks run in `RulesPack::from_content` (`check_regions`):
    - rows, names and places are unique and bounded;
    - a place is a `location` or `port` entry;
    - a polity is a tag some lane organization carries, and an organization is a lane
      organization;
    - a row has at most one de facto holder, and one hold per (holder, kind).
  - The kind is seed-side, the twenty-second of `OWNED_KINDS`.
  - `RegionsApplier` runs at phase 12, after the ports, and does three things:
    - it makes every tag's polity, `polity_name(tag)`: `republic` becomes "Republic";
    - it makes each row's region;
    - it seeds the region's holds through `write_control` (strength 3), with their contests,
      **only when it made the region**.
  - An organization holder's faction is its own (`ensure_organization_faction`).
- The reads:
  - `polity(w, tag)` and `polity_of_org(w, OrgId)` (the table, read from the tag on the latched
    lane);
  - `control_at(w, place) -> Option<PlaceControl { region, de_jure, de_facto, covert, economic,
    de_facto_since_ic }>`;
  - `papers_authority(w, place, enforcer) -> Option<FactionId>`: Q-50-52, the enforcer's own
    polity where it claims, else the claimant on record.
- Lane v45 seeds three rows from doc 11 §5:
  - the Kirkwood approach: de jure the Freeport; de facto the Co-op, contested by the Republic;
    economic Tycho;
  - Ceres station (the location and its port): de jure and de facto the Freeport; economic
    Occator and Ceres Clearing;
  - Phobos and Stickney Yards: the Republic in all four kinds; economic also Ceres Clearing.
- The bring-up no longer makes the Republic. It stops if the lane has not made it.

**Tests.**
- `tests/domain_world/polities.rs` is the fresh-world story on the production roster. It checks:
  - the six polities, with no organization;
  - every lane organization's polity;
  - the three rows' holders, contest and dates;
  - no reading for a place outside the block;
  - the papers rule with one claimant and with two;
  - `+territory` as a player;
  - that a flip stands through a re-apply of the block.
- Lane pins:
  - `kc0_strip_regions_rebuilds_the_published_v44`;
  - `lane_chain::strip_regions` as the strip chain's new head;
  - the v45 and relay v36 digests and versions in `lane.rs`, `cost_lane.rs`, `sites.rs`,
    `drones.rs`, `mill.rs`, `stations.rs`, `boot_parity.rs` and `relay_registry.rs`.
  - the lane-plan counts that the new entry moves by one, in `commerce.rs`, `ports.rs`,
    `sites.rs`, `vehicles.rs` and `moon_ports.rs`.
- Rigs that made a faction named "Concord" or "Republic" by `@faction/create` now meet the
  lane's polity:
  - `field_ap6`, `field_ap7`, `field_ap11` and `field_fr1` accept "already exists";
  - `m8_world`'s deployment no longer makes the Republic.
- Mutation-checked on conclav.dev. Each of these failed the story:
  - seeding a region that already stands (a flip undone by a re-apply);
  - the enforcer never reading its own papers;
  - dropping the seeded contest;
  - the polities not made.
  Each edit was undone by a reverse edit.
- Also run on conclav.dev, all green:
  - mush-faction, and tacsim's `domain_economy` (697), `domain_field` (330) and `domain_world`
    (279);
  - tacsim's other binaries, and mush-server `split_boundary` (227);
  - clippy `-D warnings` on tacsim and mush-faction.
- Run locally: the contract guards (`ci_policy`, `engine_contracts`).

**Divergences** (the smallest choice where doc 50 is silent).
- A polity's faction name is its tag with a capital letter ("Freeport", "Company",
  "Townships"). "Republic" is the name the launch deployment already reads.
- A holder may be a lane organization as well as a polity, because doc 11 §5 and Q-50-11 need
  the Co-op to hold de facto.
- The Tether and OTS hold covertly in doc 11 but are no lane organizations, so Kirkwood and
  Ceres carry no covert row.
- Region names follow the `@region/create` rule (no `&`): "Kirkwood approach", "Ceres station",
  "Phobos and Stickney Yards".
- The block seeds a region only when its apply makes the region, so a lane bump never undoes a
  clock's flip. A region renamed in content would be a new region.
- The polity table is computed on read from the latched lane, not stored.
- Where several claim de jure and the enforcer is none of them, it reads the first claimant on
  record.
- The block's retire leaves the regions, the control and the polities standing.

**Not built.**
- Isidis (OP.1), the moons, Titan, Deimos and Occator claims rows.
- No public call yet opens a front whose clock flips control on a faction's board by
  `FactionId`; it did not fall out of this unit (KC.1).
- No polity has a backing organization or a treasury (KC.2, Q-50-14).
- No compiled caller reads `control_at` yet (KC.2 on).

**Fixes from doc 50b** (PR "50b customs P2").
- **One regions block a pack.** The decode refuses a second block, which every read would have
  ignored. Story: `kc0b_a_second_regions_block_is_refused`.

## KC.1

Customs Assertion on the Republic's board, seeded at world start, completing through the
engine's one path with a control flip (Q-50-12, Q-50-13; with Q-50-11, Q-50-14, Q-50-16, Q-50-51;
doc 50a A-1; doc 11 §6). Stacked on KC.0. No lane bump.

**Survey.** As §3.2 says:
- `record_customs_assertion` made the front lazily, on the first incident, on the Freeport
  Registry's faction (the license issuer's org).
- It advanced the clock by hand, appended the consequence sentence to the incident's own line,
  resolved the front and flipped nothing.
- Its three callers in `space/picket.rs` (the fine, the seizure and the run) pass a canon draft
  whose `source` already names the incident: `space.picket.fine/<hull>/<contract>`, and
  `.../seize/` or `.../run/` with `<hull>/<ic>`.
- A Cleared ruling never calls it.
- `voyage.rs`'s picket stories read the front by title, not by board.

**Built.**
- `mush-faction`, the opener KC.0a and KC.0 left undone:
  `open_faction_front(ctx, board: FactionId, spec: &FrontSpec, draft: CanonDraft) -> Result<FrontId, String>`.
  - `FrontSpec { title, clock_title, segments, initial, consequence, control: Option<(RegionId, ControlKind)>, visibility }`.
  - The clock watches no canon kind (Q-50-51), so only `advance_clock_by_cause` moves it.
  - It is idempotent by title on the board and makes one canon append.
  - The private `open_front_clock` now carries the clock's control and visibility.
- `seed_customs_assertion_front(ctx, republic, approach, draft)`:
  - six segments opening at one (`CUSTOMS_ASSERTION_SEGMENTS`, `CUSTOMS_ASSERTION_OPENS_AT`);
  - the flip is the approach, de facto;
  - the consequence is `CUSTOMS_ASSERTION_CONSEQUENCE`.
- `record_customs_assertion(ctx, board: FactionId, draft)` now takes the board, not an org:
  - It finds the seeded front and moves it one through `advance_clock_by_cause`, with the
    incident's `source` as cause id. The fill fires `fire_consequence`: the Republic takes the
    approach de facto, displacing the Co-op, the hold is dated in IC time, and the front resolves.
  - After the fill, an incident still commits its line and moves nothing.
  - A front not seeded on that board is an error. The picket swallows it, as it did before.
- TacSim:
  - The regions applier seeds the front after its rows, on `polity(republic)`. Its flip is the
    row holding the `location` with `kirkwood_picket`.
  - `polity::customs_board(w)` is the Republic. The picket's three calls pass it; the fine is
    still paid to the Registry (Q-50-14).
- Nothing reads the new holder yet (the transit duty is KC.2). `+territory` and `+chronicle`
  show the flip.

**Tests.**
- `tests/domain_world/polities.rs`,
  `kc1_picket_incidents_fill_customs_assertion_and_the_republic_takes_kirkwood`, on a fresh
  production world. It checks:
  - the front on the Republic's board at 1 of 6, with its flip, and on `+clocks` from the first
    day;
  - four incidents fill it to 5, and a retried cause moves nothing;
  - the sixth flips Kirkwood de facto to the Republic, dated at the flip's IC time, and moves the
    Co-op out;
  - the front resolves, and a later incident records its line and moves nothing;
  - `+territory` and `+chronicle` show the flip.
- Mutation-checked on conclav.dev; each failed the story, and each was undone by a reverse edit:
  - the seed with no flip;
  - incidents bypassing the cause call;
  - the front not seeded;
  - the clock opening at zero.
- The picket's own stories in `voyage.rs` and the rest of tacsim, mush-faction and mush-server
  `commerce_composition` ran on conclav.dev; clippy `-D warnings` too.

**Divergences** (the smallest choice where doc 50 is silent).
- The consequence line is new: "Republic Customs holds the Kirkwood approach: the picket now
  answers to the Republic." The old sentence said the picket "seizes unlicensed strategic ore
  shipments", which Q-50-15 rules does not change.
- The consequence is now its own `front.progress` canon line. It is no longer appended to the
  sixth incident's line.
- The front is seeded by the lane's regions applier, the apply that makes the Republic and the
  approach, not by a boot pass.
- A world whose lane has no picket row, or no Republic, has no front, and its incidents advance
  nothing (they did, lazily, before).
- A seizure's and a run's cause id is `<hull>/<IC second>`, as the picket already wrote it. Two
  such incidents on one hull in one IC second would count once.

**Not built.**
- The transit duty, the payee and any reader of the new holder (KC.2).
- No push-back clock (Q-50-16).
- `record_relay_discovery` and `advance_relay_terms_front` still advance by hand.

## KC.2

The terms schedule and the transit duty (Q-50-5, Q-50-6, Q-50-10, Q-50-11, Q-50-14, Q-50-53 to
Q-50-55). Stacked on KC.1, with main (CR) merged in. **A lane bump: lane v47 and relay v38**, on OP.1's v46 (#719, merged into this branch). It
must land after OP.1's.

**Survey.**
- **The picket's submission** (`space/commands.rs`, `submit_hull_to_customs`) chooses Cleared,
  Fined or Seized and files one `tacsim_space_customs` action. The tick levies the fine through
  a contract to the Freeport Registry (`picket.rs`, `levy_customs_fine_and_advance_front`). A
  pilot who cannot pay the fine has the contraband seized.
- **The picket opens** only when a connected player stands on the hull or the hull has a
  link-loss record (`composition/sim_routes.rs`, `boundary`).
  - A link-loss hull is submitted by the backup seat.
  - A hull with nobody aboard and no record passed the picket untouched.
- **CR's one hold reading** is `port::ship::hold_reading` (pooled plus lots, in the good's ledger
  unit). It includes cargo under a haul's hold, which is a reservation on the pool.
- **The value.** A good's catalog baseline is `GoodDef.reference_major`, in major credits per
  quote unit; every good's quote unit is its ledger unit. Ship's stores are the goods with a
  `consumable` (propellant, provisions).
- **There is no filing verb.** A passage is filed by `+ship/undock` (`prepare_departure`, which
  writes a `PendingVoyage`). The launch writes `Departed`, which outlives the passage.
  `+helm/autopilot` from free flight writes no pending; it rewrites `Departed`.
- **The shortcut.** `owed_interdicted_leg` reads the port of departure's `routes[dest]`. Ceres
  lists no `deimos`, so a free-flight autopilot from Ceres' halo to Deimos owed nothing (§4.3).
- **Debts with grace** already exist: `mush_economy::create_debt_due` in `tacsim.debt`, with
  `debt_due` (28 IC days) as port security uses it.

**Built.**
- **The schedule** (`tacsim::polity`):
  - A regions row gains `terms: BTreeMap<polity tag, TermsRow>`.
  - `TermsRow { payee, transit_duty_permille, transit_floor_major, import_duty_permille,
    import_goods, restricted: Vec<RestrictedGood { good, paper }>, audit_permille,
    fine: Option<FineScale { per_t_major, floor_major }> }`.
  - It is checked pack-wide: a known polity, a lane org as payee, rates of 1,000‰ at most,
    majors of 1,000,000 at most.
  - The reads:
    - `terms_at(w, place) -> Option<PlaceTerms { holder, since_ic, row }>` gives the de facto
      holder's polity's row (`row: None` means the terms as they are today);
    - `picket_place(w)`;
    - `lane_org_account(w, lane id) -> Option<(OrgId, AccountId)>`.
- **The duty** (`tacsim::space::duty`):
  - `hull_terms(w, hull)` gives the terms pinned on the passage, else the live ones.
  - `dutiable_value_major` is every good with no `consumable`, pooled or as a lot, at its
    catalog baseline.
  - `quote_duty` is `transit_duty_permille` of the value, never under the floor, and nothing for
    an empty hold or a row that levies no duty.
  - `levy_transit_duty(ctx, hull, debtor)` opens a debt to the payee in `tacsim.debt`, due in 28
    IC days, and pays it at once when the account covers it. `DutyOutcome::{Paid, Owed(id)}`.
  - Also `duty_line`, `assess_absent`, `settle_absent` and `fine_payee`.
- **At the picket:**
  - `+helm/submit` levies the duty on the inspected hull, whatever the finding, and the line
    names it. It never changes the finding: lawful cargo is never seized for a duty.
  - A hull with nobody aboard and no link-loss record crosses the interdicted leg with no picket.
    The leg's boundary notes her in `SpaceBubbleBook::absent_duty`, because that hook cannot read
    a hold. The next space tick assesses her by `assess_absent` (`settle_absent`). The debtor is
    the hull's customs principal; her absent tenant is told.
  - A link-loss hull goes through the backup seat's submission as before, and so pays the same
    way.
  - Running stays a present pilot's choice.
- **The payee** (Q-50-14). The contraband fine goes to `fine_payee`: the terms row's payee once
  the holder has a row, else the Registry, as built. Lane v47 adds the Republic Customs Office
  (`republic_customs`, at Stickney, capital 10,000, provisional), the payee of the Republic's
  Kirkwood row.
- **Pinning** (Q-50-10):
  - `PendingVoyage.terms` is set at filing to the picket's live terms, and copied to
    `Departed.terms` at the launch.
  - An engagement from free flight rewrites `Departed.terms`.
  - The picket reads the pinned terms.
- **The shortcut** (Q-50-55). A destination the port of departure lays no route to owes the
  first interdicted leg of any route out of that port, so Ceres' halo to Deimos flies through
  the picket. Luna, routed by the long way, owes nothing, and the outward run stays free.
- **Lane v47:**
  - the Kirkwood row's `terms.republic` (payee `republic_customs`, 20‰, floor 50 cr);
  - the customs office row;
  - **for LW.1**, the Ceres row's `terms.freeport`: payee `freeport_registry`, `strategic_ore`
    restricted unless `freeport_prospecting`. That paper id stands for the picket's two-paper
    test (Q-50-32, Q-50-65): a Freeport Registry prospecting license and a registered claim, both
    held. It is inert in KC.2: the duty reads only the Kirkwood row's transit duty and payee, and
    nothing reads `restricted` until LW.1;
  - relay v38, re-signed to require it.

**Tests.** In `tests/domain_world/voyage.rs`, on the picket rig:
- `kc2_the_transit_duty_before_and_after_the_flip_is_pinned_at_filing`, three cases:
  - under the Co-op, no duty;
  - the Republic holding Kirkwood at filing: 200 cr on 50 t of metals held as a lot (10,000 cr
    of catalog value), paid to the customs office, and cleared;
  - filed under the Co-op and flipped en route: no duty.
- `kc2_a_pilot_who_cannot_pay_is_cleared_and_owes_the_duty`: a 200 cr debt to the office, the
  hull cleared, the metals untouched.
- `kc2_a_hull_whose_pilot_is_away_is_assessed_and_cleared`: no picket, and the duty is paid
  from her pilot's account.
- `kc2_the_free_flight_shortcut_to_deimos_owes_the_picket_leg`: Deimos owes the interdicted leg
  and Luna does not.
- Lane pins: `kc2_strip_terms_rebuilds_the_published_v46` (OP.1's digest); `lane_chain::strip_terms` as the new
  head; the v47 and relay v38 digests and versions.
- The count pins the customs office moves (`plumbing.rs`, `ports.rs`, `boot_parity.rs`) and
  `ic_sortie_voyages.rs`'s departure record (now carrying the pin).
- Mutation-checked on conclav.dev; each failed a story and was undone by a reverse edit:
  - ignoring the pin;
  - a duty never paid at once;
  - the tick never settling the absent hull;
  - the shortcut left open.
  A first absent mutation, which suppressed only the line, survived: `assess_absent` still
  levied. The tick-side mutation replaced it.
- Green on conclav.dev on the merged branch (4e645b10a) with OP.1 in, all green: clippy `-D warnings` (tacsim,
  mush-faction), every tacsim binary (`domain_economy` 700, `domain_field` 330, `domain_world`
  288, the lib 2,501 among them), mush-server `split_boundary` (227), `commerce_composition`
  and the `player_market` lib tests.
- Earlier, before the merge: clippy `-D warnings` on tacsim; tacsim's `domain_economy` (698),
  `domain_world` (291) and `domain_field`. With the duty levied at the boundary, before the move
  to the tick: tacsim's other binaries and mush-server `split_boundary` (227) and
  `commerce_composition`.
  - `split_boundary`'s `ic_field_a_dt_recover_whose_carrier_is_lost_keeps_the_unit_in_the_field`
    failed once and passed on the base and on a rerun with the patch: a flake.
- Locally: rustdoc in both modes and the contract guards.

**Divergences** (the smallest choice where doc 50 is silent).
- **Every good with a `consumable` is a store, even held as a lot or under a haul's hold.** Q-50-53
  makes propellant in a lot cargo; the known gap it accepts for unreserved propellant is widened
  to every propellant and provision. Flagged for tuning.
- **The duty is levied at the submission** (the command), not when the ruling lands, and on any
  finding but a run. A seized hull owes it as a debt like anyone who cannot pay.
- **The shortcut owes the picket's leg** rather than flying the long way's legs. The rule reaches
  every destination the port of departure lays no route to: from Ceres that is Deimos and Titan.
- **The pin is the picket's terms**, taken whatever the route, on every filing.
- **The payee row lives at Stickney** (a port's org row needs a port). The fine scale, the import
  duty, the restricted goods and the audit rate are carried and checked, and read by nothing yet.
- **Q-50-39's penalties past the grace** are not built for the duty: the debt stands in `+debts`
  and goes delinquent, and nothing quarantines on it yet.

**Not built.**
- The duty estimate on `+ship/plan`, the filing and the job board, and the reason clause (KC.3).
- The Co-op's share by control (KC.4).
- The import duty and the restricted goods (LW.1, OP.5).
- The penalties past the grace.

**Fixes from doc 50b** (the outside review's KC.2 findings; PR "50b customs P1").
- **The shortcut rule is narrowed (Q-50-55).** The fallback leg is never owed for the departure
  port's own venue (a Ceres halo's way home, autopilot or the link-dead return), and is owed for
  a destination only when a lane route with no interdicted leg joins it to that route's far end
  (from Ceres: Deimos and Isidis, not Titan). Story:
  `kc2b_the_return_from_the_ceres_halo_meets_no_picket`.
- **The fine before the duty, one duty a crossing.** `levy_transit_duty` pays the duty only from
  what a fine found on the same crossing leaves (`reserve_minor`), else it is owed. `levy_once`
  charges it once a crossing, keyed by hull and journey cursor (`SpaceBubbleBook::duty_crossings`),
  so a resubmission pays none. Story:
  `kc2b_the_fine_is_paid_before_the_duty_and_the_ore_is_not_seized` (1,100 cr, a 1,000 cr fine:
  the fine paid, one duty owed, the ore not seized).
- **An absent hull is assessed the same way (Q-50-54).** `assess_absent` reads the manifest
  through `customs_finding`, the helper `+helm/submit` now uses too, and files a fine or a
  seizure as a ruling with the principal as its officer. A clean hold is cleared at once. Story:
  `kc2b_an_away_pilots_hull_is_fined_for_unlicensed_ore`.
- **An absent crossing's leg is marked cleared** at the boundary, so another carrier's open
  picket never takes her in as awaiting heave-to.
- **A restart.** The live hook records each Kirkwood crossing (`picket_crossings`). Before any
  interdiction is promoted, the space tick queues every crossing a journey's cursor has passed
  that the hook never saw (boot recovery skips it) as an absent one, its leg marked cleared.
  Story: `kc2b_a_restart_across_the_picket_leg_still_charges_the_duty_once`.
- **Divergences** (the smallest choice where the review is silent):
  - The far-side test looks one route hop from the picket route's far end.
  - A passage that arrives during an outage leaves no journey to read, and is not assessed.
  - An absent hull's ruling lines name the "Kirkwood picket" where a corvette would be named.

**Fixes from doc 50b, the rest** (PR "50b customs P2").
- **A duty past its grace binds (Q-50-54).** An owed duty writes an offense row
  (`OffenseKind::UnpaidDuty`, `Evidence::Duty`) of the payee's polity against whoever answers
  for the hull, carrying the duty's debt. The departure hold reads it as a fine, and the trade
  restriction counts it only once the debt is past its grace. Story:
  `kc2b_a_past_due_duty_holds_the_hull_and_restricts_trade`.
- **The seizing authority follows the terms payee (Q-50-14, Q-50-44).**
  `seized::seizing_authority_for` routes a seizure through the payee of the hull's terms row,
  at the lane port that lists it, else the Registry. Story:
  `kc2b_after_the_flip_a_seized_lot_goes_to_the_customs_office_and_is_offered`.
- **A hauling house's fine is the house's (Q-50-56).** The contraband fine is checked and paid
  from `house_account_of_helm`, as the duty is.
- **Lane content, not built here:** the customs office's org row gains `"roles": ["acceptor"]`
  in OP.7b's v51, without which a fine after the flip cannot be accepted by its payee.

## KC.3

The duty line on `+ship/plan`, the filing and the job board, and the reason clause; the first
acceptance story, which also measures the long way round (Q-50-8, Q-50-9, with Q-50-55). Stacked
on KC.2. No lane bump.

**Survey.**
- **`+ship/plan`** is `port/voyage.rs`'s `plan_block`, one block per destination, ending at the
  ETA and the window line.
- **The filing's reply** is `undock_as`'s `CLEARANCE_FILED`, and KC.2 already pins the terms on
  the `PendingVoyage`.
- **The haul board** is `+market/jobs haul`. Its rows come from `services::haul::services`, with
  the `subject` rendered by mush-market's `JOB_LINE`. A haul carries its origin and destination
  venues, its good and its quantity, but no route.
- **A route crosses the picket** when one of its legs is interdicted and ends at the picket's
  host.
- **Q-50-9's IC date** is the `since_ic` KC.0a stores.
- **No shared 120-column constant** exists; tests assert widths line by line.

**Built** (`tacsim::space::duty`).
- `duty_on_value(w, value_major, row) -> Option<i64>`: the duty arithmetic on a value, for cargo
  not yet aboard. `quote_duty` now uses it.
- `route_crosses_picket(w, route) -> bool`.
- `reason_clause(w, &PlaceTerms) -> Option<String>` gives `Republic hold since March 1, 2350`:
  the de facto holder's name and the IC date its hold began.
- `estimate_line(ctx, hull, route, &terms, pinned)` gives
  `Kirkwood duty ~200.00cr on 10000 cr of cargo (Republic hold since …).`, with `, pinned at
  filing` when `pinned`.
  - `+ship/plan` shows it under the live terms, on a route that crosses the picket.
  - The filing shows it under the terms it pinned.
- `haul_clause(w, origin, destination, value_major)` adds ` (Kirkwood duty ~50 cr)` to a haul
  row's subject when the origin port routes to the destination through the picket, under the
  live terms. The value is the good's catalog baseline × quantity.
- The picket's charge (`duty_line`, now taking the hull) ends with the same clause: `… is paid
  (Republic hold since …).` and `… (see +debts; Republic hold since …).`
- `+territory` and `+chronicle` already show the same hold and date (KC.0a, KC.1).

**Tests.**
- `tests/domain_world/voyage.rs`,
  `kc3_the_same_run_before_and_after_the_flip_names_the_duty_and_why`: one Dray-P with 50 t of
  metals, on one run.
  - Under the Co-op, `+ship/plan` shows no duty.
  - After the flip, `+ship/plan` shows the 200 cr estimate with the reason.
  - `+ship/undock` shows the pinned estimate with the reason.
  - The picket's charge carries the same clause.
  - Every duty line fits in 120 columns.
- `tests/domain_economy/ex6_haul.rs`, `kc3_the_job_board_shows_the_duty_a_hauler_will_meet`: a
  6 t metals haul to Stickney shows no duty under the Co-op, and `(Kirkwood duty ~50 cr)` (the
  floor) after the flip, within 120 columns.
- `tests/domain_world/moon_ports.rs`,
  `kc3_the_long_way_round_costs_time_the_direct_run_pays_the_duty` (pure planner, laden 100 t,
  one soul, full throttle, at the epoch):
  - Direct Ceres to Stickney: 1,448,368 s (16.8 IC days), 47 t.
  - The long way by Luna and Deimos: 2,516,632 s (29.1 IC days), 82 t.
  - The detour's extra 35 t is 1,400 cr at Ceres' 40 cr a tonne. The duty on 100 t of metals
    is 400 cr, so the direct run stays worth taking for ordinary cargo at the provisional 2%.
- Mutation-checked on conclav.dev; each failed its story and was undone by a reverse edit:
  - the plan's line dropped;
  - the reason clause's date dropped;
  - the filing's line dropped;
  - the board's clause dropped.
- Green on conclav.dev: clippy `-D warnings` on tacsim, and every tacsim binary. Locally:
  rustdoc in both modes and the contract guards.

**Divergences** (the smallest choice where doc 50 is silent).
- **The reason clause names the holder and the date, not the region** (`Republic hold since
  March 1, 2350`); the line says Kirkwood. Every line fits in 120 columns that way.
- **The board's duty is the major-credit ceiling** of the duty on the catalog value, under the
  live terms. A hauler who files later pins the terms of that day.
- **A haul shows the duty when any of its origin port's routes to the destination crosses the
  picket**, whichever route the hauler later flies.
- **The plan's estimate reads what is aboard now**; an empty hold shows no line.

**Not built.**
- The Co-op's share by control (KC.4).
- A duty line on a queued departure's filing. It files through the executor, which writes no
  reply.

**Fixes from doc 50b** (PR "50b customs P2").
- **The board walks the haul (Q-50-8).** `haul_clause` reads the fewest hops of listed routes
  between the two venues, port to port, so a Ceres to Isidis haul, flown by Stickney, carries the
  duty clause. Story: `kc3b_the_boards_duty_walks_a_haul_port_to_port`.
- **A queued order names the duty.** The queued-order projection adds the estimate under the
  live terms, and the order's filing tells the pilot the estimate under the terms it pinned.
- **The acceptance story (Q-50-47).** `kc3_the_same_run_before_and_after_the_flip_names_the_duty_and_why`
  gains a second account that files the same run before the flip: no duty line, the Co-op's
  terms pinned, and the flip leaves them so.
- **Divergence (the owner's ruling of 2026-11-20):** the story stays at 3,600:1, not 24:1.

## KC.4

The Co-op's enrollment share by control (Q-50-7). Stacked on KC.3. **A lane bump: lane v48 and
relay v39**, from KC.2's v47.

**Survey.**
- The share was one number, the lane programme's `share_permille` (250). It is frozen on the
  enrollment at `+coop/enroll` (`EnrolPlan.share_permille`) and read by the proceeds at every
  sale.
- Two lines of copy said "a quarter": the registrar's line (`REGISTRAR_LINE`, shown by `+coop`)
  and the `+coop` help (`COOP_HELP`).
- `+coop`'s terms line printed the programme's permille. `docs/help/coop.md` describes the share
  in the same words.

**Built.**
- A terms row gains `coop_share_permille: Option<u16>`, checked at 1,000 at most. Lane v48 sets
  300 on the Republic's Kirkwood row.
- `port::coop::signing_share_permille(w, programme_share) -> u32` gives the share a new
  enrollment signs at: the Kirkwood approach's live terms row's `coop_share_permille` when the
  holder's row sets one, else the programme's. `+coop/enroll` freezes it on the enrollment.
  Standing enrollments keep what they signed at; the proceeds still read the enrollment.
- **The copy:**
  - `REGISTRAR_LINE` is a template (`{share}`), filled by `registrar_line(permille)` with
    `share_words` ("a quarter" at 250, "three tenths" at 300, "half" at 500, else the permille).
  - `+coop`'s terms line prints the signing share.
  - `COOP_HELP` and `docs/help/coop.md` say the share is a quarter, or more while a holder of the
    Kirkwood approach sets it so, fixed at signing.
- **Lane v48:**
  - the Republic row's `coop_share_permille: 300`;
  - relay v39, re-signed to require it;
  - `lane_chain::strip_coop_share` as the chain's new head, with
    `kc4_strip_coop_share_rebuilds_the_published_v47` holding KC.2's digest;
  - no venue word or other client-pinned list moved, so the SSH and web pins are unchanged.

**Tests.**
- `tests/domain_economy/coop.rs`,
  `kc4_new_enrollments_sign_at_the_holders_share_and_standing_ones_keep_theirs`:
  - under the Co-op's hold, `+coop` offers a quarter and 250‰, and the first captain signs at 250;
  - after the flip, `+coop` offers three tenths and 300‰, and the second captain signs at 300;
  - the first captain's enrollment keeps 250.
- `ic6_enrol_once…` reads the registrar line through `registrar_line(250)`.
- Mutation-checked on conclav.dev; each failed the story and was undone by a reverse edit:
  - the signing share ignoring the row;
  - "three tenths" spelled as a quarter.
- Lane pins moved to v48 and relay v39: `lane.rs`, `cost_lane.rs`, `sites.rs`, `drones.rs`,
  `mill.rs`, `stations.rs`, `boot_parity.rs`, `relay_registry.rs`.
- Green on conclav.dev: clippy `-D warnings` on tacsim, every tacsim binary, mush-server
  `split_boundary` and `commerce_composition`, and mush-bot's `coop_sentences` (the registrar
  line's last sentence, which the bot mirrors, is unchanged). Locally: rustdoc in both modes, the
  contract guards and the help lint.

**Divergences** (the smallest choice where doc 50 is silent).
- The share is read from the Kirkwood approach's terms (the region the flip changes), whichever
  port the Co-op signs at.
- Shares with no word of their own print as permille in the registrar's line.

**Not built.**
- Nothing of Q-50-7 is left. The Co-op's other terms (stake, buyout) are unchanged by control.

## LW.1

**Survey** (at `f7342fe34`, #720's head with the Freeport's Ceres row):
- **The schedule.** KC.2's `TermsRow` carries `restricted: Vec<RestrictedGood { good, paper }>`,
  and nothing read it. Lane v47's `ceres` row restricts `strategic_ore` under paper
  `freeport_prospecting`, the Freeport's terms. KC.2 added the row for this unit.
- **The market's seam.** `ProceedsPolicy::proceeds` is asked at a sale into a bid and when a
  personal lot ask is bought, with the world only. `may_list` is asked at a personal ask's posting
  without the good. Nothing is asked of a buyer of a house ask, or of a public bid's poster.
- **The papers.** The picket's test is `licence_gate` (the Registry's license, in force or in
  grace) plus a live claim on a strategic rock in an orbital cluster, held by the actor, by their
  gateway account, or by an organization they are an active member of
  (`space/mining.rs` `actor_holds_cluster_block_claim`).
  - The license reads from the world.
  - The claim is a `tacsim.claims` action-state record, read only through tacsim's own lens. A
    market command's context is the market's, so it cannot read it.

**Built:**
1. **`ProceedsPolicy::may_trade(w, trader, market, commodity)`** (mush-market, default yes).
   - The market asks it, through `module::law_of_trade`, of either side of a public trade:
     - the seller at `sell_into_bid`;
     - the buyer at `quote`;
     - the poster of a public `post_ask` (Q-50-33: a listing is refused when posted);
     - the poster of a public `post_bid`.
   - A private offer (`private_to`) is never asked (Q-50-34).
2. **`crate::port::law`:**
   - `restriction_at(w, venue, good)` is the venue's de facto holder's `restricted` row
     (`polity::terms_at` at the venue's port entry).
   - `papers_clear(w, actor, now)` is the picket's test, Q-50-65's one test.
   - `may_trade` refuses a restricted good to anyone without its paper, with
     `RESTRICTED_REFUSED`. An organization is refused, because it holds no prospecting license.
     A paper this crate does not know lifts nothing.
3. **The claims mirror (`PapersMirror`).**
   - The port tick copies every claim on a strategic rock into the store (`law::refresh`).
   - The board reads the copy.
   - Both the board and the picket ask one predicate, `space::mining::claim_covers`, factored
     out of the picket's live read.
4. **`CoopProceeds::may_trade`** wires it.

**Tests:**
- `domain_economy` `lw1_restricted_goods::lw1_the_ceres_board_trades_strategic_ore_only_with_the_picket_papers`:
  - A licensed prospector with no claim is refused selling strategic ore into the Exchange's bid.
    With a claim as well, the sale goes through.
  - Their public ask is refused to a trader with no license, and still refused to the trader
    holding a claim.
  - A private offer to the same trader is quoted.
  - After Ceres station is handed to the Republic, whose terms restrict nothing there, the
    trader may quote the public ask.
  - Mutation-checked, each failing the story: the law never restricting; the papers asking the
    license alone; the papers asking the claim alone; the market asking a private offer too.
- **Stories that trade strategic ore on the Ceres board now hold the papers**
  (`support::licence::hold_the_papers`): `in11_matter`'s purchase and `plumbing::sm5`'s sale.
  `coop`'s two port-trade frame stories pick a live ask other than strategic ore.
- Re-run beside it:
  - `mush-market` lib;
  - `domain_economy` `ex1b_venue::`, `held_lots_48e::`, `ex4_charges::` and `lw2_offense_record::`;
  - `domain_world` `voyage::sfr26c`, `sfr6`, `sfr5` and `cr_*` (the picket on the factored claim
    test);
  - the tacsim lib's proceeds, mining, picket and law tests.

**Divergences:**
- **A new policy method, not a wider `may_list`.** Q-50-33's correction has `may_list` gain the
  commodity. `may_trade` is asked at every public posting and also of buyers and sellers. Its
  rule is per venue and good, not per source.
- **The board reads a mirror of the claims, at most one port fire old.** The market's policy
  sees the world alone, and the claims are a lens-guarded family. A claim staked or lapsed this
  fire is seen on the next. The picket keeps its live read; both share `claim_covers`.
- **Houses are not asked.** A house's own posting or restock never passes through the personal
  paths. A player who buys from a house ask or sells into a house bid is asked.
- **The buyer is asked at the quote, not again at the accept.** Papers that lapse between the
  two do not stop the purchase.

**Not built:**
- **Possession as an offense, found by a port audit** (Q-50-65's third place): LW.3.
- **Import duty and Isidis's restricted goods:** OP.5.
- **No lane bump.** The Ceres row is KC.2's v47.

## LW.2

**Survey** (at `f668f082a`, #713's head):
- **Port security's record** was `tacsim:security:{actor}` (`surface/security.rs`), one per
  character, rows copied from a site's evidence log at the proof (a vacated `recovery` contest,
  `claim.rs:3258`, or `+salvage/report … registry`). Nothing else wrote or read it, beside the
  rs7 story.
- **`SECURITY_VENUES = ["ceres", "stickney"]`** gated only the fine at docking (`on_docked`,
  called from `composition/sim_routes.rs` for a port, never a site). The fine went to the Freeport
  Registry's account at both ports.
- **The quarantine** (`quarantine_line`, `port/voyage.rs` `prepare_departure`) had no venue test.
  A past-due fine held every hull its debtor helms or holds title to, at every berth, a site's pad
  included (A-10c).
- **Who answered** was the helm holder plus a character title holder. For an organization's hull
  docked with nothing filed, `helm_holder` is `None` and the title holder is an organization, so
  nobody answered (A-10b).
- **KC.0's read:** `polity::control_at(w, place)` takes a lane port entry id (`ceres_port`,
  `stickney_port`). Sites are in no region. `polity_of_org` maps the Freeport Registry to the
  Freeport.
- **Each port's `authority`:** Ceres is `freeport_registry`, Stickney is `stickney_yards`.

**Built:**
1. **The record (`crate::offense`).**
   - The key is actor by polity: `OffenseKey { offender: Offender, polity: FactionId }`, stored as
     `tacsim:offense:{c<obj>|o<org>|a<account hash>}:{polity}`.
   - The actor is `Offender::Character(ObjId)`, `Organization(OrgId)` or
     `Account(mush_core::AccountId)` (Q-50-68).
   - The kinds are a closed list, `OffenseKind`: `SalvagePickup`, `SalvageScrap` and
     `SalvageRecovery` today.
   - Evidence stays where it is, and each row names it: `Evidence::Site { site, act }` copies the
     ring row. `Proof` moved here.
   - A row is `ProvenOffense { kind, evidence, proof, proven_ic, debt }`. The record is
     `OffenseRecord { schema, key, rows ≤ 64 }`, loaded at boot. An unreadable record claims its
     key and proves or fines nothing.
2. **The API LW.3, LW.4 and LW.5 build on:**
   - Writing: `record_offense(ctx, key, offense, settled)` is the one function every prover
     calls. It dedupes, and a full record drops its oldest settled row.
   - Reading: `record_of(w, &key)` and `records_against(w, offender)`.
   - Where an offense binds: `jurisdiction_at(w, place)`, the place's de facto holder, and
     `binds_at(w, polity, place)`.
   - Who answers: `answerer_for_hull(w, hull)` is the organization for an organization's hull,
     else the helm holder.
   - Receipts: `put_receipts` (crate-only) rewrites a record whose fine receipts changed.
   - The polity that proves a salvage offense: `surface::security::proving_polity(w)`.
3. **Port security on the record.**
   - `prove` writes `Offender::Character(act.actor)` under `proving_polity`. That is the
     Freeport Registry's polity, the Freeport.
   - `on_docked(ctx, hull, entry, port)` fines the record of (the hull's answerer, the port's de
     facto holder). The fine goes to the port's governing authority's account.
   - `quarantine_line(ctx, hull, entry, (name, port))` and `holds_at(w, hull, entry)` hold the
     hull that docked, only at a port its polity holds.
   - `SECURITY_VENUES`, `answers_at` and the old `SecurityRecord` are gone. Pre-launch, so there
     is no migration of `tacsim:security:` documents.
4. **An organization's hull answers through its organization.** Its fine is drawn on the
   organization's account (`port::berth::org_account`, now `pub(crate)`).
5. **The fine's lines** name the authority (`The {authority} fines you …`). The owed line says
   the fine holds her "at this polity's ports".

**Tests:**
- `tacsim` lib `offense::tests`: the key is actor by polity, and a document key parses back. The
  record's kinds and its one-actor rule are checked.
- `domain_economy` `lw2_offense_record::lw2_an_offense_binds_only_where_its_polity_holds_and_the_house_answers_for_its_hull`:
  - A House's Dray carries a Freeport offense against the House, seeded as a stored record the
    boot reads.
  - Docked at Stickney, the Republic's port, nothing is fined.
  - Docked at Ceres, the House is fined 5,000 cr to the Freeport Registry, and the receipt is on
    the House's record.
  - The grace runs out while she is at Stickney, and she lifts from there.
  - Home at Ceres, she is held. Staff hand Ceres station to the Republic de facto, and she may
    lift.
  - Mutation-checked: an offense binding wherever its polity proved it; an organization's hull
    answered by its pilot; the quarantine ignoring the port. Each fails the story.
- `port_security_rs7::` (both sites) is moved onto the record: O's Freeport record, its kinds and
  its receipts, `holds_at` at Ceres.

**Divergences:**
- **The proving polity is the Freeport Registry's polity**, not the de facto holder of the place
  of the act. Sites are in no region, and the Registry is the one adjudicating. If Ceres changes
  hands, a Freeport proof binds nowhere until the Freeport holds a port again.
- **An organization's fine is levied with staff authority.** At a docking, no character acts for
  the organization, and the economy lets only an authorized actor open a debt on an organization's
  account. A character's fine is still opened and paid as that character.
- **A member's own offense is not fined when they dock an organization's hull.** The hull answers
  through the organization only, as Q-50-68 words it. The member answers when they dock a hull
  they command.
- **A fine with no authority account at the port waits** (logged). Every lane port with a
  jurisdiction authors one today.

**Not built:**
- **No path writes an organization's or an account's offense yet.** No salvage verb takes
  `as=<org>`, so the story seeds one. The license bar on the account is LW.4.
- **An offender with no hull is not fined when proven** (Q-50-68's last line). Port security
  still fines at a docking only; immediate fines belong with LW.4's penalties.
- **No lane bump.**
- Fine sizes, the standing loss, the trading restriction, `+record` and `+debts`: LW.4.

## LW.3

**Survey** (at `05ca38005`, #724's head, with #718's LW.5 merged in for the authority's vault):
- **Inspections.** No port inspection existed. The picket's is the only authority inspection.
  CR's `port::ship::hold_reading` reads lots and the pool. KC.2's terms row carries
  `audit_permille`, which Ceres leaves unset.
- **Standing.** `mush_faction::public_standing(w, faction, who)` answers a band, from hostile to
  trusted. No staff verb sets one.
- **The market's settlement ear.** It tells the proceeds policy a sale settled
  (`trade::settled_sale`) only for a sale into a bid, or for an ask that carries a Co-op cut. A
  private lot ask with neither was never told, and nothing recorded a private sale's papers.
- **Port security.** It fined at docking only (LW.2), with fixed salvage fines (`fine_major`).

**Built:**
1. **The port audit (`crate::port::audit`).**
   - On the port tick, each hull newly seen docked (an `AuditBook` of hull, port and first-seen
     IC) at a port whose holder's terms restrict a good rolls once: `roll_permille`, a keyed hash
     of the hull, the port and the docking.
   - The chance is by the band of her answerer's standing with the holder (`chance_permille`,
     Q-50-69): trusted 0, friendly 125‰, neutral 250‰, unfriendly 500‰, hostile always.
   - The audit reads the hold for each restricted good. Held without the paper (the papers test
     of LW.1), it records `OffenseKind::UnlicensedPossession` (`Evidence::Audit`, `Proof::Audit`)
     under the holder against the answerer, and fines it at once.
   - Staff may audit a docked hull now with `+law/audit #<hull>`.
2. **Fines by the tonne and fined when proven.**
   - `security::fine_major` now covers the salvage kinds only. `fine_of` fines the new kinds by
     the tonne: from the terms row's `fine`, else the picket's scale (500 cr a tonne, at least
     1,000, provisional).
   - `security::fine_now(ctx, key, entry, told)` fines a record at once (Q-50-68's offender fined
     when proven). `on_docked` shares its path.
3. **The receipt (`crate::port::law_report`, Q-50-66).**
   - mush-market gains `ProceedsPolicy::private_sale` and `PrivateSale`, told once per settled
     sale on a hidden listing.
   - For a good the venue's law restricts, each character party gets a `SaleReceipt` in
     `tacsim:sale_receipts:{party}` (16 kept). It records the seller, the buyer, the venue, the
     good, the units, the price, the IC time, the polity, and each party's papers at that moment.
   - `+law` lists your own receipts.
4. **The report (`+law/report <quote> go`, Q-50-35, Q-50-67).**
   - A party brings a receipt no more than one IC day old against the other party, who lacked
     the papers.
   - A reporting buyer's goods go to the port authority's vault (LW.5's
     `seized::ensure_vault`, through the new `seized::port_authority`). They move in the report's
     own action, `tacsim_law_report`, which relocates the lot (or seizes a pooled sale's units)
     and writes the `tacsim.law_report` row together. A buyer who no longer holds the goods is
     refused.
   - On the port tick the landed row becomes `RestrictedSale` (or `RestrictedPurchase`, for a
     reporting seller) against the other party. It is proven by the reporter, fined at once, and
     carries a `Payout`.
5. **The payout (`security::pay_reporters`, Q-50-43, Q-50-67).**
   - Once the fine is `Satisfied`, the reporter is paid the price a reporting buyer paid, plus
     `REPORT_REWARD_PERMILLE` (200, provisional) of the fine. It comes from the account the fine
     was paid to, never exceeds the fine, and is paid once.
   - The offender is never told who reported.

**Tests:**
- `domain_economy` `lw3_law_audit::lw3_an_audit_finds_strategic_ore_held_without_the_papers`:
  - Staff audit a licensed prospector holding 6 t of strategic ore with no claim. The audit
    records the possession under the Freeport and fines 3,000 cr, paid.
  - Holding a claim as well, the next audit finds nothing.
- `lw3_a_buyer_reports_a_private_sale_hands_over_the_lot_and_is_repaid`:
  - A private sale of 2 t at 300 cr writes a receipt for each party, recording that the seller
    lacked the papers.
  - The buyer's report moves the lot into the Registry's vault.
  - The seller is fined the 1,000 cr floor, paid at once.
  - The buyer is paid 600 cr back plus 200 cr, and a second report is refused.
- Mutation-checked, each failing a story: the audit ignoring papers; the report not handing the
  goods over; the payout never paid; the receipt recording the seller as holding the papers.
- `tacsim` lib `audit::tests`: the bands' chances, and a keyed roll that is fixed by the docking
  and moves with it.
- **LW.5's story now gives its trader the papers** (`voyage::lw5_…`): with LW.1 merged, the Ceres
  board sells seized strategic ore only to a holder of them. Whichever of #718 and #724 reaches
  main second needs that line.
- Re-run beside them:
  - `lw1_restricted_goods::`, `lw2_offense_record::` and `port_security_rs7::` (fines through
    the refactored path);
  - the market stories, `in11_matter::`, `plumbing::sm5_` and `coop::port_trade_`;
  - `domain_world` `voyage::lw5_`, `cr_`, `sfr26c` and `sfr6_`.

**Divergences:**
- **The audit's chance ignores the row's `audit_permille`.** Q-50-69's band table is the chance.
  KC.2 carries the field, which Ceres leaves unset, and reading it would need a lane row.
- **The docking is the IC second the tick first sees the hull docked**, kept in RAM. A restart
  sees each docked hull afresh and rolls her again.
- **Staff can order an audit** (`+law/audit`). Doc 50 does not name such a verb; the story pins
  an audit with it.
- **A reported or audited offense is fined at once**, not at the offender's next docking.
  Q-50-68 says an offender with no hull is fined when proven; this does it for every offender of
  these kinds.
- **An organization has no standing band**, so its hull is audited at the neutral chance.
- **The reporting buyer's goods go to the venue port's authority**, the same vault LW.5 makes for
  a seizure there.

**Not built:**
- `+record` and `+debts` showing receipts and fines, and the standing loss: LW.4.
- **A witness who is not a party** (Q-50-35's withdrawn "present at the venue"): not built, as
  ruled.
- **A private bid's sale** (a pooled `Deliver`) writes receipts too, and a reporting bidder's
  units are taken from their pool into the vault (`seize` with a destination). No story covers
  that path; the story's sale is a private lot ask.
- **The audit's goods stay aboard.** Q-50-65 makes possession an offense with a fine, not a
  seizure.

## LW.4

**Survey** (at `42b4c9f91`, #727's head):
- **Standing.** `mush_faction::record_standing` writes a delta under a stable cause, once.
  `public_standing` reads the score and band. Nothing in the law wrote standing.
- **`+debts`.** It read this layer's debts, the berth and locker arrears, and the release fees
  (`DEBT_SANCTION_PREFIXES = ["release:"]`). The Registry mind's inspection fines are sanctions
  keyed `hull/<hull>/inspection/<seq>`, so `+debts` never listed them.
- **The record.** LW.2's `tacsim:offense:*` rows carry the kind, the evidence, the proof and the
  fine's obligation. No verb showed them to the offender, and no notice was sent when one opened.
- **The license desk.** `+license/buy` checked the gateway account, its record, the entitlement
  and the funds. Nothing read the law.

**Built:**
1. **The standing loss (`crate::port::record`, Q-50-69, provisional sizes).** It is recorded with
   the polity that proved the offense, under a cause made of the offense's key and evidence, so a
   retry records it once.
   - A sale proven by a party's report (LW.3's `RestrictedSale` or `RestrictedPurchase`): −5.
   - Running the picket (`+helm/run`, the `Ran` finding): −10 to the pilot, with the polity of the
     picket's fine payee (`space::duty::fine_payee`).
   - An audit's possession finding paid at once: nothing. Not paid at once: −5.
   - Standing never decays, and paying a fine restores nothing (Q-50-42).
2. **The trading restriction (`record::restriction`, Q-50-39).** It is in force while the actor's
   standing with a polity is −10 or lower, or a fine on their record there is unpaid (`Active`,
   `Grace` or `Delinquent`).
   - **The board.** `port::law::may_trade` refuses a restricted good to a restricted character at
     a venue that polity holds, even with both papers.
   - **The license.** `+license/buy` refuses when any character on the buyer's gateway account is
     under the restriction of the license port's holder (`record::account_barred`; the bar binds
     the account, Q-50-68).
   - Nothing holds a character (Q-42-7).
3. **The notice (Q-50-40).** `offense::record_offense` tells a character offender at once when a
   new row opens: which polity recorded it and what. It never names a reporter.
4. **`+record` (Q-50-40, Q-50-70).** A pure read of the actor's own record:
   - each proven offense, the polity that proved it, and the kind of evidence (a vacated claim, a
     witness's report, a party's report of a receipt, a port audit), never who reported;
   - its fine: paid, due with its grace in IC and in real hours, or past its grace;
   - each restriction in force and what lifts it (paying fine #N, or standing rising above −10
     through lawful work);
   - the actor's receipts of private sales of restricted goods;
   - a core under a live Registry impound on a hull the actor holds title to.
   - It has a `legacy-handlers.toml` row (read-only).
5. **`+debts` lists the Registry mind's fines** (`DEBT_SANCTION_PREFIXES` gains `hull/`), and
   `+debts/pay` pays them.
6. **A text fix to LW.3's fine levied when proven**: it read "it holds the your ship" and now
   reads "it holds the ship".

**Tests:**
- `domain_economy` `lw4_record::lw4_an_unpaid_fine_restricts_until_it_is_paid`:
  - A prospector with no money holds 6 t of strategic ore with a license and no claim. A staff
    audit fines her 3,000 cr; she is told at once and cannot pay. She loses 5 standing and is
    under the Freeport's restriction.
  - Holding both papers now, the board still refuses her strategic ore, and `+license/buy go` is
    refused.
  - `+record` shows the offense, the port audit, the fine with its grace, and that paying the
    fine lifts the restriction. `+debts` lists the fine.
  - Paid through `+debts/pay`, the restriction lifts and the board buys from her. Her standing
    stays at −5.
- `domain_world` `voyage::sfr9_…`: running the picket leaves the pilot at −10 with the picket's
  polity, under its restriction.
- `domain_field` `ic_sortie_surface::m8d_the_registry_fines_the_holder_the_inspection_recorded`:
  `+debts` lists the levied fine.
- `tacsim` lib `record::tests`: the sizes against the band (one sale does not restrict; two do,
  as does one run).
- Mutation-checked, each failing its story: the board ignoring the restriction; the license desk
  ignoring it; an unpaid fine not restricting; no notice; running the picket costing nothing;
  `+debts` without the `hull/` prefix.
- Re-run beside them:
  - `domain_economy` `lw3_law_audit::`, `lw1_restricted_goods::`, `lw2_offense_record::`,
    `port_security_rs7::`, `l2_title::`, `in11_matter::`, `plumbing::sm5_`, `coop::port_trade_`,
    `claims::` and `mill::sm12a_only`;
  - `domain_world` `voyage::lw5_` and `cr_`;
  - `domain_field` `field_rs5::` and `crew_custody::` (`+debts`' other debts);
  - the tacsim lib's record, audit, offense, security, law, license and retrieval tests.

**Divergences:**
- **An audit's finding not paid at once costs −5.** Q-50-69 names only the paid case (nothing).
  The smallest loss in the table is taken for the unpaid one.
- **"Below neutral" is −10 or lower**, the unfriendly band's top, read off the doc 47 bands.
- **The license bar also refuses an extension.** Q-50-39 says no new license; a restricted
  actor's renewal is treated as one.
- **The run's polity is the picket's fine payee's** (the terms row's payee, else the license's
  issuer), the polity whose terms the picket enforces.
- **`+record` lists restrictions over every polity the actor has a record or a standing with.**

**Not built:**
- **The monthly lawful-work grant** that earns standing back (Q-50-69). It belongs with the OP
  chain's producers, which have not landed.
- **Appeal** (Q-50-41: none) and **decay** (Q-50-42: none), as ruled.
- **An organization's restriction.** Organizations have no standing band; an organization's
  unpaid fine holds its hull (LW.2) but bars no trade.

## LW.5

**Survey** (at `563a24b6a`, #716's head with main merged; CR #711 in main):
- **The seizure** (CR) took the hold's unheld strategic-ore lots whole (`consume_products`) and
  the pool past a haul's hold (`InventoryFamilies::seize`), and credited nobody.
- **The census** (`port/census.rs`) scans every pool and lot of a chain good. A seizure that
  destroys ore therefore lowers `pools` or `lots`, and `balance` moves. Its `seized` term is fed
  only by an in-flight burn's receipt (`seized_t`), which nothing sets today.
- **The payee.** The picket's fine is paid to the Freeport Registry, the licence issuer
  (`resolve_registry_principal_and_account`). The Registry is an organization at Ceres with an
  account but no vendor row, so it has no storefront or vault. Stickney's authority,
  `stickney_yards`, is a vendor.
- **The market** lists only for a registered vendor (`list_goods`). `register_vendor` works at
  runtime, one vendor per organization. A `Supply::Stock` ask draws from the vendor's vault, and
  `restock_ask` raises a standing one. The stock clerk lists only content's `asks`, so an
  authority missing from content needs its own listing call.

**Built:**
1. **`InventoryFamilies::seize(…, into)`.** With a destination, the taken units are credited to
   that endpoint's pool (made when absent), and their lineage is carried as `transfer` carries it.
   It is still one write of each pair.
2. **`crate::port::seized`:**
   - `seizing_authority(w)` is the one payee lookup: the Freeport Registry at the licence port,
     today. KC.2 re-points it at the Republic customs office after the flip (Q-50-14).
   - `ensure_vault(ctx, &authority)` uses the authority's vendor vault when it has one. Otherwise
     it makes an organization-owned vault (a container in LIMBO) and registers the authority as
     the port's market vendor. It runs at runtime, so there is no lane change.
   - `offer(ctx, &authority, vault, good, units)` raises a standing ask from the vault, or lists
     one ask of the whole pool. It posts as the port's stock clerk, at `SEIZED_PRICE_PERMILLE`
     (750, provisional) of the good's reference (`price_minor`).
   - The new vendor's vault gives the port's clerk its `vault.withdraw` grant, as the content
     applier gives every vendor.
   - `sells_at(w, venue, org)`: the venue gate's `port_seller` admits the authority at its port,
     so a hull's hold may take delivery of seized goods as it does from the port's counters.
3. **Customs:**
   - The filing makes the authority's vault and carries it on the ruling as
     `PendingCustomsRuling.into`.
   - The prepare relocates the hold's lots there and seizes the pool into it, in the ruling's
     transaction. The haul's hold release and its ledger failure from CR are unchanged.
   - When the ruling lands, the space tick offers the ore.
   - With no authority, the old destroy path is kept as a logged fallback.

**Tests:**
- `mush-inventory` `pool::seizure::tests::…` adds a seizure into a vault: the taken units land
  there, free.
- `domain_world` `voyage::lw5_a_seizure_moves_the_ore_to_the_authority_which_offers_it_at_its_port`:
  - An indigent pilot's 51 t is seized at the picket.
  - The census `balance` is the same before and after, and `seized` is 0.
  - Her hold reads 0 and the Registry's vault 51.
  - A Ceres ask of 51 t stands at three quarters of the live reference (600 cr a tonne today).
  - A trader at Ceres buys a tonne. It moves from the vault to their hold, and the census still
    balances.
  - Mutation-checked: the seizure without its destination (the census loses 51,000 kg), and the
    offer of no units (no ask). Each fails the story.
- `voyage::cr_the_picket_reads_ore_held_as_a_lot…` now finds the lot in the authority's vault.
- Re-run: `voyage::cr_*`, `sfr26c`, `sfr12`, `sfr6`; `domain_economy` `mill::sm12a_only_the_clerk…`
  (its doc notes the transfer), `ex6_haul::` and `lw2_offense_record::`.

**Divergences:**
- **The authority's vault is made at runtime**, with the authority registered as its port's
  market vendor on first use. This avoids a lane bump. Its container is a Thing in LIMBO, not a
  storefront in a port room.
- **The offer is posted when the ruling lands**, not by the stock clerk's daily walk, which reads
  only content's asks. A refused offer is logged, and the ore waits for the next seizure's offer.
- **The price is provisional:** three quarters of the good's live reference
  (`good_reference_minor`, as a haul's declared value reads it), 600 cr a tonne of strategic ore
  today.
- **A sale pays the authority's own account**, the Registry's, through the market's vendor seam.
  No share goes to a polity treasury; no polity has one yet (KC.2).

**Not built:**
- **The Republic customs office as payee after the flip.** KC.2 adds the office and re-points
  `seizing_authority`.
- **An authority that is already a vendor** (Stickney Yards, if a seizure is ever enforced there)
  takes seized goods into its storefront. No story covers that path: today's only seizure is the
  picket's, enforced by the Registry at Ceres.
- **No lane bump.**

## OP.0

The campaign engine's five additions and the seam at the composition root (Q-50-57, ruled at its
default; doc 50a A-2a to A-2h).

**Survey.** Each of the five held in code at `ecb96979`, at the anchors doc 50a gives:
- `CampaignStore::mint_operation` is `pub(crate)`, and only `@op/declare` and `@op/activate`
  declare. The tick's funding check ran on every live state, so a `Declared` operation carrying
  `@op/terms bond=` (no hold until activation) paused "unfunded" on the next fire, and
  `@op/activate` never cleared it (A-2g, live).
- `@op/terms` parsed sides, defenders and the forfeit beneficiary as `#<obj>` only;
  `notice::defenders` skipped every non-object; `render::acts_for` fails closed for an
  organization, so an organization's member could not read the notice either.
- `tick::objective_met` read a `CanonRecorded` objective every fire, met mid-window as the
  declarer's `success`.
- `settle_root` commits one kind, `campaign.operation.resolved`, with the outcome in `detail`
  only, and `WorldEvent::CanonRecorded` carries no detail. Pins: tacsim's 24 edges
  (`domain_arena/composition_edges.rs`), the campaign crate's five (`e7c9_p1`).
- `outcome_forfeits` was `== "forfeit"`: every other outcome released the bond.

**Built.**
- **Declare and activate in one step** (`crates/mush-campaign/src/declare.rs`):
  `pub fn declare_and_activate(ctx: &mut Ctx<'_>, declaration: Declaration) -> Result<OperationId,
  DeclareError>`, with `Declaration { declarer: PrincipalId, objective: ObjectiveSpec,
  legal_targets, stakes, bond: Option<BondTerms { account, amount, forfeit_to:
  Option<(PrincipalId, AccountId)> }>, sides: Vec<SideSpec>, eligible_defenders: AudienceSpec,
  notice_secs: i64, window_secs: i64, visibility: EconomicVisibility }`. It applies the per-issuer
  cap, mints, counts the notice from now, re-runs the terms constructor and the bond-account and
  outcome-stake checks `@op/terms` makes, places the hold first, and writes the operation
  `Activated`; the tick pushes the notice on its next fire. `DeclareError` is `NoCampaign`,
  `IssuerCap`, `Full`, `Window`, `Terms`, `Refused` or `Bond`. A refused call writes nothing but
  the allocator, which is persisted and never reissues an id.
- **The A-2g defect, fixed at its cause:** the tick's funding check skips a `Declared` operation
  (`tick.rs`), and `@op/activate` clears a stored `Unfunded` pause.
- **Organizations as declarer, side and defender.** The public call takes any `PrincipalId`, and
  `@op/terms` takes `org:<id>` beside `#<obj>` for `side`, `defender` and `bond.forfeit`
  (`org:<id>:<account>`). A defender organization's notice goes to its active members, at most
  `MAX_NOTICE_RECIPIENTS` (256) an operation, through the existing seam: an official notice,
  delivered at once (`mush_mail::post_notice`). `render::member_of` admits an active member to the
  notice lens, never to the plan and never to any authority.
- **A close-only objective with its own polarity:** `ObjectiveKind::ProbeAtClose { probe: SmolStr,
  met_favors: Favors }` with `Favors { Declarer, Defenders }`, also as `@op/terms
  <id>/objective.kind=probe:<name>:<declarer|defenders>`. The tick reads it only once the window
  has closed, through `CampaignModule::with_close_probe(Arc<dyn CloseProbe>)`, where `trait
  CloseProbe { fn read(&self, w: &World, probe: &str, operation: OperationId) -> Option<bool>; }`.
  `close_outcome(met, favors)` gives the declarer's word: a met probe that favors the defenders is
  `failure`. `None` is a wait: the operation keeps running past its close, pages once, and asks
  again each fire.
- **The bond is forfeit on the declarer's failure:** `outcome_forfeits` is `forfeit` or
  `failure`, and the bond goes to `bond.forfeit_to` (for a compiled declarer, the defenders'
  treasury).
- **The seam, TacSim's side** (`crates/tacsim/src/operations.rs`): `pub trait OperationFrame:
  Send + Sync { fn declare(&self, ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64,
  String>; fn status(&self, w: &World, operation: u64) -> Option<OperationStatus>; }`.
  `OperationRequest { declarer, summary, probe, met_favors: MetFavors, sides: Vec<(String,
  PrincipalId)>, defenders, legal_targets: Vec<String>, bond: Option<OperationBond>, notice_secs,
  window_secs, public: bool }`. `OperationStatus { phase: OperationPhase
  (Declared|Notice|Running|Settling|Settled), declarer, sides, defenders, legal_targets, window:
  (i64, i64), outcome: Option<OperationOutcome> }`, and `OperationOutcome { DeclarerSucceeded,
  DeclarerFailed, Partial, Abandoned }`. A TacSim arm finds it with
  `tacsim::operations::frame(w) -> Option<Arc<dyn OperationFrame>>` (the slot is
  `OperationFrameSlot`). The other direction is `tacsim::operations::close_probe(w, probe,
  operation) -> Option<bool>` over `CLOSE_PROBES: &[(&str, CloseProbeFn)]`, empty in OP.0.
- **The composition root** (`crates/mush-composition/src/campaign_operations.rs`):
  `CampaignOperationFrame` implements the trait over `declare_and_activate` and the
  `CampaignStore` reads; `TacsimCloseProbe` implements `CloseProbe` over `close_probe`;
  `CampaignLink` composes the campaign module and seats the frame beside it. `server_modules`
  composes `CampaignLink::new(CampaignModule::new()...with_close_probe(TacsimCloseProbe))`. No
  pinned edge moves: tacsim gains no campaign or mail edge, and the campaign no faction edge.

**Tests.** `crates/mush-campaign/tests/op0.rs` (10): the defect (`op0_a2g_…`); the public call as
an organization and its three refusals (`op0_a1_…`); the defender organization's notice and lens
(`op0_a2f_…`); the probe read only at the close, its four answers and the wait (`op0_a2a_…`);
forfeit on failure, release on success and on abandonment (`op0_a2c_…`).
`crates/mush-composition/tests/op0_frame.rs` (4): a fake owner that uses only TacSim's side
declares through the seated frame and reads who won both ways; unlinked, there is no frame; and the
production probe answers nothing. Mutations, each caught: the tick's `Declared` guard removed
(a2g), the probe read before the close (a2a), `failure` releasing (a2c), the organization's members
dropped from the notice (a2f); the polarity table flipped fails to compile (exhaustive match). Two
e7c9 tests have the issuer resolve `=failure`; they pass, since that bond names no beneficiary and
is released, and one message is reworded. All of mush-campaign passes.

**Divergences.**
- The frame reaches TacSim as a **world-state slot** registered beside the campaign module by
  `CampaignLink` (a wrapper carrying the campaign's own name), not as a builder on
  `TacsimRuleset`. The tacsim module is built in three places in `mush-server/src/boot.rs` and in
  tacsim's own helpers; the slot needs none of them and leaves the module roster (`boot_parity`)
  as it was.
- **A close probe is named, and answered from its owner's table.** TacSim names no probe in OP.0;
  OP.4 adds the tank farm's row to `CLOSE_PROBES`.
- **An unanswered probe is a wait, not a pause**, so it needs no `=retry`; staff may still decide
  it with `@op/resolve`.
- **Forfeit needs a named beneficiary.** A failing bond that names none is released, since the
  economy cannot capture a hold made to nobody. The issuer may still resolve their own operation
  `=failure`, conceding the bond to the beneficiary their own terms named; `forfeit` and `abort`
  stay staff's.
- **The canon record is unchanged** (one kind, the outcome in `detail`). Who won reaches TacSim
  through `OperationFrame::status`, the seam the ruling names. A faction clock bound to the kind
  still advances on any outcome (A-2d); Q-50-51's typed calls replace that path.
- An organization side or declarer still gets no plan lens (`acts_for` fails closed for one, as
  before); its members see the notice when it names them as defenders or audience. OP.7's
  `+operation` is the player page.

**Not built.** Any TacSim arm, Isidis, a front or a probe (OP.1 to OP.5). Organization members on
the resolution's canon entities (A-2e; Q-50-51 moves polity clocks by typed calls instead). A
capability for an organization's officers to read or move an operation's plan.

## OP.1

Isidis as a place, with its routes (Q-50-17, Q-50-58; Q-50-52's two claimants; doc 11 §5).
**Lane v46 and relay v37 land with this unit.** The next lane bump starts from v46.

**Survey.** At KC.0's `f668f082`, Isidis was prose only: four mentions in cast and atlas text, no
place, port, route, region row or organization. The moon ports of doc 19 cut A.8 are the shape the
ruling names. Each is a lane `location` (role `port`, on its body), its routes, and a `port` entry
whose `area` is a station neighborhood pack. A region and neighborhood pack pair rides
`STATION_SOURCES`, and a cast pack (the vendors' signatories, the lore pool) rides `CAST_SOURCES`.
A port end pays no landing floor: `passage::site_end` is `None` for role `port`, so a Dray docks
on Mars through the ordinary port path. Only a site (role `extraction`) would have refused it, at
131‰ against 1,200‰. Port orgs belong to one port entry, and a vendor names an org of its own
port.

**Built.**
- **Three packs, all at version 1:**
  - `region.isidis_concession`: Isidis Planitia on Mars, a 400 m surface region with its port
    locality.
  - `neighborhood.isidis_concession`: five rooms on one floor.
    - I01, the Landing Field Gallery. It is the port's `gallery` and carries `port.dock`.
    - I02, the Factor's Office (role `harbourmaster`).
    - I03, the Concession Market (`exchange`), where the venue's board stands.
    - I04, the Tank Farm Counter (`receiving`).
    - I05, the Tank Farm Spine (`spine`, restricted), holding the tank farm's storefront, its
      intake manifold.
  - `tacsim.cast.isidis_concession`: Factor Beatrix Oyelaran in I02 (doc 11's named Factor), the
    tank-farm superintendent Ilse Achterberg-Mensah in I04 (the bid's Warrant signatory), and the
    `port_isidis` lore pool.
  - The region and neighborhood packs are the last two of `STATION_SOURCES` (now 12), and the
    cast pack is the last of `CAST_SOURCES` (now 6). `ISIDIS_ROOMS` is 5.
- **The lane, v46:**
  - The place `isidis_concession`: label "Isidis Concession", role `port`, body `mars`.
  - The routes `stickney_isidis` and `isidis_stickney`: one direct leg each way, `coast_hauler`,
    `base_s` 42, which is ⌈9,444 km / 226⌉, the sky's Phobos–Mars chord. No Kirkwood leg and no
    picket.
  - `stickney_port`'s routes gain `isidis`. Isidis is reached from Stickney alone.
  - The port `isidis_port`:
    - venue `isidis`, the tariff-free market; authority `isidis_factor`, the Factor's office,
      with the standard 1,500 cr release fee;
    - two orgs, both tagged `freeport`: "Isidis Concession Factor" and "Occator Isidis Tank Farm"
      (an acceptor);
    - one vendor, the tank farm: a vault stocked with 3,000 t of propellant and a standing bid
      for propellant at **50 cr** for 2,000 t, on the receiving and exchange boards.
    - 50 cr is provisional and sits between Ceres' 40 cr ask and the 60 cr asks at Stickney and
      Deimos. There is no refresh and no consumption: the drain and the budget are OP.4's.
  - The regions block's fourth row, `isidis` ("Isidis Concession", places `isidis_concession`
    and `isidis_port`):
    - de jure the Republic **and** the Freeport (Q-50-52);
    - de facto the Freeport;
    - economic Occator and Ceres Clearing.
- **Relay v37** is v36 re-signed to require lane v46, its entries unchanged.
- **The strip chain** has a new head, `lane_chain::strip_isidis`, which removes the four rows,
  Stickney's `isidis` key and the region row. `strip_regions` calls it first.

**Tests.**
- **The story** is `domain_world::voyage::op1_a_hull_flies_stickney_to_isidis_and_back`, on
  voyage's composed rig.
  - A chartered Dray flies Ceres to Stickney. It plans Isidis: one leg, no Kirkwood. It flies,
    hears "docks at Isidis Concession", and is docked at venue `isidis`, Direct on the
    Concession's domain, in I01.
  - The pilot goes ashore to the market. The board's wanted line names the tank farm, 2,000 t
    and the price, and the price sits in 40..=60, over Ceres' ask and under Stickney's and
    Deimos's.
  - `control_at` reads both places with two de jure claimants, de facto the Freeport, and
    economic Occator and Ceres Clearing. Each polity reads its own papers there, and
    `+territory` shows the row.
  - The hull boards and flies the one leg home to Stickney.
- **Pins:**
  - `lane::op1_strip_isidis_rebuilds_the_published_v45` (the v45 digest byte for byte);
  - the v46 and relay v37 digests and versions;
  - the lane's entry list (69) and places;
  - `ALL_ROUTES` (the two hops, authored within 100 s of the sky's figure);
  - the twelve station digests;
  - the boot list (53) and `station_packs` (32);
  - the region count (4);
  - the `via=` place lists;
  - the plan counts the new entries move.
- **Mutations.** Each is a lane edit, re-signed, with the story run alone:
  - the Freeport's de jure hold dropped is caught at "two de jure claimants";
  - Stickney's `isidis` key dropped is caught at the plan ("No passage leaves Stickney Yards for
    that port");
  - the bid raised to 65 cr is caught at the 40-to-60 band;
  - a Kirkwood leg added to the hop is caught at the plan's first line ("by way of Kirkwood
    Depot").
  - The lane was restored from a saved copy and compared byte for byte.
- **Two field stories re-seeded.** `field_mw14a` and `field_mw14c` pin
  `cfg.field.seed = Some(1_577_058_309)` in their own tune closures.
  - The field's draws key on the units' object ids, and the Isidis packs add boot objects, which
    moves every later id (the roster's units went from #548 to #565). Under the rig's
    `FIELD_SEED`, three of their stories then failed deterministically.
  - On the base commit they pass, and the clock at field open is the same in both runs.
  - Of eight seeds tried, only this one plays all fourteen of the two files' stories as
    written. That they pass by luck is a follow-up, not this unit's to fix.
- **Run on conclav.dev:** all of tacsim (4,982) and mush-server `split_boundary` (227). The gates
  are listed in the PR.

**Divergences.**
- **The tank farm is its own lane organization**, "Occator Isidis Tank Farm", tagged `freeport`,
  rather than the Ceres-side `occator` org. A port's vendor names an org of its own port entry.
  The region's economic hold still names `occator`, per doc 11.
- **De facto is the Freeport polity.** Doc 11 has Meridiani Security hold it on the Freeport's
  contract, but Meridiani is no lane organization, so like the Tether and OTS it is not seeded.
- **No covert hold** is seeded (doc 11's Tether and OTS; KC.0 likewise).
- **The landing field is the berth gallery.** How a consignment comes down to the ground is left
  unsaid in the room text (Q-50-17 leaves the fiction to the owner).
- **The route's `base_s` is 42 s**, the Phobos–Mars chord the sky measures, as the moon hops are
  timed.
- **The market is the venue, not a vendor.** The only line on its board is the tank farm's bid.
  No ask is authored: Q-50-58 makes the stockpile propellant only, and the import duty is OP.5's.
- **No premium and no emergency block**: neither is required, and both are economy tuning.
- **The board's live figure moves.** At the story's hour the board read 47 cr, not the authored
  50, from the market's own price movement. The story checks both the authored and the live
  figure against the 40-to-60 band.
- **The IC-return table** (`ic_return::PORTS`) gains the Concession as its sixth port, reached by
  ship, with its arrival in I01. Its `+ic` help names it.
- **The field stories' seed** is pinned (see Tests). Pinning was the smallest fix that leaves
  their scripts untouched.

**Not built.**
- The stockpile objective, its drain and the Freeport arm's budget (OP.4).
- The import duty and its schedule (OP.5, Q-50-59).
- A license at Isidis (Q-50-52 needs none in the first build).
- A Hod lander, surface ground or a field at Isidis.
- Any front or clock (OP.2).

## OP.2

The scheduled tick as a typed cause, the Concession Review and Hold the Concession, and the hull's
flag (Q-50-20, Q-50-61; Q-50-51; doc 11 §6). Stacked on OP.1. **No lane bump.**

**Survey.** At OP.1's head:
- Neither Isidis front existed: no code or content named the Concession Review or Hold the
  Concession.
- The scheduled tick was a cause deferred since M25. No front moved on IC time.
- No hull carried a flag.
- KC.1's `open_faction_front` / `FrontSpec` and KC.0a's `advance_clock_by_cause` are the public
  seams, so mush-faction needs no change.
- A clock keeps the cause ids that moved it (`Clock::causes`, at most 64), persisted with the
  clock.
- A front records no opening time, and canon can be read back only from its open tail
  (`records_for_source`).
- The picket's seizure ruling lands in `space::picket::record_customs_seizure`, whose canon line
  already carries the cause id `space.picket.seize/<hull>/<IC second>`.
- A Co-op pool hull is titled at boot to the Co-op Trust, an org tagged `freeport`
  (`port::hull`); the title is on the economy's asset record (`house_sweep::title_of`).

**Built** (`crates/tacsim/src/concession.rs`).
- **Both fronts at world start.** `seed_isidis_fronts` runs from the regions applier after
  Customs Assertion, when the lane has the `isidis` row. It is idempotent.
  - **The Concession Review**, on the Republic's board:
    - clock "Assembly votes & seized Freeport haulers", six segments;
    - it opens at one, and the seeding month's Assembly vote takes it to **two of six** in the same
      apply;
    - no control flip: the fill is the Republic's decision, which the declaring arm acts on
      (OP.5), and the flip is the operation's;
    - consequence `CONCESSION_REVIEW_CONSEQUENCE`.
  - **Hold the Concession**, on the Freeport's board:
    - clock "Reinforcements, stockpiles & haulers through the pickets", six segments, opening at
      **two**;
    - no flip, and nothing moves it yet.
- **The scheduled tick, a typed cause:** `tacsim.assembly`, coarse, every 60 s.
  - Each fire, `assembly_votes` moves the Review once for each whole IC month (`IC_MONTH_S`,
    30 IC days) since its anchor that the clock has not counted.
  - Each vote goes through `advance_clock_by_cause`, cause id `assembly.vote/<IC month index>`.
  - The anchor is the seeding month's vote, the least `assembly.vote/` cause on the clock. The
    clock's persisted cause list is the memory, so the tick keeps no state of its own.
  - A restart, a missed fire or a paused world moves it neither more nor less; a missed month
    catches up on the next fire.
  - It stops at the fill.
- **A seized Freeport-flagged hauler:** `record_flagged_seizure(ctx, hull, source)`, called by
  `record_customs_seizure` after Customs Assertion's line, with the same `source` as cause id.
  - A hull whose flag is the Freeport's moves the Review one segment, a retry none.
  - Any other hull moves nothing.
  - The picket's change is one `let` and one call.
- **The flag (Q-50-61):** `hull_flag(w, hull)` is the polity of the organization that holds the
  hull's title, from the lane's org table (`polity_of_org`). A personally titled or untitled
  hull flies none.

**Tests.**
- **The story** is `domain_world::polities::op2_the_concession_review_moves_by_the_assembly_and_by_a_flagged_seizure`,
  on a fresh production world first booted at IC month 5, so a count from month zero cannot pass
  for one from the seeding month.
  - Both fronts are on their boards at 2 of 6 with no flip, both on `+clocks`, and the anchor is
    the seeding month's vote.
  - The same month votes nothing. A month later the tick votes once, and a second read that
    month votes nothing.
  - A Co-op pool hull flies the Freeport's flag; an untitled object flies none.
  - A seizure on the pool hull moves the Review. A retried cause, an unflagged hull and a hull
    titled to Stickney Yards (Republic-flagged) do not.
  - Two more months fill it: the consequence is recorded and the front resolves. Hold stays at
    2, and nothing moves the Review after.
- **Mutations**, each caught by the story, each restored from a saved copy and compared byte for
  byte:
  - no seeding vote (caught at "two of six");
  - the tick counting from month zero, not the anchor (caught at the first month's count);
  - any flag moving the Review (caught at the Republic-flagged hull);
  - the tick unregistered (caught at "the monthly vote, on the tick").
- **Run on conclav.dev, green:**
  - all of tacsim (4,990 tests; the one failure in that run was this story's own `+clocks`
    line, since fixed, and the story and polities binary pass);
  - mush-server `split_boundary` (227) and mush-contract-tests (208);
  - `cargo fmt --all --check`;
  - clippy `-D warnings` on tacsim and mush-server, all targets;
  - rustdoc `-D warnings` in both modes.

**Divergences.**
- **The Review opens at one and votes to two at seeding.** The seed's own vote is the anchor the
  monthly tick counts from. Doc 11's world-start vote ("late 2349 the Assembly votes to review")
  is that vote.
- **A month is 30 IC days** on the world's IC clock, counted from the seeding month. A kept world
  that takes the lane late is anchored where it takes it.
- **Neither front flips control.** Doc 11 has the Review's fill declare an operation (OP.5),
  whose resolution flips administration (Q-50-28); Hold's consequence is a statement.
- **Hold the Concession has no cause in this unit.** Its doc 11 causes are the stockpile at
  threshold (OP.4), Meridiani reinforcements (Meridiani is no lane organization) and a hauler
  that runs the pickets, which with Q-50-25's losses is OP.3's to wire.
- **The flag reads the title only.** A Co-op charter leaves the title with the Co-op Trust, so
  "or its charter" adds nothing for any hull the game makes today.
- **Only a seizure ruling moves the Review**, not a fine or a run (Q-50-20 as corrected).
- **The code is its own module** (`tacsim::concession`), one line in the regions applier and one
  in the picket, to keep clear of KC.2's polity and picket edits.

**Not built.**
- What moves Hold the Concession (OP.3, OP.4).
- The declaring arm that acts on the Review's fill (OP.5).
- A hull flying for a side by a live job (Q-50-61's second half, for OP.3 and OP.6).
- Any player-facing line naming a hull's flag.

## OP.3

Posting as an organization, escorts for a haul, and the haul's loss handling (Q-50-23, Q-50-25,
Q-50-63; doc 50a A-8, A-8b). Stacked on OP.2. **No lane bump.**

**Survey.** At OP.2's head:
- **Posting.** `+haul/post` and `+escort/post` posted only as the actor, from their own account.
  A haul ran from the actor's locker at one port to their locker at another; lockers are
  player-keyed and charge rent (`port/locker.rs`). Acceptance already took `as=<org>`
  (`services::taker`, via the economy's `acts_for`).
- **Vaults and capabilities.** An organization's only vaults at a port are its lane storefronts
  (the port map's `vendors`). Every lane organization belongs to one port, so none has vaults at
  two. Capabilities are a frozen vocabulary (`composition::domain_org_capability_vocabulary`),
  checked through `mush_org::authorize_in_world`.
- **The escort**, the two defects (doc 50a A-8b):
  - It named a hull its poster held.
  - A passage ended when the protected hull docked at any port other than its origin
    (`journey_now`), so it settled at the first hop.
  - The departure gap was `TOGETHER_IC` (3,600) IC seconds from the watch's start. At a high
    clock rate, one port fire of 5 real seconds is past it, so an escort undocking one fire later
    read as "left".
- **Loss.** CR built the seizure: the ruling releases the haul's hold, writes `LegKind::Seized`,
  and the arm lapses the job. A wreck or a capture did nothing: the goods stayed held to the job
  until the hold's deadline. `escort::wrecked` and `escort::captured` are the existing
  predicates.

**Built.**
- **Posting as an organization (Q-50-23).**
  - The new capability `tacsim.jobs.post` (`services::JOBS_POST_CAPABILITY`) is in the domain
    vocabulary.
  - `services::poster(ctx, as)` is the actor and their account, or, with `as=<org>`, the
    organization when the actor holds the capability on its root compartment, and its credit
    account.
  - `services::org_vault(w, org, venue)` is the organization's storefront vault at a venue.
  - `+haul/post ... as=<org> [into=<org>]` ships from the organization's vault at the origin to
    its vault at the destination, or `into=`'s there. It is vault to vault, the fee comes from
    the organization's account, and the job is posted with the organization as poster.
    - `HaulJob::shipper_org` records the organization; `shipper` is the member who posted.
    - A late return goes to the organization's vault where the hull lies, and waits for a port
      that has one.
  - `+escort/post ... as=<org>` posts for an organization: of a hull it holds the title to, or
    for a haul it posted.
  - Acceptance refuses the posting organization's own job.
- **Escorts for a haul (Q-50-63).**
  - `+escort/post haul=<job> fee=<cr> [as=<org>]` is posted by the haul's shipper, or for its
    organization. The poster may post it before the haul is taken.
    - `EscortJob::haul` names the haul, and `protected` is `UNBOUND` until a hull takes the haul.
      The watch then binds it to the hauler's hull.
    - A haul that closes untaken calls the escort off, the fee and any bond back.
    - The journey is the haul's passage, origin to destination, on the origin's board.
  - **The two defects, fixed for every passage escort:**
    - **The end is the destination.** Under way means off any berth on a journey begun since the
      escort was taken. Docked at a port on the way is a hop: nothing counts while she lies
      there, and the escort is not "left" for lying there too.
    - **The departure grace** runs from her latest launch (`Watch::launch_ic`, `launch_wall`), so
      it restarts at every hop. It is the longer of `TOGETHER_IC` and `LAUNCH_GRACE_WALL_S`
      (60 real seconds, provisional).
- **The haul's loss (Q-50-25).**
  - A hull wrecked or captured with the goods aboard owes the new `LegKind::Lost` leg. Its
    prepare releases the job's hold in one write and moves nothing, so the goods are free in the
    wreck's hold, part of the wreck; a hold already gone leaves nothing to release.
  - On the ledger's `Lost`, the arm lapses the job as a seizure does: the fee back to the
    shipper, the hauler's bond to the shipper, and both told.
- **Help:** `+haul` and `+escort` help and `docs/help/exchange.md` name `as=`, `into=` and
  `haul=`.

**Tests** (`tacsim::domain_economy::op3_hauls`, the composed world).
- `op3_an_org_haul_and_its_escort_run_ceres_to_isidis_vault_to_vault`, at 24:1:
  - Occator's factor is refused without the capability. Once granted it, the factor posts 20 t
    of volatiles from Occator's vault at Ceres into the Isidis tank farm's, the fee escrowed from
    Occator's account.
  - An escort for the haul is posted `as=` Occator, unbound. A guard takes it, and a hauler
    takes the haul: the goods load, and the escort binds to her hull.
  - Both fly Ceres to Stickney: the escort is still watching (a hop). Both fly Stickney to
    Isidis: the escort is kept.
  - The hauler delivers into the tank farm's vault, and the contract settles.
- `op3_a_hull_wrecked_with_a_hauls_goods_releases_them_and_fails_the_job`:
  - The goods are aboard, all held. Staff's `@unit/damage` takes every cell of her unit to its
    least, so she reads as a wreck. The contract guards forbid writing the space book from a
    test.
  - The job closes, the contract is expired, and the hold reads all 20 t available.
- `op3_an_escort_one_fire_behind_at_a_high_rate_has_not_left`: at 3,600:1, the escort still
  docked one fire after she launched, past `TOGETHER_IC`, is still watching.
- **Mutations**, each caught, each restored from a saved copy and compared:
  - the first port ending the passage (A-8b back);
  - the escort never binding;
  - anyone posting for an organization;
  - a wreck owing no lost leg;
  - the lost leg releasing nothing;
  - no real-second grace.
- **Re-run beside them:** `ex6_escort_passage` (the G2 and G3 stories), the `ex6_` haul and escort
  stories, CR's seizure stories in `voyage`, and the services unit tests.
- **On conclav.dev, green:**
  - all of tacsim (4,993 tests);
  - mush-server `split_boundary` (227) and mush-contract-tests (208);
  - `cargo fmt --all --check`;
  - clippy `-D warnings` on tacsim and mush-server, all targets;
  - rustdoc `-D warnings` in both modes.
  - The first full run's two guard failures were the wreck fixture's direct write to the space
    book (the fiat budget, and the rule that tests never mutate it). It now goes through
    `@unit/damage`, and the guards, the stories and the two wreck mutations were rerun green.

**Divergences.**
- **`into=<org>`.** Every lane organization keeps vaults at one port only, so an organization's
  haul between two ports needs a destination vault that is another organization's. OP.4's arm
  hauls from Occator at Ceres into the tank farm (OP.1 made the tank farm its own organization).
  The receiving organization needs no grant: a delivery is a deposit.
- **An organization's vault is its lane storefront**, and the post refuses a port where it keeps
  none. Lockers stay players'.
- **The capability is checked on the organization's root compartment**, as the economy's posting
  authority is.
- **An unbound escort's `protected` is `UNBOUND` (`ObjId(0)`, Limbo)**, not an optional field, so
  the record and every reader keep their shape.
- **The defect fixes apply to every passage escort**, not only a haul's. The G3 story's "past the
  gap" case still fails as before, at 24:1.
- **The grace is the longer of the two**, so 24:1 behaves as before and a high rate gets 60 real
  seconds.
- **A loss is a lapse**, the shipper's remedy for a lost bond, as the seizure's is. The goods
  stay with the wreck for its salvage.

**Not built.**
- The record that moves the other side's clock when a hull flying for a side of an operation is
  lost (Q-50-25). No operation has sides until OP.5, which can call it from the haul arm's loss
  and the seizure.
- A hull flying for a side by a live job (Q-50-61's second half).
- The NPC arm's use of the organization path (OP.4).
- The untaken haul carried by EX.10's house (Q-50-24, OP.4).

### Fixes from doc 50b

Doc 50b (the outside code review of 2026-11-20) found four defects on OP.3's paths. Fixed together
with OP.6's two, below; no lane bump.
- **A haul taken in its escort's own hull** (P2). Acceptance compared the escort's hull with a
  protected hull still unbound, and binding never looked again, so a hull could escort herself
  and the record then failed its own check on reload. Now, when the haul is taken in the escort's
  hull, whichever contract was taken first, the escort is called off through `cancel_job` and
  both parties are told (`escort::TOLD_SAME_HULL`).
- **A delivered haul stuck aboard** (P2). The arm looked for a wreck or capture before
  acknowledging a delivery the ledger had committed, so a hull lost after docking owed a lost leg
  the ledger refused for ever. The committed delivery is acknowledged first.
- **Hold capacity** (doc 50 §7's cargo reading). Mining's hold reading
  (`mining::hull_hold_used_tonnes`) counted the four ores only; it now counts, by mass, every good
  the ore compartments take, bulk trade goods included, and rounds up. The haul's admission read
  pools only; it reads what the hold holds (`port_tick::read_held`, `haul::hold_has_room`), and the
  load re-reads the room before each try, calling the haul off with `haul::TOLD_NO_ROOM` when
  there is none.
- **The escort's grace at a test rate** (P3). The proximity test compared departures against
  `TOGETHER_IC` alone, so at 3,600:1 an escort inside the launch grace failed the passage. Both
  now read `escort::together_ic`: `TOGETHER_IC`, or the IC seconds `LAUNCH_GRACE_WALL_S` make at
  the clock rate, whichever is longer. At 24:1 nothing changes.
- **Tests** (`domain_economy::op3_hauls`): `op50b_a_haul_taken_in_its_escorts_own_hull_calls_the_escort_off`;
  `op50b_a_delivery_committed_before_the_hull_is_lost_stays_delivered` (two hops to Isidis, the
  delivery lands, then the hull is wrecked: the contract settles);
  `op50b_a_full_dray_p_can_neither_mine_more_nor_take_a_haul` (alloys held as a lot fill the hold:
  the haul refuses her and `+mine/status` reads the ore hold full); and the unit test
  `escort::op50b_the_together_gap_follows_the_clock_rate`. Mutations, each caught: no self-escort
  check; the delivery acknowledged after the loss check; the ore reading of ores only; the
  admission reading pools only; no rate in the gap.

## OP.4

The stockpile, its drain and the Freeport arm's budget; NPC hulls at the picket (Q-50-21, Q-50-22,
Q-50-24, Q-50-56, Q-50-60). On OP.3 and KC.2 to KC.4. **Lane v49 (relay v40)**, taken from KC.4's
v48 with the lead's word.

**Survey.** At OP.3's head with KC.2 to KC.4 merged:
- **The tank farm** (OP.1) holds 3,000 t of propellant and a standing bid of 2,000 t at 50 cr,
  with no refresh and no consumption.
- **Close probes.** OP.0 named none (`operations::CLOSE_PROBES` empty).
- **Hold the Concession** has no cause (OP.2).
- **EX.10's hauling house** sails trade trips only, from a sailing plan the IC11 executor flies as
  the house's `DARK` helm. Under way the house's mind holds her helm, so the Kirkwood picket
  promotes her into its bubble. Nothing more of the picket was tested.
- **Procurement** is the precedent for an arm with a budget: an agent of the House's, a monthly
  cap counted from its own rows.
- **What the stories found** (all live defects, fixed here where OP.4 needs them):
  - **A house hull at the picket was never inspected.** Her passage commits all her reaction mass,
    so she enters the bubble with 0 kg, cannot brake, and is never hove to. Had she been, nobody
    was aboard to submit her: the captain resolver fell back to the tick's own actor, and the
    customs ruling's offer (`filed_by`) wanted an actor aboard. She sat in the bubble until it
    let her go, uninspected.
  - **A haul's first contract could not move.** The haul leg's offer took contract numbers from
    1, and a world's first contract is `ContractId(0)`. The arm's first post on a fresh world is
    one.
  - **A tank-good haul blocked its own fuel.** Propellant stows in a Dray's tank, not her hold.
    The executor's `fill` counts held units as aboard, while the departure burn spends only free
    units, so a hull carrying a propellant haul fills short and the burn is refused.

**Built.**
- **The stockpile** (`tacsim::stockpile`; Q-50-21, Q-50-58).
  - The good is propellant and the threshold is `STOCKPILE_THRESHOLD_T`, 4,000 t (provisional:
    1,000 over what a fresh farm holds).
  - The port tick reads the farm's storefront every fire and keeps the level on the arm's record
    (`tacsim:freeport_arm`, `FreeportArm::level`).
  - The close probe `isidis.tank_farm` (`operations::CLOSE_PROBES`) reads that record: met at the
    threshold, `None` before any reading. OP.5's objective names it, with the defenders' polarity.
- **The threshold is news** (Q-50-21 as corrected): an IC month in which a reading stands at the
  threshold moves Hold the Concession one segment. The cause id is `stockpile.threshold/<month>`,
  and the clock's line is the chronicle's record. It decides no operation.
- **The drain** (Q-50-60), lane v49: a `consume` row on the tank farm, 20 t an IC day, with an
  export share of 1,000 cr (the good's price for the units), so EX.9's arm uses it up and the
  faucet keeps the farm's treasury funding its bid.
- **The Freeport arm** (Q-50-22, procurement's pattern).
  - Its agent is a `DARK` Thing of Occator's at Ceres' exchange, granted `tacsim.jobs.post` on
    Occator's root compartment.
  - It posts through the players' own verbs (OP.3's `as=` and `into=`), at most once an IC day,
    while the farm is under its threshold and fewer than `ARM_MAX_OPEN` (2) of its hauls are open:
    - a haul of `ARM_HAUL_T` (20 t) of propellant from Occator's vault at Ceres into the tank
      farm's at Isidis, fee `ARM_HAUL_FEE_CR` (600 cr);
    - an escort for it, `haul=<job>`, fee `ARM_ESCORT_FEE_CR` (400 cr).
  - Its money is Occator's treasury. A month's posts are capped at `ARM_MONTHLY_CAP_CR`
    (20,000 cr), counted from its own rows (`ArmPost`, `FreeportArm::within_cap`). Posting stops
    when the cap or the treasury runs out; nothing is minted.
  - The standing bid at Isidis is the tank farm's own, refreshed by lane v49 (40 t an IC day, back
    to 2,000).
  - **Staff lay it up** with `@tacsim/port arm off` (and open it with `on`), as EX.10's house is
    laid up: laid up, it reads nothing and posts nothing, and what it posted runs on. The stock
    rigs whose stories predate OP.4 lay it up beside the house (`support::lay_up_haulers`, and
    the three `commerce_composition` rigs), so it makes no agent there and moves no contract
    number, no stock and no dice. `op4_stockpile` opens it.
- **An untaken haul** (Q-50-24): an arm's haul still untaken `FALLBACK_IC_DAYS` (7) after it was
  posted is taken by the hauling house at its origin.
  - A docked house hull with no plan running funds her purse for the bond and takes it as her
    helm (`+haul/accept`), so the goods load under the haul's hold.
  - She carries it hop by hop (`Trip::haul`): at each port short of the destination, a plan of
    the fill and a departure toward it (Ceres to Stickney by the Kirkwood route, then Isidis).
    At the destination the helm delivers it (`+haul/deliver`), and the trip closes with the purse's
    surplus, the fee among it, sent home.
  - She is a real hull: the picket inspects her, and she can be intercepted.
- **NPC hulls at the picket** (Q-50-56), by rule:
  - **She heaves to by rule.** In the backup seat's bubble step, a hauling house's hull under a
    customs stop takes the stopping ship's velocity, whatever her tank holds, and is submitted.
  - **Her helm is her captain.** `resolve_hull_captain_aboard` answers a house hull's helm when
    nobody is aboard her. So the customs submission, the make-for-port burn and every queued space
    burn are the helm's, never the tick's. The customs ruling's offer reads the hull a house helm
    sails, so the helm records it.
  - **The house pays.** The transit duty's debtor account is the house's own account for a house
    helm (`hauler::house_account_of_helm`), not the helm's purse.
- **The fixes the arm needs:**
  - the haul leg's offer takes contract 0;
  - a carried haul of the propellant good buys the held tonnes back over the fill.

**Tests** (`tacsim::domain_economy::op4_stockpile`, the composed world; a unit test in
`stockpile`).
- `op4_the_freeport_arm_posts_a_haul_and_its_escort_for_occator`:
  - The agent is made and granted. The farm reads short, and the probe is not met.
  - The arm's haul is 20 t of propellant, posted by the agent for Occator, from Occator's vault
    at Ceres into the tank farm's. Its escort names the haul. The row records 1,000 cr.
  - There is one post an IC day, a second the next day, and never more than two open.
- `op4_the_tank_farm_drains_and_at_its_threshold_hold_advances`:
  - The farm uses 20 t an IC day.
  - Seeded past its threshold, the next reading meets the probe and moves Hold one segment, its
    cause the month's. A later reading in the same month moves it no further, and the arm posts
    nothing while the farm stands there.
- `op4_an_untaken_haul_is_carried_by_the_hauling_house`:
  - Seven IC days after the arm posts, no player has taken it. Staff open Ceres' house, and VCC-01
    takes the haul.
  - She flies it by Stickney to Isidis through the Kirkwood picket, and delivers. The contract
    settles.
- `op4_a_house_hull_pays_the_kirkwood_duty_from_the_house`:
  - With the Republic holding the approach (`@faction/control`, as KC.1's fill makes it), VCC-01
    sails for Stickney laden with water and meets the picket.
  - She is inspected and cleared. The duty reaches Republic Customs from the house's own
    account, and her helm's purse is untouched.
- `op4_the_arms_cap_counts_its_own_months_rows` (unit): exactly at the cap fits, past it does not,
  and another month's rows do not count.
- The lane's pins move to v49 and relay v40. `op4_strip_tank_farm_rebuilds_the_published_v48`
  pins the new strip-chain head (`lane_chain::strip_tank_farm`). `e48_consumers_…` counts five
  consumers, and the OP.0 frame test names the tank farm's probe.
- **Totals that count the new consumer:** `ex9_the_consumers_use_up_what_they_buy_…` adds the
  farm's refunds to the faucet's payout, and `ex0_the_census_holds_across_a_burn_and_a_rearm`
  reads the farm's whole days of propellant beside the Board's provisions.
- **Mutations**, each caught, each restored from a saved copy and compared:
  - Hold never moving at the threshold; the probe never met (the drain story);
  - no limit on the arm's open hauls (the arm story); no monthly cap (the unit test);
  - the house never taking a haul; no held fuel bought back; contract numbers from 1; the helm
    unable to record the ruling (the untaken-haul story);
  - no heave-to by rule; the duty from the purse (the duty story);
  - no helm as captain (both).
- **On conclav.dev, green:**
  - all of tacsim (5,072 tests, on main merged in);
  - mush-server `split_boundary` and `commerce_composition` (320), mush-contract-tests (208),
    mush-composition (6) and the bot's library (579);
  - `cargo fmt --all --check`;
  - clippy `-D warnings` on tacsim, mush-server and mush-composition, all targets;
  - rustdoc `-D warnings` in both modes.
- **Stories that predate OP.4** (`ic11_` in `orders`, `lw5_` in `voyage`, `sm8_` in `unloading`,
  `ex9_`, `ex0_`, `lane3_` in `field_director`) first failed on the arm and the new consumer: the
  arm's posts moved contract numbers, its agent moved the dice, and the farm's day of use fell in
  their fires and sums. The arm's lay-up, and the farm counted where a story sums or counts, cure
  them.

**Divergences.**
- **The level is read every fire**, kept on the arm's record when it changes. Q-50-21 says "read on
  the port tick", and a once-a-day reading saw an unstocked farm at 0 t on a fresh world.
- **A post marks its day; a refusal does not.** The arm looks again on the next fire, so a post
  refused while Occator's vault is still being stocked is not lost for the day.
- **Hold moves once an IC month at the threshold**, not once ever: a farm kept full keeps
  advancing it, as doc 11's cause reads.
- **The cap counts what the arm committed**, the fee and the escort's, as procurement counts its
  funded rows. A refunded lapse still counts in its month.
- **Only the arm's hauls fall back to the house.** Q-50-24 is the operation's; a player's untaken
  haul lapses as it does today.
- **The haul is the house's helm's**, bonded from her purse, which the house funds, and paid
  into it; the purse's surplus goes home when the trip closes. The house's own account pays the
  picket.
- **Heaving to by rule sets her velocity.** Q-50-56 asks for compiled code, not flying: a house
  hull's passage commits all her reaction mass, so she could never brake for the picket herself.
- **The executor's `fill` is not changed.** A carried propellant haul buys its held tonnes back by
  count. A player's `+ship/order` fill with a tank-good haul aboard still fills short: see Not
  built.
- **The lane's figures are provisional**: the 20 t drain, the 1,000 cr export share and the
  40 t-a-day refresh, as are the threshold, the haul's size and fees, and the cap.

**Not built.**
- **The executor's `fill` counting held units as aboard** (a live defect): a player whose tank
  holds a propellant haul fills short and is refused at the burn. This is a fix for the IC11
  executor's owner.
- **The objective's use of the probe** and the declaring arm (OP.5).
- **A house hull flying for a side by a live job**, and the other side's clock on her loss
  (Q-50-61, Q-50-25; OP.5).
- **The Belt Raider taking a house hull at the picket** was not exercised by a story here; the
  bubble promotes her, as EX.10 showed.

**Fixes from doc 50b** (the outside review of 2026-11-20; items by its numbering in the lead's
brief):
- **Item 1, the stranded house hull (P1).** Isidis sells no stores, so a house hull that delivered
  there could price no passage home and retried every fire.
  - A departure for a port whose houses author no ask for reaction mass or for provisions now
    fills for the leg home too, as one for a site does. Only the fill is sized so: the departure
    is not refused for it. This applies to a player's fill as well.
  - The house prices a passage from a port that sells no stores out of what is aboard, valued at
    the lowest ask the lane posts, and a look there that finds nothing waits a day.
  - A carried haul that ends away from the port she took it at sails her back there hop by hop
    before the trip ends, so she can carry the arm's next haul. A hop whose plan holds ends the
    trip where she lies.
  - The OP.4 story runs on past the delivery: home by Stickney to Ceres, and a second haul taken.
- **Item 3, the stale close probe (P2).** The probe answered from the cached reading, which a
  delivery or a drain since could have changed, and a failed read kept the old answer.
  - The probe reads only `&World`, and the vault's pools are read through a module's `Ctx`, so the
    probe cannot read the vault itself. Instead, on the first fire at or after the operation's
    close, the port tick reads the vault once for that operation (`FreeportArm::close_read`). The
    probe answers from that reading alone, and is `None` (the verdict waits) until it is taken.
    A failed read keeps nothing.
  - A delivery that lands between the close and that first fire counts. One fire at 24:1 is some
    12 IC minutes.
- **Item 7, the fuel fill (P2).** The executor's `fill` now counts reaction mass held for a haul
  aboard as weighing but not burning: the tank's target is the burn over it. The house's buy-back
  is gone. The same fill serves a player's `+ship/order`.
  - The held units are read from the hull's hauls of propellant aboard, not from the pool's
    reservation book, which the planner (a `&World` reader) cannot reach. Nothing else holds a
    tank's pool today.
- **Item 8, Q-50-22 (P2).** The tank farm's standing bid spent the farm's own money, outside
  Occator's cap.
  - A sale into the bid past the month's cap, or past what Occator's account holds, is refused
    (`FARM_BUDGET_SPENT`).
  - Each settled sale is kept once under its quote (`FreeportArm::fills`), counted in the month's
    spend beside the posts, and paid back to the farm from Occator on the port tick. The market's
    `SettledSale` now names the house (the quote's buyer), since the bid's listing may no longer
    stand when the policy is told.
  - A sale quoted but not yet settled is not counted, so two sales at once can overshoot the cap
    by one sale. The figures stay provisional.
  - The story (`op5_occator_funds_the_tank_farms_bid`) shows the refusal while Occator's account
    is empty and a funded sale counted and paid back; the cap's arithmetic, the fills beside the
    posts, is `stockpile`'s unit test.

## OP.5

The declaring arm, the operation frame and the resolution into control and the import duty
(Q-50-18, Q-50-19, Q-50-28, Q-50-29, Q-50-59, Q-50-64). On OP.4 (#732). **Lane v50 (relay v41)**, taken on OP.4's v49 with the lead's word.

**Survey.** At OP.4's head:
- **The frame** (OP.0): `operations::frame` finds the composed frame; `OperationFrame::declare`
  declares and activates in one step; `status` answers the phase and, once decided, who won. The
  campaign reads the tank farm's close probe at the window's close (OP.4). No TacSim arm declares.
- **The Review** (OP.2) fills on the Assembly's votes and the flagged seizures, and its fill does
  nothing: the clock names no control flip.
- **The import duty.** `polity::TermsRow` has `import_duty_permille` and `import_goods` since
  KC.2, and nothing reads them. The Isidis region row has no terms block.
- **The proceeds policy** (`port::proceeds::CoopProceeds`) is asked for every sale into a bid and
  every sale off a person's ask (`mush_market::fees::ask_policy_proceeds`): its cuts come out of
  the seller's proceeds, and the line is the seller's.

**Built.**
- **The declaring arm** (`tacsim::isidis_op`; Q-50-19), on the Assembly's tick.
  - When the Concession Review fills, on a clock it has not answered, it declares one operation
    through the frame and activates it in the same step:
    - declared by the Republic's customs office (`republic_customs`, the Republic's treasury in
      the lane), with a bond of `BOND_CR` (5,000 cr, provisional) held on its account and forfeit
      to Occator's on its failure (Q-50-57 item 5);
    - two sides, `republic` (the customs office) and `freeport` (Occator), and Occator defending;
    - 28 IC days of notice and a 28-day window, set by the arm;
    - the objective the tank farm's close probe, met in the defenders' favor (Q-50-28);
    - its legal-target list `carrier` (Q-50-26 reads it; OP.6).
  - Unwired, with no frame composed, it declares nothing and waits.
  - The record (`tacsim:isidis_op`) holds the clock answered, the operation, when it was declared
    and who won; a refused declaration is kept for staff and tried again on the next tick.
- **The resolution** (Q-50-28, Q-50-29): once the frame says who won, the arm writes it once.
  - The Republic's success makes it the Concession's de facto holder, through the one control
    write (`mush_faction::write_control`, Q-50-51); already there, nothing is written.
  - The defenders holding leaves the Freeport there; a partial or abandoned operation changes
    nothing.
- **The import duty at sale** (`tacsim::port::import_duty`; Q-50-29, Q-50-59, Q-50-64).
  - A good on the venue's holder's `import_goods`, sold at the venue into a bid or off a listing,
    pays `import_duty_permille` of its catalog value (`reference_major`) out of the seller's
    proceeds: one cut of the proceeds policy, paid to the holder's terms row's payee, never more
    than the sale less the Co-op's share.
  - The sale's line names it, with its reason (who holds and since when, as the transit duty's
    does), and what she receives is the sale less every cut.
  - Only the duty follows the holder (Q-50-64): the venue's fees, berth, locker, wages and release
    fee stay with the port's authority.
- **Lane v50**: the Isidis region row gains `terms.republic` = { payee `republic_customs`,
  `import_duty_permille` 50, `import_goods`: `metal_ore`, `ice_ore`, `carbon_ore`,
  `strategic_ore`, `volatiles`, `propellant` }, and nothing under the Freeport. The 50 permille
  is a **provisional economy default**, and the row's note says so: `TermsRow` gains `note`, read
  by nothing. The strip chain's new head is `lane_chain::strip_import_duty`.
- **A sale carries two cuts** (`mush_market::MAX_PROCEEDS_CUTS`, from one): the Co-op's share and
  a place's import duty, so a Co-op hull selling at Isidis under the Republic pays both. The
  market's own tests bound it there (`ic10_the_proceeds_split_is_exact_and_its_payees_bounded`
  refuses one past it; the quote record loads two and refuses three).
- **The farm's reading answers in any world**: the port tick reads the tank farm whether or not
  the Freeport arm is laid up, so the operation's close probe is answered there too.

**Tests.**
- `domain_world::op5_operation` (the production roster; a fake frame, below):
  - `op5_the_republic_declares_on_the_reviews_fill_and_its_success_takes_isidis`: nothing is
    declared while the Review is short. Filled (staff's `@clock/advance`), the next tick declares
    once: the customs office, the probe and its polarity, Occator defending, the sides, 28 and 28
    IC days, the bond forfeit to Occator. While it runs nothing changes hands. The Republic's
    success makes it Isidis's de facto holder, and it is never declared again.
  - `op5_when_the_defenders_hold_the_freeport_keeps_isidis`.
  - `op5_with_no_frame_composed_nothing_is_declared`.
- `domain_economy::op5_import_duty::op5_the_import_duty_follows_the_concessions_holder`: a pilot
  flies her Dray-P Ceres to Stickney to Isidis and sells propellant from her hold into the tank
  farm's bid. Under the Freeport nothing is levied. With the Republic holding the Concession
  (`@faction/control`), the same sale pays Republic Customs 5% of the catalog value out of her
  proceeds, and the line names the duty, its reason, and her net less it.
- **Mutations**, eight, each caught, each restored from a saved copy and compared:
  - the arm never declaring; the arm declaring every tick; success writing no de facto holder;
    the defenders holding flipping it too; the polarity flipped (the arm stories);
  - the duty not cut; the line keeping the net before the duty; the farm unread while the arm is
    laid up (the duty story).
- **On conclav.dev, green:**
  - all of tacsim (5,077 tests);
  - mush-server `split_boundary` and `commerce_composition` (320), mush-market (275),
    mush-contract-tests (208) and mush-composition (6);
  - `cargo fmt --all --check`;
  - clippy `-D warnings` on tacsim, mush-server, mush-composition and mush-market, all targets;
  - rustdoc `-D warnings` in both modes.
- **The first full run** found the duty refused on a Co-op hull: the market allowed one cut, and
  the share already took it. The limit is two now, and the market's tests are bound to it.

**Divergences.**
- **The fake frame.** TacSim has no campaign edge, so the arm's story seats a stand-in for the
  frame in its slot (`OperationFrameSlot`, a row in `state-slot-owners.toml`). The real frame is
  OP.0's, proven over the campaign in `mush-composition`'s `op0_frame`; the probe is OP.4's.
- **The declarer is the Republic's customs office**, the one Republic organization whose money is
  the state's in the lane.
- **Occator alone defends.** Meridiani is no lane organization (OP.2); Occator is the Freeport's
  backer at Isidis (Q-50-22).
- **One operation a Review clock.** A Review that fills again on a new clock (a front reopened)
  declares again; the same clock never does.
- **The arm is on the Assembly's tick**, a coarse minute, not the port tick: the Review moves on
  that tick, and a declaration a minute after the fill is soon enough.
- **The duty is on catalog value**, as the transit duty is (Q-50-53), capped at what the Co-op's
  share leaves of the sale.
- **The market's cut limit rises to two** rather than the duty being folded into the share's
  cut: each payee keeps its own row and label on the sale.

**Not built.**
- What a repeat starts from beyond the drain (Q-50-60 is OP.4's) and any reset of the Review after
  an operation.
- `+operation`, the notices and the Republic's bid (OP.7); legal targets in a bubble and breaking
  off (OP.6).

**Fixes from doc 50b** (the outside review of 2026-11-20; items by its numbering in the lead's
brief):
- **Item 2, organization listings paid no import duty (P1).** The market asked the proceeds
  policy only for a person's lot ask, so an `as=<org>` listing at Isidis sold duty-free.
  - `SaleTerms::seller` is now the seller principal, a person or an organization. The import duty
    reads no seller, so it falls on every listing.
  - The Co-op's share reads the seller only where it needs a person. An organization selling out
    of a chartered hold is refused as another's, as it is at posting.
  - The story: under the Republic an officer lists 10 t of propellant from her locker at Isidis
    for her organization. The buyer's quote shows the duty, and Republic Customs is paid it out
    of the organization's proceeds.
- **Item 4, an outcome before it is final (P2).** The declaring arm acted on any outcome the frame
  showed, and the campaign's frame shows its root's outcome while the operation settles, when it
  can still be aborted. The arm now acts only in `Settled`. The fake frame can show an outcome
  while settling: nothing is written then, and an abort that follows takes nothing.
- **Item 5, the quote omitted the duty (P2; Q-50-59).** `ProceedsPolicy::venue_cuts`, empty by
  default, names each cut a venue takes from any sale of a good there, with its reason.
  `+market/quote` on a lot ask shows each ("Out of it the seller pays …"). A bid is sold into and
  never quoted; a house bid's sale line already names the duty and its reason (OP.5). The Co-op's
  share hangs on the hold the goods leave, which a quote does not know, so it stays the sale
  line's to show.
- **Item 6, the shares counter (P3).** The enrollment's `shares_minor` folds only the Trust's
  share, never the duty.

**Doc 50b item 9: Meridiani defends** (Q-50-19 as amended by the owner, 2026-11-20). **Lane v52
(relay v43)**, on OP.7b's v51 with the lead's word.
- **Lane v52** seeds Meridiani Security Group as an organization at Isidis (`meridiani`): capital
  100,000 cr (provisional), polity `freeport`. Doc 11 §4.4 places it under retainer to the
  Freeport, defending the Concession; its Republic operating licence is not an edge, so it answers
  to the Freeport here (the smallest choice: a lane organization names one polity).
  `strip_meridiani` heads the strip chain.
- **The declaring arm names it a defender beside Occator** (`isidis_op::DEFENDER_ALLY_ORG`). It is a
  defender and not a side: the Freeport's side is still Occator's, and the bond still forfeits
  to Occator. Its members get the campaign's defender notice, and the page and the lane's notice
  name it.
- **The lane's notice skips only those the campaign tells.** `OperationStatus::told`, filled at
  the composition root from `mush_campaign::notice::defenders`, is who the campaign's own notice
  reaches within its 256-recipient cap. A defender's member past the cap, with a hull on the lane,
  hears the lane's notice; before, every defender member was skipped.
- **Stories:** OP.5's arm story names Meridiani among the defenders; OP.7's story has the notice
  and the page name it, and a pilot the campaign tells hears nothing from the lane.

**Owner rulings, 2026-11-20.**
- Hold the Concession keeps advancing once each IC month the farm stands at the threshold, as
  built.
- The repeat-cycle guard (the declaring arm keeps the Review clock's id) stays latent, out of
  scope until the repeat cut.

## OP.6

Legal targets in the operation's bubble, and breaking off (Q-50-26, Q-50-61, Q-50-62). On OP.5
(#735). **No lane bump.**

**Survey.** At OP.5's head:
- **In a space bubble every carrier is friendly to every other** (doc 47's open call). A carrier
  joining a bubble takes a party of its own (`next_available_carrier_party`) and is marked
  friendly to every carrier already there (`space/tick.rs`, the join).
- **A carrier leaves a bubble** when it is clear to resume (`is_carrier_clear_to_resume`): no
  operational ship of a party hostile to hers remains, the picket has cleared her, and nothing is
  in flight. With others still held, she detaches alone; otherwise the bubble closes.
- **The operation** is read through `isidis_op::status` (OP.5): its phase, window, sides (the
  Republic's customs office, Occator) and legal targets (`carrier`).
- **A hull's flag** is `concession::hull_flag` (OP.2), and the Freeport arm's jobs are
  `stockpile::arm(w).posts` (OP.4).

**Built.**
- **Whose side a hull flies for** (`tacsim::space::op_sides`; Q-50-61). Only while the operation
  runs (phase `Running`) and names `carrier` among its legal targets:
  - by its flag: its title holder's polity is a side's commanding organization's polity; or
  - by a live job of a side's arm: a Freeport arm haul taken or aboard, or an escort of one taken
    or watching.
  - A hull that would fly for two sides, or none, flies for none. Two hulls on different sides
    are operation foes (`op_sides::foes`).
- **Legal targets in a bubble** (Q-50-26). A carrier joining a bubble is hostile, both ways, to
  each carrier already there that is her operation foe, and friendly to every other, as before
  (`ActiveInterdictionBubble::relate_joining_carrier`). Each names the other among its foes
  (`BubbleCarrierEntry::op_foes`). Outside the window nothing changes.
- **Breaking off** (Q-50-62). A carrier with operation foes that holds outside `BREAK_OFF_M`
  (25,000 m, provisional) of every operational foe for `BREAK_OFF_WALL_S` (60 s of wall time,
  provisional) breaks off (`track_break_off`, on the space tick beside the salvage hold's
  expiry). Coming back inside starts the time again.
  - A carrier that broke off is no longer held by her foes (`is_carrier_clear_to_resume`): she
    detaches and resumes her passage. She loses the engagement and nothing else.
  - Every other hostile holds her as before: a picket, an interdictor or a raider is not escaped
    by distance.

**Tests.**
- `space::tick` unit tests, on a built bubble:
  - `op6_a_carrier_joining_her_operation_foes_is_hostile_to_them_both_ways`: hostile both ways,
    each names the other, foes hold each other; a carrier who is no foe joins friendly.
  - `op6_holding_apart_from_every_foe_for_the_time_breaks_off`: inside the reach no time runs;
    outside it the time starts, short of the time she has not broken off, coming back inside
    starts it again, and held for the whole time she breaks off and resumes, as does her foe.
  - `op6_breaking_off_leaves_every_other_hostile_holding_her`: the interdictor still holds a
    carrier that broke off.
- `domain_world::op6_sides::op6_in_the_window_a_freeport_hull_and_a_republic_hull_are_operation_foes`
  (the production roster, OP.5's fake frame): a Co-op pool hull (Freeport-flagged) and a hull
  titled to Stickney Yards (Republic) are foes, both ways, only while the operation runs; two
  Co-op hulls are not, nor is a hull flying for no side.
- **Mutations**, eight, each caught, each restored from a saved copy and compared:
  - a joining foe made friendly; nobody ever breaking off; breaking off at once; coming back
    inside keeping the time; breaking off freeing her from every hostile (the unit tests);
  - foes before the window opens; the flag ignored; one side foes to itself (the story).
- **On conclav.dev, green:**
  - all of tacsim (5,081 tests);
  - mush-server `split_boundary` and `commerce_composition` (320) and mush-contract-tests (208);
  - `cargo fmt --all --check`;
  - clippy `-D warnings` on tacsim and mush-server, all targets;
  - rustdoc `-D warnings` in both modes.

**Divergences.**
- **Foes were fixed when a carrier joined.** Replaced by doc 50b's fix (below): the foes are
  reconciled on every bubble step.
- **Breaking off frees her only from her operation foes.** Q-50-62 reads "every hostile hull";
  read that way a raider or an interdictor could be escaped by distance, which would change every
  bubble outside the operation. Q-50-26 rules that outside the operation today's rule stands.
  **Confirmed by the owner (2026-11-20, Q-50-62):** the break-off stays as built, timed from
  operation hostiles only.
- **The reach and the time are provisional**: 25,000 m (past the interdiction standoff of
  18,000 m) and 60 s of wall time, so a player at 24:1 is not held by a foe who logged off.
- **A side's polity is its commanding organization's**: the Republic's customs office is the
  Republic's, Occator is the Freeport's.
- **A job-borne side** (a Freeport arm haul or escort) is read as built, and no story here carries
  one into a bubble.

**Not built.**
- `+operation`, the notices and the Republic's bid (OP.7).
- The other side's clock moving when a hull flying for a side is lost (Q-50-25's last half): built
  by doc 50b's fixes (below).
- A line telling a carrier she has broken off; she resumes with the resume's own lines.

### Fixes from doc 50b

Doc 50b's two findings on OP.6's ground, fixed with OP.3's (above).
- **Operation hostility set once, at join** (P1). The foes were read when a carrier joined a
  bubble and never again, so two carriers stayed hostile after the window closed or a job ended,
  and carriers already sharing a bubble when it opened stayed friendly. Every bubble step now
  reads `op_sides::foes` for each pair of its carriers (`ActiveInterdictionBubble::
  reconcile_op_foes`, before the break-off is timed): a pair newly qualified turns hostile both
  ways and names each other; one no longer qualified turns friendly again and drops them; a
  carrier left with no foes is no longer timed or broken off. Only carrier-to-carrier relations
  move, so an NPC's hostility stands (Q-50-26).
- **A side's loss moves the other side's clock** (Q-50-25 as corrected, Q-50-51). When a haul's
  cargo is seized or lost in a hull flying for a side inside the window
  (`op_sides::side_of`), the other side's front clock advances one segment by a typed cause,
  `haul.loss/<contract>`, so a retry counts once (`haul::loss_moves_the_other_side`). The
  Republic's clock is the Concession Review and the Freeport's is Hold the Concession. The
  Review's flagged-seizure path is left as it is.
  - **A divergence:** in the first operation the Review is full once the window opens, so a
    Freeport-side loss moves nothing; only a Republic-side loss moves Hold.
- **Tests**: `space::tick::op50b_operation_foes_are_reconciled_each_step` (the window closing and
  opening; an NPC's hostility left standing) and
  `domain_world::op6_sides::op50b_a_sides_loss_moves_the_other_sides_clock_once`. Mutations, each
  caught: the reconcile changing nothing; no loss moving a clock.
- **On conclav.dev, green**: all of tacsim, mush-server `split_boundary` and
  `commerce_composition`, mush-contract-tests, mush-composition and the bot's library (its census
  gains the two new lines); fmt; clippy `-D warnings` on tacsim and mush-server; the M7 pass;
  rustdoc in both modes.

## OP.7

`+operation`, the notices and the job board's tags (Q-50-30). On OP.5 (#735). **No lane bump.**
The Republic's bid (Q-50-27) is OP.7b, below.

**Survey.** At OP.5's head:
- **The operation reaches TacSim** only through the frame's seam: `isidis_op::record` (the
  Review clock answered, the operation, when it was declared, who won) and `isidis_op::status`
  (phase, sides, defenders, window, outcome).
- **The defenders' notice is the campaign's** (OP.0): pushed on activation to Occator's active
  members. Nothing tells anyone else, and nothing is said at the resolution.
- **The pieces a page would join** each have their own reader: the two fronts' clocks
  (`concession::review_clock`, `hold_clock`), the tank farm's last reading
  (`stockpile::arm(w).level`), the Freeport arm's posts (`FreeportArm::posts`), Isidis's
  control and its terms rows (`polity::control_at`, the region row's `terms`).
- **The job board** is TacSim's `ServiceBoard`: a haul or escort row's subject is the host's
  words, so a tag is the host's to add.
- **Two LW.4 strings** carried runs of spaces from a lost line continuation: the offense notice
  ("its      fine") and `+record`'s fine line before `(+debts/pay …)`.

**Built.**
- **`+operation`** (`tacsim::op_page`; Q-50-30), a read-only verb with a `legacy-handlers.toml`
  row and a line in `help faction`. One page:
  - who declared against whom: the declarer, for the Republic, against the defenders, for the
    Freeport;
  - **the stake**, as what each side's win changes in the terms at Isidis, read from the lane's
    terms rows (`polity::terms_of`, new): under the Republic, the goods on its row pay its
    import duty to its payee; under the Freeport, cargo pays none;
  - the notice and the window, in IC dates and in real hours from now; once decided, who won;
  - how it is decided: the tank farm's threshold at the close, and its level now;
  - both fronts, by their titles, with their clocks;
  - while the operation is in notice or in its window, **the work for it**: each of the
    Freeport arm's hauls and escorts still untaken, with its pay and the verb that takes it;
  - **who holds Isidis now**, since when, and what that puts on a sale there: the newcomer's
    line;
  - **past operations** with their result (`IsidisOp::past`, new, at most 16).
- **The job board tags the operation's work** (`op_page::job_tag`): while the operation is in
  notice or in its window, an arm's haul or escort on `+market/jobs` reads "(work for the
  Isidis operation, the Freeport's side)". Once it is decided, the tag goes.
- **The notices** (Q-50-30 as corrected, Q-45-17's channel). An official notice, delivered at
  once, to every player with a hull on the lane (`op_page::lane_audience`: a live player, not the
  God object, with a hull whose hold they may trade from):
  - **at the declaration**: who declared for whom against whom, when the window opens (IC, and
    real hours) and how long it runs. Occator's active members are left out, since the
    campaign's own defender notice reaches them;
  - **at the resolution**: who won, and what it puts on a sale at Isidis.

  A connected player hears it; an absent one has it queued for login (`port_tick::notify`).
- **The LW.4 strings** read with single spaces.

**Tests.**
- `domain_economy::op7_operation::op7_the_lane_follows_the_operation_and_a_newcomer_reads_who_won`
  (the composed world, a fake frame in the slot, as OP.5's arm story seats one):
  - Before: a newcomer with no hull reads that nothing is declared, the Review at 2 of 6, and
    the Freeport administering a duty-free Isidis.
  - Staff fill the Review, and the Assembly's tick declares. A pilot with a hull hears the
    declaration's notice once, and the newcomer hears nothing.
  - The page shows the declarer, the defender, both sides' terms (the 5% duty to Republic
    Customs under the Republic, none under the Freeport), the notice and window, the farm's
    level, and the Review full.
  - The opened Freeport arm posts its haul: the page lists it with `+haul/accept`, and
    `+market/jobs haul` tags it.
  - The window opens, then the Republic wins. The pilot hears the resolution's notice, which
    names the duty. The newcomer, who never joined, reads "Decided: the Republic won", that the
    Republic administers Isidis and since when, and the duty a sale there pays. The board's tag
    is gone.
- `op_page::tests`: a list in words; the past kept once and bounded.
- **Mutations**, each caught by the story at its own assertion: no declaration notice; no
  resolution notice; no board tag; the notice to every player, hull or none; the holder line
  always the Freeport's; the stake naming the Freeport's terms for both sides.
- **On conclav.dev, green:**
  - `domain_economy`'s `op7_`, `op5_`, `op4_`, `ex5_jobs`, `ex6_haul`, `ex7_wages`, `lw4_record`
    and `lw3_law_audit` stories, and `domain_world`'s `op5_operation`;
  - all of the tacsim lib (2,551 tests);
  - mush-server `split_boundary` and `commerce_composition`, mush-contract-tests (the help lint
    and the state-slot owners among them), mush-composition and the bot's library;
  - `cargo fmt --all --check`; clippy `-D warnings` on tacsim and mush-server, all targets; the
    M7 memory-size pass; rustdoc `-D warnings` in both modes.
- **The first run** found the page naming each front's clock by its own title (its causes)
  rather than the front's, and a connected pilot's notice said live where the story read only
  the queue. The page names the fronts, and the story reads both.

**Divergences.**
- **The notice goes to whoever may trade a hull's hold** (`trade_hulls`: a charter, a title, a
  commissioned ship, a ferry, a house helm). Q-50-30 says "a hull on the lane", and the lane is
  the one game.
- **No IC mail**: Q-50-30 as corrected makes the notice an official one, delivered at once.
- **The declaration's notice leaves out the defender's members**, who have the campaign's. The
  resolution's goes to everyone with a hull: the campaign sends nothing at the resolution.
- **The page reads one operation**, the arm's record, and keeps the past on it. There is one
  Isidis operation at a time.

**Not built.** The Republic's bid (OP.7b). A tag for a Republic job: the Republic posts none in
the first operation (Q-50-27 as corrected). A notice for an organization's members who hold no
hull.

### OP.7b

The Republic's bid at Stickney (Q-50-27). Stacked on OP.7 (#738). **Lane v51 (relay v42)**,
taken on OP.5's v50 with the lead's word.

**Survey.**
- **The Republic Customs Office** (KC.2) is a Stickney organization with an account and no
  storefront, so nothing can receive what a bid buys. Its capital is 10,000 cr, and OP.5's bond
  holds 5,000 of it.
- **A personal bid** (`mush_market::seller::post_bid`) is the actor's own and refuses `as=`; a
  storefront bid is a vendor's (`post_wanted`), and a vendor's storefront is an authored item in
  the port's staff spine that backs no vault yet (`port::vendor`).
- **Stickney's spine holds three items**, and each backs a vendor's vault. Its works room is
  the spine, and backs the works' vault.
- **A new item** in `neighborhood.stickney_yards` bumps it to v5. Nine packs require v4 exactly
  (`check_requires` is exact): the Stickney cast pack, `neighborhood.phobos_exchange`, both
  passenger packs, four quest packs (`survey_work` through `first_steps`) and the relay. The
  immutable `passengers.stickney` v1 fixture, which `domain_world::knowing` boots as an old
  binary's world, requires v4 too.
- **A new boot object** renumbers every later one. Tried first (a `DARK` Thing the applier
  made in the spine), it moved the dice of two field stories
  (`field_director::lane3_…` and `gc11_…`), both green on main twice and red here twice.

**Built.**
- **The store the clerk keeps** (`port::vendor::CLERK_STOREFRONT`). A vendor row whose
  `storefront` is `clerk` names no authored item: its storefront vault is backed by the port's
  stock clerk, the `DARK` Thing the applier makes once in the staff spine.
  - No object is made, so nothing renumbers, and no atlas pack changes.
  - The map row records the clerk as its storefront, so a re-apply or a restart keeps it.
  - A recorded clerk that is gone is refused, as every recorded storefront is ("never
    re-created — repair the map by hand").
  - At most one vendor of a port names it (`check_ports`), since one object backs one vault.
- **Lane v51**: Stickney's `vendors` gains the Republic Customs Office's row, `clerk`, with no
  standing bid, its signatory Ilunga-Brandt of the customs detail. The office's capital rises from
  10,000 to 20,000 cr (provisional): the 5,000 cr bond and the bid's 11,200 cr, with room. The
  row's `note` says why, and `VendorDef` gains `note`, read by nothing. The strip chain's new head
  is `lane_chain::strip_customs_store`. The office gains the `acceptor` role, without which a
  contraband fine after the Republic's flip failed its acceptance ("Registry customs fine
  acceptance failed") and the ship was never cleared (found by KC's doc 50b story).
- **The Republic's bid** (`op_page::post_republic_bid`, Q-50-27 as corrected): when the arm
  declares, it posts a standing bid through `mush_market::post_wanted` for `BID_T` (200 t) of
  propellant at `BID_CR` (56 cr, provisional) into the office's store, and records the listing
  (`IsidisOp::bid`). At the resolution it withdraws it (`withdraw_republic_bid`).
  - 56 cr is over the tank farm's 50 at Isidis, so a hauler has a second buyer, and under
    Kasei's 60 ask at Stickney, so nobody buys propellant there to sell it back.
  - Every tonne sold to the Republic is one not landed at Isidis.
- **`+operation`** lists the bid among the work while it stands, with `+market/sell`.

**Tests.**
- `domain_economy::op7_operation::op7b_the_republic_bids_at_stickney_while_the_operation_runs`:
  - the clerk keeps the store, in the spine, `DARK`, the vendor registered;
  - a restart keeps the same store and vault;
  - the Review fills, the Republic declares, and its bid stands at Stickney: 200 t at 56 cr, the
    office able to fund all of it beside its bond;
  - `+operation` shows the bid with its `+market/sell` line;
  - the defenders hold, and the bid is withdrawn.
- `domain_economy::commerce::op7b_the_clerk_keeps_the_customs_store_and_a_reapply_keeps_it`: the
  store's vault is backed by the clerk. A later lane version, applied by the shelf, re-runs the
  applier over the recorded row: the same store and vault, and no object made.
- `domain_economy::lane::op7b_strip_customs_store_rebuilds_the_published_v50`.
- **Pins moved** with the lane: v51 and relay v42 everywhere OP.5's v50 stood; the shipped
  lane's digest; Stickney's four vendors (`commerce::ic8_…`, `ports::ic3_…`), its vendor line
  in the readiness report, the treasury's 17,471,000 cr and the office's 20,000 cr base
  (`plumbing::sm5_…`).
- **Mutations**, each caught: no bid posted; the bid never withdrawn; a re-apply re-keeping the
  clerk's store (the applier refuses a second vault on it, "that item already backs vault #…",
  and the shelf's apply stops part way).
- **On conclav.dev, green** (main with OP.6 and OP.7 merged in): all of tacsim; mush-server
  `split_boundary` and `commerce_composition`; mush-contract-tests; mush-composition; the bot's
  library; fmt; clippy `-D warnings` on tacsim and mush-server; the M7 pass; rustdoc in both
  modes.

**Divergences.**
- **The clerk keeps the store, not a new item or Thing.** The cascade (nine packs, and the
  immutable passengers v1 fixture), and then a new boot object, were each worse; the lead ruled
  between them. The vault is what the economy holds, and a container is only its backing.
- **The bid is the arm's, not a lane row**: posted at the declaration and withdrawn at the
  resolution, so it stands only while the operation runs (Q-50-27: "in the first operation").
- **The figures are provisional**: 56 cr, 200 t and the 20,000 cr capital.

**Not built.** Escorts for the Republic's own supply hulls (Q-50-27 as corrected): the Republic
posts no haul in the first operation, so there is no hull to escort. The owner ruled on
2026-11-21 that none are built for now (Q-50-27's amendment). Salvage of interdicted hulls,
which waits for a contract of its own.

## OP.8

The empty run and the second acceptance story (Q-50-47, as corrected and as amended on
2026-11-20), and an audit of the other two. On main after the doc 50b fixes (#743 to #749). **No
lane bump.**

**Survey.**
- **No test ran the operation on the real frame.** TacSim has no campaign edge:
  `composition_edges` refuses even a dev-dependency that names `mush-composition`. So OP.5's and
  OP.7's stories seat a fake frame, and staff fill the Review in OP.7's newcomer story. The real
  frame is composed in mush-composition and mush-server only.
- **What doc 50a said blocked the empty run**: a resolution between organizations advanced no
  clock, and no NPC hull answered the picket. KC.0a and OP.5 write control directly, OP.4 and
  EX.10 put a house hull on the direct run under her house's mind, and doc 50b item 1 sends a
  hull that delivered at Isidis back home.

**Built.**
- **`mush-composition/tests/op8_campaign.rs`**, a new test binary. It composes TacSim's own test
  roster by path (`driving`'s rig and `tests/support`, as mush-server's `commerce_composition`
  takes `vertex.rs`). It seats the campaign beside it as `server_modules_with` does:
  `CampaignLink`, with TacSim's close probe, the composition's defender notice and the formation
  authority.
  - `driving_rig::EXTRA_MODULES`, a once-set list of extra modules. Only this binary sets it;
    TacSim's own binaries never do, and their roster is unchanged.
  - The rig runs at the production 24:1. It reopens the Freeport arm and Ceres' hauling house,
    which the stock rig lays up and a production world has open. It steps six IC hours at a
    time, and a picket's bubble a wall second at a time until it closes.
- **Found and fixed: the house never came home for the arm's work.** On the first empty run, both
  Ceres hulls set out on trades on day 1, one to Luna and one onward to Deimos. From day 19 one lay
  idle at Stickney, where no trade paid, and she never went back to Ceres. The arm's hauls are
  taken only at Ceres, so each lapsed at its 28-day acceptance window and was posted again. No
  NPC hull ever carried one, and the farm drained to nothing by day 150.
  - **`hauler::called_home`**, with **`stockpile::awaiting`** (Q-50-24). A hull lying idle away
    from her home port, while an arm's haul is on the board there (posted, untaken and inside its
    window, however young), sails home empty one hop at a time, before she looks for a trade.
    Each hop is a trip of its own and ends where she docks.
  - `hop_toward` now answers whether it wrote the hop's plan.
  - After the fix, VCC-01 carries four of the arm's hauls into the farm. She meets the Kirkwood
    picket on days 53, 88, 123 and 157, heaves to under her house's mind and is cleared each time.
- **`+record` shows the standing lost** (Q-50-42, and the third story's "the standing loss stays
  and shows on `+record`"). Under `STANDING_LOST`, it lists each polity that proved an offense
  against the actor and the actor's standing with it, while that standing is below zero, paid or
  not. Before, standing showed only while it alone kept a restriction in force, so a paid
  smuggler at −5 saw nothing. `RECORD_HELP` says so.
- **LW.3's private sale** is lifted into `lw3_law_audit::sell_privately`, shared with the third
  story.

**Tests.**
- **`op8_campaign::empty_run::op8_with_nobody_connected_the_review_fills_and_the_operation_resolves`**
  (about 175 s). No player is connected, and nobody types a line after the rig's setup.
  - The Assembly's votes move the Review on days 30, 60, 90 and 120, and every cause is an
    `assembly.vote/…`.
  - On day 120 the arm declares through the campaign's frame. The operation goes from notice to
    its window on day 148 and is settled on day 176.
  - The house takes and delivers the arm's hauls, and clears the picket.
  - The Republic wins: it holds Isidis de facto, and Isidis's terms row levies the import duty.
- **`op8_campaign::newcomer::op8_a_player_who_never_joined_reads_who_won_from_the_terms`**
  (about 180 s), with two accounts on production verbs.
  - The hauler takes the arm's first haul (`+haul/accept`). She flies the direct run, heaves to at
    Kirkwood and submits (`+helm/thrust`, `+helm/submit`), and is cleared. She carries on by
    Stickney to Isidis and delivers into the tank farm (`+haul/deliver`).
  - She logs off. The Review fills on the votes, the campaign settles, and her queue holds the
    lane's notices of the declaration and of who won.
  - The newcomer is made only after the operation, with no notice queued. At Isidis, `+operation`
    reads "Decided: the Republic won", "Isidis now: the Republic administers the Concession", and
    the import duty a sale there pays.
- **`domain_economy::lw4_record::op8_a_reported_smuggler_who_cannot_pay_is_restricted_until_paid_and_keeps_the_loss`**,
  the third story, at Ceres with two accounts.
  - A prospector with no claim sells a trader 2 t of strategic ore privately, then spends
    everything. The trader reports the sale and hands the lot over.
  - The smuggler is fined the 1,000 cr floor at once and cannot pay. They are told at once, are
    5 standing down, and are under the Freeport's restriction. The trader is not paid yet.
  - `+record` shows how the sale was proven (never by whom), the fine and its grace, what lifts
    the restriction, and the standing lost. The board refuses the smuggler's strategic ore.
  - Paid through `+debts/pay`, the restriction lifts and the trader is paid 600 cr back plus
    200 cr. The standing stays at −5 and still shows on `+record`, and the board buys from the
    smuggler again.

**The audit (Q-50-47's three stories).**
- **First, KC.3's `kc3_the_same_run_before_and_after_the_flip_names_the_duty_and_why`: meets the
  ruling as amended.** Two accounts (`chartered` and `chartered_as`, each mapped to its own
  gateway account). The second files the run before the flip: no duty line, and the Co-op's
  terms are pinned and left so by the flip. The first plans under the Co-op, then plans and files
  after the flip with the duty and its reason, and the picket charges it. It runs at 3,600:1, as
  the owner's amendment allows. The flip is a fixture (`republic_holds_kirkwood`, one control
  write), which the ruling does not forbid.
- **Second: OP.7's newcomer story was not it.** Staff filled the Review and a fake frame decided
  it. The new `newcomer` story above is.
- **Third: missing as one story before this cut.** `lw3_a_buyer_reports_a_private_sale_…` had
  the report, with the fine paid at once. `lw4_an_unpaid_fine_restricts_until_it_is_paid` had the
  unpaid fine, the restriction and the payment, from a staff audit rather than a report. Neither
  showed the standing on `+record` after payment, because the page did not show it. The new
  `lw4_record::op8_…` story and the `+record` line above close that.

**What it found that this cut does not fix.**
- **Hold the Concession cannot move in the empty run, nor can the Freeport hold**, at the
  provisional figures. The farm starts at 3,000 t against a 4,000 t threshold and drains 20 t an
  IC day, so it is empty from day 150. The house's hauls are 20 t each, about one a month. So
  only the Review moves, though Q-50-47 says both fronts do. The defenders need some 1,000 t over
  the starting stock, plus 20 t a day, landed by the close. The second story's single player haul
  does not change that. The figures are Q-50-21's, Q-50-22's and Q-50-60's, all provisional, and
  the owner's to tune. **Retuned below** (OP.8's retune).
- **A house hull at home still trades.** She can set out on a trade before an arm's haul there
  is seven days old, and is called home only once she lies idle somewhere else. On the empty run,
  both of the first two hauls lapsed this way. **Fixed in OP.8's retune** (`waits_home`).

**Divergences** (the smallest choice where doc 50 is silent).
- **The two stories live in mush-composition, not TacSim**, because only there are the campaign
  and the lane composed together.
- **They run at 24:1**, the production rate, not the 3,600:1 the owner's amendment allows.
- **The rig's setup is staff's**: the arm and Ceres' house opened, and the rate set. The hauler's
  propellant and stores are seeded at each port (`load`), as OP.3's story does, standing in for
  buying them. The newcomer arrives at Isidis by teleport.
- **The second story has no escort.** Its two accounts are the hauler and the newcomer, and the
  arm's escort for the haul is left untaken. An escort's hull would meet the picket beside the
  hauler's, which no rig drives yet.
- **Called home comes before a trade**, even one that pays where she lies.
- **The standing line lists only polities that proved an offense**, and only a standing below
  zero.

**Mutations.** Each was applied to a fresh clone of the cut and caught by its story:
- a hull never called home: the empty run, at "the house took an arm's haul";
- no standing line on `+record`: the smuggler story, at the standing heading;
- the Republic's control write skipped: the second story, at "Isidis now: the Republic
  administers the Concession".

**On conclav.dev, green:**
- all of tacsim, mush-composition and mush-contract-tests, the help lint, census, panic budget
  and state-slot owners among them. That is 5,380 tests. The smuggler story's first run failed
  at its last step, because the claim it seeds had not been seen by a fire; it was fixed and re-run.
- mush-server's `split_boundary` and `commerce_composition` (320);
- `cargo fmt --all --check`; clippy `-D warnings` on tacsim and mush-composition, all targets;
  rustdoc `-D warnings` on tacsim in both modes.

### OP.8's retune: the Isidis figures (lane v53, relay v44)

The owner on 2026-11-21, of this cut's finding that neither front could move: "Yes, retune them".
#752 had merged, so the retune is its own PR, from main. **A lane bump: lane v53 and relay v44.**

**The targets** (the lead's brief):
1. the empty run moves both fronts;
2. with no players the Republic wins, narrowly;
3. players decide it;
4. the arm's budget covers the hauls, with no new money.

**Measured first** (probes on conclav.dev, four variants a batch, in parallel):
- **The house has one hull for the arm.** VCC-01 is Ceres-based; VCC-02's home is Luna, so she
  is never called to Ceres.
- **VCC-01's round trip is about 35 IC days**: she takes a haul at Ceres, delivers it at Isidis
  about 18 days later, and is home 17 days after that.
- **Haul size:** a 30 t haul flies the direct run. At 40 t and 60 t she takes the haul and never
  sails, so 30 t is the most her tankage allows.
- **So the house lands about 0.86 t an IC day.** The drain is whole tonnes, so 1 t a day is the
  least that can still beat the house.
- **In the window the picket takes the house's haul as the Freeport's** (OP.6's operation foes),
  and her unescorted hull is lost, around day 147 to 152.

**The figures** (all provisional; before → after):
- **The threshold** (Q-50-21): 4,000 → 2,960 t. The starting stock stays 3,000 t.
- **The drain** (Q-50-60, lane): 20 → 1 t an IC day. Its export faucet drops from 1,000 to 50 cr,
  in step with what is used.
- **One haul** (Q-50-22): 20 → 30 t.
- **The arm's reserve** (new): the arm posts while the farm is under its threshold plus two hauls
  (60 t), where before it stopped at the threshold.
- **Unchanged:** the fees (600 and 400 cr), the monthly cap (20,000 cr), the two open hauls and
  the 7-day fallback (Q-50-24). A 2-day fallback was measured and moved the close by nothing.

**Why these** (on the measured runs):
- **The start stock must be over the threshold.** With the house slower than the drain, the start
  is the farm's highest point, so a threshold above it could never be reached. The start stock
  alone would fall under the threshold by day 40.
- **The house holds the farm at its threshold** month after month, and Hold the Concession moves
  on those months (target 1).
- **The close reads 2,945 t on the empty run**, 15 t short, and the Republic wins (target 2).
- **With one player haul more it reads 2,977 t**, 17 t over, and the Freeport holds (target 3).
  The threshold sits between the two, half a haul from each.
- **The reserve is why a player's haul counts.** Without it, a delivery that put the farm at its
  threshold paused the arm's posts and cost the house a trip, which cancelled the haul.
- **The budget** (target 4): the arm posts at most once a day with two open, about two a month at
  1,000 cr each, against the 20,000 cr cap. It posted to the end of both runs from Occator's
  treasury.

**The small gap, fixed: a house hull at home waits for the arm's work** (`hauler::waits_home`).
- Docked at her home port while an arm's haul is on the board there, she sets out on no trade
  (idle for the day, with a note saying why). On the first run VCC-01 left on a trade on day 1;
  now she takes the arm's first haul on day 7.
- **Called home now runs before the daily sale of what is aboard.** Before, a hull away from home
  with a hold nobody there bought tried to sell every day, and never reached the call home. She
  now carries it home.

**Tests changed.**
- **`empty_run`** now asserts:
  - the Republic wins, the farm short of its threshold at the close;
  - Hold the Concession moved on a threshold month after the start stock alone would have run
    short;
  - the Review starts at two and Hold at two or three, since a fresh farm's first reading meets
    the threshold.
- **`newcomer` is target 3's story.** Her one haul is additive: the house's one Ceres hull leaves
  the second open haul on the board. The story asserts the farm at its threshold at the close,
  that the Freeport holds Isidis, and that the newcomer reads the Freeport's terms: "the Freeport
  held the Concession", and no import duty. This divergence was chosen over a second full story
  (180 s); the empty run is the story without her.
- **`op4_stockpile`:**
  - a fresh farm stands inside the arm's reserve;
  - the drain is 1 t;
  - the first reading moves Hold;
  - seeded past the threshold and the reserve, the arm posts nothing;
  - the close-probe story first draws the farm under its threshold, a day at a time.
- **The lane bump's pins** (v53, relay v44, both digests), and `lane_chain::strip_farm_drain` at
  the strip chain's head, with `lane::op8_strip_farm_drain_rebuilds_the_published_v52`.
- **`op4_stockpile`, two more.** The untaken-haul story steps a picket's bubble a wall second
  at a time, as production does (`ahead_through_bubbles`). A 30 t haul brakes more slowly than a
  20 t one, so an hour's jump left her leg paused at Kirkwood past the haul's due time. The
  Kirkwood duty story keeps the arm laid up, since a house hull at home now waits for the arm's
  haul and that story is a trade's cargo.
- **Three more pins:**
  - `op7_operation` reads the threshold (2,960 t) and Hold's count off the clock;
  - `ex0_matter_sinks` counts the farm's whole days at a tonne;
  - `voyage::cr_burning_past_the_picket_…` lands only the pilot's own customs ruling.
- **`claims::sm10_a_heard_sounding_…` lays up the house and the arm.** Both stories' rigs leave
  them open. Since `waits_home`, VCC-01 takes the arm's haul and meets the picket, or burns, in
  their scenes. A bisect on conclav.dev put both failures on `waits_home` alone, and both pass on
  main.

**Mutations** (each applied, its story run, then restored by re-applying the original text):
- **the arm's reserve at 0:** the newcomer story fails at "the arm posts a haul". A fresh farm
  stands over its threshold, so with no reserve the arm never posts at all.
- **no threshold news:** the empty run fails at "Hold moved after the start stock ran short".

**On conclav.dev:**
- **The full gate** ran all of tacsim, mush-composition and mush-contract-tests (5,381 tests), with
  6 failures. They are the four stories and pins above, and the two `waits_home` scenes. The
  failing ones and every `claims::sm10_` story then passed on the fixed patch.
- **Also green:** mush-server's `split_boundary` and `commerce_composition`; `cargo fmt --all
  --check`; clippy `-D warnings` on tacsim and mush-composition (all targets) and mush-server
  (tests); rustdoc `-D warnings` on tacsim in both modes.
- **Both OP.8 stories pass** with their outcomes asserted, about 3 minutes each.

**Divergences.**
- **The start stock is over the threshold**, against OP.4's "a fresh world starts short". Target 1
  is read as "the house's traffic holds the farm at its threshold"; above, it could never be
  reached.
- **Hold the Concession fills on the empty run.** Every month before the window has a reading at
  the threshold after a delivery, so the clock goes from two to six. Its consequence line is
  news; nothing reads its completion.
- **The arm posts past its threshold**, up to a reserve of two hauls.
- **The contest is about one haul wide.** A future change to the house's timing or the picket can
  move the close by a haul either way; both stories assert their outcome, so it will show.

**Not done.**
- Another Ceres hull for the house, which would widen the contest. It is a lane change to EX.10's
  roster, the owner's call.
- No escort on either story.

### OP.8's second Ceres hull (lane v54, relay v45)

The owner on 2026-11-21, of #755's leftovers: "add a second ceres hull, leave the other for
later". Nothing reads Hold the Concession's completion, and that stays as it is. On main after
#755.

**Built.**
- **VCC-03, a second Ceres-based hull** in Ceres' hauling house (EX.10's roster, in the lane):
  lane v54, relay v45 re-signed, the pins moved, and `lane_chain::strip_second_hull` at the head
  of the strip chain (`lane::op8_strip_second_hull_rebuilds_the_published_v53`).
- **`stockpile::posting`**, which says whether the arm is posting: open and granted, the farm short
  of its threshold plus its reserve, and a slot free.
  - `hauler::waits_home` now also keeps a Ceres hull at home while the arm is posting.
  - Measured: in the newcomer story the player takes the arm's only posted haul on day 0. With
    nothing on the board on day 1, both house hulls left on trades and took nothing until days
    37 and 51, so her haul added 2 t to the close, not 30.

**The figures** (provisional; the retune's figure → this cut's):
- **threshold:** 2,960 → 2,905 t;
- **drain:** 1 → 2 t an IC day, its faucet 50 → 100 cr;
- **the arm's reserve:** two → five hauls (60 → 150 t);
- **open hauls:** 2 → 3;
- **unchanged:** the 30 t haul, the 3,000 t start stock, the fees, the cap and the 7-day fallback.

**Why** (measured on conclav.dev; drains of 1, 2 and 3 t and a 25 t haul, run in parallel):
- **The two hulls land 60 t about every 35 days**: VCC-01 and VCC-03 deliver on days 26, 61,
  96 and 131, and both are lost in the window, on days 151 and 152.
- **At 1 t a day the house wins**: the farm reads 3,065 t at the close and the Freeport holds with
  no players.
- **At 2 t a day it doesn't**, and the empty run's close is 2,890 t. With one player haul more it
  is 2,922 t, the haul adding 32 t once the house waits for the arm's posts. The threshold sits
  between them, 15 t from the empty run's close and 17 t from the player run's.
- **The player needs one haul**, and that haul is additive: the third open haul is hers.
- **The reserve has to clear the farm's peak**, 3,008 t on the empty run. At the old two hauls,
  threshold plus reserve (2,965 t) would sit under the 3,000 t start stock, and the arm would not
  post. Five hauls (3,055 t) clears it.
- **The budget** is at most three posts open, at 1,000 cr each, under the 20,000 cr cap, from
  Occator's treasury.

**Divergences.**
- **Hold the Concession fills on the empty run**, as at the retune.
- **A Ceres hull waits at home while the arm is posting**, even before it has posted.
- **The second hull is VCC-03, home Ceres**, on the house's lot, goods and design (a Dray).

### Isidis bring-up (2026-11-22)

What a fresh world needs for the Isidis operation, checked against the production boot (lane v54,
relay v45), before the owner's wipe and redeploy of dev1 and dev2. The OP.8 stories' setup is
staff's three lines: the Freeport arm opened, Ceres' house opened and the rate set to 24. The
stock rig lays the arm and the house up, so the stories open them again. A production world
needs none of the three.

**What boot does on its own.** `mush-server`'s boot composes the content module with the game
packs whenever `[tacsim].enabled` is on (`crates/mush-server/src/boot.rs:524`), and the lane is
one of them (`crates/tacsim/src/composition/mod.rs:740`). Its appliers run on the first boot of
an empty store.
- **The Concession Review and Hold the Concession**: seeded by the regions applier on the
  `isidis` row (`crates/tacsim/src/polity.rs:413` → `crates/tacsim/src/concession.rs:134`;
  the row is `crates/tacsim/content/lane/ceres_phobos.json:3390`). Both start at two of six.
- **The Assembly's votes**: the `tacsim.assembly` tick (`concession.rs:293`, registered from
  `crates/tacsim/src/port_tick.rs:718`), one an IC month. On the same tick the declaring arm
  declares on the Review's fill and posts the Republic's bid (`crates/tacsim/src/isidis_op.rs:157`,
  the bid at `:200`).
- **The Freeport arm**: open by default (`crates/tacsim/src/stockpile.rs:118`, `laid_up` is
  false unless staff lay it up). The port tick runs it (`port_tick.rs:2209` →
  `stockpile.rs:682`), and it makes its agent and grant on its first fires. Its budget is
  Occator's treasury (`ceres_phobos.json:2786`, 50,000 cr and a 250,000 cr raise), capped at
  `ARM_MONTHLY_CAP_CR` (`stockpile.rs:85`).
- **The tank farm's start stock**: 3,000 t of propellant, its bid and its 2 t a day drain
  (`ceres_phobos.json:3166`–`3170`).
- **Ceres' hauling house, VCC-01, VCC-02 and VCC-03**: `ceres_phobos.json:2917`–`2918`. It is
  open by default (`crates/tacsim/src/port/hauler.rs:1268`), and the port tick seeds its hulls
  and trades from the IC day after it first sees the house (`port_tick.rs:2207` →
  `hauler.rs:1335`).
- **Meridiani as a defender**: its organization is `ceres_phobos.json:3163`, named in the
  request by `isidis_op.rs:45`.
- **Republic Customs' acceptor row**: `ceres_phobos.json:2946`.
- **The OP.7b storefront**: the clerk's store row at Stickney (`ceres_phobos.json:3008`). The
  bid is posted at the declaration (`isidis_op.rs:200`).
- **The rate**: `IC_RATIO=24` in `deploy/dev1/instance.env:103` and `deploy/dev2/instance.env:103`,
  seeded into the world timeline on an empty store. The stories' `@tacsim/time rate=24` is that
  rate.

**The campaign is seated in production.** `CampaignLink` sits in `compose_server_modules`'
roster with no condition (`crates/mush-composition/src/lib.rs:434`–`444`). It has the
formation authority, the defender notice (`campaign_notice.rs:17`) and `TacsimCloseProbe`, whose
probe table names the tank farm (`crates/tacsim/src/operations.rs:180`). All three of the
server's roster paths reach that function: the launch (`lib.rs:201`), mind commerce (`:172`) and
the units roster (`:139`). `driving_rig::EXTRA_MODULES` is the test binary's copy of the same
seat. No test boots the server's roster and finds the frame; `op0_frame` composes the link by
hand.

**The checklist after a wipe.** No command is needed. These read-only lines confirm it, as #1
once the bring-up is done:
1. `@tacsim/time`: 24 IC seconds per real second.
2. `@tacsim/port arm`: "Freeport arm: open"; after a few fires, a reading near 3,000 t.
3. `@tacsim/port haulers ceres_port`: open, with VCC-01, VCC-02 and VCC-03 seeded from the next
   IC day.
4. `+operation`: nothing declared, the Review at 2 of 6, Hold at 2 or 3, the Freeport
   administering Isidis.

Don't run `@tacsim/port arm off` or `@tacsim/port haulers ceres_port off`. They are the test
rigs' lines.

**Where it belongs.** Staff bring-up is `deploy/README.md`'s *Named launch and the player
market* (*The bring-up script*, *The sequence by hand*), and `launch-bringup.py` sends no Isidis
command. The checks above belong there as a last step after the Active boot, once the redeploy
is done.

**What could make a redeployed world misbehave.**
- **The Assembly's votes, fixed below.** Before the fix they fell on absolute IC month
  boundaries while the house ran from boot, so where in the calendar the first boot landed moved
  the tuned contest. By the v54 timeline (an estimate), a first boot about 14 to 21 IC days before
  a boundary let the Freeport hold the empty run. They now count from the instant the Review was
  seeded (*The Assembly counts from the seeding*, below), so any wipe time runs the stories'
  contest.
- **Downtime.** IC time runs on wall time, so it passes while the server is stopped. The
  Assembly catches up every month it missed, counted from the seeding. The farm drains at most
  once a fire's day, and the house's hulls don't move. A long stop during the run moves the
  contest off its tuned figures.
- **The launch's Houses are not in the stories.** The bring-up seeds their Drays at Ceres
  (`launch-bringup.py:1525`). Nothing in the minds or procurement takes `+haul/accept`, but the
  stories have never run beside them.
- **Hold the Concession's completion changes nothing**, as noted under OP.8's retune.
- The sky's tables run to IC 2400. The IC year is about 2353.6 today, so they last about two
  real years at 24:1.

### The Assembly counts from the seeding (the bring-up's fix)

The owner's word on the boot-timing risk (2026-11-22): "Fix it in code". **No lane bump.**

**Built.**
- **The seeding instant is the seed vote's cause.** The Review's seeding vote, which takes it
  from one to two of six, is now cause id `assembly.vote/0@<IC seconds>`
  (`concession::seed_cause`), the world's IC instant at the seeding. The cause list persists with
  the faction clock, so a restart keeps it. It is what anchored the votes before, so nothing new
  is stored and no state slot is added.
- **Vote `n` falls `n` IC months after it** (`concession::votes_due`, `seeded_at`), cause id
  `assembly.vote/<n>`. The tick votes once for each whole month since the seeding that the clock
  has not counted. Downtime is caught up from the seeding, and a cause already counted moves
  nothing.
- **What stays on the calendar's months:** the threshold news on Hold the Concession and the
  arm's monthly cap. Neither decides the operation, and the cap never binds at three posts.

**Tests.**
- `concession::tests`: the first vote of a world seeded 15 or 25 IC days before a boundary falls
  30 days after the seeding, with none at the boundary. Missed months are counted from the
  seeding. The instant is read back from the seed vote's cause, and neither a later vote nor a
  seizure is taken for it.
- `op8_campaign::empty_run`: the run is now `the_empty_run(ic_phase)`, booted on a boundary as
  before and also 15 and 25 IC days before one (`rig::composed_phased`,
  `driving_rig::stocked_phased`). Each run checks that the first vote falls within a day of 30 IC
  days after the seeding, and that the farm still reads short at the close.
- `domain_world::polities::op2_…`: the anchor cause is the seed's, at the boot's IC instant.

