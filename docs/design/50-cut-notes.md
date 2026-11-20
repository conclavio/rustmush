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

Not built.

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

Not built.

## LW.4

Not built.

## LW.5

Not built.

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

Not built.

## OP.3

Not built.

## OP.4

Not built.

## OP.5

Not built.

## OP.6

Not built.

## OP.7

Not built.

## OP.8

Not built.
