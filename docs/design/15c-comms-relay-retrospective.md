# R1–R10 communication relay retrospective

Review date: 2026-10-24. Reviewed commit: `57c5747dc02370e6854188e009dda61b9108df94`.
Scope: design 15, implementation plan 15a, cut/evidence ledger 15b, production
transport and composition, consumer integrations, acceptance scripts and CI.
This is a review and proposed follow-up programme, not implementation authorization.

## Acceptance repair follow-up

The owner authorized acceptance repair after this review: fix R10 CI, rerun the
R9b recipient-transfer criterion on dev1, update the evidence, then commit/push and
pause. The findings below describe the reviewed R10 commit, not the later repair.
[The cut ledger](15b-comms-relay-cut-notes.md) records the repair result and source
identity. Admission fairness, private-link permissions, congestion policy, hub timing,
message-size behavior and additional consumer interfaces remain separate follow-ups.

Current disposition after dev1 acceptance repair: RR-2 now has valid recipient-transfer
proof, including frozen flight timing and exactly-once inbox delivery across restarts.
RR-3's four failures are repaired and all affected local/Linux suites pass; the new
push's normal CI remains a separate result. RR-8's current help is corrected. RR-1
and RR-4 through RR-7 remain open. The live transfer is a shore-recipient relocation
after real docking, with moving coverage and passage restart proved earlier.

## Availability/privacy follow-up

The owner subsequently authorized gate 2 after the acceptance-repair push.
[Design 15d](15d-comms-availability-privacy.md) records the admission, private-access
and explicit organization-agency contract and its integration gate. It passed 150
checked live commands and four SIGKILL/restart boundaries on dev1, with 2,490
passing automated tests and eight pre-existing ignored tests. **RR-1 and RR-4 are
closed by this follow-up.** Private pairs default closed on upgrade; Exchange
charter links remain public. The explicit sender identity is separate from payer
and receiver authority.

The acceptance-repair push's CI subsequently exposed the same fixture flag mismatch
in the isolated bot arena. That fixture is repaired here; its full 717-test suite
passes. Every other required job and CodeQL on that prior push passed. CI for this
new push remains a separate result. The findings below retain the reviewed R10
behavior; RR-5 through RR-7, further consumer interfaces and live Discord acceptance
remain open.

## Policy/bandwidth follow-up

The owner subsequently authorized RR-5 through RR-7. [Design 15e](15e-comms-policy-bandwidth.md)
records the reconciled model: physical capacity per pair, admission once per operator,
versioned attributed schedules and durable multi-unit serialization independent of
message size. The gate passed 2,086 passing automated tests, 3 pre-existing ignored tests, 93 checked live commands and 6 SIGKILL/restart boundaries. The real dev1 run includes a commissioned
one-unit private link, real first-unit payment capture, crashes and actual inbox
arrival. Dev1's baseline was restored and dev2 unchanged.

**RR-5, RR-6 and RR-7 are closed by this follow-up.** The findings below retain the
reviewed R10 behavior. Additional consumer interfaces, a separately designed aggregate
hub bottleneck and live Discord acceptance remain future work. See the
[acceptance report](evidence/relay-policy-bandwidth-2026-10-24/README.md); publication
CI remains a separate result.

## Assessment

The core chain is built. It is substantially more than a registry or a deferred
integration exercise: orders/reports/acknowledgements, all IC mail, and IC channels
share a persistent transport; actual economy and industrial assets drive service
and private/mobile installations; destruction and campaign consequences have real
consumers. Retain that architecture and the per-milestone live integration stops.

Do not yet label the programme unconditionally accepted. The review reproduced an
admission-starvation defect, found a false-positive R9b live criterion, and identified
privacy/policy gaps and several material differences between the setting design and
runtime behavior. R10 normal CI also has three library-test boot failures and one
help-command failure. Live Discord acceptance remains explicitly open.

Three isolated characterization probes passed on the VPS build cache. This means
the reported behaviors were reproduced, not that they are correct. The six reviewed
source files used by those probes match the local R10 tree by SHA-256. The probes
used in-memory worlds and did not change live dev1. See
[evidence and reproduction notes](evidence/relay-retro-2026-10-24/README.md).

## Prioritized findings

### RR-1 — P1: one ordinary sender can exhaust admission for unrelated traffic

