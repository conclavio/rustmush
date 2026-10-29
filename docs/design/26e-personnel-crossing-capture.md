# DT.2b2b.1 — Personnel crossing capture

This slice records the merged immutable personnel crossing format for prepared
physical commits from [26d](26d-prepared-physical-transitions.md).
**Owner correction, 2026-10-29:** [doc 27](27-ic-field-operations-design-and-survey.md)
requires current containment at preparation and launch, not recorded embarkation.
Retain the existing code and record compatibility for now; crossing certification
is not a prerequisite for NPC hiring, activation or launch.

## Evidence and limits

Each new physical commit writes exactly one Sim-owned typed `personnel_crossings`
batch, keyed by that commit token. A batch records the actual character and
carrier pairs entered or left, the operator and root, an IC preparation stamp,
the plan digest, before/after ancestry digests and the physical receipt digest.
An empty batch is meaningful: that covered transition changed no carrier
membership. It is still paired with its exact physical receipt.

Every batch requires `coverage: uncertified_legacy_paths`. There is no certified
variant, mutable trust flag, current-aboard interval cache, eligibility answer or
payable duration. An uncovered departure and return cannot restore or extend an
old interval because this slice creates no intervals. Legacy preparation reports
that require personnel history must be corrected during activation integration.

The former future-certification programme is withdrawn. Current location supplies
eligibility directly; it need not promote these captures into historical evidence.
Existing crew do not acquire historical boarding records during startup.

## Derivation and transaction

The Sim provider reads its own canonical spatial state through the actual
preparation context. It derives both sides using the canonical plan and an
overlay of precisely the planned Core object projections. Placed actors must
agree with their actual physical interior or attachment. Unplaced Player/NPC
objects are found in authored rooms and nested contents, including the legacy
single-room interior format. Nested carrier ancestry is followed; arbitrary
domain hosts are not interpreted as carriers.

Crossings are set differences. Walking between rooms of one carrier produces no
crossing. A transfer produces an exit and an entry. Reanchoring an occupied
nested hull can change its occupants' outer-carrier membership with no direct
object location writes.

The actual Spatial participant independently derives the capture and binds its
digest into the physical plan digest. Core retains every additional World
dependency, and Sim retains the corresponding canonical state dependencies.
Newly discovered people and hulls pass the same control, custody, recovery and
journey exclusions as the original structural subtree.
The provider checks the resulting receipt before inserting the typed batch in
the same transaction. Preparation publishes nothing; acknowledged installation
and recovery retain the same physical, spatial and capture bytes.

The existing physical limits remain in force. Capture additionally limits each
transition to 256 distinct actors, 256 crossing pairs, 512 dependency objects,
8,192 traversal steps and a 128 KiB encoded batch. Every examined edge is charged
before filtering. A transfer of 128 people between two carriers consumes 256
crossing pairs; nested carriers can consume more pairs per person. Inconsistent
inverse contents, ambiguous ancestry and exceeded bounds refuse the whole
transition instead of emitting a partial personnel list.

## Clock and compatibility

The record uses `world_ic_now()` at actual preparation, including its exact
WorldIc generation. It never substitutes the command submission time or a
caller-supplied clock. Final handoff permits ordinary forward progress within
that generation, retaining the frozen preparation stamp. A generation change
or backward time refuses the prepared transition. After handoff, acknowledgement
and replay use the retained bytes without retimestamping the event.

Physical action, participant and outcome version 2 identify the capture contract.
Exact version 1 definitions remain readable for historical receipts, with no
version 1 execution provider. The typed physical receipt family retains its
existing encoding. Startup validates every capture against the actual Core
receipt and requires a capture for every version 2 receipt. A version 1 receipt
must have no capture; migration does not synthesize one.

The batch's semantic digest is recomputed from its persisted stamp, ancestry
digests and crossing rows, then combined with its original plan digest. This
must equal the actual physical receipt digest. Startup also verifies the commit
key, operator, root, descriptor pair and record format. Unknown fields, missing
coverage, duplicate or unordered character/carrier pairs, orphan captures and
missing version 2 captures refuse boot.

## Withdrawn follow-up work (2026-10-29)

The [former movement-coverage gate in 26d](26d-prepared-physical-transitions.md#withdrawn-personnel-history-gate-2026-10-29)
is withdrawn. Docs 26g–26j are superseded and must not be extended. Ordinary
movement remains on its existing live path. NPC hiring records a contract;
activation and launch check current containment. Any later wages accrue from
that contract, not certified intervals or these crossing captures. Retain merged
code for now, with possible deletion of the unused v2 provider in a later cut.

## Verification

Validated locally on 2026-10-28:

- All 779 Core and 77 Sim library tests pass. Capture regressions cover nested
  hull cabins, first/last interior anchors, explicit and implicit placement,
  contradictory ancestry, exact limits and newly discovered control hosts.
- Sixteen production DT coordinator tests pass, including ten physical tests.
  Capture checks cover zero-projection movement, same-carrier walks, distinct
  reentry commits, lost acknowledgements, restart, exact retry, clock generation
  and reversal, strict receipt pairing and historical v1 records without backfill.
- Thirty-two server composition tests and sixteen Atlas window tests pass.
  Atlas restart fixtures preserve objects while registering fresh runtime
  handles, matching the actual reboot boundary.
- Clippy passes with warnings denied for Core, Sim, Server and Atlas libraries
  and tests. Formatting, diff checks, panic budgets, state-slot ownership and
  the 26 typed-state, restriction, command-classification and help checks pass.
