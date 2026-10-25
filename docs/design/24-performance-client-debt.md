# Performance and client debt delivery

Owner scope (2026-10-24): complete the performance/client group, integration-test
on disposable `conclav.io` dev1, commit and push each accepted gate, then pause.
Sight caching is investigation only. No production cache or field-diff sheet
protocol is authorized by this delivery.

| Gate | Scope | Status |
|---|---|---|
| 1 | B-85 split-process health | Accepted on dev1 |
| 2 | T-99 SSH echo; history correlation/dedup; keyframe coordination | Accepted on dev1 |
| 3 | T-102 restart-safe reply framing and completion ordering | Accepted on dev1 |
| 4 | T-106 parallel pilot frames; T-107 sight-cache investigation | Functional acceptance passed; parallel remains opt-in |

## Gate 1: authoritative health

The server publishes its world iteration timestamp, loop lag and persistence
health over IPC once per second. The gateway retains the source timestamp:
buffered samples cannot make a stalled world look fresh. Startup, a sample older
than five seconds, a future timestamp, unavailable persistence, broken integrity,
or abandoned supervision yields `ok: false` and HTTP 503. Healthy responses use
HTTP 200. The response adds `world_ok` and `world_max_age_s`; the existing fields
remain available. A disconnected boundary clears the stamp, and a replacement
server must establish its own health.

IPC version 8 requires deploying gateway and server together. This health feed
is ephemeral; it creates no Redis records and does not count mirrored observations
as gateway world-loop histogram measurements.

Validation: 317 configuration, IPC, gateway and web unit/integration tests passed,
including real split-process restart and persistence tests. Both TLS release
executables built successfully. The live dev1 gate checked a nonzero authoritative
stamp, SIGSTOP causing HTTP 503 with a live gateway, SIGCONT recovery, SIGKILL
causing an unhealthy interval, and replacement-server recovery under the same
gateway PID. Dev2's gateway PID did not change. The first harness invocation raced
listener startup; the harness now tolerates connection refusal while polling,
and the rerun passed all seven assertions.

Evidence: [live samples](evidence/performance-client-2026-10-24/health-live.json),
[test log](evidence/performance-client-2026-10-24/health-tests.log),
[release build](evidence/performance-client-2026-10-24/health-release.log).
Reproduce with `scripts/client-health-live.py` on dev1 after deploying both peers.
The script changes no configuration or database contents and resumes a stopped
child even on failure.

Accepted release SHA-256:

- server: `61a7cc0ff3622d4c1ba8490e36bcf5a74bb259962539d0c80f20533dbcf74306`
- gateway: `15497cc1aafb327481e1bc67bffe3153bf2fc56de7b69107d0ab80eebd05caf9`

## Gate 2: SSH and client synchronization

PTY ECHO=0 now suppresses server-side draft echo, including asynchronous redraws;
prompt masking overrides it temporarily, and normal prompts restore the negotiated
mode. Production SSH receives the gateway's envelope sequence without taking on
web receipt-ACK retention. History requests have distinct connection-local ids,
mode generations reject obsolete queued requests, and the TUI deduplicates live
narratives against its priming/history pages. A wholly duplicate priming page still
advances the cursor. The clear/backlog invariant is explicitly asserted.

Sheet queries invalidate the next pushed baseline through the shared
`SheetView::query_keyframe` helper, used by both generic and toy query handlers.
The next push is a keyframe; subsequent unchanged-schema pushes remain full-data
deltas. Both clients replace the full data map, so omitted staff fields disappear.
No field-diff protocol was introduced. Dev1's game composition has no legacy
`VitalsSource` sheet provider: the server query/mutation integration is exercised
by the toy ruleset tests on the VPS, not claimed as a playable live sheet flow.

Live acceptance also exposed a transport shutdown defect: disconnecting the whole
SSH connection immediately could discard queued farewell bytes. The pump now sends
exit status, EOF and channel close in order, and disconnect cleanup runs once.
Real OpenSSH now receives Goodbye and exits 0.