`dispatch::has_capacity` counts all accepted and reserved source copies against one
128-route limit; mail allows 64 pending letters / 128 copies without a per-principal
pending allowance. The order admission path shares that global limit.

A normal player sent 64 letters to two unresolved recipients each. After handoff,
an unrelated `+order` was refused with `Transport pending queue is full; try again
later.` No staff authority was needed by the sender. Unresolved traffic can retain
capacity until endpoint resolution or expiry; envelope TTL is 30 IC days. Queue
aging cannot help an order that was never admitted. This is an availability problem
across mail, channels and tactical traffic, not merely a busy-link delay.

References: `crates/mush-propagation/src/dispatch.rs:62`,
`crates/mush-mail/src/transit.rs:165`,
`crates/mush-propagation/src/routing.rs:547`.
Probe: `retro_mail_reservations_block_other_producers`.

Recommended correction: bounded per-principal admission, explicit producer/control
reserves, and typed backpressure with retry advice. Keep the global cap. Test
saturation by one issuer, multiple issuers, unresolved addresses, and returning
acknowledgements while unrelated orders remain usable. Do not solve this by simply
raising limits or allowing clients to self-designate privileged traffic.

### RR-2 — P1 acceptance: R9b did not actually test recipient transfer

The R9b harness issues the pilot teleport without checking its result, waits for
mail delivery, and records `nearby-coverage-and-recipient-transfer` as passed.
The retained server reply explicitly refuses the teleport because the pilot is
aboard Long Vigil. The recorded delivered envelope has `forwardings: 0` and still
targets the ship room. Delivery succeeded without the advertised transfer.

References: `scripts/relay-r9b-live.mjs:89`,
`docs/design/evidence/relay-r9b-2026-10-24/live.json:518` and `:1265`;
required criterion: `15a-comms-relay-implementation-plan.md:463`.

The nearby/outside-coverage tests are separate and remain useful evidence. This
finding does not establish that runtime forwarding is broken; it establishes that
this mandatory real-server criterion has no valid proof. R10's improved placement
helper and ordinary voyage tests do not rerun a recipient leaving a commissioned
mobile relay's coverage.

Recommended correction: reopen this single criterion; require successful movement,
assert the authoritative endpoint changed before arrival, observe the new forwarding
leg and its not-before timestamp, and verify one inbox delivery after restart.
Correct the acceptance ledger after a real dev1 rerun.

### RR-3 — P1 acceptance: R10 normal CI fails boot and help tests

The completed `test (tacsim-lib)` job reports 1,477 passed, 3 failed, 1 ignored.
The failures are the two centre readiness tests and lobby capacity test that use
`match_::centre::tests::whole_centre`; all panic during boot with
`Fleet/Postmaster receipt missing`. R10 repaired the previously enumerated R9 CI
failures, but that does not make this new run green.

The later completed rest shard also failed one help-command contract: `relay.md`
advertises `+relay/rates`, which is not registered. The existing `+relay` board
already displays tariffs. The startup readiness hint repeats the same bad command.
The TacSim integration shard and CodeQL passed.

The immediate dependency for the boot failures is `relay_world::boot`: an applied full-star pack requires
a mobile-programme/Postmaster receipt. `relay_mobile::boot` can return successfully
without one when the Fleet port or hull is absent. Inspect this partial-composition
contract before choosing whether to fix the fixture, seed dependency declaration,
or boot behavior; do not silently suppress damaged receipts in a real world.

