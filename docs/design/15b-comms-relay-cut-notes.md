# 15b — Relay backbone implementation evidence

## R1 — Registry and first seed (2026-10-23)

**Candidate:** `916e8fee`, based on `origin/main` at `7c886408`.
**Status:** implemented and deployed; targeted and live integration checks passed.
The owner subsequently authorized committing/pushing R1 and continuing with R2.
R1 reached main at `193001e1`; CI found stale boot goldens (28 packs/22 institutions
instead of 29/23). `7dc36d48` corrects both and pins the relay pack identity; the
corrected boot-parity test passes. Normal CI status is recorded below; no criterion
is waived. This section records Gate 1 of
[15a](15a-comms-relay-implementation-plan.md).

### What this cut implements

`mush_sim::relay` owns one versioned catalog at `mod:sim:relay:registry`.
It records stable pair/half/node identities, installed object bindings, explicit
room coverage, operator identity, capacity and link policy. Link state is derived:
both halves enabled/powered/staffed means Live; any unavailable half means Degraded;
a destroyed half means Dead. R1 exposes no destruction or replacement operation.

The production content pipeline installs `tacsim.relay.luna_ceres@1`: Malapert
Exchange, two nodes, one pair, two halves. The Luna endpoint occupies Aitken's service
spine; the Ceres endpoint occupies its maintenance room. These are real authored
places and persistent objects. The dedicated Malapert compound and remaining star
nodes remain R10 content. Coverage is 12 Aitken rooms and 13 Ceres rooms.
The seeded capacity of 8 units/IC-second is metadata; byte accounting and scheduling
arrive with routing.

`@relay/list|check|show|enable|disable|power|staff` is restricted to SIM_ADMIN or
Royalty. Mutations are durable-ack gated and repeated settings do not advance the
revision. Reboot preserves identities and availability; planning the unchanged
pack reports no work and leaves the catalog untouched. Movement,
ordinary destruction and removal of a referenced binding are refused. Power and
staffing are explicit operator settings, not a new crew simulation.

Bounds are enforced on load and installation: 32 nodes, 64 pairs, 64 covered rooms
per node and 128 KiB per document. Diagnostics walk this bounded catalog, not the
world. A boot-only scan detects surviving or extra hardware without matching records.
Missing operators/objects, wrong locations or binding schemas, duplicate identities,
invalid bounds and malformed documents stop startup with a repair diagnostic.
The stored evidence is kept; boot never silently replaces it.

The catalog remains on the bounded legacy document lane. Its command, namespace
delegations and org-allocator access are recorded in the repository contract files.
R2 must reassess this lane before any authoritative mechanics depend on the catalog.
No mail/channel/order/report producer or consumer was rerouted in R1. The all-mail-is-IC
ruling remains binding; its transport implementation belongs to R6a.

### Local verification

**262 passed, 0 failed, 0 ignored** on the corrected candidate:

| Target | Passed |
| --- | ---: |
| mush-server / relay_registry | 5 |
| mush-contract-tests / legacy_handler_allowlist | 4 |
| mush-contract-tests / module_ns_owners | 4 |
| mush-contract-tests / state_slot_owners | 5 |
| mush-sim / transport | 24 |
| mush-mail / mail | 27 |
| mush-comsys / comsys | 48 |
| mush-propagation / e7c7 | 88 |
| tacsim / content | 36 |
| tacsim / ports | 21 |

Reproduction:

```sh
cargo test -p mush-server --test relay_registry \
  -p mush-contract-tests --test legacy_handler_allowlist --test module_ns_owners --test state_slot_owners \
  -p mush-sim --test transport -p mush-mail --test mail -p mush-comsys --test comsys \
  -p tacsim --test content --test ports -p mush-propagation --test e7c7 -j 4
cargo clippy -p mush-sim -p tacsim --lib -j 4 -- -D warnings
```

Clippy passed. Changed Rust files passed rustfmt; the workspace-wide cargo-fmt wrapper
exceeded Windows' command-length limit, so rustfmt was run directly on the affected
source files. `git diff --check` and `node --check scripts/relay-r1-live.mjs` passed.
Full workspace CI was not run in this session; the table is the tested scope.
After tightening the unchanged-pack assertion, the five `relay_registry` tests
were rerun and passed. Only the test and live-runner assertions changed after the
deployed candidate; shipping code remained at `916e8fee`.

Tests found and fixed two defects before deployment:
- Content boot initially lacked the declared content-to-sim persistence delegation.
  The existing-world restart test caught objects surviving without their catalog.
  Boot now asserts a durable catalog before any staff command can mask a missing write.
- Organization ID 0 is valid. Older-port composition tests caught a validator that
  rejected it when the Exchange was the world's first organization.

### Real-server acceptance

Target: the owner's disposable `conclav.io:/opt/rustmush/dev1`, split gateway/server,
Redis 8 on 6788, game SSH on 6789 and HTTPS/WebSocket on 6790.
The previous source, binaries, configuration, Redis snapshot and data directory are
retained under `/opt/rustmush/dev1/r1-evidence`. Dev2 was not changed; its gateway PID
was 539467 before and after the gate.

The build uses the repository's release profile and deployment script, a freshly
built embedded web client and matching source stamp. The VPS's system Node 18 was
too old for the checked-in web toolchain; Node 22 was installed under
`/opt/rustmush/dev1/r1-tools` without changing the system Node installation.

The fresh instance uses an IC ratio of 24 and configured epoch `0987-04-20 06:00`.
R1 has no route due times, throughput scheduler or delivery ledger to measure.
Seed: `tacsim.relay.luna_ceres@1`, content digest
`59f32f5d013fc15188f5e50763084e371ce489d26302c446acdd26790a65a6d9`.

| Live check | Result and retained evidence |
| --- | --- |
| Upgrade the existing store | Passed: all 351 pre-existing object identities remain; two nodes, two halves and their real room/object bindings validate. Full service restart leaves the catalog identical. [existing-store.json](evidence/relay-r1-2026-10-23/existing-store.json) |
| Fresh bootstrap | Passed: the repository reset procedure cleared only dev1's store/data, preserved SSH host identity, created a staff account and booted the configured packs. VPS log: `r1-evidence/fresh-reset.log`. Subsequent checks use this fresh world. |
| TLS WebSocket commands | Passed: 33 commands cover both halves, no-op content plan, enable/disable, power, staffing, idempotency, staff authorization, hidden-name refusal parity and binding lifecycle guards. [exercise.json](evidence/relay-r1-2026-10-23/exercise.json) |
| Existing communication | Passed between two actual connected characters while the pair is disabled: room speech, page, channel, mail notice and readable mail body. This proves R1 does not reroute traffic; it does not claim the future mail-transit cut is implemented. Same exercise transcript. |
| Confirmed-write crash boundary | Passed: SIGKILL the server child after a confirmed disable at revision 16; the gateway starts a new child. Catalog identity, coverage and disabled state are exactly preserved, then five WebSocket commands verify and restore Live at revision 17. [crash-restart.json](evidence/relay-r1-2026-10-23/crash-restart.json), [verify.json](evidence/relay-r1-2026-10-23/verify.json), [before-restart.json](evidence/relay-r1-2026-10-23/before-restart.json) |
| Malformed or missing registry | Passed: startup refuses the malformed document and a missing document with surviving hardware; neither is silently reseeded. [negative-cases.json](evidence/relay-r1-2026-10-23/negative-cases.json) |
| Duplicate identity or contradictory binding | Passed: duplicate half ID is refused; swapping the two physical half objects is refused as a location/binding contradiction. The exact injected document remains stored in each case. Same negative-case evidence. |
| Real SSH client | Passed through `ssh -p 6789 play@conclav.io`, in-band login and `@relay/check`; output agrees with WebSocket and persisted state. [ssh.json](evidence/relay-r1-2026-10-23/ssh.json) |
| Final health | Gateway and Redis active, Live pair at revision 17, dev2 PID unchanged. [environment.json](evidence/relay-r1-2026-10-23/environment.json), [final-registry.json](evidence/relay-r1-2026-10-23/final-registry.json) |

Final fresh-world bindings: operator #22; Ceres node #664 in room #112, 13 covered
rooms; Malapert node #665 in room #213, 12 covered rooms; Luna half #666 and Ceres
half #667. These numeric IDs are this instance's allocations; canonical identities
are the stable contract.

The release server SHA-256 is
`1993391ad4ca9af6f570d662c908e55b5779a21d0bb2bd07733859a2c0ec838b`;
gateway SHA-256 is
`7f2f19b830cc6050ee2193570898389572214c6cd80aaf4d884da10f0bbad566`.
Both are built from `916e8feec5d87e27a4be869c6fcdde7ebc98f1ee`.

### Live reproduction and retained artifacts

These commands are for the owner's **disposable dev1 instance only**. The upgrade
and fresh-reset procedures are destructive; their pre-R1 backups remain under
`/opt/rustmush/dev1/r1-evidence`. Credentials remain there in a root-only file and
are not committed. The JSON evidence in this repository contains no password or
authentication token.

The local production-composition test is the Cargo command above. On the VPS,
the final client run used the committed `scripts/relay-r1-live.mjs`, copied to
`/opt/rustmush/dev1/relay-r1-live.mjs`:

```sh
export PATH=/opt/rustmush/dev1/r1-tools/node_modules/node/bin:$PATH
export NODE_EXTRA_CA_CERTS=/opt/rustmush/dev1/tls/cert.pem
node /opt/rustmush/dev1/relay-r1-live.mjs exercise https://conclav.io:6790 \
  /opt/rustmush/dev1/r1-evidence/credentials.json /opt/rustmush/dev1/r1-evidence 6788
# Kill the mush-server child belonging to dev1mush-gateway; await its replacement.
node /opt/rustmush/dev1/relay-r1-live.mjs verify https://conclav.io:6790 \
  /opt/rustmush/dev1/r1-evidence/credentials.json /opt/rustmush/dev1/r1-evidence 6788
python3 /opt/rustmush/dev1/r1-negative.py
python3 /opt/rustmush/dev1/r1-ssh.py
```

The exact child-PID selection, executable-path assertion, SIGKILL and restart wait
are retained in `/opt/rustmush/dev1/r1-finish.sh`. It runs the sequence above and
ends with `R1_LIVE_GATE_PASSED` in `r1-finish.log`. Other retained procedures are
`r1-build.sh`, `r1-upgrade.sh`, `r1-existing.py`, `r1-fresh.py` and `r1-metadata.py`.
Build and upgrade logs are `r1-build.log` and `r1-evidence/upgrade.log`.

Before recording the final results, three test-harness assertions were corrected:
the unchanged-pack check now verifies a valid no-op **plan**; bad-record diagnostics
use a journal cursor per attempt; and SSH comparison normalizes terminal line
wrapping. The final evidence above comes from rerunning those corrected checks.

### Stop and outstanding work

R1's implementation and real-server checks are complete. The owner authorized
the push and R2 continuation. Main's graphics fixes through `5ab00035` were merged
before the push; R2's implementation and persistence reassessment follow below.
The first CI run's stale goldens were corrected, rather than treating the failed
job as accepted.

## R2 — Stationary routing, durable legs and FIFO (2026-10-23)

**Base:** R1 plus current main, `193001e1`; R1 CI correction `7dc36d48`.
**Deployed code candidate:** `1182c36f3c890aa9b0a06fd84c659e73f56b03a2`.
Subsequent changes add test automation, CI inventory and evidence only.
**Status:** implementation, focused checks and the deployed dev1 live gate pass.
Full §5 acceptance remains pending normal candidate CI. The initial stop before
R5 was subsequently released by the owner after the corrective dev1 rerun below.

### Implemented boundary and ownership

The existing `mush-propagation` delivery pass is the only scheduler.
`Envelope.kind` distinguishes orders, reports, ACKs and the reserved mail/channel
kinds. Old envelopes infer their kind from their existing references; old queued
traffic starts at its last recorded relay, preserving the existing hop history.
Only orders supersede standing orders. Reports still enter the existing tactics
contact-sharing consumer after arrival, including when their ACK arrives first.
No mail or channel producer is switched in this cut.

| Record | Owner, writer and reader | Compatibility and failure behavior |
| --- | --- | --- |
| `mod:sim:relay:registry` | Existing sim catalog; content/bootstrap and staff availability commands write; propagation reads one validated snapshot | R1 schema unchanged. Binding contradictions still refuse boot. |
| `mod:propagation:envelope:<id>` | Existing propagation owner; order/report/ACK commands and delivery pass write; party queries, standing-order and report consumers read | Adds typed kind and optional version-1 transit. Legacy documents decode. Invalid route cursors, bounds, arithmetic or clock identities are held with diagnostics, not rewritten. |
| `mod:propagation:relay_transport` | Propagation command admission and delivery pass write; the pass consumes shared capacity | Version 1, at most 64 pair buckets. Malformed state or missing state beside routed envelopes refuses startup; capacity cannot silently reset on reboot. |
| Existing propagation allocator | Existing owner and commands; delivery tick now also persists scan-cursor movement | Preserves bounded scan progress even on a fire that advances no envelope. |