Validation on dev1:

- The broad client/server run exercised 899 tests: 896 passed initially and three
  failed under concurrent build load. Focused reruns passed. The persistence
  probes now retry only pre-admission Busy within a deadline, and the observer
  board test waits for the arena's acknowledged exit. Both complete repaired
  suites passed (64 tests). The split arena story also accepts an already
  classified first observation while still requiring the classified contact.
- SSH library and real loopback regressions: 345 passed, including ECHO and farewell.
- Web sheet/game stores: 102 passed. Strict affected-crate Clippy passed; the
  final SSH shutdown change also passed its own all-target Clippy.
- The real front door passed ECHO=0/1, one-copy history, TUI entry/exit followed by
  history, and graceful QUIT on both sessions. The TLS release built successfully.
- All 20 changed Rust/web source files in this commit were matched by normalized
  SHA-256 against the tested VPS tree. Earlier gate source remains the deployed base.

Evidence: [live SSH](evidence/performance-client-2026-10-24/ssh-live.json),
[broad run](evidence/performance-client-2026-10-24/client-final-tests.log),
[repaired scenarios](evidence/performance-client-2026-10-24/client-scenario-repaired.log),
[SSH regressions](evidence/performance-client-2026-10-24/ssh-close-tests.log),
[web tests](evidence/performance-client-2026-10-24/client-web-tests.log),
[Clippy](evidence/performance-client-2026-10-24/client-clippy.log),
[SSH Clippy](evidence/performance-client-2026-10-24/ssh-close-clippy.log),
[release](evidence/performance-client-2026-10-24/client-final-release.log),
[source manifest](evidence/performance-client-2026-10-24/client-source-manifest.json).
Reproduce front-door acceptance with `scripts/client-ssh-live.py` on dev1.

Accepted release SHA-256:

- server: `24c27a40dc0da32de6aab533224b7b7f45c05963f06b175f4dd07adc7814c029`
- gateway: `677b8c9a211d72fd3af9d3af6d5fca8d9721840ee73fd98a2299a1554f99f1d0`

## Gate 3: connection reply completion

The gateway now retains `OUTPUTPREFIX`/`OUTPUTSUFFIX` for the physical connection
and restores them before reattachment completes. The command engine still validates
settings through the ordinary command/input-context path. IPC version 9 requires
upgrading both peers together. Standalone embedded Engine callers retain the older
engine-owned framing; deployed world loops explicitly delegate it to the gateway.

One connection serializes typed replies. Its prefix is the old setting and its
suffix is the newly accepted setting, preserving the setter/clear semantics.
Completion follows the input-custody release handshake, exact world delivery and
the sequential staff executor. `@dump` waits for the store's acknowledgment.
Malformed or locally refused actions do not wait for nonexistent server work.
A stalled completion produces an explicit interruption; an unresponsive live
producer closes that connection so late output cannot nest inside its next reply.
The completion wait uses the existing bounded five-second barrier budget. The
staff fence is conservative: it also waits for earlier work in the shared staff
executor. Ordinary delivery barriers stay independent so staff work can call back
into the world without deadlocking itself.

SSH-local history, TUI transitions, pixel and sound commands share the connection
reply lock. Ordered transport writes and pump fences keep their bodies inside the
pair. TUI coalescing preserves raw delimiter boundaries. QUIT carries its suffix
after Goodbye in the final event, before channel close. Asynchronous game events
can still arrive between delimiters; continuous pixel-control traffic remains
unframed. These delimiters identify command completion, not an isolated event feed.

The split restart regression initially exposed a custody race: the old barrier
could run before the command's release round trip. The repair waits for the
connection's existing custody entries, atomically queues release and disposition
output, then sends the completion fence. A direct IPC regression pins that order.

