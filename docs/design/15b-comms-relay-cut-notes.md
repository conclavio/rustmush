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
Full §5 acceptance remains pending normal candidate CI. **Stop before R5.**

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
targeted and live results do not substitute for it. No later milestone is started.

R2 implementation and deployed integration are complete. Full acceptance remains
pending any CI criterion explicitly marked outstanding. **Do not start R5, R3 or
R6 until the owner reviews this gate and authorizes continuation.**