Transit stores the current committed position, destination, remaining legs, queue
ticket, frozen flight, completed physical history and work counters. Capacity and
envelope post-images share the same store batch. The bounded legacy persistence
lane is retained after reviewing the production world task's exact receipt fence:
dirty output, subsequent world commands and effects cannot pass the pending write;
write failure stops the world. Admission explicitly uses the durable-ack gate.
This cut does not claim a migration of propagation into the separate authoritative
bundle framework. Real process death after the confirmed queue/flight writes
demonstrates recovery through the existing boundary.

TacSim injects the stationary metric through `MetricProvider`; propagation gains
no dependency on TacSim or messaging modules. The adapter maps authored port rooms
and their atlas neighborhoods to bodies and uses the existing `Sky::light_lag_s`.
A transmitted leg freezes its lag, endpoints, Sky revision and due IC scalar.
Clock-generation changes retain that scalar. Explicit node coverage has zero local
ingress/egress; other same-body short legs use the local metric or round a positive
radio leg to one IC second. Unknown strategic positions wait; in-transit and
recovery rooms remain unresolved until R5.

Routes use deterministic cost/hop/identity ties. Only live Exchange-policy pairs
participate; private-policy links wait for their later authorization work.
A registry change replans from the last committed position after an in-flight leg
finishes. It never accelerates or replays that leg. A moved recipient requires a
new bounded forwarding leg. A payload too large for a pair's one-second budget
chooses conventional delivery instead of permanently blocking the FIFO.

### Bounds and player visibility

- Catalog: 32 nodes, 64 pairs. Each plan performs at most 65 metric lookups and
  512 directed edge visits, independently of success. The shipped two-node route
  measures **5 lookups / 8 edge visits**. A synthetic cycle's hit and miss both
  measure **9 lookups / 24 edge visits**.
- At most four pair hops in the existing four-entry `HopRef` history; at most six
  legs in one itinerary, eight physical legs over its lifetime, four recipient
  forwardings, and fourteen internal transitions per envelope per fire.
- At most 128 pending dispatches and 64 reserved pending copies per pair. FIFO
  tickets sort by committed IC enqueue time and envelope identity. Existing
  accepted traffic can migrate without being rejected by the new admission cap.
- Version-1 message units are `ceil(payload UTF-8 bytes / 128)`; the existing
  512-byte order limit means one to four units. The seed permits eight units per
  IC second, shared in both directions. Restart and backwards time cannot refill
  an already consumed bucket. Unused skipped seconds do not accumulate burst credit.
- The existing 64-envelope per-fire scan and 4,096-record store bound remain.
  Unknown endpoints retain the existing 30-day IC expiry. Cadence and queue service
  can add delay; the physical due time is a not-before bound, not a delivery SLA.
- Incoming undelivered traffic is absent from player lists and payloads. Outgoing
  party views hide remote hop progress and arrival state until a transported ACK.
  Staff-only `@prop/route <id>` reports route legs, frozen due time, queue time,
  units and work counters; denied and unknown-ID probes do not disclose routes.

### Focused verification

The cumulative `mush-server --test relay_registry` suite now has **10 passing
tests**: the five R1 cases plus pair/ACK/restart and visibility, disabled fallback
and repair, FIFO capacity/restart/overflow, recipient forwarding, and unknown
endpoint admission/expiry. The propagation crate has **110 passing tests**
(13 unit, 5 E7c11, 88 E7c7, 4 E7c8). The server library has **299 passing tests**
and its two pre-existing Redis-only ignored tests. Contract ownership, mail,
channels, content and port regressions also pass. Boot parity's 20 unaffected
tests passed, and its corrected seed-count test passed on rerun. Targeted Clippy
with `-D warnings`, changed-file rustfmt and `git diff --check` pass.

The real-process CI proof is
`mushd --test durability_e2e relay_queue_and_frozen_leg_survive_process_death_redis_gated`.
It uses the same production composition, a fresh isolated Redis database, real
SSH clients and a hard-killed child; it checks queued restart, one arrival,
returning ACK, frozen physical restart and link repair. It reuses the existing
child harness, database lock and account bootstrap. The `redis-faults` CI job
explicitly invokes it, and the gated-test census includes it.

The process proof passed on the VPS with fresh Redis logical database 13:
**1 passed, 0 failed, 0 ignored**, 18.17 seconds, using the actual Linux child
binary. [process-proof.json](evidence/relay-r2-2026-10-23/process-proof.json)
records the exact command and binary hash. All **six** CI inventory tests pass.
The preliminary Windows-to-VPS Redis run hit startup/account busy refusals over
the WAN tunnel; it is not counted as passing. Running beside Redis matches the
CI environment. The harness retries the existing initial connection boundary
within a fixed timeout and prints child logs on failure; it changes no server
timeout or durability rule.

### Deployed live evidence

Environment: the owner's disposable `conclav.io:/opt/rustmush/dev1`, split
gateway/server, Redis 8.10.2, TLS WebSocket :6790 and SSH :6789. IC ratio 24,
epoch `0987-04-20 06:00`. TLS verification remained enabled with the instance's
private CA certificate. Credentials remain root-only on the VPS.

| Check | Observed result |
| --- | --- |
| Upgrade | All **670** existing objects and the exact R1 catalog survived. [upgrade.json](evidence/relay-r2-2026-10-23/upgrade.json) |
| Pair order and returning ACK | Passed via real connected characters in ordinary covered rooms; one pair hop each. Recipient query hidden before arrival; sender query hides delivery until ACK. |
| Queued crash | SIGKILL the verified dev1 server child; gateway starts a replacement. The entire committed envelope, FIFO time and progress survive exactly. |
| Full queue and FIFO | **64 accepted, 65th refused**. All accepted copies cross once in FIFO order; maximum **8 units per IC second**. |
| Conventional flight and crash | Pair disabled, physical lag **1,063 IC seconds**. SIGKILL/restart preserves the frozen flight exactly. Repairing the pair does not accelerate it. No payload before due. |
| Physical arrival and returning ACK | Order due **74,162,165,479**, arrived **74,162,165,496**. ACK due **74,162,166,583**, arrived **74,162,166,600**. Both are 17 IC seconds late under the 24× coarse tick, never early. Repeated pass leaves arrival unchanged. |
| Missing route | Unplaced recipient waits invisibly; placing it in Ceres resumes and completes routing. |
| Invalid state | Bad capacity version and missing capacity refuse boot; bad cursor and inconsistent history clock are held. All four injected documents remain unchanged; good state is restored. [negative-cases.json](evidence/relay-r2-2026-10-23/negative-cases.json) |
| SSH parity | Actual SSH login, relay check and route inspection agree with WebSocket/Redis truth. [ssh.json](evidence/relay-r2-2026-10-23/ssh.json) |
| Final health | Gateway/Redis active, Live pair, registry revision **21**, dev2 gateway PID **539467** unchanged. [environment.json](evidence/relay-r2-2026-10-23/environment.json) |

The full **103-command** transcript, exact queue/flight/arrival snapshots and crash
PIDs are in [live.json](evidence/relay-r2-2026-10-23/live.json). Measured client command
latency was median **51 ms**, maximum **787 ms**; largest response **18,735 bytes**
(the bounded order list). These are observations on this instance, not CI
performance thresholds. Binary hashes are in environment.json.

Reproduce the live checks on this explicitly disposable instance:

```sh
export PATH=/opt/rustmush/dev1/r1-tools/node_modules/node/bin:$PATH
export NODE_EXTRA_CA_CERTS=/opt/rustmush/dev1/tls/cert.pem
node scripts/relay-r2-live.mjs https://conclav.io:6790 \
  /opt/rustmush/dev1/r1-evidence/credentials.json /opt/rustmush/dev1/r2-evidence 6788
python3 scripts/relay-r2-negative.py
python3 /opt/rustmush/dev1/r2-ssh.py
```

The live runner creates test traffic and deliberately kills only the child whose
resolved executable is dev1's server. Negative tests stop dev1, inject bounded
bad records and restore them in a finally block. Backups, build/upgrade logs and
SSH helper remain in dev1's `r2-evidence` and `r2-*.{sh,py,log}` files.
The first client attempt used a remote display name where object matching requires
an addressable reference; the corrected runner uses the authenticated character
ID. That was a harness correction, not a routing bypass.

### CI and stop