Validation on dev1: 738 affected core/protocol/session/server library tests passed
(two ignored); 347 SSH library/real-loopback tests; 83 gateway library and split
restart/TUI/SSH/web tests; 15 wire/parity tests; and 49 persistence/in-process tests.
The final local-refusal and gateway reruns passed (16 and 71 tests respectively).
Strict affected-crate Clippy and the TLS release build passed. All 26 changed
source/script files match the tested VPS tree by normalized SHA-256.

The live front door passed all six acceptance groups: world/staff/store replies,
SSH-local history/pixel/sounds/TUI replies, three pipelined commands, same-connection
SIGKILL/relaunch with retained delimiters, clearing settings, and QUIT/farewell/
suffix before exit status 0. The gateway PID stayed 1408489 across the child restart.
The bounded delayed-commit and held-BGSAVE test also proves that a second command
cannot nest its markers and that an ordinary world barrier remains responsive.

Evidence: [live acceptance](evidence/performance-client-2026-10-24/reply-live.json),
[SSH](evidence/performance-client-2026-10-24/reply-ssh-tests.log),
[split regressions](evidence/performance-client-2026-10-24/reply-split-tests.log),
[core tests](evidence/performance-client-2026-10-24/reply-core-tests.log),
[persistence](evidence/performance-client-2026-10-24/reply-inprocess-tests.log),
[final refusals](evidence/performance-client-2026-10-24/reply-refusals-final.log),
[wire tests](evidence/performance-client-2026-10-24/reply-wire-final.log),
[Clippy](evidence/performance-client-2026-10-24/reply-clippy-final.log),
[final gateway Clippy](evidence/performance-client-2026-10-24/reply-gateway-clippy.log),
[release](evidence/performance-client-2026-10-24/reply-release-accepted.log),
[source manifest](evidence/performance-client-2026-10-24/reply-source-manifest.json).
Reproduce with `scripts/client-replies-live.py` on dev1.

Accepted release SHA-256:

- server: `02857922144e7e4e1244453283df60128e740b3ca9aa4a13aa121aad51fd2b2b`
- gateway: `323018f4416cb0b19db8dad962befa14213d0cdb70c14e8cf34f9286cc8b318f`

CI lookback: gate 1 run 40140644435 failed only the pre-admission `fence create:
Busy` fixture in `m5_module_persistence_round_trip_in_memory`. Gate 2 (`bcf27b59`)
already repaired that fixture; the full persistence suite is green on dev1.
Both earlier CodeQL runs passed. Gate 2 and gate 3 main CI subsequently failed;
the repairs and reruns are recorded below.

### CI acceptance repairs during gate 4

Gate 2 CI (`40143307286`) exposed one new SSH lock `expect` against the unchanged
panic budget and a `sim_units` fixture retry that left its sentinel unread. Gate 3
added a second lock site. Both new lock sites now fail closed; the fixture drains each attempt's
sentinel before retrying a pre-admission refusal. The observer suite had the
same retry bug in its replay assertion and now drains its sentinel as well.
Both full-world fixtures now hold a two-server concurrency permit: eight
four-thread worlds at once starved a five-second reply deadline; the affected
timeout cases passed in isolation. The
production deadline remains unchanged.

Gate 3's process-cut CI (`40146150173`) exposed a real transport-progress defect:
waiting for reply completion inside russh's data callback prevented that same
SSH event loop from flushing the accepted-action notice. The fault fixture could
not see the notice to release its crash latch, so the five-second completion
budget expired first. Connected typed commands now run on one bounded 64-line
worker per physical connection. The callback only enqueues; the worker handles
both world and local commands in order using a transport Handle. It is aborted
on connection teardown. Queue overflow disconnects rather than blocking the SSH
loop. Local replies drain previously queued output even when no delimiters are
set. The regression requires progress to reach a real SSH client before the test
releases a held command, then proves the queued local/world answer order. The
completed run also exposed panic-relaunch and observer catch-up failures while
waiting for SSH output; those suites are included in final acceptance. A full
observer rerun exposed two additional five-second timeouts that passed alone,
and the replay sentinel bug that reproduced alone. The two-world permit and
sentinel repair address those fixture defects without relaxing production limits.

