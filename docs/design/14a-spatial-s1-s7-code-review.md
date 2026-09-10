# S1–S7 spatial implementation code review

Reviewed 2026-09-10 at `origin/main`, `d713d17fca51a9d74a7014fe12a84577fce522ee`.
The review covers the S1–S7 implementation from S1's parent `4bfb38d`, rather than
only the final merge's changes. The target contains doc 14 Draft 0.1; the later
design revisions in the separate design-review checkout are not in this commit.

**Assessment: changes requested.** Five focused regression tests reproduce the
defects below. No production code was changed. Reproductions are retained as
`review_*.rs` integration tests; their assertions express the expected behavior
and intentionally fail at the reviewed commit. They are active tests, so test
runs that include these targets remain failing until the defects are fixed.

## Findings

### 1. [P1] Tactical movement bypasses ordinary exit locks

Location: `crates/mush-atlas/src/window.rs:2104–2115`.

When stepping across a room boundary, the only access check is the destination's
atlas `Access::Restricted` value. The eventual `mush_sim::place` transition checks
structural containment but does not run exit lock evaluation or the normal
`MoveAttempt` gate. An ordinary player can therefore cross a locked doorway by
using cells instead of the exit command.

Reproduction: open a window over the starter Lobby, enter it as a player, and set
the Lobby-to-Breakroom exit's basic lock to false. `breakroom` correctly replies
`You can't go that way.` Walk to cell `(33, 13)`, then issue `+move east 8`:

```text
You move 8 cells east to (41, 13) in Breakroom (cost 8.00).
```

The player's actual room becomes Breakroom. Route boundary crossings through the
same live movement authorization used by narrative movement, including the
crossed portal's exit locks and applicable room/module gates.

Test: `review_tactical_move_must_respect_the_locked_breakroom_exit` in
`crates/mush-atlas/tests/review_window.rs`.

### 2. [P1] NPC interface replies broadcast private results to bystanders

Location: `crates/mush-npc/src/module.rs:1520–1526`.

The ability provider correctly reads with the asker's authority, but every
successful reply then goes through `say_as_npc`, which broadcasts to the room and
emits a public speech event. `request.status`, for example, returns the asker's
private request titles. The output step undoes the provider's privacy filtering.

Reproduction: Alice files a request titled `Confidential review of Alice's
character`; Bob cannot see that title through `+job 1`. With both in the Lobby,
complete the front desk NPC's `request.status` ability turn for Alice. Bob receives:

```text
FrontDesk says, "#1 Open — Confidential review of Alice's character"
```

Preserve the result's authorized audience through output delivery. Private
results should reach the asker without becoming public speech, room history, or
stimuli for other NPCs. Public answers can remain public when the provider
explicitly permits that audience.

Test: `review_interface_reply_must_keep_request_titles_private` in
`crates/mush-atlas/tests/review_window.rs`.

### 3. [P1] The text accompanying `+map` exposes hidden participants

Location: `crates/mush-atlas/src/map_cmd.rs:165–179`.

The text summary enumerates every participant in the domain and filters out only
the viewer. It prints names and coordinates without applying the room visibility
and surface filtering used by `frame_for`. This leaks occupants of restricted
rooms even when the structured map correctly hides them.

Reproduction: open a window spanning Lobby and the restricted Production Office;
place God in the office and an ordinary player in the Lobby. The player's map
frame contains only their own unit, while bare `+map` also prints:

```text
On the map: God (270, 224).
```

Build the text from the same permitted view as the frame. The summary's anchor
names also need that filtering: this reproduction names Production Office in the
header despite hiding its cells.

Test: `review_map_text_must_hide_private_participants` in
`crates/mush-atlas/tests/review_window.rs`.

### 4. [P2] Revoking staff privileges does not revoke live proposal application

Location: `crates/mush-worldgen/src/proposal.rs:1078–1080`.

`go` authorizes the proposal's original author, then chooses the live staff apply
path using `rec.staff`, a snapshot taken when the proposal was created. It never
requires that the applying actor still has staff privileges. A demoted builder
retains the ability to apply their previously prepared staff packs directly.

Reproduction: give Builder `WIZARD`, prepare a ready bakery proposal, remove
`WIZARD`, assert `is_staff` is now false, then run `+build/go 1`. It still creates
the bakery as an authored site and replies:

```text
Applied site.p1_a_bakery_on@1: 3 created, 0 updated, 0 retired; ticket 1 filed for the record.
```

Recheck the applying actor's current authority before entering the staff apply
path. Retain `rec.staff` as provenance rather than a lasting grant of authority.

Test: `review_a_demoted_author_must_not_apply_a_staff_proposal` in
`crates/mush-worldgen/tests/review_plan.rs`.

### 5. [P2] Ground engagements exclude NPC occupants

Location: `crates/mush-tactics/src/ground.rs:163–166`.

The ground driver's initial participant filter accepts `Player` and `Thing` but
omits the first-class `Npc` object kind. An NPC standing alongside a player when
the engagement opens gets no spatial placement, so participant enumeration and
the shared geometry queries cannot include it in the engagement.

Reproduction: place a player and an NPC in Backlot, then open
`@domain/create <backlot>=tactics.ground` and complete the composite. The player has
a pose; the NPC does not. Include actor kinds (`is_actor()`) alongside eligible
objects when collecting participants.

Test: `review_ground_window_must_include_npc_occupants` in
`crates/mush-tactics/tests/review_ground.rs`.

## Remaining room/space integration scope

The S7 implementation ledger explicitly defers parts of the unified design:

- The TACSIM seed latch remains separate from `mush-content`'s applied-pack latch.
- Corridor locations remain seed-created rooms; mapping them into atlas records
  awaits the non-surface neighborhood/site decision.
- Placement-graph carrier journeys remain alongside the atlas's character and
  room-model vehicle journeys (D-P-4's promised S7 fold is unfinished).
- Indirect-fire geometry is available as a query, but does not gate the pulse
  tick's `Fire`/`Launch` intents.

These are documented deferrals, distinct from the reproduced defects. Calling S7
landed does not mean room and space integration is complete. See the S7 ledger in
`14-spatial-implementation-plan.md:917–946` and decisions D-P-4/D-14-33.

## Validation

- Passed the existing suites with
  `cargo test --offline -p mush-content -p mush-atlas -p mush-worldgen -p mush-tactics -p ruleset-tacsim`
  before adding the review reproductions.
- Passed `cargo test --offline -p mush-npc --quiet` (138 tests).
- Passed `cargo test --offline -p mush-sim --features testkit --quiet` (296 tests).
  Without `testkit`,
  the existing `e7c8` test target does not compile because `claim_for_test` is
  feature-gated; that helper/call is not an S1–S7 regression.
- All five review regression tests fail at their expected behavior assertions.
  Their fixtures successfully reach the relevant command/completion paths.

Run the reproductions independently, since Cargo stops after the first failing
test target:

```powershell
cargo test --offline -p mush-atlas --test review_window -- --nocapture
cargo test --offline -p mush-worldgen --test review_plan -- --nocapture
cargo test --offline -p mush-tactics --test review_ground -- --nocapture
```

This was a code review with focused integration tests, not a full workspace test
run or a live-server/LLM playtest.
