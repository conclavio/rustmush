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

The story also reads the actual retained proving chunks and their hashes, opens
the first completed proving game through public BENCH commands, and reaches its
exact terminal cursor. This includes the stop, settlement and ledger acknowledgement
events sharing the final timestamp; stopping at the timestamp alone is insufficient.
It retains the reference hash and replay evidence alongside the author story.
The exact frozen start and result bytes are retained with their manifests. Every
candidate seal must name the first saved revision, its effective source, Class II
and the complete original Hold card and rules; every opponent must actually select
Line. The comparison script verifies those bytes and identities again, and
reconciles all thirty retained game outcomes and summed faults with the summary.

Frozen proving starts use format 2, with shared ground/source maps and one zlib
stream per exact game body. Both the complete artifact and each raw body remain
bounded at 16 MiB. The story and Python comparison independently enforce exact
stream completion, no trailing bytes, length/SHA and summary/body agreement before
reading seals. They preserve the canonical snapshot state string and all existing
identity/card/outcome assertions. Old private format 1 is explicitly refused;
there is no migration, and tape format 1/proving-result format 2 are unchanged.

`crates/mush-bot/tests/arena_mind_isolation.rs` exercises baseline and loaded
normal sorties and two proving jobs on the same production server through separate SSH accounts. It
checks bounded worker/queue admission, duplicate and overflow refusal, settled
compute progress for both jobs, continued live snapshots, and cancellation with
quota refund. Baseline and loaded STATUS latency samples are retained in
`live-isolation.log` as `M6_LIVE_PROVING_ISOLATION` JSON; host timing is report-only.
The rig waits for the first public cockpit frame before its baseline samples.
It then publicly concedes that normal baseline sortie and waits for the retained
terminal result before preparation. After both jobs are admitted it starts and
boards a fresh normal sortie on the same server, with the same configuration.
Both actual match IDs and the baseline terminal result are retained. The loaded
progress and compute anchors are taken after fresh boarding and both admissions,
so neither baseline nor preparation progress can satisfy the live-under-load
assertion. A missing loaded snapshot prints its retained terminal result and the
pilot transcript rather than hiding the teardown behind an Option failure. Each
preparation duration is retained separately from the baseline and loaded STATUS samples.
Preparation has a finite 600-second liveness guard per request and the complete
isolation case has a 1,500-second guard (two preparation guards plus 300 seconds
for the functional checks); explicit preparation refusals and closed channels
fail immediately with their reason. These guards are distinct from the retained timing
measurements, which remain report-only.
Its account quota covers the complete retained plan, including one final native
step per game: 100 games × 12 units × 600,250 ms = 12,005 unit-minutes. A faster
first worker cannot exhaust an artificially smaller quota while the second job
prepares. Worker/queue bounds, settled-charge fairness and cancellation refunds
remain asserted; this fixture setting does not change the production default.

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
package. It runs the focused picture tests, same-server isolation check, production
author story, then the fresh external interval; output directories are never
overwritten. Each external side saves its transcript, report, process output,
exit status and captured documents before validation. It requires matching
run UUID, Git revision, current port source SHA, scenario-content SHA, complete
frozen-ground SHA, configuration SHA, chassis, opponent class/profile/source and
startup/duration before comparing results. The embedded run records fifteen paired
seeds; the external live seeds are fresh independent samples, pooled 15+15 by side.
All thirty wins/losses/draws/timeouts stay in each denominator. Exact 95% Wilson
intervals are recomputed. Acceptance requires the Field Pilot's pooled point win
rate to lie inside the freshly measured external bot's 95% Wilson interval, as
doc 17 M6 specifies. Interval overlap is a separate reporting rule: overlapping
intervals do not support a difference claim, but do not replace that acceptance
criterion. Actual proving must also report zero mind faults; the current retained
fault metric counts both sides, so the comparison preserves that aggregate and
marks unavailable candidate/opponent subtotals as null rather than guessing.
Either behavioral failure returns a failing status after retaining the complete
`comparison.json`, including both intervals, all outcomes and refusal reasons.
The owner's report-only host-timing ruling does not waive either requirement.
The tree must remain clean at the same Git revision before each child command and
the final comparison. Keep output outside the checkout or under ignored `target/`.
The story binds the exact retained proving-manifest bytes by SHA-256, and external
side totals must agree with all thirty actual completed match outcomes.

An existing completed invocation can be checked again with `compare` instead of
`run`. Its required artifacts are `acceptance-run.json`, `author/author-story.json`,
`author/proving-manifest.json`, `author/proving-start-manifest.json`,
`author/proving-start.json`, `author/proving-result.json` and
`external/external-interval.json`; the result is
`comparison.json`. The source and manifest must still match the tested content.

## Evidence status

Candidate `34a22ac1` includes the port and production-runner checks. The seven
Field Pilot tests pass in the fresh Linux library gate; final long acceptance is
still pending. Validator correction `8ac8f159` initially passed twelve Python
tests and enforced the outcome/fault requirements above. The subsequent exact
artifact and per-game reconciliation corrections pass 21 Python tests. The production author story,
thirty-game report, win-rate comparison and new same-server isolation/proving
replay additions are not accepted until their actual runs produce the evidence.