The repaired SSH library and loopback suites pass all 348 tests (315 + 33); the
simulator-unit suite passes all eight. All three real-Redis authoritative
process-cut tests pass against a separate disposable Redis on port 6798, which
was shut down afterward. The live dev1 Redis on 6788 was not used for these
destructive fixtures. All five panic-relaunch tests also pass, including survival
of the active SSH session across a faulted world. The panic-budget suite passes
all eight tests without raising the budget. The final observer suite passes all
32 tests, and strict SSH/gateway/TacSim plus changed-fixture Clippy passes.

## Gate 4: pilot frames and sight investigation

Pilot frame construction reads one immutable arena state through a bounded,
process-wide reusable worker pool. Notice/catch-up consumption, change detection,
and publication remain serial and sorted by pilot id. The library defaults to
one lane. Fresh and resumed hosts use `RUSTMUSH_FRAME_WORKERS` only when explicitly
set; invalid or absent values select serial execution. The host clamps it to
available CPUs and a requested budget of four; construction uses at most three
lanes including the caller, with at least four pilots per lane on average. This
admits the shipped twelve-seat lobby. The pool has at most three worker threads
shared by all arenas. Short cohorts stay serial and retain the original
single-allocation construction path. Pool creation failure
falls back to serial execution; a scoped worker panic propagates to the existing
arena panic boundary. The policy is absent from hashes, saved snapshots, and draw
labels. Rayon provides the safe borrowed scope and reusable workers; no arena
state is cloned or handed to an unscoped job.

The first experiment created scoped OS threads each pulse. Five interleaved
serial/parallel rounds on the eight-vCPU VPS rejected that version: median wake
p50 regressed 10.8% at CI reach and 4.2% at authored reach. Reducing the worker
budget to two also regressed. That result motivated reusing workers before
acceptance. All logical counters, final state hashes and snapshot checks matched
through both experiments. The 24-pilot cohorts use three lanes when the requested
budget is four, including the caller.

The reusable pool also failed the performance acceptance gate. Five interleaved
rounds produced the medians below (milliseconds; negative changes are faster).
All deterministic fields and final hashes match. No timing baseline was blessed.

| Metric | CI serial | CI parallel | Authored serial | Authored parallel |
|---|---:|---:|---:|---:|
| Wake p50 | 10.317 | 11.467 (+11.2%) | 14.357 | 14.582 (+1.6%) |
| Wake p99 | 14.193 | 15.060 (+6.1%) | 18.784 | 18.522 (-1.4%) |
| Window p50 | 20.718 | 23.320 (+12.6%) | 29.003 | 29.472 (+1.6%) |
| Window p99 | 25.549 | 28.342 (+10.9%) | 35.312 | 34.932 (-1.1%) |

**Rollout decision:** ship the bounded implementation as opt-in; keep production
serial. Automatic activation is deferred until a representative cohort shows a
repeatable gain. This corrects T-106's original unmeasured claim that it pays on
the eight-vCPU VPS. The two cohorts also exceed the existing 22/32 ms window
budgets even in serial mode; those ceilings stay unchanged. Reproduce the A/B
with `scripts/measure-pilot-frames.py` and an optimized harness binary. The
harness knob is `TACSIM_FRAME_WORKERS`; its default is now serial as well.

Validation covers the original 1,623-test TacSim library/integration run, then
142 integration tests after switching to the reusable pool. The final cutoff
passes exact serial/parallel parity for 1, 4, 8, 12 and 24 pilots and all five
arena-library contracts. Two pre-existing ignored tests remain ignored in the
broad run; performance measurements are report-only, with no baseline blessing.

### Sight investigation (no production cache)

