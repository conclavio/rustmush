# K6 look-back — tactical authority, control, contacts, and projection

**Draft 0.1 — 2026-09-04 — reviewed on merged main at `b0e3b52`.**

Status: the K side's closing input to the E5/K6 bilateral review required by
`09-rules-engine-contracts.md` under "Integration gate before K7". This memo distinguishes
production authority from contract fixtures, records the remaining owners agreed in
`08i-e-k-dependencies.md`, and freezes what "K6 complete" may mean to later plans. It does not
claim the unfinished production migrations or presentation wiring as K-delivered.

## 1. Executive result

K6 established the reusable kernel contracts and exercised the difficult transaction and
projection shapes, but it did not deliver a complete production tactics game loop.

- Production-ready kernel/state: control leases and safe-boundary handoff transitions; private
  contact state and lifecycle; opaque-contact resolution; cross-owner revision guards; durable
  action windows; deterministic simultaneous-intent collection and resolution.
- Production-registered first authorities: `mush-sim` owns the control-lease family and handoff
  actions; `mush-tactics` owns the contact family and its lifecycle actions.
- Fixture-complete proofs: atomic source/target tactical mutation, production-shaped unit and
  contact projections, cross-owner placement guards, simultaneous fire, and recursive leak tests.
- Not production-complete: tactical-unit typed persistence, placement typed persistence,
  `unit.status` authorization/feed wiring, transport renderers, sensor-policy ingress, contact
  compaction, controller-selection policy, and a production `DomainTick` driver.

Consequently, later documents may say "the K6 kernel contracts are complete" or name an exact
production row below. They must not use bare "K6 complete" as evidence that production unit,
placement, sensing, or viewer paths exist.

## 2. Status against the six K6 work items

| K6 item | Status | Production truth | Remaining work and owner |
|---|---|---|---|
| 1. Component tactical definitions and provider | **Fixture complete** | The registry/provider contracts are production kernel code. The `toytactics` fire/submit/resolve providers are contract fixtures, not production unit authority. | E7 cut 8 registers the production unit family and provider on `TacticalUnit`; K reviews contract conformance. |
| 2. Control leases and safe-boundary handoff | **Production mechanism complete; policy open** | `mush-sim` registers the authoritative `ControlLeaseState` family, validator, request/cancel/apply actions, and durable one-shot boundary schedules. Grants and lease guards are revalidated at execution. | E7 cuts 5/10 own campaign ingress, controller selection, cancellation authority, reasons, fallback declarations, and choice of a genuinely safe boundary. They reuse the K family and actions. |
| 3. Atomic source/target tactical action | **Fixture complete** | One root transaction can prepare both unit post-images, window state, placement guards, RNG transcript, and outcome, then install all or none. Cross-owner placement revisions are bridge-gated. | E7 cuts 6/8 own production placement/domain/unit families and production ruleset providers. No second transaction path is permitted. |
| 4. Contacts, sharing, generations, private truth | **Production lifecycle complete; sensing open** | `mush-tactics` registers the sole production `ContactState` family and authoritative observe/refresh/share/unshare/loss/reacquire actions. Projection and errors conceal truth. | E7 cut 7 owns bounded sensor-policy ingress and scoped phantom subjects. It must drive the tactics family through trusted coordinator ingress, not register another contact-lifecycle family. Confidence/provenance may be a non-authoritative side family or a reviewed contact-schema revision. |
| 5. Projected tactical DTOs and feeds | **Fixture shape complete** | The fixtures prove one purpose-specific DTO can feed `unit.status`, line output, TUI/web-shaped output, and LLM context without leaking hidden truth. | E7 cut 8 owns the production `unit.status` DTO, authorization, keyframe/feed wiring, and renderers, composed from unit, placement, contact, and authorized station information. |
| 6. Failure, simultaneity, and bridge proofs | **Fixture complete** | Tests cover disabled/stale units, contact loss, lease/grant revocation, target movement, bridge ambiguity, deterministic intent replacement/order, atomic volley resolution, and recursive projection leak checks. | Production providers inherit these gates and add ruleset-specific golden tests; K7.5/K8 extend the shared crash and asynchronous-decision matrix. |

## 3. Ownership lines after the bilateral review

These assignments override stale text that predates the merged K branch.

- K owns the rules-neutral record types, codecs, validators, coordinator, persistence lanes,
  clock/offer/window vocabulary, `mush-npc`, `mush-llm`, and K7/K8 kernel work.
- E owns production tactical-unit persistence and provider wiring in cut 8, production placement in
  cut 6, production domain authority/driver in cut 8, the `unit.status` feed, sensing policy and
  trusted contact ingress in cut 7, and campaign control/fallback policy in cuts 5/10.
- `mush-tactics` remains the sole contact-lifecycle authority. `mush-propagation` may own durable
  scoped phantom subjects and observation policy, but not a second `ContactState` lifecycle.
- The production unit family contains `TacticalUnit` only. `PlacementRecord`, `DomainRecord`,
  `HostStations`, and `HostAssignments` remain separate authorities. Projection may join them;
  storage must not.
- Foreground focus is world-owned ephemeral state keyed by immutable account identity and resets at
  boot. Durable mechanical authority remains the K6 `(subject, channel)` lease. A focus change must
  revoke or advance the exact grant/lease witnesses so retained work becomes stale.

## 4. Open K prerequisites carried forward

The following are K work, but they are not evidence that the completed K6 mechanisms above are
missing:

1. Add an owner-scoped `DomainClockProvider` that receives runtime domain id and generation. The
   composition registry stays frozen; dynamic domain promotion never registers a new provider.
2. Add a narrow immutable hook read context. Owner-scoped typed state currently reaches commands,
   ticks, boot callbacks, observers, connection/scene hooks, and async completion, but not movement,
   carry, render, or perception hooks.
3. Freeze K7.0 delayed-decision types and immediate deterministic-controller semantics before E7
   cut 5 depends on them.
4. Add K8.0 mixed-Ledger bundle closure before K8.1: replayable Ledger materialization, its seal in
   the authoritative bundle, memory/Redis conformance, and failure/restart probes.

Production unit, placement, domain, contact-ingress, and viewer work is not on this list because it
has been explicitly handed to the E cuts named in section 3.

## 5. Gate accounting

- **Gate 1:** mechanically closed by K4/K5 fixtures and production coordinator paths.
- **Gate 2:** transaction and tactical-source/target shape closed in fixtures; production unit and
  placement migrations remain E7 work.
- **Gate 3:** projection boundary and recursive leak shape closed in fixtures; production tactical
  authorization/feed/rendering remain E7 work.
- **Gate 4:** open; K7.0–K7.5 owns delayed NPC/LLM decision custody and re-entry.
- **Gate 5:** open; K8 owns the complete production/fault matrix. K8.0 closes the Ledger migration
  prerequisites before that matrix expands.

The separate typed-state migration accounting remains: mixed Ledger replay is open; owner-scoped
typed reads are closed for ordinary module contexts but open for movement/carry/render/perception
hooks; folding the Ledger seal into the bundle is open. This is why economy/housing records may be
valid domain work without yet being eligible for authoritative migration.

## 6. K7 entry decision

The E5/K6 histories and formatting now agree. K7 code may begin only after the following
convergence slice lands:

1. this K6 status memo;
2. the hook-read contract correction;
3. K7.0's doc-level contract freeze;
4. the dynamic `DomainClockProvider` contract; and
5. the named K8.0 milestone.

K7.1 then adds durable capture/dispatch without changing `ScheduleActionDraft`, the canonical
`scheduler` principal class, or `ActionCoordinator::prepare_and_stage_resolved`.
