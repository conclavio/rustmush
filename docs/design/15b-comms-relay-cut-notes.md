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