`scripts/investigate-sight.py` instruments only an explicitly marked scratch tree
outside Git. Every sight is still computed normally. Two serial 300-unit harness
runs, including their warm-up and 30-second measurement window (166 substeps),
recorded the following on dev1:

| Measurement | CI reach | Authored reach |
|---|---:|---:|
| Sight calls | 73,791 | 391,727 |
| Repeated canonical endpoint/height keys | 11,585 (15.7%) | 84,620 (21.6%) |
| Repeated directed keys | 6,697 (9.1%) | 55,536 (14.2%) |
| Differing answers for repeated keys | 0 | 0 |
| Cell probes on repeated calls | 39,668 / 365,856 (10.8%) | 524,518 / 2,638,296 (19.9%) |
| Horizon probes on repeated calls | 48,012 / 371,682 (12.9%) | 490,510 / 2,397,610 (20.5%) |
| Peak unique keys in a substep | 477 | 2,342 |
| Key + answer payload at that peak | 41 KiB | 201 KiB |

The payload estimate is 88 bytes per entry and excludes container/allocation
overhead. Sensor passes contributed 4,832 / 28,268 repeats; share passes contributed
6,753 / 56,352. No calls fell outside a substep in these cohorts. Directed and
canonical checks both found zero inconsistent answers, including probe counts.
Instrumented timings are invalid for performance claims. Both reports match the
uninstrumented serial run on every deterministic baseline field and final state
hash, and both snapshot hash checks pass.

Recommendation: keep the production cache deferred. Repetition is measurable but
concentrated in sharing; a lookup on every call and storage overhead may absorb
much of the saved work. A future opt-in prototype should compare actual trace time
with lookup cost on a quiet host before claiming a gain. A bounded per-substep map
could be tested with a 4,096-entry cap and compute-on-full fallback; these two
cohorts do not establish a universal capacity bound.

Any implementation needs the terrain identity/revision and both endpoint cells
and eye heights in its key, normalized by the sight path's canonical endpoint
rule. Keep ownership inside one arena and clear at every substep; invalidate on
terrain replacement or revision. Cache only the pure terrain sight result, never
observer contacts, classification, jamming, or the directional fire/cover trace.
The current terrain is immutable inside a step, and `Horizon::sight` already
canonicalizes its ends. That makes this narrower than caching an observer's view.

Two design decisions still precede production work: amend doc 16 section 3.2's
explicit no-cross-observer-cache rule, and distinguish logical probes charged from
physical probes performed. Replaying stored probe counts preserves today's
budgets and deterministic pins, but those counters would cease to mean actual
work. Returning zero cost on a hit changes existing accounting and baselines.
The investigation changes neither rule and returns no cached answer.

Evidence for the investigation and performance decision:
[reusable-pool comparison](evidence/performance-client-2026-10-24/frames-pool-comparison.json),
[initial scoped-thread comparison](evidence/performance-client-2026-10-24/frames-scoped-four-comparison.json),
[two-worker comparison](evidence/performance-client-2026-10-24/frames-scoped-two-comparison.json),
[all raw A/B reports and logs](evidence/performance-client-2026-10-24/frames-measurements.tar.gz),
[CI-reach sight probe](evidence/performance-client-2026-10-24/sight-ci-arena.sight-probe.json),
[authored-reach sight probe](evidence/performance-client-2026-10-24/sight-authored-arena-authored.sight-probe.json),
and [sight parity checks](evidence/performance-client-2026-10-24/sight-parity.json).
The final twelve-seat cutoff leaves the measured 24-pilot cohorts at the same
three lanes; parity coverage additionally includes 1, 4, 8, 12 and 24 pilots.

### Final dev1 acceptance and handoff

