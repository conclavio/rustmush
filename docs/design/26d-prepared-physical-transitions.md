# DT.2b2a — Prepared physical transitions

This slice establishes the transaction boundary needed by the actual embarkation
history required in [doc 27](27-ic-field-operations-design-and-survey.md). A
character's present location cannot prove when they embarked, and an observer
that writes history after movement can lose that history on a crash. The physical
change and its owner records must belong to one acknowledged commit.

## Ownership and atomicity

Core owns object location and inverse contents. Sim owns the canonical spatial
records and their derived index. A registered Sim action submits a bounded
transition request to Sim's real planner. Providers cannot supply arbitrary
object postimages or bypass the planner by constructing a public `Plan`.

The prepared Sim participant supplies its exact admitted projection moves through
an opaque binding. Core derives the corresponding object images internally,
merging changes when several moves share a source or destination. Duplicate
moves, inconsistent inverse contents, cycles, oversized changes and stale
preimages refuse the whole transaction. A structural change can have no object
projection moves and still require the Sim participant.

The ordinary coordinator retains the object images and the participant patch in
one authoritative bundle. Preparation publishes neither. Final take rechecks the
planner's facts, authority, hooks and exact before-state. Only acknowledgement
installs both images, without invoking business callbacks during installation.
Recovery replays the retained canonical images; retrying the same ingress returns
its original receipt.

Sim's existing placement gate may recognize only the exact prepared transition
through its opaque binding. Foreign movement and carry vetoes remain in force.
The exemption cannot become a general permission to move a placed object.

## Initial admission

The foundation exposes a typed internal action with the existing Sim admin gate.
It covers bounded `Place`, `Unplace`, `Space` and `Members` requests and verifies
Core's immutable dispatch actor before submission. It preserves deployed and IC
custody exclusions. Affected station or assignment hosts, even empty ones, and
active focus refuse until their control-generation changes can join the same transaction.
Underway or held journey state also refuses, including a reanchor with no object
projection changes.

The existing canonical planner clones spatial state and rebuilds global indexes.
This foundation therefore checks a whole-state census and streaming byte ceiling
before planning and binding: at most 4,096 resident spatial records, 4 MiB for
spatial data and indexes, and a separately bounded resident audit tail. A request
also has a 16 KiB input limit, 256 projection moves, and bounded dependencies and
room contents. Exceeding a limit refuses the operation. An incremental owner
overlay can remove the conservative whole-state restriction in a later cut.

Carrier journey moves, domain retirement and destruction quarantine remain
excluded. Journey state, driver lifecycle, activation, control refresh and recovery
records must be prepared by their actual owners before those operations can migrate.
Existing synchronous legacy callers
retain their current interface; they do not silently enqueue a second deferred
operation.

## Gate before personnel history becomes authority

This foundation does not issue embarkation intervals, wage evidence, departure
manifests or NPC eligibility. It does not infer past embarkation during startup.
Existing planning reports must continue to identify those prerequisites as
unresolved.

The following paths must use the prepared boundary or refuse a relevant change
before intervals can authorize personnel or wages:

- Ordinary exits, home, teleport, carry operations and direct World moves.
- Object creation and destruction, including occupants evacuated during cleanup.
- Spatial placement, unplacement, interior membership/reanchoring and domain or
  mount projection changes.
- Journey departure, arrival, cancellation and rollback, including nested hulls.
- Custody transfers, putting occupants ashore, quarantine, recovery and boot repair.

Checking only current membership cannot close this gap: an unrecorded departure
and return could revive an old interval. Each real ancestry change needs its own
recorded transition or an explicit refusal.

Affected personnel enumeration must include both placed actors and unplaced
Player/NPC objects in every authored interior member room. Moving a nested hull
can change an occupant's outer-carrier ancestry without changing that person's
object location. A walk between rooms of the same interior creates no embarkation
event. Enumeration and encoded-size bounds refuse the complete transition on
overflow; truncating the crew list is not acceptable.

Intervals will use the world IC clock's generation and timestamp, the actual
character and carrier identities, and the physical commit identity. Field rescue,
transfer and recovered crew need real crossing events, not a last-port-only rule.
Departure manifests must join the actual journey departure commit. Those records
then provide inputs for the later crew-contract, wage and activation slices.

## Verification

Validated locally on 2026-10-28:

- The complete Core library suite passes: 777 tests, including ten prepared
  physical-image tests and the bounded resident-ledger check.
- The complete Sim library suite passes: 57 tests, including fifteen structural
  transition tests. Coverage includes empty control hosts, implicit legacy rooms,
  unplaced focused occupants, actual authored spaces and excluded lifecycle owners.
- Six production coordinator tests cover lost acknowledgement and restart, exact
  retry, foreign Carry/Move vetoes, stale World and Sim preimages, zero-projection
  reanchoring and immutable dispatch attribution. The six existing DT planning
  and unit-authority coordinator tests and 32 server composition tests also pass.
- Clippy passes with warnings denied for Core, Sim and Server libraries and test
  targets. Formatting, diff checks, panic budgets, state-slot ownership, typed
  state lenses, restriction policy, command classification and help checks pass.
