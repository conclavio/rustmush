# Field Pilot acceptance for M6

This is the independent Picture/Pilot/Doctrine port required by
[17d M6.10](17d-unit-minds-through-m6-plan.md#m610-acceptance-and-documentation).
It is acceptance content, not Registry Doctrine and not a replacement for Standard.
The external `mush-bot` executable, planner, memory and SSH loop remain independent.
Only the development test rig starts the production server under test.

## Source and public inputs

The source is `crates/tacsim/content/minds/acceptance/field_pilot.rhai`; its adjacent
envelope supplies three custom profiles, Hold, Orbit and Close. The manifest records
exact LF source bytes, SHA-256, function and flat-memory-key inventories. Hold uses
the external pilot's shipped knobs: 450 m, opening wait, follow enabled and
reapproach disabled. The operator still selects the declared Class II, not a
6000-operations think: it has 3000 operations including fees every 500 ms.

The independent Rhai body reads only the public own-hull view, published contacts,
notices and authored map. It chooses fresh classified hostiles, keeps a lock until
a challenger is 100 m nearer, stops briefly for unidentified plots, arms carried
guns through the native table, and flies the hold band's 50 m slack with 25 m
starting hysteresis. It remembers two public target cells for a bounded course,
turns toward the target's rear in automatic orbit when that course exists, and
uses the twenty-second direction change only when a course is unavailable.
Own damage, one-track steering, held-ground detours, opening room/cover, halted
fire, broken-target follow/return, lost-target hunt and the public B search are
separate branches. B is approached within 100 m, then the pilot travels 800 m
along the starting line through B and returns. An active Hold objective takes
precedence over the no-hostile search.

Two public-map reads close gaps in the existing M6 picture API, within API 4:

- `terrain_at([x,y])` returns `[kind, tracked_passable]`, or `()` outside the map,
  without an own cell, or beyond 320 m Chebyshev distance from the own cell.
  `kind` is the existing public terrain label. `tracked_passable` is the authored
  tracked movement row, not occupancy, slope clearance, entry-zone permission,
  or the capability of a damaged hull. Every attempt costs four operations;
  at most sixteen attempts per think, including unavailable/malformed reads.
- `spot_cell(letter)` returns an already-public surveyed marker's cell or `()`.
  The fixed 22-letter table and authored order are exactly those in `arena.map`.
  Missing/invalid labels return the same value; every lookup costs four operations.
  It reads no entity or visibility state.

A unit test compares every spot lookup with the actual cockpit public map payload.
The terrain tests cover boundaries, negative/out-of-range cells, unavailable own
position, fee/cap/reset behavior and independence from contacts or hardware.

This port is not a transcript or state-hash twin. Embedded thinking uses the fixed
Class II cadence and native conditional-fire authority instead of SSH polling,
acknowledgement and fire-lattice housekeeping. Contact velocity is derived from
published cell plots rather than a terminal's more detailed track sentence.
Opening cover/room is sampled through bounded local terrain reads rather than
reading a full VIEW BRIEF ray table. The optional reapproach branch is a bounded
approximation and is disabled in all acceptance profiles; it is not evidence of
full optional-knob equivalence. Behavioral outcomes are measured independently,
with these differences visible, rather than inferred from a common implementation.

## Gates and retained evidence

`minds::acceptance` uses the actual bounded Rhai runner and production six-picture
checker. It asserts Class II limits, all three profiles, sixteen contact rows,
four odds rows, flat memory/array bounds, target/unknown admission, range-hold
hysteresis, public-course orbit, B search and unarmed/stale-picture behavior.
A structural manifest is not an operation measurement. The checked source must
pass these tests before the longer live run is meaningful.

`crates/mush-bot/tests/arena_mind_author_story.rs` boots the production composition
and uses a real signed public-key SSH `mind push|pull|check` session for every
source operation. It creates FieldPilot, replaces the exact editable envelope,
checks six real pictures (three unarmed), saves, selects, checks out, releases and
reads a trace. It saves a second edit while the first sortie is still running and
compares the complete persisted seal. It refuses a stale optimistic edit and
source access from another character on the same account, an unrelated observer,
an opposing human and staff. It reads the match report, runs thirty private MUSTER
games on the first revision and opens a private reference bench of the actual live
sortie (step/back/run/picture/trace/eval/close). A separate saved infinite-loop mind
must drop to captured ROM after real strikes. A final opposing-human match must
preserve the accepted personal-draft-to-Doctrine substitution. Source is never
accepted from a client-supplied validation flag.

`crates/mush-bot/tests/arena_mind_port_rate.rs` launches two real external bot
processes concurrently for fifteen unposed MUSTER matches on each side. Each has
an independent world, store, loopback listener, host key and scratch directory;
results are combined in red/blue order. Each side retains its 210-minute deadline,
so the combined external interval fits inside the six-hour CI job alongside the
author story and build. It retains raw bot reports/transcripts
and reads actual completed match records and frozen tape-header identities only
after play. These test assertions never feed hidden data to the bot. The configuration
is shared explicitly with the author story: 2000 ms startup, 600000 ms duration,
60000 ms boarding, normal native tempo, 1000 ms snapshots and unattended closure
disabled so releasing the hull does not terminate the author story.

After committing the exact tested source, with the Cargo build slot free:

```text
python scripts/mind_port_acceptance.py run <new-output-directory>
```

The script first verifies the bot's normal/build graph contains no other RustMUSH
package. It runs the focused picture tests, production author story, then the fresh
external interval; output directories are never overwritten. It requires matching
run UUID, Git revision, current port source SHA, scenario-content SHA, complete
frozen-ground SHA, configuration SHA, chassis, opponent class/profile/source and
startup/duration before comparing results. The embedded run records fifteen paired
seeds; the external live seeds are fresh independent samples, pooled 15+15 by side.
All thirty wins/losses/draws/timeouts stay in each denominator. Exact 95% Wilson
intervals are recomputed; overlapping intervals do not support a difference claim.
The tree must remain clean at the same Git revision before each child command and
the final comparison. Keep output outside the checkout or under ignored `target/`.
The story binds the exact retained proving-manifest bytes by SHA-256, and external
side totals must agree with all thirty actual completed match outcomes.

An existing completed invocation can be checked again with `compare` instead of
`run`. Its required artifacts are `acceptance-run.json`, `author/author-story.json`,
`author/proving-manifest.json` and `external/external-interval.json`; the result is
`comparison.json`. The source and manifest must still match the tested content.

## Evidence status

Preparation only. The actual metadata parser accepts the current source. Rustfmt,
Python syntax and patch application checks have run. No Class II operation result,
production author-story pass, thirty-game report or win-rate comparison is claimed
here until the real Cargo and live acceptance runs produce those artifacts.

The M6 source/API addition needs the normal API 4 inventory/ABI and compatibility
record. It changes no physics or live source by itself. The M7 economic-stability
checkpoint remains a separate required gate after M6 acceptance.