The twelve-seat live test passed through the real TLS gateway, authoritative
server, and Redis: all twelve pilots boarded opposite sides, powered up, steered,
and received fresh movement frames containing only their own cockpit callsign.
The real server had exactly three reusable frame-worker threads. Aborting the
match returned every pilot to their team room, with no active match or stranded
custody marker. The fixture uses distinct local source addresses to retain the
normal per-IP connection cap and honors the HTTP authentication rate limit.
Its first full run incorrectly required a new pushed frame for unchanged
stationary views; the accepted run changes every pilot's view before requiring
a fresh movement frame. Reproduce with `scripts/client-arena-live.mjs` using the
fixture-only `ws@8` dependency installed under dev1's existing `r1-tools`.

Both real SSH scripts passed again on this release: all six reply-completion
groups, a real child SIGKILL/relaunch with the same connection and delimiters,
PTY ECHO=0/1, history/TUI ownership, and clean farewell/suffix draining on QUIT.
The temporary `RUSTMUSH_FRAME_WORKERS=4` systemd drop-in was then removed and dev1
restarted with the **serial default**. HTTP health returned 200 with a fresh
authoritative timestamp; dev2's gateway PID remained 539467. No temporary Redis
test process remains. Sixteen changed source/fixture files match dev1 by
normalized SHA-256. Two concurrent documentation-only commits on main were
incorporated after acceptance; they change no runtime source.

Accepted release SHA-256:

- server: `91722cf42c774d562a71e265621f8fbef2212c27aa30ad471212cc163603fe0e`
- gateway: `54bd962dcd83cbb0c8fd576931ed9da55fb6e9f6f82ec93b1544d41803be552c`

| Check | Evidence |
|---|---|
| Twelve real pilots, parallel workers, private frames, custody return | [Arena live](evidence/performance-client-2026-10-24/arena-live.json) |
| Live SSH framing/restart and echo/history/TUI | [Replies](evidence/performance-client-2026-10-24/client-ci-reply-live.json), [SSH](evidence/performance-client-2026-10-24/client-ci-ssh-live.json) |
| 348 SSH tests and eight simulator-unit tests | [SSH tests](evidence/performance-client-2026-10-24/client-ci-ssh-tests.log), [simulator tests](evidence/performance-client-2026-10-24/client-ci-sim-units-final.log) |
| Three Redis process-cut, five panic-relaunch, 32 observer tests | [Process cuts](evidence/performance-client-2026-10-24/client-ci-process-kill.log), [relaunch](evidence/performance-client-2026-10-24/client-ci-panic-relaunch.log), [observer](evidence/performance-client-2026-10-24/client-ci-observer-final.log) |
| Unchanged panic budget; strict production and fixture lint | [Panic budget](evidence/performance-client-2026-10-24/client-ci-panic-budget.log), [production Clippy](evidence/performance-client-2026-10-24/client-ci-clippy.log), [fixture Clippy](evidence/performance-client-2026-10-24/client-ci-fixtures-clippy.log), [final TacSim Clippy](evidence/performance-client-2026-10-24/frames-clippy-accepted.log) |
| Broad TacSim run, reusable-pool integration, final parity/contracts | [Broad tests](evidence/performance-client-2026-10-24/frames-tests.log), [pool integration](evidence/performance-client-2026-10-24/frames-pool-integration.log), [parity](evidence/performance-client-2026-10-24/frames-parity-final.log), [contracts](evidence/performance-client-2026-10-24/frames-contract-accepted.log) |
| Final TLS release, source identity, restored health | [Build](evidence/performance-client-2026-10-24/frames-release-final.log), [source manifest](evidence/performance-client-2026-10-24/frames-source-manifest.json), [health](evidence/performance-client-2026-10-24/client-final-health.json) |

The authorized performance/client implementation and sight investigation are
delivered. Automatic parallel-frame activation remains deferred by the measured
performance result; a production sight cache remains a separate, unimplemented
follow-up. The next GitHub CI run is still required to confirm the final push on
its runners; the failures identified in gates 2 and 3 have passing local/dev1
regressions recorded above.


### CI follow-up: rematch reply framing