References: `crates/tacsim/src/match_/centre.rs:475`,
`crates/tacsim/src/relay_world.rs:116`, `crates/tacsim/src/relay_mobile.rs:142`.
[CI job and complete log](https://github.com/conclavio/rustmush/actions/runs/40110674650/job/120921544228).
The relevant library failure excerpt is retained with this review; the later rest-shard
failure is recorded with the acceptance repair. No correction was made during the
review itself.

### RR-4 — P2: private pairs have neither positive use authorization nor opaque discovery

The public board filters private pairs, but direct `+relay/status <id>` first
resolves any registry pair and reports service before checking an operator capability.
An unprivileged outsider received `independent: Served.` for the fixture's private
pair and `No such public link.` for an absent ID. The routing planner selected that
pair for the same outsider with no grant: `service::allowed` checks sanctions and
negative restrictions, not permission to use a private network.

References: `crates/mush-propagation/src/service.rs:547` and `:660`,
`crates/mush-propagation/src/routing.rs:421`,
`crates/tacsim/src/relay_service.rs:223`.
Probe: `retro_private_access_disclosure_and_size` uses a controlled persisted
independent-pair fixture with the real planner and command handlers.

This does not prove outsiders can read payloads. It means the current meaning of
private is independently owned, not access-restricted or undiscoverable. The design's
private-network language and distinct discovery/publication acts need a clear rule.
I recommend separate discover, use, manage, inspect and publish rights; private use
should require a grant, with public carriage an explicit operator choice. Unknown
and undisclosed IDs should have the same player-facing result. Enforce the same
policy at planning, pre-transmission checks, status and receipt projection.

### RR-5 — P2 plan gap: declared congestion is global seeded policy, not operator control

The published board and timestamp evidence exist, as do classes, real capacity,
aging and Deferred windows. However, `service::Config` is one global policy supplied
by `RelayService::seed`; there is no per-link declared-congestion record or authorized
operator setter. Private operators also inherit that policy. A seeded 60-second
Standard delay demonstrates a dishonest board, but cannot implement the per-link,
operator-set posture described in design 15 section 5.2 and R3's scope table.

References: `crates/mush-propagation/src/service.rs:109`,
`crates/tacsim/src/relay_service.rs:165`, `docs/design/15-comms-relay-backbone.md:214`.

Recommended correction: versioned operator/link policy with attributed changes,
explicit admission-delay/window/burst settings, and preservation of already accepted
commercial terms. A scheduling mind can later drive this same checked API. It should
not be necessary to build a new mind merely to make operator policy editable.

### RR-6 — P2 design reconciliation: two spokes impose two declared waits

The design repeatedly says two hops, one Luna queue and corridor-wide shared hub
congestion. Runtime queues and capacity are keyed by pair, and every subsequent
pair gets a fresh service ticket and Standard wait. The probe observed Ceres–Luna
queued at 0, transmitted at 60; Luna–Mercury queued at 60, transmitted at 120.
There is no separate aggregate Luna hub resource.

References: `docs/design/15-comms-relay-backbone.md:118` and `:172`;
`crates/mush-propagation/src/routing.rs:887` and `:1059`.
Probe: `retro_two_spoke_standard_waits_twice`.

This is a waiting-time/model mismatch, not evidence of a duplicate retail debit.
The implementation notes already describe per-pair capacity. Decide which model is
intended and reconcile the setting, help and code. My preference is physical capacity
per traversed pair, with a separately defined hub/operator admission delay rather
than accidentally repeating the same declared delay. Add a shared hub bottleneck
only if corridor-wide cross-link contention is intended gameplay.

### RR-7 — P2 product limitation: bandwidth also acts as a maximum message size

The planner excludes a pair when the message requires more units than that pair can
transmit in one IC second. Both manufactured/private and mobile pairs are seeded at
one unit per second; one unit is 128 accounted content bytes. Consequently a
129–1,024-byte message that is otherwise short enough for relay takes conventional
transport, instead of spending several seconds on the narrow link. Mail accounting
includes its content fields, not just the visible body. The probe confirms a one-unit
message selects the private pair and a two-unit message cannot.

References: `crates/mush-propagation/src/routing.rs:438`,
`crates/tacsim/src/relay_foundry.rs:1185` and `:1481`.
This is a documented R2 simplification (`15b:225`, help `relay.md:171`), not hidden
unfinished fragmentation work. It becomes much more consequential with R9's narrow
links and future telemetry/market producers.

Recommendation: separate maximum message size, burst allowance and sustained rate.
Persist serialization progress/reservation across IC windows and crashes, exposing
the payload only after complete arrival. Internal multi-window service need not add
user-visible fragments or bulk streaming. Alternatively retain the limit and describe
private relay as a tiny-message service explicitly.

### RR-8 — P3: current help and completion claims need reconciliation

`docs/help/relay.md:161` still says R1 has no destruction or replacement and the next
paragraph describes the initial Aitken service-spine installation. R10 supplies
structural loss and moves the endpoint into the Exchange compound. Historical cut
notes can preserve earlier states; current player help should describe today's game.
The blanket R9b transfer-pass claim must also be qualified until RR-2 is closed.

## Plan-to-build reconciliation

| Milestone | What is present | Remaining qualification |
|---|---|---|
| R1 | Bounded versioned registry, installed objects, coverage and boot reconciliation | Complete core; preserve explicit upgrade and damaged-state refusal contracts. |
| R2 | One persistent routing pass, bounded graph walk, capacity, conventional fallback and recovery | RR-1 admission isolation; RR-7 one-second payload limit; per-pair versus hub semantics. |
| R5 | Journey-derived moving endpoints, frozen flight times and forwarding | Keep real relocation assertions; mobile transfer criterion remains open. |
| R6a | All mail, including reply/forward/quick paths, uses IC transport and arrival-gated visibility | No OOC mail escape hatch. Shares the admission problem. |
| R6b | Per-recipient IC-channel arrival/history and observer-safe bridge integration | Controlled provider tests passed; live Discord still untested because dev1 has no enabled bot/mapping. |
| R3 | Classes, entitlements, fair draining, timing receipts, evidence/publication and scandal consequences | Per-link mutable declared congestion absent; double-wait semantics need a decision. |
| R4 | Endpoint service states, scoped management, sanctions, audited restrictions and queued-hop rechecks | Private use/discovery and explicit human acting-for-organization identity need completion. |
| R7 | Structural loss, permanent pair death, canon/faction/insurance consumers | Staffing/availability is an explicit state, not a staffing simulation; no general strategic combat UI is implied. |
| R8a/R8b | Real holds/settlement, paid per-message service, subscriptions/treaties and arrears | Product/tariff evolution and broader organization dispatch APIs remain future work; no second ledger needed. |
| R9a | Finite manufacture, title/custody, real freight and checked installation | Initial finite foundry is a vertical slice; positive private use policy and useful size/rate behavior need decisions. |
| R9b | Funded Concord programme, real refit, moving coverage, removal, power and terminal loss | RR-2 acceptance hole; checked mobile capability is restricted to Halcyon and the single programme. |
| R10 | Five-node/four-pair/eight-half star, shared Postmaster/fronts, upgrade and split/all-in-one acceptance | RR-3 CI; earlier open criteria survive the final gate. Bounded short soaks are not endurance certification. |

Two broader setting promises were not delivered as reusable interfaces:

- **Market ticks and remote economic information.** Design 15 names market ticks,
  stale prices and lost contracts, but the dispatch kinds are Order, Report,
  Acknowledgement, Mail and Channel. The cross-owner adapter merges only mail and
  comsys intents. No market producer is wired into this path. Treat this as adjacent
  consumer work, not a missing R6 mail implementation.
- **Operator interception.** Design section 6.1 says the operator can read traffic;
  the implemented inspection/evidence paths expose authorized timing/load facts,
  not a general payload tap. Earlier T15 text puts interception later, so the documents
  conflict about scope. Explicitly defer or design an audited observer-safe tap; do
  not mistake aggregate load inspection for content interception. Encryption and
  forged orders remain expressly outside this programme.

Other explicit non-goals should stay non-goals: bulk attachments/couriers, general
player unit construction, unlimited foundry replenishment, detailed uplink geometry,
and travel gates. Their absence is not evidence that R1–R10 was left unfinished.

## What I would change in retrospect

1. **Keep the owner boundaries and single delivery clock.** Payloads remain with mail
   or comsys; durable references, terminal receipts and idempotent consumers bridge
   owners. Strategic geometry is supplied through an adapter. Real economy/industry
   APIs prevented a pretend-payment or free-manufacture shortcut. These choices paid off.
2. **Specify adversarial admission and privacy before adding producers.** The current
   bounds keep work finite but do not distribute availability fairly. A link being
   physically reachable is also not sufficient to decide who may use or learn it.
3. **Separate physical transport, operator products and campaign scripting sooner.**
   `service::Integration` now mixes identity, policy, billing, evidence and story
   callbacks; mobile eligibility consults the single Fleet programme. Split narrow
   interfaces when closing the concrete gaps, without replacing the working engine.
4. **Make optional seed dependencies explicit.** A generic full-star transport pack
   should not accidentally require one campaign hull to make a simulator test boot.
   Distinguish valid partial composition from missing/corrupted durable authority.
   Centralize reusable production test composition rather than copying growing rosters.
5. **Treat successful commands and verified postconditions as different assertions.**
   Movement requires endpoint confirmation; manufacture requires conserved inputs and
   actual output; delivery requires authorized inbox/history and timing. The R9b false
   positive is the clearest process defect. Never let a transcript label substitute
   for a checked state transition.
6. **Strengthen targeted load tests before calling the system operationally ready.**
   R10's four-client, roughly half-minute traffic bursts usefully exercise bounded
   concurrency. Add sustained churn, producer starvation, full retention tables,
   repeated account lifecycles and multi-link contention with measured per-fire costs.
   Preserve failed attempts and source identities, as the current evidence does.
7. **Use deterministic time for the broad cadence matrix, real time for selected
   boundaries.** The hour-long real Fleet funding run is valuable evidence. Keep a
   small number of genuine cadence/process tests and test combinations with the
   injected clock; do not fabricate financial or industrial receipts to save time.

## Interfaces to provide next

| Priority / interface | Consumers and contract |
|---|---|
| First: checked dispatch and source registration | Extend the existing handoff, not a second bus. Register producer/schema and bounded intents; use stable idempotency keys, owner-held payloads, frozen byte accounting, deadlines and typed terminal outcomes. Orders retain their special supersession semantics. Backpressure must distinguish issuer quota, producer reserve and global capacity. |
| First: explicit represented principal and private-network policy | Human officials, faction agents, ship/unit controllers. Carry actor, represented principal and authorization scope separately from payer and endpoint. Verify scoped authority; never infer it from membership or payment. Current `principals()` makes characters personal and titled non-actors speak for owners; buying an organization subscription does not itself let a human send official organization mail. |
| First: observer-safe communication knowledge | Minds, NPC plans, web/TUI, map overlays and logistics. Supply known route/service state, observation time, quote/estimate with uncertainty, and delivered/failed knowledge for that observer. Separate transport truth from what the sender has learned; a remote delivery acknowledgement must itself arrive. No direct registry or global receipt reads by AI/UI consumers. |
| Next: economic and industrial information streams | Market bulletins, price snapshots, contract notices, production/mining reports, supply orders and distress requests. Include source sequence, issued/received timestamps, freshness and expiry. Delayed information changes decisions; authoritative ledger/market execution remains with its existing owner. Start with one market bulletin and one logistics order/report consumer. |
| Next: typed lifecycle facts | Canon, factions, industry, insurance and operations. Stable identities for commissioned, disabled, restored, destroyed, service-consumed and terminal-dispatch events; replay-safe consumers. Facts about world truth and their IC announcements are distinct. Avoid consumers parsing command text or reaching into relay-private records. |
| Next: generic installed mobile capability | Travel, refit/repair, future capital hulls and mining fleets. Resolve socket, installed half, carrier, power and coverage through checked definitions and installation records. The Concord campaign grants/funds the installation; the generic capability does not depend on one story's identity. |
| Alongside these: operator policy and diagnostics | Audited per-link policy revision; observer-filtered estimates; staff/operator metrics by producer, principal, class and link. Measure reservation age, refusals, expiry, retries and work, while preserving payload and topology privacy. |

Do not put every economy mutation or global canon record through IC mail. Preserve
immediate authoritative state changes and explicitly classify which observations,
notifications and commands must transit. That boundary is particularly important for
minds: an agent must not react to a remote event before learning it.

## Proposed follow-up gates

These are recommendations, not work started by this review. Continue the established
pattern: focused contracts, real dev1 integration, inspect results, then commit/push.

1. **Acceptance repair:** resolve the three R10 boot failures; rerun the actual mobile
   recipient transfer; reconcile the ledger/help. Complete live Discord only when a
   real configured provider is available, or keep that criterion explicitly open.
2. **Availability and privacy:** close RR-1 and RR-4 together with explicit represented
   principals; demonstrate that one sender cannot prevent another's orders and that
   unauthorized actors cannot discover/use a private pair.
3. **Policy and bandwidth:** decide hub semantics; implement mutable per-link declared
   policy and independent message-size/rate/burst rules. Test two-spoke timing and
   a multi-unit narrow-link message across a crash without early content visibility.
4. **First additional consumers:** expose the checked producer/knowledge interfaces;
   integrate one remote price bulletin and one industry/logistics report end to end.
   Test stale information and delayed acknowledgement, not only eventual delivery.
5. **Generalize mobile installations when another hull needs them:** extract the
   campaign-independent capability behind the already proven physical lifecycle.

The next work should close acceptance and shared-service contracts before adding
more relay-dependent gameplay. The engine does not need a rewrite; it needs a small
number of explicit boundaries and stronger evidence for the cases above.