R1 main push: [CI 39994259374](https://github.com/conclavio/rustmush/actions/runs/39994259374).
Its boot golden failure was fixed in `7dc36d48`; the follow-up is
[CI 39998822477](https://github.com/conclavio/rustmush/actions/runs/39998822477).
The R2 candidate [CI 40003067213](https://github.com/conclavio/rustmush/actions/runs/40003067213)
finished with three failed jobs. Kernel contracts and the rest shard both caught
seven new production `expect` sites. TacSim integration caught six failures:
two restart fixtures restored module documents without their world objects,
three station goldens omitted the relay seed/node, and one real upgrade defect
prevented a kept Ceres v1 world from booting because the relay seed requires v2.
The Redis/process-fault matrix, including R2's kill/restart proof, passed.

The CI repair removes all seven new panic sites without raising either budget.
Capacity post-images use infallible string/integer JSON construction, checked
against the existing wire at integer limits. Route selection and optional module
state use checked branches. Restart fixtures now retain objects; station goldens
explicitly include the seed, dependencies and hardware.

For a kept Ceres v1 world, the production composition explicitly defers the new
relay seed until staff upgrade Ceres with `@atlas/pack plan` and `apply`. Boot logs
name the pending seed and required versions. Staff then restart or manually apply
`tacsim.relay.luna_ceres`. Fresh worlds and existing relay latches behave as before.
This opt-in deferral applies only to older active dependencies; missing, retired,
held, newer and digest-mismatched dependencies still refuse boot. Published pack
bytes and digests are unchanged. The station upgrade regression checks the
explicit upgrade, subsequent relay installation and a further idempotent restart.

The corrected geology, sites and stations suites pass **65 tests**. Propagation
passes **111 tests**, and the content apply suite passes **9 tests**, including the
deferral/refusal matrix. The corrective push must receive a full CI result;
targeted and live results do not substitute for it.

R2 implementation and deployed integration are complete. Full acceptance remains
pending any CI criterion explicitly marked outstanding. The owner then authorized
Gate 3 / R5 after the corrective dev1 test and took over monitoring GitHub CI.
The corrective `7bed1a2a` dev1 rerun passed: exact registry/object preservation,
queued and in-flight SIGKILL recovery, 64 accepted / 65th refused, FIFO capacity,
timed fallback and returning ACKs. The Linux real-Redis process test and kept
station upgrade test also passed. The rerunnable client now checks the specific
outgoing row (old ACK history may legitimately say Delivered), reuses gateway
sessions across child restarts and respects login throttling.

## Gate 3 — R5: ticketed ships underway (2026-10-23)

**Base:** CI repair `7bed1a2aabf3a34f3e0c8adcc734bd1bcd17da3e`.
**Deployed code candidate:** `a401e7e49e5c818864bb1ed589f3647d009ad377`.
Subsequent changes refine the live runner and record evidence; deployed Rust code
is unchanged. **Status:** implementation and deployed integration gate passed.
Full §5 acceptance is **not yet declared**: normal candidate CI remains required,
and the owner is monitoring it. **Paused before R6a.**

### Implementation and compatibility

TacSim's injected metric now reads the real `tacsim.ticket` through its immutable
owner-stamped hook lens. The production command and coarse tick supply that lens;
detached world-only calls cannot fabricate a ship ticket. Propagation receives a
bounded metric projection, with no TacSim write delegation or new kernel API.

The ticket key, route identity, departure instant, frozen leg durations and pinned
route bodies must agree with the active `CarrierJourney`. Missing, unreadable,
unsupported or contradictory passages wait under the existing envelope TTL.
The active journey cursor selects a frozen plan leg. Elapsed IC time interpolates
its frozen kilometre distance; the nearer endpoint wins, with the origin winning
an exact midpoint tie. Radio distance is that offset plus the current sky distance
between endpoint bodies. No moving ship coordinate is introduced. An overdue
journey cursor holds at that leg's endpoint until the journey owner advances it.

Actual spatial membership and ancestry resolve passengers and nested interiors
to one hull endpoint. Moving between its rooms does not spend a forwarding hop;
boarding or disembarking can. At physical arrival the recipient's endpoint is
revalidated. A missing ticket cannot settle a remote delivery. Existing four
forwarding/eight physical-leg bounds remain in force.

Positive distances round up to whole kilometres, then whole IC seconds, with
checked wide arithmetic. Transmission freezes due time and a `ship.v1` digest of
the metric inputs (sky revision, timestamp, endpoints, bodies, offsets and ticket
leg provenance). Restart, sky edits and world-rate changes cannot accelerate that
flight. R2's envelope/flight wire already permits object endpoints and bounded
basis text: no new durable record, pack version, migration or scheduler is added.
Old stationary itineraries and existing tickets retain their bytes and meaning.

A snapshot visits at most 33 port maps, 65 hull rows and 64 typed ticket keys;
each passage is bounded by the existing 32-leg cap. Oversized or duplicate hull
maps refuse the snapshot, never silently truncate it. Endpoint ancestry is bounded
by the existing depth-8 graph walk. Route work retains R2's 65 metric-read and
512 edge-visit bounds. `+ship`, while aboard, prints physical one-way lag to a live
public Exchange endpoint and explicitly excludes queue and returning-ACK time.
It distinguishes unknown passage data from no live Exchange pair. OOC channels
are unchanged.

### Integration gate

`ic_slice_loop::r5_real_dray_radio_freezes_forwards_and_recovers` boots the shipping
composition and buys the real trade cargo, fuel and provisions, boards a chartered
Dray and launches it through the ordinary clearance and burn. It checks boarding
forwarding, departure, exact quarter/three-quarter lag arithmetic and nearer-end
selection, a real leg boundary, docking/disembarkation forwarding, ticket loss and
recovery, saved-flight restart, rate changes, hull-room movement and delayed ACKs.
The existing six ship story tests and ten relay integration tests also pass.

The final code candidate passed these checks, with no failures or ignored tests
in the selected suites:

| Command | Result |
| --- | --- |
| `cargo test -p mush-propagation` | 111 passed |
| `cargo test -p mush-server --test ic_slice_loop --test relay_registry` | 17 passed: seven ship stories and ten registry tests |
| `cargo test -p mush-contract-tests --test panic_budget` | 8 passed; no budget increases |
| `cargo clippy -p tacsim -p mush-propagation -p mush-server --lib --tests -j4 -- -D warnings` | Passed |
| `cargo fmt -p tacsim -p mush-propagation -p mush-server --check` | Passed |
| `cargo test -p mush-server --test ic_slice_loop r5_real_dray_radio_freezes_forwards_and_recovers -- --exact --nocapture` on dev1 | 1 passed, 0 failed/ignored, six filtered; 55.22 seconds |

The Linux story output is preserved in
[story-test.log](evidence/relay-r5-2026-10-23/story-test.log). The release server and
gateway were built from the named candidate on the VPS, including matching web
assets. The upgrade retained the exact registry and all **675** existing objects.
No content pack or durable schema changed.

### Deployed live evidence

The live gate used authenticated TLS WebSocket clients against the split dev1
gateway/server at `conclav.io:6790`, with Redis on 6788. Certificate verification
remained enabled. A newly enrolled pilot bought the real cargo, fuel and provisions,
boarded Dray **#393**, and departed through ordinary clearance. Its frozen ticket
ran Ceres → Kirkwood → Stickney for **1,710,394 IC seconds**. Accelerating the real
IC clock between checkpoints did not substitute for journey transitions.

| Check | Observed result |
| --- | --- |
| Departure | Ship radio flight and returning ACK each froze at **1 IC second**. |
| Mid-leg crash and rate change | A **1,087 IC second** conventional flight survived verified server-child SIGKILL exactly. Rate changes 24→12→24 retained its due instant and metric basis. Repairing the pair did not accelerate it. The payload stayed hidden until due; its returning ACK had **9 IC seconds** of physical lag. |
| Hull movement | Walking between hull rooms spent zero forwarding hops. |
| Missing ticket | With dev1 stopped, removing only the fixture's typed ticket made lag unavailable and left a new order queued with no route or recipient payload. Restoring the exact ticket and restarting resumed delivery. No injected fault remains. |
| Nearer end and leg boundary | At the later first-leg checkpoint, traffic and ACK each had **54 IC seconds** of physical lag. After the real cursor advanced to the second leg, each had **46 IC seconds**. Exact nearest-end arithmetic is also asserted by the Rust story. |
| Docking and forwarding | A **199 IC second** radio flight spanned actual docking at **74,164,077,948**. After the pilot disembarked, the order forwarded exactly once from hull to berth; it arrived at **74,164,078,092**, after its original due instant **74,164,078,051**. The docked return ACK also took **199 IC seconds**, arriving at **74,164,081,212** after its due instant **74,164,081,195**. |
| OOC | Messages arrived in **52–53 ms** during the passage. |
| Final health | Relay bindings valid, gateway and Redis active, world rate restored to **24**, all original objects present, and dev2 gateway PID **539467** unchanged. |

The **93-command** transcript, frozen flights, arrivals, ticket and journey snapshots,
and crash PIDs are in [live.json](evidence/relay-r5-2026-10-23/live.json).
[environment.json](evidence/relay-r5-2026-10-23/environment.json) records source,
runner and binary hashes, service PIDs, object counts and command timing. Credentials
and database/binary backups remain root-only on the VPS and are not committed.
Client command latency was median **51 ms**, maximum **77 ms**; the largest reply
was **12,326 bytes**. These measurements describe this instance, not CI thresholds.

Reproduce on this explicitly disposable instance with Node 22:

```sh
export NODE_EXTRA_CA_CERTS=/opt/rustmush/dev1/tls/cert.pem
/opt/rustmush/dev1/r1-tools/node_modules/node/bin/node scripts/relay-r5-live.mjs \
  https://conclav.io:6790 /opt/rustmush/dev1/r1-evidence/credentials.json \
  /opt/rustmush/dev1/r5-evidence 6788
```

The runner deliberately creates traffic, changes the IC rate and kills only the
verified dev1 child. It removes the fixture ticket only with dev1 stopped and
restores it in its cleanup path. `R5_RESUME=1` continues the same recorded voyage
and skips completed checkpoints, using its existing root-only pilot credentials.

Two harness assumptions were corrected during this gate. Global port status can
remain unready while unrelated mining sites map; the real enrolment and departure
commands enforce this trade path's readiness. A short nine-second radio flight
also arrived normally during the first restart attempt, so it could not prove
in-flight retention. The successful crash check disables the pair first to obtain
the longer real fallback flight, then repairs it to test frozen-flight behavior.
The transcript retains both SIGKILL events; only the longer flight establishes
retention. Client resumption kept the actual ship and ticket, and unique payloads
prevent matching older traffic. No server behavior was relaxed to pass the gate.

The owner subsequently authorized R6a, followed automatically by R6b after R6a's
live gate and push. GitHub CI monitoring remains with the owner as requested.

## R6a — All mail over transport (dev1 gate passed)

All compose, quick, reply, reply-all and forward paths now persist mail-owned
content, a Sent copy and a bounded dispatch intent. Submission says queued, not
delivered. Recipient inboxes, line commands, web queries, unread counts and notices
are populated only when the transport receipt is consumed. The sender learns
delivery/refusal only when the return dispatch reaches their current endpoint.
Self mail follows the same local settlement. Legacy messages have no dispatch and
remain already delivered, with unchanged stored message encoding.

### Ownership and recovery

| Record | Owner / writer / readers | Compatibility and durable boundary |
|---|---|---|
| `mail:dispatch:<id>` v1 | Mail submits/settles; composition reads intents | New, fail-closed boot validation; source intent and content/Sent are one durable batch. Up to 64 letters / 128 recipient copies. |
| `mail:handoff_cursor` | Mail settlement | New scalar continuation, 16 letters per fire; absent starts at zero. |
| `mail:msg:<id>.delivery` | Mail settlement; sender's projections | Additive optional map of returned knowledge. Absent means legacy delivered; empty means awaiting confirmation. Never a remote truth query. |
| `propagation:envelope:<id>.dispatch` v1 | Propagation accepts/routes/reaps; composition reads receipts | New bounded reference, stable `mail:v1:<id>:<recipient>:out/return` key. No body or authority to read it. Duplicate/corrupt external identities fail boot. Existing order/report records retain their behavior. |

Composition injects read-only adapters into both owners. The existing propagation
tick reconciles the complete bounded source snapshot, accepts each stable key once,
and retains terminal receipts until the source durably withdraws the intent. Mail
consumes receipts only on a later coarse step, after transport's receipt has been
committed. Inbox insertion and outgoing return intent are one mail-owned batch;
return completion and source cleanup are another. No cross-owner write delegation,
new dependency edge, or second router is introduced.

Propagation accepts at most 16 external intents per fire, retains at most 256
external envelopes, and shares the existing 128 pending-route / 4096 envelope caps.
Admission includes source intents not yet accepted by transport. Existing expiry
and route work limits apply. Terminal external receipts cannot age out before
consumption. Mail retains content while any mailbox or pending handoff references
it, including after Sent purge; it reclaims the final orphan after completion.

Version 1 counts actual UTF-8 subject plus body bytes, in 128-byte message units.
Letters at most 1024 bytes can use a pair; larger letters take the conventional
path. The existing 512-byte subject, 64 KiB body and 32-recipient limits remain.
There is no partial body visibility or fragmentation. Destination lock, ignore,
destruction and quota checks occur at arrival; return failure/expiry never exposes
remote truth before transport. Unknown endpoints wait and expire. SSH and web Sent
rows distinguish queued mail from returned delivery knowledge.

The live gate passed. An actual legacy letter delivered by the previous binary
remained readable after upgrade and was not resent. All 680 world objects and the
existing relay registry were retained.

### Automated validation

The candidate's 298 targeted Rust tests passed: 146 mail/propagation tests, 21
production relay and passage stories, 12 real SSH/WebSocket client tests, 48
kernel/lifecycle, scheduling, panic-budget and CI-inventory contracts, and 71 core
command tests. The 21
production stories also passed on Linux on the VPS. Seven web mail-store tests,
Svelte checking (zero errors/warnings), formatting and targeted Clippy with
warnings denied passed. Normal GitHub CI remains separate and owner-monitored.

The two older client fixtures began in unplaced Limbo and assumed instant mail.
They now create a shared physical room through normal staff commands and exercise
the real local transport/receipt path. The production stories and live gate cover
strategic and moving endpoints. Boot corruption checks refuse missing intent or
content, mismatched byte accounting, and a destination entry predating its receipt.

The first live pass exposed a real command integration defect: core still refused
the documented `/mail` lock type. R6a enables it on `@lock` and `@unlock`, retains
the normal ownership checks, and changes the production refusal story to set the
lock through that public command rather than inserting it into fixture state.
The core command suite's 71 tests pass, including a normal player's set/remove
regression. The corrected command passed on dev1: the destination refused the
letter, no inbox entry appeared, and the sender learned refusal only after the
return dispatch arrived.

### Real server evidence and provenance

Base: `6cbec90181af6217bf1dc10e974f43803dd27632`. Transport candidate:
`8fa63f371c1fc871102dad42139e522c7adf2dfa`. Corrected code candidate:
`5f3955ca8f82abb68cc0ab5a93c4185066423769`. The transport/crash/expiry/ship cases ran
on 8fa63f37; the public lock and final SSH cases ran on 5f3955ca. The only production
delta is the accepted mail-lock switch. All 21 production stories were rerun on
Linux on 5f3955ca. This records the staged run explicitly rather than attributing
earlier live checks to the later binary.

Authenticated TLS WebSocket clients used the actual split gateway/server at
`conclav.io:6790`, Redis 6788, with certificate verification enabled. Normal
players sent, replied, replied-all, forwarded, and mailed themselves. A full
65,536-byte body used conventional transport and reached an offline recipient.
Before arrival, inbox queries, line reads, unread counts and reconnect revealed
no letter. Sent deletion retained the pending payload.

Verified server-child SIGKILLs exercised source acceptance, transport acceptance,
arrival receipt and mailbox insertion. The frozen flight survived exactly; the
recipient got one entry and one unread increment. Changed ignore/lock, quota and
recipient destruction refused at arrival and returned failure physically. The
quota and object-loss cases were controlled faults with dev1 stopped; the exact
configuration and synthetic object were restored afterward. They are not claims
about ordinary player destruction commands.

The queue accepted 64 letters and refused the 65th without allocating an ID.
Unplaced addresses expired at the existing **30 IC day** transport TTL. All 64
handoffs completed without recipient entries; the payload with its Sent copy
purged was reclaimed after the return handoff finished. The real world clock was
accelerated for this test and restored to 24.

Pilot #677 enrolled and provisioned Dray #412 through the registry, market and
ship commands, then departed Ceres for Stickney through Kirkwood on a real frozen
ticket. At the first-leg checkpoint, shore letter #84 and reply #85 each included
a positive **9 IC second** ship flight. Both arrived and returned confirmation.
The final SSH client read the arrived reply and sent/read a new self-letter.

The [live transcript](evidence/relay-r6a-2026-10-23/live.json) contains 726 checks;
[SSH evidence](evidence/relay-r6a-2026-10-23/ssh.json) contains four checks.
[Environment](evidence/relay-r6a-2026-10-23/environment.json) records binary/runner
hashes, runtime provenance and timings (median 26 ms, maximum 785 ms, largest
reply 14,310 bytes; observations, not performance thresholds). Linux stories,
core lock tests and Clippy logs are beside them. Credentials and database/config
backups remain root-only on the VPS. Final services are healthy, relay bindings
valid, rate 24, and dev2 gateway PID 539467 unchanged.

Reproduce using `scripts/relay-r6a-live.mjs` with the same four arguments as R5,
substituting `/opt/rustmush/dev1/r6a-evidence`, then run `relay-r6a-ssh.py` on the
VPS. `R6A_PREPARE=1` establishes the legacy fixture before upgrading;
`R6A_RESUME=1` resumes completed milestones. `R6A_DEFER_LOCK=1` was used only while
the command correction built and cannot mark the gate passed. Harness fixes
closed old sockets on reconnect, distinguished old replies sharing a subject,
read the actual 30-day TTL, and checked the ship leg of a multi-leg route instead
of requiring the first shore-to-relay leg to use the ship metric. No transport
limit or physical lag was relaxed.

**Live milestone gate passed; full acceptance still awaits normal candidate CI.**
The owner monitors GitHub and authorized continuation to R6b after commit/push.
R6b must finish its own live gate; the next owner stop is before R3.


## R6b — IC channel fanout and observer bridge boundaries (2026-10-23)

IC channel speech now uses the existing propagation handshake. Comsys owns the
frozen content, eligible dispatch roster, and each member's arrived history;
propagation owns routing, bandwidth, expiry and durable terminal receipts.
Composition supplies read-only adapters and shares admission with pending mail.
OOC channel behavior remains immediate. Membership, see lock, ignore and gag are
rechecked at arrival. A sender's accepted content and physical origin survive
source loss. New members and rejoined members receive only later dispatches.

The channel's private source sequence never reaches an IC recipient. Each member
gets a dense arrival sequence, permitting a later fast line to overtake an older
physical flight without revealing the gap. An opaque persisted membership epoch
resets web/SSH history caches on rejoin. State, backlog, recall, live text, unread
and mentions all read the same arrived knowledge. Legacy IC history migrates as
already delivered to existing members and never creates transport intents.
All IC receipt/content records are durable, including channels without PERSIST;
LOGGED controls whether arrived content is retained for recall. The existing
1–10,000 history cap applies per member, and the final reference releases content.
IC/OOC mode cannot change after traffic; create a new channel to change mode.

| Persisted comsys record | Meaning and lifecycle |
| --- | --- |
| `ic:msg:<channel>:<seq>` v1 | One bounded frozen content body and pending recipient set; retained by pending copies or arrived history references. |
| `member:<actor>:<channel>.ic` | Membership epoch, private join boundary, local arrival counter and bounded received references; removed on leave. |
| `ic:progress` | Rotating handoff cursor and aggregate diagnostic counters. |
| `ic:relay` | Last 256 accepted provider IDs and content fingerprints, plus a monotonic retired-ID floor; old retries cannot become fresh submissions. |

Limits: 32 IC members per channel, 64 pending source lines, 128 pending copies,
16 source lines settled per coarse fire, and 16 KiB UTF-8 message text. Existing
larger legacy rosters keep their history but refuse new speech until reduced.
No roster is silently truncated. Propagation's shared route caps, message-unit
accounting, 1 KiB pair limit and 30 IC day TTL remain in force. Wizard-only
`@channel/transit` reports pending lines/copies, oldest age and settlement work;
ordinary player queries do not expose transport internals.

Discord guild ingress and IC alias commands in followed DMs retain the provider's
message ID. Slash commands and notice reply modals also retain their interaction
ID, so a retried modal cannot bypass channel deduplication. Authentication still binds the linked account, selected actor and
physical endpoint. A repeated ID with identical content is an accepted no-op;
changed content or a retired ID is refused. Acceptance waits for the real writer
barrier. The direct service path now drains deferred module persistence before
committing, using the engine's normal finalization path. Shared IC mirrors are
disabled. Followed DMs render only that actor's arrived IC channel frames, with
quiet mode respected; ordinary OOC mirror and DM command paths remain available.

The real bridge integration test uses the shipping bridge, world loop, commit
writer, account oracle and DM renderer; only Discord REST is controlled. It found
and repaired the renderer's previous blanket rejection of channel frames. The
web cache tests also found a pre-existing plain-string backlog body rendering
problem, now corrected alongside membership epoch handling.

### Automated validation and provenance

The initial candidate passed 734 regression tests across comsys, mail, propagation,
Discord, SSH, IPC and the split gateway, plus 50 protocol tests, 27 ownership,
scheduling, panic-budget and CI-inventory contract tests, and three gateway service
parity checks. All 26 production relay/passage stories passed on Linux. The final
Discord ingress correction passed all 98 gateway/Discord unit tests and the real
bridge test, including duplicate modal submissions; that bridge test also passed
on Linux. The web controller and channel store passed 103 tests, Svelte reported
zero errors/warnings, and targeted Clippy passed with warnings denied.

The state-ownership contract exposed an R6a test fixture directly changing mail's
live quota state. It now changes the persisted mail configuration and reboots;
that refusal scenario passed on both hosts. No ownership allowance or panic budget
was raised. The web controller also refetches an active tab when its membership
epoch changes, including when an older backlog response is still in flight.

Base: `24384bf2`. Initial transport candidate: `cdf8190a`; web refresh and persisted
quota fixture: `1d2e2cd6`; final bridge-identity candidate: `e0b1841a`. The Linux
26-story suite ran on cdf8190a; the changed quota fixture was rerun on 1d2e2cd6.
The expanded bridge test and **all live dev1 checks ran on e0b1841a**. The three
candidates were built locally/on dev1 before the final gate push; normal GitHub CI
is separate and owner-monitored.

### Real server gate

The [live transcript](evidence/relay-r6b-2026-10-23/live.json) contains 68 checks
through authenticated TLS WebSockets at `conclav.io:6790`, with certificate
verification enabled. Legacy IC history created by the R6a binary remained readable
and created no new dispatch. A verified server-child SIGKILL preserved fanout and
the frozen physical flight. A later paired line arrived first; recall, backlog,
member sequence/unread and mention surfaces did not expose the earlier payload.
The earlier line then arrived while its recipient was offline, with one additional
receipt and unread increment. The uncovered destination received its own copies
only after their flights.

Leaving and rejoining replaced the membership epoch and denied the older in-flight
copy. A see lock revoked during flight denied arrival, with no retained body after
access was restored. The actual underway Dray from the R6a departure supplied both
a moving recipient and a ship-to-shore source; both routes recorded positive ship
flights. OOC chat remained immediate. A controlled source-object deletion with
**dev1 stopped** left accepted content deliverable; the exact synthetic object was
restored afterward. This is an object-loss fault test, not a claim about the game's
normal destruction command. The diagnostic expired counter includes cancellation
of copies addressed to the absent object.

[SSH evidence](evidence/relay-r6b-2026-10-23/ssh.json) records five checks through the
real `play` front door on port 6789: arrived history, queued speech, arrived self
copy, immediate OOC chat and refusal of staff-only transport diagnostics.
[Environment](evidence/relay-r6b-2026-10-23/environment.json) records actual runtime
binary/runner hashes, 680 preserved objects, rate 24, a healthy pair, zero pending
channel copies and unchanged dev2 gateway PID 539467. Command replies had median
52 ms and maximum 78 ms, with a largest reply of 855 bytes; these are observations,
not performance thresholds. Database/object backups and credentials remain root-only
on the VPS. Test logs and [automated counts](evidence/relay-r6b-2026-10-23/automated.json)
are alongside the transcript.

Reproduce with `scripts/relay-r6b-live.mjs` using the same four arguments as R6a,
substituting `/opt/rustmush/dev1/r6b-evidence`, then `relay-r6b-ssh.py` on the VPS.
`R6B_PREPARE=1` creates the legacy IC fixture before upgrade. This harness reuses the
R6a synthetic accounts and its legitimately departed Dray.

**Dev1 checks passed. Live Discord provider acceptance remains open:** the deployed
configuration has Discord disabled and zero relay mappings. Controlled-provider
results do not claim a live Discord run. Full acceptance also awaits normal CI.
**Paused after R6b; R3 has not started.**


## R3 — Service classes, declared congestion and publication (2026-10-23)

The existing propagation queue now schedules Sovereign, Priority, Standard and
Deferred copies. Class entitlement is operator-granted through scoped IC
organization authority; no purchase, debit or payment is claimed. All mail remains
IC and ordinary OOC channels remain immediate. R4 service restrictions and R8
billing are separate gates.

The versioned `content/relays/service-v1.json` policy defines 128 content bytes per
unit, a 1,024-byte short-message limit, an eight-unit burst, Standard's 60 IC second
extra wait, and Deferred's minimum 7,200 IC seconds plus the first 3,600 seconds of
each 86,400-second IC day. The existing pair seed supplies eight units per IC
second, shared across directions. Idle capacity does not accumulate. Actual
content bytes include mail subject/body and channel attribution, not the transport
reference. Each pair traversal consumes one bucket entry per recipient copy; no
second hub charge is introduced.

Eligible older work gains a class rank every 300 IC seconds. The bounded pass can
skip a capacity-blocked head, a jammed exit or a closed Deferred window. It keeps
R2's 64 copies per link, 128 pending copies, 64 visited envelopes per delivery pass,
rotating durable cursor and coarse cadence. These are earliest service times;
cadence and contention may make arrival later. Existing conventional flights retain
their frozen physical timing.

A frozen ticket sits on each queued envelope. Capacity, ticket transitions and
receipts share the existing transport commit. The optional v1 service record
migrates old transport state without resetting spent capacity; losing the record
beside class tickets or loading malformed policy refuses boot. Full legacy queues
acquire tickets without attempting a second admission.

`+relay` and the advertised `relay.board` query share the same public projection.
`+relay/receipts` exposes only the actor's arrived, still-visible traffic; mail
retention and channel membership/see lock/history are rechecked. Private actual
load, audit and discovery require scoped `relay.inspect`; class grants require
`relay.manage`. Publication separately requires `relay.publish` and private
knowledge of retained evidence. Discovery uses the NPC knowledge owner's API;
publication uses existing canon and faction/front owners. It creates the authored
congestion-board front and public accounting clock once. Neither path exposes
message payloads. Restoring Standard frees a grant slot; re-discovery restores a
rolled-off intel entry without duplicating it.

Retained state is bounded to 256 class overrides, 512 rolling receipts, 64 evidence
records, 64 pair publication IDs and 128 rolling audit events. The classes and
evidence are durable across restart. Price terms, subscriptions, receiver denial
and sanctions are not part of this cut.

### Validation and provenance

Base: `5b4bb042`; initial candidate: `adf77a50`; final runtime candidate:
`97b4106b`. On the final candidate, local owner-crate tests passed 313 checks and
all five focused R3 production stories passed. Ownership, scheduling, RNG,
panic-budget and CI inventory passed 23 checks. Changed Rust files pass rustfmt;
targeted Clippy denies warnings. Linux passed all 52 relay, passage and boot-matrix
tests and the affected library lint before its release build.

Review and tests repaired a missing advertised board feed, full legacy queue
readmission, default-grant retention, intel-refiling behavior, mixed-switch
authority checks and expired-ticket load accounting. The R5 passage
helper now advances to the recorded service eligibility time. The web hello golden
explicitly includes the new feed. No work limits, ownership allowances or panic
budgets were increased.

### Real server gate

The [fresh-world check](evidence/relay-r3-2026-10-23/fresh.json) booted and
rebooted the release server against previously empty dev1 Redis database 15. It
retained the v1 policy, two nodes and one pair without grants or receipts. The
existing dev1 database had no service record before upgrade; it gained the policy
without a world reset. Only dev1 was used.

The [live transcript](evidence/relay-r3-2026-10-23/live.json) records 71 authenticated
TLS WebSocket commands and one board query. The public query matched line output
and ignored forged actor/actual-load parameters. With a normal jam at the relay
exit, four eight-unit letters acquired distinct class tickets. A verified child
SIGKILL preserved their complete persisted envelopes. After recovery, Sovereign
and Priority transmitted 5 and 7 IC seconds after queueing; Standard transmitted
at exactly 60 seconds despite free capacity observed at second 10. Deferred
transmitted after 10,860 IC seconds, inside its configured off-peak window.

An ordinary observer was denied actual load and discovery. Scoped inspect
authority filed private intel; publication remained denied until separately
authorized. Publication created front #0 once. A second child SIGKILL retained
that front and made a repeated publication an idempotent reply. The mixed-producer
gate then queued 13 real mail/channel/order copies at the jammed relay exit and
drained them under the shared bucket. Leaving/rejoining the channel removed old
receipt visibility. OOC speech remained immediate.

The [SSH transcript](evidence/relay-r3-2026-10-23/ssh.json) passed five checks through
the real `play` front door on port 6789: board, arrived receipts, private-load
refusal, discovery refusal and immediate OOC speech.
[Environment evidence](evidence/relay-r3-2026-10-23/environment.json) records exact
runtime hashes and raw/normalized runner hashes. All 27 retained pair receipts
were unique and every IC second spent at most eight units. No pending copies
remained. Command latency was 51 ms median and 53 ms maximum; the largest reply
was 2,666 bytes. These are observations, not performance thresholds. Dev1 was
restored to rate 24 with both halves healthy; dev2 gateway PID 539467 was unchanged.

The final runtime is `97b4106b`. The runner-only correction included with this
evidence targets the actual relay node rather than its coverage room, and waits
for channel-owner settlement before reading history. Database backups,
credentials and fresh-world configuration remain root-only on the VPS. Linux
test/lint logs and automated counts accompany the evidence; no credentials or
backups are committed.

Reproduce with `scripts/relay-r3-fresh.py` on an empty dev1 database 15, then run
`relay-r3-live.mjs` with arguments `https://conclav.io:6790`, the root-only staff
credentials file, `/opt/rustmush/dev1/r3-evidence`, and `6788`, followed by
`relay-r3-ssh.py`. The live script reuses the R6a synthetic accounts and verifies
TLS. It is an explicit real-server gate, not a test that silently succeeds
without Redis.

**Live milestone gate passed.** The previous CI repair on `5b4bb042` is green;
normal R3 CI is triggered by the push and remains owner-monitored. The earlier
R6b live Discord-provider gap remains recorded above; R3 changes no bridge code.
**Paused after R3, before R4.**


## R4 — operator service policy (2026-10-23)

Base `0bb5a25f`. The owner authorized all remaining R4–R10 gates, each with
integration testing followed by commit/push before dependent work. R4 adds
operator-wide Served/Throttled/Suspended restrictions with independent sources,
IC actor/time/reason audit, scoped `relay.manage` and player-only own status.
The new `restrictions` array defaults empty on R3 records. Tickets add optional
base class and a default-false throttle marker; legacy tickets retain their class.
No new state slot, namespace, crate dependency or budget exception is introduced.

The real producer is `+relay/policy pair/#character|org:id=state,operator.source,reason`.
The existing propagation planner and bounded pass consume it at both endpoints
before each untransmitted link hop, including work queued before the change.
A character speaks personally; a titled unit additionally represents its actual
legal owner through the economy read adapter. Login, membership, database owner
and payer aliases confer no representation. Throttles compose once by greatest
severity. Served clears only the specified source, reserving automatic billing
and incident sources for their future owner writers. A legitimate throttle is
excluded from congestion-fraud proof. Consumed service and frozen physical flights
are preserved. Independent links use their own node operator; both halves must
agree, every node's operator must exist, and the seed reconciler permits additional
independent topology while checking every seeded identity and binding.

The independent-pair acceptance fixture is controlled topology input with normal
object/behavior/organization setup, not a shipped fabrication or deployment verb.
Its routes and policy decisions use the production consumer. R9 still owns the
finite-input manufacture, shipment and installation producer. The live gate
restores the seeded topology and removes fixture bindings afterward.

Prior R3 CI run `40051264494` exposed a durability-test timing assumption: its
60-second ACK wall timeout was also the new Standard minimum IC wait. The R2
process-kill scenario now explicitly grants Priority to both endpoints, preserving
its physical/crash premise; R3 class stories retain Standard delay assertions.
This correction must pass on actual dev1 Redis/child processes before pushing.

**Implementation/live gate passed.** Runtime candidate
`c5a393b17f85fba21602dc573c1552cc432d336d` adds NPC personal identity coverage to
`233fbb2e`. The exact final runtime passed 55 cumulative Linux production stories,
library Clippy and the real Redis/process-death CI regression (1 test, 3 other
scenarios intentionally filtered). Local checks passed 419 unique propagation/sim
owner tests, 23 architecture contracts and the 3 R4 production stories; changed
integration tests passed Clippy and changed Rust files passed formatting.

[Evidence](evidence/relay-r4-2026-10-23/automated.json) includes exact commands and
logs. [Real-client evidence](evidence/relay-r4-2026-10-23/live.json) records sender
and receiver throttling, independent source restoration, suspension fallback despite
Sovereign entitlement, frozen-flight preservation, scoped operator refusal and
successful independent private-link delivery. Two actual SIGKILL boundaries prove
confirmed policy/queue and receiving-endpoint suspension recovery. A fresh Redis-14
world booted and rebooted as two real processes. Upgraded Redis-0 retained R3
receipts/classes and acquired the optional R4 fields on its first service write.
The SSH front door passed six checks. 85 WS command checks had median 51 ms, maximum
763 ms; these are measured observations, not a raised performance budget.

The live harness needed two lifecycle corrections, with no further runtime change:
accept an omitted legacy `restrictions` field as empty; and remove the four
controlled fixture bindings during the same stopped-server interval as restoring
its topology. The registry correctly refused boot with unrecorded bindings during
the initial teardown attempt. The repaired cleanup resumed only after verifying
that all six policy/independent-operator scenarios had passed; it also handles
omitted empty behavior maps and waits for the gateway front door. No product boot
guard was weakened. Final runner hashes and runtime hashes are recorded in
[environment.json](evidence/relay-r4-2026-10-23/environment.json).

Final dev1: original two nodes/one live pair, no active restrictions, no pending
envelopes, world rate 24×, unique pair-service receipts and capacity within the
8-unit Exchange / 4-unit fixture limits. Dev2 remains PID 539467. Normal R4 CI is
triggered by the main push and remains owner-monitored; the known prior CI failure
is repaired and reproduced green. The standing authorization continues to R7
only after this evidence commit/push.


## R7 — structural loss and political consequences (2026-10-23)

Base `3a042341`; runtime candidate `5bdec3b8`. The checked relay-station pack adds
class `station`, core and power structure, and starts its power component online.
Stations are fixed installations, excluded from the simulator lobby; this cut does
not add a strategic siege UI. The live persistent writers are
`mush_tactics::module::apply_plan` (`@unit/damage` and checked game callers) and
`custody::check_in` (the actual arena-return writer). Both emit a synchronous
`TacticalUnitChanged` notification on their persistence batch. Void returns do not
announce damage. A standing command carries its authenticated actor; an arena
return with no established attacker is recorded unattributed, never blamed on the
finishing timer or on a free-form cause string. The retired fire lane is unused.

The game adapter installs default-compatible `damaged` half flags and permanent
node-loss records through the simulation owner. Power-structure loss independently
disables service; the operator switch cannot override it. Core loss kills every
installed half on that node, orphans counterparts, and never clears on structural
repair. Existing routing revalidates queued hops and preserves frozen flights.
A bounded per-node incident identity drives public canon, Exchange-wide anathema
for both endpoints, institutional condemnation, legitimacy clocks and the existing
recovery insurance consumer. Manual operator policy cannot clear anathema. Faction
cause receipts prevent duplicate penalties; an existing affiliated `attention`
pool earns one, capped at 1000, while an existing legitimacy pool loses one. No
attention pool or actor is seeded. Canon append failures leave terminal outage and
an unannounced incident for boot retry rather than restoring service.

R1–R4 records default to `structural_version = 0`; migration attaches checked units
once and records version 1. Thereafter absent/held units or a different exact
structural definition refuse boot, instead of silently recreating healthy hardware.
The new station does not change older chassis digests, weapons, cores or arena
admission. Explicit source/catalog census tests were extended by one station.
No crate dependency, state slot or namespace was added. The narrow new persistence
delegations are `tacsim -> sim` and `content|atlas -> tactics`.

Insurance cancellation changes `loss_is_covered`, the actual grant-time predicate,
and removes a prior insured-loss cooldown waiver. `@recovery/status` renders that
same live decision. The VPS gate uses a documented historical **ended** covered
grant fixture and temporarily enables the existing recovery provider; it tests the
consumer, not E7 loaner issuance. It snapshots upgraded Redis-0 into isolated
Redis-11, preserves expirations, tests destructive loss, and restores the whole
approved disposable test world afterward. Snapshot/config/credentials remain
root-only. This reset is not a product command capable of reviving entanglement.

Prior R4 CI run `40055195406`, job `120756539207`, failed Clippy on a needless
borrow in the new restriction-bound test. R7 removes that borrow. The broader local
ownership check also exposed an older missing inventory row for R3's already
declared `propagation -> npc` permission; that inventory now matches production.
Both checks are reproduced locally rather than left for the long CI run.

**Implementation/live gate passed.** The exact final runtime passed 59 Linux
production boot/relay/passage stories and the actual Redis/process-death regression.
Local owner/content checks passed 130 tests, owner libraries 66, the full faction
suite 59 (12 overlap those libraries), and 13 affected extension census/class tests.
Changed library and integration-test targets passed Clippy with warnings denied.
[Automated evidence](evidence/relay-r7-2026-10-23/automated.json) lists scopes and logs.
The extension census changes are test-only additions after the runtime candidate.

[Live evidence](evidence/relay-r7-2026-10-23/live.json) covers real power disablement,
repair, permanent node loss during a physical flight, queued fallback, orphaning,
permanent Exchange sanction, actual recovery cooldown change, one faction/front
consequence, attention saturation, real SIGKILL, and repeated repair/destruction.
Both letters arrived conventionally without relay-service receipts. All returns
drained. A fresh Redis-12 world booted and rebooted; the upgraded world persisted
structural version 1. SSH passed six checks. The 39 WS command checks measured
51 ms median and 52 ms maximum. No performance budget was raised.

The first fixture attempt enabled recovery without its mandatory pool account/tier;
config validation correctly refused boot before gameplay. The corrected fixture
uses existing funded clearing account 1 only as provider identity, issues no grants
and spends no money. The same runtime binaries then passed. Runner-only corrections
also require durable migration before the baseline snapshot and verify the backup's
content hash before restore. Final runner/runtime hashes are in
[environment.json](evidence/relay-r7-2026-10-23/environment.json).

The whole upgraded test baseline was restored after acceptance: two healthy nodes,
one live pair, no losses/restrictions/pending traffic, rate 24×. Redis-11 retains the
root-only snapshot; original expiration deadlines were preserved. Dev2 remains PID
539467. Secrets, raw snapshots and configuration are not committed. Prior R4 CI has
only the already-repaired test(rest) failure at this check; the slow remaining shard
is owner-monitored. Commit/push completes R7 before automatic continuation to R8a.

## R8a — funded Exchange and per-copy retail (2026-10-23)

Base `42f610ec`; runtime source tree `7376f2c61b5bcce6b2a9b4adc73038be4a4961e4`.
The runtime candidate is an archive of a staged Git tree, built and
accepted before its branch commit. The existing web archive checker uses the parent
commit marker; `.source-tree` and the binary hashes identify the exact candidate.
The final evidence records both. A descriptive parent marker was refused by the web
checker on an earlier build, before deployment; the corrected marker is a full hash.

The Exchange owns one real operating account. A once-only, idempotent 1000 cr transfer
comes from the existing finite Ceres Clearing account, never a new mint. On a fresh
world that transfer waits for the existing port funding cadence; quotes truthfully
remain unavailable until it settles. On upgrade it uses available clearing money.
A missing market beside its operating account refuses boot rather than recreating
capital. The checked market identity/tariff cannot silently change on reapply.

Normal `+relay/outbox`, `/quote`, `/buy`, `/cancel` and `/purchases` sell one recipient
copy: actual UTF-8 subject/body bytes, ceil(bytes/128), at most 1024 bytes. Tariff v1
charges Standard 1 cr/unit and Priority 5 cr/unit. Quotes last 600 IC seconds; unused
reservations release by 86400 IC seconds from quotation, on cancellation, or when
the copy terminates without Exchange consumption. Change an unused reservation by
cancelling then quoting the new class. The first Exchange crossing consumes the
whole fee; subsequent Exchange hops do not charge again, and consumed service has
no refund. Large letters remain conventional. New unpaid Exchange traffic uses
free Deferred. Existing accepted envelopes retain pre-retail class semantics using
a checked allocator boundary; independent operators retain their own Standard default.
Explicit, audited courtesy grants remain noncommercial and never claim payment.

Propagation owns at most 256 purchase rows in its existing strictly decoded transport
document, with 32 live quotes/reservations per issuer, bounded terminal eviction and
8 release attempts per pass. The injected TacSim adapter calls the existing economy
post/engine-reservation APIs. The protected `relay.message` purpose prevents direct
staff/player hold capture/release. Actual authenticated payer authority and account
approval are enforced by the money owner; a payer never substitutes for either IC
endpoint. Reserved money and purchase post-images share one strict-durable step.
Both timer delivery and `@prop/pass` make a provisional route/capacity/receipt update,
capture at most one new purchase, and install only after successful settlement.
That whole update shares the money batch. Failed capture advances no paid service.
Expired reservations still awaiting bounded release cannot be captured.

Restart audits match every live purchase hold and every live protected hold back to
one purchase. A derived held-reservation purpose index makes the reverse audit
bounded without walking unrelated holds. It rebuilds from the existing journal and
is not persisted authority. Terminal hold projections may be garbage-collected after
the economy's seven-real-day retention; their atomically committed purchase receipts
remain. Frozen economy repair mode still boots and refuses money writes. No new
crate edge, state slot, namespace, monetary ledger, or general marketplace is added.
The new persistence declaration is `tacsim -> propagation`; account ID allocation uses
the existing economy-owner allocator-only boundary. `+relay` is classified at its
mixed command's economy ceiling, with the retained transport boundary documented in
the handler inventory.

The local gate caught a real migration cutoff off-by-one and the bounded-expiry
backlog bug; both have regressions. It also checks frozen-payee settlement refusal,
missing market/purchase and forged-price boot refusals, zero-fund/payer refusals,
normal conservation, fanout and UTF-8 prices, two Exchange hops with a restart,
and independently delayed free traffic. The legacy tests now explicitly select
courtesy service when they isolate physics, and retain funded market records when
modelling old tickets rather than deleting a real account's owner record.

R7 CI run `40059174892`, job `120768743119`, exposed three stale inventories:
`+relay` absent from the handler list, the station absent from intentional lookless
units, and three relay capabilities missing from the server's expected count.
These are corrected and rerun. Its fourth failure required confirmation to arrive
before a self-ban ejected the caller; the split test now asserts committed ban state,
ejection, refused admission and lift. Non-self-ban confirmation coverage remains
in the existing persistence test. No production admission timeout is weakened.

The completed R7 CI run also exposed one stale fielded-definition census in job
`120768743071`: the expected set omitted `tacsim.unit.relay_station`. Adding that
explicit schema entry passes all 25 definition tests and lint. This additional
change is test-only; the live runtime remains the staged tree recorded above.

**Implementation/live gate passed.** Final Linux validation passed 39 production
relay stories, 15 ownership/handler/look contracts, the capability-vocabulary test,
four split-gateway tests, the real Redis/process-death test, and 25 definition tests.
The immediately preceding candidate also passed 21 boot and seven IC-voyage stories;
the final private-operator default correction reran all affected relay stories.
Local validation includes 206 economy tests, 17 propagation tests and 89 extension
tests, plus changed library/test lint and formatting. The web build reports zero
errors/warnings; the existing bundler size advisory remains. Counts and scope are in
[automated.json](evidence/relay-r8a-2026-10-23/automated.json).

[Live evidence](evidence/relay-r8a-2026-10-23/live.json) records 298 authenticated TLS
WebSocket command checks, eight SSH checks, and four actual child SIGKILLs: quoted
before reservation, held before transmission, captured with the transmission, and
released before restart. Fresh Redis-10 boot/reboot and upgraded Redis-0 both retain
one funded market. Real journal/hold receipts, per-copy UTF-8 prices, two-recipient
Standard fanout, Priority, denied receivers, cancellation/upgrade, expiry, empty
payers, large conventional letters and free Deferred delivery pass. Money is
conserved; retries do not charge again. WS latency was 26 ms median and 583 ms max;
these are observations during the gate, with a Linux regression test also running,
not an isolated load benchmark or a raised budget.

The initial live expiry fixture jammed the relay station but left the recipient's
physical room reachable. Receiver suspension correctly chose conventional delivery;
the fixture's assertion that the letter would still be pending was wrong. The
corrected fixture jams the station and recipient room for the expiry interval and verifies the
reservation releases while the copy is still pending. The failed evidence is
retained root-only; the verified whole-world snapshot was reset and the entire live
gate rerun successfully on the same runtime binaries. Only runner/test files changed
after the runtime archive; final runner hashes are recorded in
[environment.json](evidence/relay-r8a-2026-10-23/environment.json).

The accepted upgraded baseline was restored after testing: Ceres Clearing 49,000 cr,
Exchange operating capital 1,000 cr, no test purchases or pending traffic, two healthy
nodes/one pair, no restrictions/losses, world rate 24×. Redis-9 retains the root-only
snapshot with original expiry deadlines; dev2 remains PID 539467. No credentials,
configuration or raw database snapshot is committed. R7's completed CI run has only
the five reproduced and repaired failures described above; R8a's normal CI is
triggered by its main push and remains owner-monitored. Commit/push completes this
gate before automatic continuation to R8b.

## Gate 10 / R8b — subscriptions, Sovereign treaties and arrears

**Implementation/live gate passed (2026-10-24).** Base `ec669fb9`; runtime tree
`680568ec8100b69060d0c0e1ce61ff7facc4ee7c`. Commit/push follows this verified real-server
result, then the authorized train continues to R9a.

### Implementation

The Exchange now offers versioned daily subscriptions: Standard 25 cr, Priority
100 cr, and Sovereign 500 cr. A represented person or organization quotes terms
and separately authorizes the actual payer account. Enrollment creates an ordinary
World-IC economy obligation. Its first installment is due at enrollment; service
becomes eligible only after a real settled payment. Account choice never changes
sender or receiver identity.

One unpaid installment throttles; two suspend. The service projection reads the
actual obligation on every routing decision. A later successful installment does
not forgive old arrears. `+relay/cure` quotes the full unpaid amount and requires
the displayed missed/cured counters on confirmation. The economy journals one
balanced payment, retains its receipt for duplicate confirmation, and restores
only the payment-related lifecycle. Independent suspension holders, operator
incident sanctions and anathema survive. Cancellation ends future collection but
retains debt, and paying that debt cannot re-enroll a cancelled service.

Sovereign service requires an Exchange treaty with a recognized institution,
pinned through existing port/institution identities for Concord, Republic, Ceres
Freeport and Saturn Charter. An authorized representative enrolls against the
real treasury. Offers last 2–365 IC days; at least a full billing period must remain
when accepted. Terms are immutable through expiry. The economy checks the exclusive
end before posting, so a delayed observer or restart cannot charge an expired term.

All new data is bounded: 128 subscriptions/quotes, eight live quotes per issuer,
32 treaty counterparties, and 32 observer reads/eight transition notices per fire.
A derived economy index audits service-owned obligations at boot without walking
unrelated money records. Legacy obligations omit the optional service terms and
retain their previous wire shape. Missing owner records and forged obligation
terms refuse boot. Existing economic notices provide transition information;
help and the public board show prices, cancellation, arrears and cure commands.

### Validation

Local: 72 cumulative production scenarios (21 boot, seven IC-loop, 44 relay),
207 economy tests, 19 propagation tests, and 134 kernel contracts passed; the full
contract suite has eight explicit ignored tests. The final runtime correction
reran all 44 relay stories, all 207 economy tests, the panic-budget and ownership
contracts, and Clippy. The final runtime also passes all 21 port scenarios and all 44 relay scenarios; changed CI tests pass Clippy. Rustdoc for the changed public APIs passes with warnings denied.

Linux: 72 production scenarios, 207 economy tests, 19 propagation tests,
23 targeted kernel contracts including the panic budget, one capability-vocabulary
check, four two-process gateway tests, and the real Redis process-death test passed.
The additional CI reproductions cover the money-hold census, bridge and all 21 port scenarios.
Release build and web checks passed (zero Svelte errors/warnings). The final
compiled runtime passed the complete live gate and baseline restoration.

The live scenario checks actual TLS WebSocket and SSH entry points, scheduled
installments, Priority/Standard/conventional delivery changes, empty and unauthorized
payers, exact cure with a concurrent incident, duplicate confirmation, cancellation,
a real government treasury, inbound delivery to a titled government unit, treaty expiry,
notices, OOC channels and money conservation. It records six SIGKILL/restart
boundaries. Fresh boot/reboot uses previously empty dev1 Redis database 8; a
verified whole-world baseline is retained in database 7. Test payments and all
world changes are restored only after passing evidence is captured. The dev2 PID
must remain unchanged. Outbound titled-unit Sovereign dispatch is covered by the production-engine scenario. The server has no network puppet command; the live test does not claim to exercise one.

### CI repairs included

R8a run `40065228623` reported the same panic-budget regression in both kernel
and rest jobs, a stale three-entry protected money-hold census, and the bridge
fixture's old unpaid-Standard assumption. Removed the new production panic sites
without raising budgets; settlement resolves its destination before the money
post, and subscription terms are created with the original obligation. Updated
the census to include the real `relay.message` money hold. The bridge fixture now
uses the normal operator command to grant Standard courtesy, preserving its
arrival gating and duplicate suppression assertions.

The final TacSim integration job exposed three real old-lane/held-port boot failures.
Retail initialization now defers capitalization when the clearing port map is absent
or held for repair, preserving the existing repair path and every committed money
identity check. The normal five-second cadence initializes the market and its
subscription book after a real lane upgrade. The fourth port test now advances the
next capitalization tick before asserting that a settled world's reboot posts nothing.
All 21 port scenarios pass, including explicit no-funding-on-held-map and
upgrade-without-another-reboot assertions.

The final compiled archive includes all runtime, CI-test and harness corrections.
The web build stamp is the parent commit because this workflow commits only after
integration. Evidence records the exact tree, archive and executable hashes.
Three failed harness runs are retained on dev1: journal wording, a nonexistent
object-execution use of campaign @force, and the lowercase obligation display.
The superseded candidate's full logs and Redis snapshot are retained under
r8b-evidence/superseded-145e8d4. Database 8 was reset for the final candidate only
after preserving that complete RDB; database 7's verified baseline was reused.

[Acceptance metadata](evidence/relay-r8b-2026-10-24/metadata.json) records 101
authenticated TLS WebSocket commands, eight SSH checks and six actual process
crashes. Observed socket latency was 50 ms median, 52 ms p95 and 79 ms maximum;
this is gate observation, not a load benchmark.
[Live evidence](evidence/relay-r8b-2026-10-24/live.json) records all six passed
scenarios. The running server SHA-256 is
`b896c9efd1c0dc4b4b5553de949ee7a137239bab5a6c3a6d52d0917bda0d7f28`;
the archive/tree and gateway hashes are recorded alongside it. Running process
executables match the built binaries. Fresh database 8 boot/reboot passed; the
4,808-key database-7 baseline was restored with original expiry deadlines, empty
billing/purchase books, one capital receipt, two nodes and one pair. Both dev1
services are active, and dev2 remains PID 539467. No credentials, configuration
or database snapshots are committed. The milestone push triggers normal CI; its
long-running result remains owner-monitored while R9a begins.


## R9a — manufactured private pairs (2026-10-24)

### Implementation and ownership

`tacsim.relay.manufacture@1` provides a finite Lunar foundry and the gated
`private_pair` recipe: one relay blank and two instrument boards, 120 hourly IC
production units, and actual economy budget consumption. The eight-blank issue
is finite; boards must come from ordinary inventory/market supply. Existing
industry work orders, inventory lineage, title/custody and the action coordinator
produce the output. No staff pair-mint command substitutes for the recipe.

The bounded `tacsim.relay_fabrications` family records the consumed completed
work-order lot; inventory compares and consumes the exact lineage-bearing product.
The legacy TacSim foundry projection durably records two unique physical half IDs
before title issuance. Boot audits both directions and never remints a missing or
destroyed half. Duplicate collection returns the original pair. Actual consumed
budget, beneficiary authority, recipe, facility, quantity and destination are
revalidated before collection; listed output cannot be collected twice.

Each half is five tonnes and occupies a real carrier bay. The vehicle adapter
contributes inert payload to both departure gates and the frozen voyage ticket.
Loading/unloading requires local docked access and active, unstolen, unfrozen title;
generic get/teleport cannot bypass cargo custody. Installation requires a local,
owned empty mount, independent operator authority and two distinct rooms. The first
mount persists without an invalid partial registry binding. The second establishes
the private pair and both checked station units after preflight. Its capacity is one
unit per IC second and it does not depend on the Exchange. Physical discovery files
real intelligence and advances the Concord anti-proliferation front once per pair.

The real production story exposed a finite-schedule ownership conflict: the kernel
advanced a cursor while industry completion deleted it in the same action. The
schedule adapter now declares completion read-only immediately before preparation;
the kernel alone writes the advanced completed cursor. Industry completes its job,
and later delivery/cancellation prunes the cursor. Completed jobs cannot fire again.
The full authoritative coordinator and live industry producer exercise this path.

### Acceptance and repairs

Parent `0e8742a5834443ad4ee938871ebab3c970663acb`; accepted source tree
`745723b53e9c301cc8328a5d145fb7f9b3b2dd70`. The compiled runtime tree is
`1d4664e68bbe24765961c74c841264a1436ff317`; the recorded diff contains only the
live and SSH acceptance harnesses. Runtime, test sources, fresh boot and release
executables are identical. The commit additionally records this documentation and
the captured evidence. The web build stamp is the parent because commits follow
the live gate.

Linux passed 73 production boot/relay/IC scenarios, 91 industry tests, 11 core
scheduler tests, 23 ownership/contract checks and one real Redis observer regression
(199 total, zero failed or skipped). Clippy with warnings denied, release server and
TLS gateway, web build/check and local rustdoc with warnings denied passed. The
R8b CI run `40073317738` failed only the missile observer race: a shooter's receipt
does not imply the victim's separate SSH socket has already delivered its own hit.
The test now waits for that victim observation before draining, retaining all
negative leak assertions. The reproduction passed on Windows and Linux.

[Acceptance metadata](evidence/relay-r9a-2026-10-24/metadata.json) records 122
real TLS WebSocket commands, nine SSH commands and seven actual SIGKILL/restart
boundaries. [The full live transcript](evidence/relay-r9a-2026-10-24/live.json)
shows two paid completed work orders, interruption/restamp, stolen/destroyed cargo
refusal, three actual freight legs, first/second installation recovery, private mail
with the Lunar Exchange endpoint disabled and discovery surviving repeat/restart
without advancing twice. [The read-only material audit](evidence/relay-r9a-2026-10-24/material-audit.json)
checks two consumed blanks, four boards, consumed output lots, four unique halves,
two consumed budget holds and no live material holds. The frozen Luna–Deimos
payload includes exactly 10,000 kg of relay cargo.

The live harness was corrected at observed safety refusals: clock generation freezes
production until restamp without immediately changing the work-order label; ordinary
players cannot build empty mounts; a creator initially carries a new mount; boarding
adds a real occupant provision/fuel allowance; SSH wraps the public board. Failed
checkpoints remain on dev1. Resumes reused the paid outputs and completed journeys;
no direct data edit advanced production, minted cargo or teleported a loaded hull.
Builder fixtures only supplied empty player-owned mounts at the physical destination.

Socket latency was 50 ms median, 53 ms p95 and 78 ms maximum (observations, not a
load benchmark). Fresh Redis database 6 boot/reboot passed. The 4,808-key baseline
retained in database 5 was restored with original expiry deadlines after acceptance;
empty purchase/subscription books, original balances and the two-node/one-pair seed
were verified. Both dev1 services are active. Dev2 remains PID 539467. No credentials,
configuration or database snapshot is committed. The source/archive/process hashes
are in the evidence directory. Normal CI follows the push and remains owner-monitored;
the authorized next milestone is R9b.

## Gate 12 / R9b: Fleet programme and mobile half

**Implemented; automated and real dev1 integration gate accepted (2026-10-24).**
Base `17a93c7e0885ce031cd610aeef3565963b114a4f`. Candidate tree
`d910a66e25f55f5ece31fcbdfb8642a118b258b3` (Rust runtime identical to
`a6824ceabe3d337b5632c4eb90f35511d1d969a1`; only live harness corrections differ).

### Implementation

Lane v13 adds a finite Concord Fleet organization and one Halcyon relief cruiser,
Long Vigil. The checked capital hull has a real interior, stores, vehicle bay,
structural/power sections and a mobile relay socket. It begins without a pair.
Its command authority comes from the existing economic asset assignment, including
wall-clock expiry, lifecycle, title and frozen/stolen checks. The Fleet hold is an
ordinary organization vault; ordinary supply purchases and real voyage tickets apply.

A Half Afloat starts dormant at 0/8. An authorized treasury appropriation pays six
actual 10,000-credit installments to the Lunar yard, at least 30 IC days apart.
Insufficient available funds wait; delayed cadence cannot catch up multiple months
in one fire. The other two segments require an attributed Postmaster decision and
the actual installation of a paid manufactured pair. The Postmaster is a registered
Exchange principal. Commissioning permits service; the once-only first-launch canon
waits for an actual under-way journey, including after restart.

`+relayfleet/refit wo#<16-hex>` accepts the same identifier industry displays. Both
healthy, titled halves must be physically at the Lunar dock beside the idle cruiser;
installation moves them into checked stationary/mobile cradles. The mobile half adds
five tonnes to the frozen passage payload. Removal at Luna disconnects the pair and
returns both halves to the dock. Refit reuses the same cradles and halves, without
new front progress or a second launch event. Terminal loss cannot be erased this way.

The routing metric reads the actual frozen journey leg. Coverage extends aboard
and within 300,000 km along a known shared edge or endpoint, with outward rounding;
unrelated edges are unknown, never silently co-located. Routes recheck remaining
coverage legs. Carrier power loss interrupts service; carrier destruction kills the
half and permanently orphans its mate through R7's existing political/canon pipeline.

### Corrections found by the gate

- Seven legacy CI fixtures (four mushd, three TacSim targets) omitted the shared
  inventory handle required by the foundry boot audit. They now use production
  inventory wiring; validation was not relaxed.
- The presentation contract caught the non-fieldable Halcyon missing from the
  explicit non-arena list. It uses section rows, like the existing strategic hulls.
- Final command review caught decimal-only refit input. The implementation and
  both production/live scenarios now use the shared canonical work-order parser.
- Existing-world content upgrades are explicit. The live harness plans/applies
  lane v12 to v13 with the actual fingerprint, restarts, and checks unchanged relay
  and foundry identities. Restoration returns the exact saved lane-v12 baseline.
- Test setup uses minor currency units, real market settlement before long clock
  advances, and preserves historical lane-v11 fixture digests without Fleet content.

### Validation and acceptance

The final Rust runtime passed **662 Linux tests**, zero failures or skips: 75
production boot/relay/IC stories, 52 content/lane tests, two mobile metric tests,
91 industry tests, 11 scheduler tests, 23 architecture contracts, 19 propagation
unit tests and 389 tests across the seven previously failing daemon/TacSim targets.
Full Linux formatting, targeted Clippy with warnings denied, web build/bundle check
and release server/TLS gateway build passed. Local final production stories and
changed test-target Clippy also passed. Windows workspace-wide formatting exceeded
the OS command-length limit; changed files and the full Linux workspace passed.
The prior normal CI failure was run `40082594435`; its seven exact failed targets
were rerun in full after repairing their shared-inventory fixture composition.

[Metadata](evidence/relay-r9b-2026-10-24/metadata.json) records 84 TLS WebSocket
commands, 12 real SSH checks and nine actual SIGKILL/restart boundaries. The
[live transcript](evidence/relay-r9b-2026-10-24/live.json) proves explicit lane-v12
to v13 upgrade without changing the old relay/foundry identities; real paid
manufacture; consent/refit; funding persistence; commissioning before launch;
removal/remount; carrier power recovery; passage restart; onboard, outside and
nearby delivery; and terminal carrier loss. The original recipient-transfer claim
was invalidated by retrospective; see the acceptance-repair entry below. Repairing the
carrier afterward did not revive its destroyed half or append another first-launch
event. Public-board and permission checks passed through the real SSH front door.

The [material audit](evidence/relay-r9b-2026-10-24/material-audit.json) verifies
one consumed blank, two consumed boards, one completed paid work order, one consumed
budget hold, no remaining output lot, two unique physical halves and exactly
5,000 kg of relay payload in the frozen passage ticket. All credits were conserved.
The six actual treasury receipts are separated by 720, 720, 724, 720 and 720 wall
seconds: 3,604 seconds from first to last at the maximum supported rate 3600.
No receipt or clock anchor was synthesized. First-launch canon remained absent at
8/8 while docked and appeared only after actual departure.

Two harness defects were repaired at observed refusals, with failed evidence kept:
the atlas fingerprint is 16 hex characters, and industrial completion must wait for
the real production-budget hold to be consumed before claiming the output. Both
retries restarted the complete live scenario from the preserved baseline; no
production safety check was relaxed. Separate read-only fuel preflights confirmed
that the changing sky still left the voyage within the finite test budget.

Fresh Redis database 4 booted and rebooted with the same identities, balances and
dormant programme, with no manufactured pair or launch. The 4,814-key baseline in
database 3 was restored with original expiry deadlines: original lane v12, no Fleet
programme, no fabricated machines, and the original two-node/one-pair registry.
The candidate executable remains installed; both dev1 services are healthy and
dev2 remains PID 539467. Measured command latency was 50 ms median, 52 ms p95 and
204 ms maximum; this is a scenario observation, not a load benchmark.

Runtime and test-source provenance are recorded in the evidence; this commit adds
acceptance documentation and sanitized evidence afterward. Text-log trailing
whitespace is normalized for the repository. No credentials, server configuration
or database snapshot is committed. Normal CI follows the main push
and remains owner-monitored. The standing authorization continues to R10 after
this milestone is committed and pushed.

## Gate 13 / R10: full Luna star and campaign acceptance

**Implemented; automated and real dev1 integration gate passed (2026-10-24).**
Base `a517e74f02ad965cd602305cb00022ceee902a86`; tested runtime tree `1b5a6dd1906e2bbb9b3c1f90ad90e00ddfa81c70`; final live-harness tree `87c6d6f9bfaa260b4441c7a12429af74286aa403`.
Production story tests ran on `21a6f9b154c59a88c5c5aa9c4c8ec80d21dc2c47`; passed baseline suites on `4efb7d542134045745c8acb1705e298e4fd77b33` and custom-lane suites on `bf70b8c3cf83cf8233f4c0960f03dee1ddb9492b` are retained with their source identities. The recorded Rust deltas to the final runtime tree contain only test-fixture corrections; affected targets were rerun. The separately recorded live-harness delta fixes acceptance assertions and placement. Acceptance documentation and sanitized evidence are added after the tested tree.

The full public Luna star has five nodes, four Exchange pairs and eight physical
halves: Malapert, Ceres, Phobos, Kirkwood and Mercury. Malapert's authored compound
includes the public counter, Postmaster's office and restricted hall. Aitken keeps
Malapert coverage; Stickney and Voltaire use Phobos. Kirkwood and Mercury add
strategic relay sites without inventing trade ports or routes. Titan, Pallas,
Venus and Vesta have no public halves. Every public pair retains eight units per
IC second; private and mobile pairs remain absent on a fresh world.

The v2 content update recognizes the exact historical v1 authored slice. It keeps
the original operator, nodes, halves, availability, private additions, money and
in-flight timing. It only creates the missing public structures and halves,
moves the original Malapert structure, and expands authored coverage. A lost or
unpowered hub cannot acquire healthy spokes during migration. An unchanged R1
charter updates through the organization owner; a customized charter is retained.
Partial expansion, identity collisions, exhausted bounds or missing old structures
refuse the upgrade. Existing packs still require explicit staff plan/apply.

The one Postmaster and A Half Afloat front introduced in R9b are reused. The new
four-segment Terms of Service front observes typed billing and operator decisions.
A throttle records two segments; a suspension records four. One named principal
owns the case; repeats and unrelated principals do not advance it. Cure preserves
history and cannot erase an independent sanction. Cancelled subscriptions with
unpaid debt remain observable. Clock changes, attributed canon and bounded receipts
share the durable commit. Missing or contradictory receipts refuse reseeding.

The content boot path now permits explicitly opted-in transitive deferral only
through an earlier deferred pack with matching version and digest. This allows an
old station/lane world to boot for explicit upgrades without silently applying
those upgrades. Missing, held, mismatched, retired and newer dependencies still
refuse. Six invalid/valid dependency-chain cases pin the boundary.

Defects found before acceptance: obsolete one-pair/pack-count fixture assertions;
the transitive boot dependency ordering; test rosters that applied game content while omitting the lane catalog, unlike
production; and the old-world Fleet boot path attempting to create its programme
before the explicitly deferred Exchange existed. The latter now waits only when
no Fleet/Exchange identity has been applied. Existing held or inconsistent state
still requires repair.

The completed R9b CI run 40096036146 exposed 14 failures in nine TacSim targets.
The assertions now include Halcyon, its Capital class, the Fleet funding and lane
v13 changes. Two recovery fixtures had discarded staff-placement persistence
operations; retaining those writes fixes their real recovery-ledger gap without
weakening the audit. The complete local driving/mining suites pass. Historical/operator-edited lane fixtures now keep the compatible historical
relay seed, preserving their lane-specific refusal and upgrade tests without
weakening the full star's exact dependency checks. The disabled-server credit
fixture also uses production's content switch. Broader Linux reruns and live
results are recorded below. Failed-run evidence is retained.

The first live attempt reached full-star traffic and real hub-outage recovery,
then its repair command requested 1,000 power internals on a station bounded at
200. The station damage handler correctly refused it. The harness now restores 200, retains the
failed transcript and restarts the complete live campaign from the saved baseline.
No production bound was relaxed.

The second attempt passed the paid voyage but exposed a soak-harness lifetime
error: an already-delivered envelope can be reaped before a later poll. The
harness retains the accepted mail ID and reads the durable inbox instead. It
also verifies all 72 recipient copies of its 24 channel lines. That failure is
retained, and the complete campaign is restarted from the baseline again.

The first all-in-one launch ran under root rather than dev1's `mush` account.
The quarantine owner guard refused startup before opening the world. Its verified
empty write probe was preserved outside the spool, and the temporary service now
uses `User=mush` / `Group=mush`. The next route assertion exposed ignored teleport
refusals: Bob was still aboard after the voyage and Alice began in an old test
carrier. The earlier split soak is therefore retained as superseded evidence,
not accepted four-location coverage. Placement now uses the real staff unplacement
command, tolerates an already-unplaced actor, and verifies each stored location;
Bob also leaves normally after docking. Both deployment campaigns passed on the corrected harness.

### Validation and operational result

The candidate passed **1046 Linux tests**, with zero failures or skips, including production boot/relay/IC stories, the affected content and legacy CI suites, owner/contract tests and the real Redis process-fault contracts. Full Linux formatting, targeted Clippy with warnings denied, web build/bundle verification and release builds of `mush-server`, `mush-gateway` and `mushd` passed, with TLS enabled for the gateway and all-in-one server. The complete local driving/mining and plumbing suites also passed (74 tests).

The split server/gateway gate passed 308 line commands and 23 panel queries, with 8 real SIGKILL recovery boundaries. Its four concurrent clients sent 72 mails and 24 IC channel lines during the bounded soak. All 72 mails and all 72 channel recipient copies arrived; no expiry was observed. Observed maxima: 17 active envelopes, 5 on a link, 32 visited per explicit pass, 11 metric reads and 32 routing edges. No configured work limit was raised. Command latency was 51.0 ms median, 532 ms p95 and 1055 ms maximum; inbox polling upper bounds are in the metadata, not claimed as precise transport latency.

The all-in-one server gate passed 220 line commands and 23 panel queries, with 4 real SIGKILL recovery boundaries. Its four concurrent clients sent 72 mails and 24 IC channel lines during the bounded soak. All 72 mails and all 72 channel recipient copies arrived; no expiry was observed. Observed maxima: 13 active envelopes, 3 on a link, 25 visited per explicit pass, 11 metric reads and 32 routing edges. No configured work limit was raised. Command latency was 52.0 ms median, 318 ms p95 and 652 ms maximum; inbox polling upper bounds are in the metadata, not claimed as precise transport latency.

The split campaign proved explicit lane-v13/full-star-v2 migration preserving identities, operator funds, the existing Postmaster and frozen traffic; all four spokes and two distinct-hop receipts; Aitken/Voltaire coverage and uncovered Titan fallback; a real hub power outage; daily subscription payment, arrears, class changes, Terms history and independent-sanction cure; and an actual funded voyage with conventional onboard mail. The all-in-one deployment repeated traffic, outage, persistence and load checks. The real SSH front door passed 18 surface/permission commands.

Fresh Redis database 2 boot/reboot passed with the same five-node/four-pair identities, finite capitalization and dormant Fleet/Terms receipts. The original dev1 world was restored from the retained database-1 baseline with original expiry deadlines: lane v12, relay v1, two nodes/one pair, no Fleet or Terms programme and no manufactured machines. The following R10 boot adds its six new Atlas geography packs, while lane/relay upgrades remain explicit. Both dev1 services are active; dev2 remained `MainPID=539467`. Installed executable hashes, retained failures, transcripts and counts are in [acceptance metadata](evidence/relay-r10-2026-10-24/metadata.json). Credentials, server configuration and database snapshots are excluded.

R9b CI run `40096036146` finished with failures in nine TacSim targets; the final R10 candidate repairs those fixtures and reruns their affected suites. Normal CI for this push remains owner-monitored and is not claimed green. The earlier live Discord-provider criterion remains open because dev1 has no enabled bot/mapping; controlled-provider bridge tests remain the existing evidence. The R1–R10 implementation sequence is complete; this is the stop for programme acceptance.


## Acceptance repair — R10 CI and R9b recipient transfer

### Correction to the original acceptance record

The retrospective of R10 commit `57c5747dc02370e6854188e009dda61b9108df94`
found that R9b's `nearby-coverage-and-recipient-transfer` milestone did not prove
transfer. The pilot's teleport was refused because they were aboard Long Vigil;
the harness ignored that response, and the delivered envelope recorded zero
forwardings. The original nearby/outside-coverage evidence remains valid, but the
recipient-transfer assertion is superseded by the corrected gate recorded here.

R10 normal CI run `40110674650` subsequently completed with four failures: three
TacSim library tests booted the full relay content with TacSim's game definitions
disabled, and the help-command contract found the nonexistent `+relay/rates` switch.
The TacSim integration shard and CodeQL passed. The repair uses one `game_content`
value for both sides of the centre fixture; production's missing-receipt guards
remain intact. Help and the startup hint now point to `+relay`, whose existing board
includes retail and subscription tariffs. Current help also describes terminal
pair destruction and the full-star Malapert compound instead of R1's initial state.

### Corrected gate

After normal docking, the harness moves the nearby shore recipient from Deimos to
Ceres using supported staff placement, then verifies the stored location and the
transport's resolved destination. It requires an increased forwarding count and a new physical flight,
checks the frozen due time across SIGKILL, rejects early inbox visibility, and checks
one inbox entry again after delivery and another restart. Moving coverage and restart
in passage are proved before docking; the pilot stays aboard until the later normal
disembarkation check. This tests a recipient leaving mobile coverage;
it does not add an underway carrier transfer or ship-to-ship EVA feature.

The complete paid campaign reran the six real installments, manufacture, physical
refit, commissioning, removal/remount, power-loss recovery and actual voyage. Its
strict transfer assertion then exposed that staff unplacement, like teleport, refuses
to remove an embarked actor underway. That failed attempt is retained. A targeted
continuation uses the same funded world after its real voyage docks, and the final
harness's exact helpers and transfer/lifecycle block; no reset, payment, manufacturing receipt
or clock anchor is fabricated. One intermediate continuation caught a stale assertion
that checked the pilot instead of the moved observer; that failure is also retained.
A second attempt reached delivery but an inbox command used a WebSocket captured
before restart. The final block reads the reconnected observer client after each
restart, and tests recipient relocation after normal docking.

Prior startup failures and an interrupted preflight attempt are retained separately.
The shared harness now checks Node >=22 and permits an explicitly scoped
acceptance-repair evidence subdirectory, preserving the original gate evidence.

### Validation result

The final continuation passed. Recipient #679 moved from Deimos room #261 to Ceres
room #101; the resolved destination changed accordingly and forwarding increased
from 0 to 1. The physical flight lasted 1,813 IC seconds, with frozen due time
74,183,962,179. There were 339 pre-arrival checks; inbox visibility was observed at
74,183,962,241, after the due time. Both the in-flight restart and a second restart
after delivery preserved exactly one inbox entry. Normal pilot disembarkation,
installation disable/enable, carrier destruction, terminal loss after hull repair,
once-only launch canon and total-money conservation passed. The acknowledged transport
envelope is retired after restart; the saved in-flight milestone retains its route.

The paid campaign and final continuation cover 11 accepted-path SIGKILL boundaries;
two interrupted continuations add two observed crashes, recorded separately rather
than counted as successful complete runs. No funded checkpoint restore was needed.
The original verified dev1 baseline was restored with its expiry deadlines: lane v12,
relay v1, two nodes/one pair, no Fleet/Terms programme or manufactured machines. HTTPS,
gateway and Redis health passed, and dev2 remained `MainPID=539467`.

Linux validation passed **1,564 tests**, with zero failures and one pre-existing
ignored library test: 1,480 TacSim library, four help contracts, 21 boot-parity,
ten IC integration and 49 relay integration tests. Full formatting, TacSim
library/test Clippy with warnings denied, release server/TLS gateway builds and
Node syntax checks passed. The 1,448 crate/Cargo/help inputs and final harness hashes
match the VPS. The runtime build did not change during the harness corrections.
See [acceptance metadata](evidence/relay-acceptance-repair-2026-10-24/metadata.json)
and [retained transcripts and procedure](evidence/relay-acceptance-repair-2026-10-24/README.md).
Normal CI for the follow-up push is a separate result and is not claimed green here.
RR-1 and RR-4 through RR-7 remain follow-up work; this closes the requested acceptance
repair and current-help correction, not unconditional acceptance of every review finding.


## Availability/privacy follow-up gate — 2026-10-24

The owner authorized this unit after the acceptance-repair push `1a0565cd`.
[Design 15d](15d-comms-availability-privacy.md) records the implementation contract.
The gate closes retrospective RR-1 and RR-4 together with explicit organization
speaking, while retaining the existing scheduler and source-owner handoff.

Admission now counts source reservations and active envelopes once, limits an
actor to 32 pending copies, shares a further 32-copy allowance across explicitly
represented officials, limits mail/channel producers to 48 each and 64 combined,
and reserves 16 of the unchanged 128 slots for acknowledgements/return receipts.
Mail stays charged to its original sender through the return receipt. Refusals are
typed and occur before source mutation. Private pairs have independent discovery
and use grants; both endpoints need use authority. Public carriage is explicit,
and manage alone cannot publish. Commands, boards and receipts filter private
knowledge. Scoped `relay.speak` agency is frozen at acceptance and revalidated at
transmission independently of the payer; organization/scope ID zero remains valid.

The acceptance-repair CI run completed with all required jobs passing except the
isolated bot arena fixture's content/runtime mismatch. This unit repairs that
fixture consistently with the earlier TacSim repair, without weakening the
Fleet/Postmaster guard. The full isolated bot suite passed 717 tests, seven ignored;
CodeQL and the prior run's other required jobs passed.

Final Linux validation passed **2,490 tests**, zero failures and eight pre-existing
ignored tests: 57 relay integration, 205 producer/transport, 31 boot-parity/IC,
1,480 TacSim library and 717 bot. The focused 19-test bot arena run is included in
that full suite, not added twice. All but the bot suite reran after the final
zero-ID correction. Full formatting, strict all-targets Clippy for the changed
production modules/server, release server build and harness syntax checks passed.

The real dev1 gate passed **150 checked commands and four SIGKILL/restart
boundaries**. It loaded the genuine six-payment R9b checkpoint through normal
Redis RESTORE with AOF enabled, made a checked finite 800,000-minor voyage payment,
bought real stores and sailed the commissioned carrier. It proved private-link
opacity, both-endpoint authority, queued revocation, frozen and revoked agency,
zero-based identity persistence, actual public-carriage inbox arrival and private
receipt filtering. One ordinary sender's 32 unresolved copies survived restart
while unrelated order, acknowledgement and IC channel traffic actually arrived.
The first successful 142-command/three-crash pass and earlier failed harness/setup
attempts remain preserved separately.

All 1,492 crate/Cargo/help/harness inputs match the checkout after CRLF normalization.
The tested and restored running server SHA-256 is
`f4e40f4af9a869c745dd669398c8f49e0a1a0e589357a2e330ec5a6a703000aa`.
The original dev1 baseline/config was restored: 4,888 keys, two nodes/one pair,
no private pair, HTTPS 200. Dev2 stayed at gateway PID 539467.
[Evidence and procedure](evidence/relay-availability-privacy-2026-10-24/README.md)
and [metadata](evidence/relay-availability-privacy-2026-10-24/metadata.json) contain
the exact result and source identities. The new push's normal CI is separate and
is not claimed green by these results.

Upgrade behavior: existing private pairs default closed until authority is granted
or public carriage explicitly opened. Exchange charter links remain public. This
unit does not begin the policy/bandwidth gate, add new gameplay consumers or create
organization inboxes; RR-5 through RR-7 and live Discord acceptance remain open.


## Policy/bandwidth follow-up gate — 2026-10-24

The owner authorized RR-5 through RR-7 after availability/privacy commit `749e0f98`.
[Design 15e](15e-comms-policy-bandwidth.md) reconciles hub admission and per-pair
physical capacity. Authorized operators now set versioned, attributed per-link
schedules through a checked API/command, preserving accepted delay/window/size terms.
An envelope incurs admission once per operator; each traversed pair spends its own
capacity. Message size, refill rate and burst are independent. Partial serialization,
first-unit payment capture and capacity share durable boundaries, with no early
recipient visibility. Policy updates cannot mint tokens. Spent courtesy traffic
cannot be purchased after transmission begins.

Validation passed **2,086 passing automated tests, 3 pre-existing ignored tests, 93 checked live commands and 6 SIGKILL/restart boundaries**. Full formatting, strict all-targets Clippy, help
contracts, release server build and harness syntax checks passed. The live gate
proved frozen admission across two spokes, one real retail debit across a partial
crash and final arrival, and a multi-unit message on the genuinely commissioned
one-unit private hardware after a real funded voyage. No receipts or hardware were fabricated.
All 1,495 build/help/harness inputs matched. Tested and restored server
SHA-256: `6bdf7529856b6d00d5e27027da9373f62539b7b7c4e16f2a7aa6919dcdf9f8a4`.

The saved dev1 database/config was restored and healthy (HTTPS 200,
2 nodes/1 pair, no private installation); dev2 stayed at PID
539467. [Evidence and retained attempts](evidence/relay-policy-bandwidth-2026-10-24/README.md)
and [metadata](evidence/relay-policy-bandwidth-2026-10-24/metadata.json) provide exact
identities and counts. This closes **RR-5, RR-6 and RR-7**. Further consumers,
aggregate hub contention and live Discord acceptance remain outside this gate.
The preceding availability/privacy CI also exposed a stale capability-vocabulary
count; this gate explicitly asserts discover/use/speak and passes the full 302-test
server library suite. No production permission check was weakened.
Normal CI for the new push is a separate result. Pause after publication.