The `test (rest)` job in GitHub run 40151828391 (baseline `8d4abdac`)
failed `a_restart_mid_match_through_the_lobby_resumes_and_the_pilots_board_again`:
`easting_m` received only `Huh?` instead of a complete `STATUS` reply.
`sim_rematch` retained the old whole-reply helper after the corresponding
`sim_units` and `observer_rule` repairs. On a transient refusal it retried
without draining that attempt's sentinel; a delayed refusal could then finish
the next attempt before its real reply arrived.

The helper now consumes its sentinel before deciding whether to retry, and
recognizes both pending-commit and control-state transients. Position and restart
assertions remain unchanged. Both real-SSH rematch integration tests passed on
the dev1 host (`cargo test -p mushd --test sim_rematch -- --test-threads=2`),
including the reported restart case: 2 passed in 51.42 s. Targeted Rust formatting
and `git diff --check` passed. [Test log](evidence/performance-client-2026-10-24/client-ci-sim-rematch.log).
The subsequent GitHub run still has to confirm the repair on its runners.


### CI follow-up during trade acceptance (2026-10-25)

GitHub run 40155411049 passed `sim_rematch` after `e5599e5e`, but failed two
other targets. The arena story read `Locked on` from an asynchronous notice next
to older weapon rows, then required the new range band immediately. It now waits
for the weapon row's `band 1 of 2` before checking the same lock and hit chance;
its whole-answer helper also drains each sentinel before transient retries.

The panic/relaunch test exposed a gateway race: the IPC reader marked its request
table down and woke failed reply fences before the supervisor published its
restart gap. A waking reply could therefore close the player transport. Restart
state now includes the current boundary's down marker; opening the explicit gap
publishes its flag before clearing the pointer. Repetition caught an earlier
window too: the failed world fence can return before IPC EOF. The final repair
captures the supervised boundary generation before the command and waits at
most two seconds for recovery after a failed fence, keeping the current reply
bracket open meanwhile. A generation change recognizes even a restart that
already completed. The reply remains explicitly interrupted; a live producer
that never enters recovery still closes after the bounded wait.

The link helpers have default in-process behavior and add no IPC wire frame or
version change. Deterministic tests cover reader-down before supervisor handoff,
a failure before EOF, a completed fast restart and a producer that stays live.
The earlier partial repair's second repetition failed, so it is not the accepted
result. Final validation and the live gateway result are recorded below.


Final validation passed: **74 gateway unit tests**, **5 real-process
panic/relaunch tests**, ten additional consecutive repetitions of the exact
flaky case, and the real-SSH arena story (92.57 s). Strict gateway Clippy and
Rust formatting passed. The final TLS gateway was deployed on dev1; the existing
reply smoke passed all six groups, including a real server SIGKILL/relaunch
while retaining the same SSH connection and output delimiters. Its helper was
pointed at the isolated acceptance world's staff credentials and evidence
folder; no credentials are included here. The post-smoke trade audit still
reconciles every account and good at rate 24. Dev2's gateway PID remains 539467.

Accepted gateway SHA-256:
`a593b8afe5e1b0e9d5cd02934a6be5df48cafa5cfe871902dfead90da92aa141`.
The game server retained the already accepted trade release; no live lane or
clock migration was performed during this gateway repair.

Evidence: [gateway regression](evidence/performance-client-2026-10-25/gateway-recovery-final.log),
[ten repetitions](evidence/performance-client-2026-10-25/gateway-recovery-repeat.log),
[arena story](evidence/performance-client-2026-10-25/arena-story-repair.log),
[live SSH](evidence/performance-client-2026-10-25/gateway-recovery-live.json),
[final trade audit](evidence/performance-client-2026-10-25/final-handoff-audit.json),
[Clippy](evidence/performance-client-2026-10-25/gateway-recovery-clippy.log),
[release build](evidence/performance-client-2026-10-25/gateway-recovery-release.log),
[source manifest](evidence/performance-client-2026-10-25/source-manifest.json).
The next push starts a fresh GitHub run; these passing results do not claim that
new run is already green.