The M6 source/API addition needs the normal API 4 inventory/ABI and compatibility
record. It changes no physics or live source by itself. The M7 economic-stability
checkpoint remains a separate required gate after M6 acceptance.

The later IC12 merge is `cad11cfc5fd831f6ef8aaff4b8799f8f144e7fc9`; final acceptance
must identify the committed tree containing its subsequent corrections. The Linux
correction run at `f734fcce` failed the same-server isolation setup because the
100-game 6v6 stock request exceeded the old frozen-start representation's 16 MiB
cap. That failed run is not passing acceptance evidence. After the initial fast
compression also exceeded the cap, default compression passes all four Windows
development codec/capacity tests: the real 100-game 6v6 artifact is 12,662,880 bytes
under the unchanged 16,777,216-byte cap. The independent validator now passes 29
Python tests, and the IC12 normal/build dependency graph confirms that `mush-bot`
still depends on no other RustMUSH package. The complete measurements are in 17b §6.
Final merged-source Linux verification, the complete author story, isolation and
external comparison remain pending; no final acceptance revision is claimed.

The first Windows live run after the capacity correction exposed two fixture
problems: `Good hunting` can precede the first cockpit frame, and the arena rig's
neutral module roster omitted the production TacSim-to-Roll publication grant.
The latter correctly refused publication after preparation completed. The rig now
waits for the public first frame and uses `server_modules_with_tacsim` with
`tacsim_module_wired`, preserving its existing module defaults and content choices.
A composition regression and a pre-boot assertion check the grant before expensive
work. The production authority check is unchanged. Candidate `44800557` passes
both the composition regression and the complete Windows live isolation case in
148.68 seconds, with preparations of 68.426 and 66.133 seconds and 7,089 ms of
fresh live progress after both admissions. The follow-up fixture quota correction
above still requires its actual run, and final Linux acceptance remains pending;
the earlier refusals are not acceptance passes. The author story also stops
immediately on explicit proving refusal or terminal failure, preserving its
existing finite completion guards and all retained-evidence checks.

The Linux invocation at `6375d6e6` passed both proving preparations (147.243 and
144.341 seconds) but failed isolation after 307.33 seconds because the original
live snapshot was absent after preparation. The log did not retain the terminal
match reason, so it does not establish timeout rather than another normal end.
The two-sortie correction above removes that preparation-lifetime dependency and
adds explicit teardown diagnostics without changing normal match behavior or
the live-under-load assertions. It remains pending actual execution; the failed
invocation did not reach the author story or external interval.

The observed 147.243/144.341-second preparations used most of the former
180-second guard. The generous finite 600/1,500-second guards above avoid treating
ordinary host timing variation as a performance gate. The failed Linux invocation
did not exceed its old preparation guard; the separate two-sortie correction
addresses its missing live snapshot. Actual preparation/STATUS timings, immediate
refusal and closed-channel failures, first-frame/STATUS guards, resource limits,
both-account settled progress, loaded live progress and cancellation refunds
remain unchanged. The smaller author-story 30-game 1v1 preparation keeps its
60-second guard. These fixture changes still require actual acceptance execution.

The author story now waits for its first ready public STATUS frame before reading
MIND. It retries only the explicit pending-panel response within the existing
20-second guard and refuses a destroyed hull; source, seal and privacy assertions
are unchanged. The lifecycle-remove protocol also rejects extra nested fields
with the same valid JSON shape and existing connection/edit authority. Its direct
and nested decoding regression, actual TypeScript export and final acceptance run
remain part of the next committed-source gate.

The Linux run at `0a2cfe71` passes the seven Field Pilot checks and both isolation
tests (328.09 seconds after compilation). Preparations take 143.953 and 141.826
seconds; the fresh loaded sortie advances 18,001 ms with settled progress for
both accounts and both artifacts below 16 MiB. The next author test fails after
6.86 seconds in fixture account setup: `@newchar Alt` was sent through the bot
transport, which correctly refuses account management commands. The fixture now
uses a narrow runner-owned public SSH character-creation helper with explicit
busy retries and success acknowledgement. Bot command restrictions are unchanged.
The correction still needs execution. This failed invocation reached neither the
signed author workflow nor the external interval and is not full acceptance.

Focused author diagnostics then exposed canonical-name assertions (`ac861084`)
and a live rule-sample clock mismatch (`5eb49643`). The latter is a real execution
bug: due events retain their scheduled time while the captured public picture
uses the substep boundary. Settlement incorrectly compared the sample's picture
time with the timer, counted valid samples as faults and dropped Field Pilot to
ROM. Admission now compares against the exact captured view, preserving program,
generation, landing and expiry checks. A regression covers three off-grid thinks
through actual collection, Rhai execution and landing, plus rejection of a sample
forged with the timer time. The live story also requires zero strikes and no ROM
drop. These corrections still require their actual test runs.

`EXIT` intentionally relinquishes side leadership. The two released sorties now
close through their retained participant's public `SIM ABORT`; their terminal
results and complete tapes remain the reference BENCH inputs. The third sortie
keeps `SIM CONCEDE` while seated. No production participant authority changes.
