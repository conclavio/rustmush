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

Not built.

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

Not built.

## KC.2

Not built.

## KC.3

Not built.

## KC.4

Not built.

## LW.1

Not built.

## LW.2

Not built.

## LW.3

Not built.

## LW.4

Not built.

## LW.5

Not built.

## OP.0

Not built.

## OP.1

Not built.

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
