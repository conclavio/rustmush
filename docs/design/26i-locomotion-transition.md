# DT.2b2b.3c — Closed ordinary locomotion transaction

> **Removed 2026-10-31 (owner ruling).** The code this document describes was
> deleted by PR #PRNUM. The last `main` commit that has it is `61a670ce`; recover a
> file with `git show 61a670ce:<path>`. It was superseded by the owner's 2026-10-29
> correction: [doc 27 FO-16](27-ic-field-operations-design-and-survey.md) requires
> current containment at preparation and launch, with no embarkation intervals or
> movement-history certification. The text below is history, not a plan.

This slice joins the captured command admission in [26h](26h-locomotion-dispatch.md)
to a Core object image and a real Sim audit participant. Normal walking and HOME
remain disconnected from this protocol until durable movement completion is
implemented. It creates no certified personnel intervals or eligibility.

## Closed input and publication

The provider first admits the actual captured command, then asks Core to prepare
that admission with the registered Sim service. There is no input for a replacement
actor, source, exit, destination, timestamp or object image. Core privately mints
the participant intent from the actual invocation and route, with the current
world-IC and UTC preparation stamps. Sim's bound output contains only digests and
bounded read dependencies. Core verifies the exact preparation binding and sole
owner participant before constructing its own postimages.

For a relocating actor, the object image changes only the actor and the source
and destination contents lists. NPC actors and HOME from no current location are
supported. An exit self-loop preserves every object byte and produces no object
writes, matching `World::move_to`; HOME while already home has no moving
admission. Corrupt inverse lists, cycles, excessive containment and over-limit
objects refuse before publication. The read proof includes dependent objects
and validates their complexity before cloning or comparing them.

The retained admission reruns ordinary MoveAttempt with the real exit, or None
for HOME. This protocol does not invoke CarryAttempt and has no structural
placement exemption. Existing Core composition binds every image before applying
any of them. Missing acknowledgement leaves live objects and Sim state unchanged.

## Spatial audit and crossing evidence

Sim appends a genuine ordinary movement record to its recovery ledger. It does
not create or rewrite placement or interior-space records. Its patch retains the
actual spatial preimage and exact ledger append for final recheck and binding.
The ordinary personnel census overlays only the actor's physical destination on
the unchanged canonical topology, including affected nested people. It retains
the original actual IC preparation stamp; later validation permits only forward
time within the same generation.

The new v1 action, participant and typed receipt/crossing families are separate
from structural v1/v2. Both directions of the receipt/crossing relationship are
audited at boot. Missing, orphaned or mismatched rows refuse boot; no historical
boarding is inferred from a character's current location. Crossing evidence keeps
the explicit `uncertified_legacy_paths` classification while other movement paths
remain outside the protocol.

## Initial restrictions and remaining integration

The first consumer conservatively refuses when Sim has an Atlas window, whose
ordinary Moved observer may change canonical placement. It also refuses affected
control, focus, custody, journey and attached-projection cases that require a
joined owner refresh. These restrictions are rechecked through installation.
They do not change ordinary commands, which still use their existing paths.

The new provider has no player-facing offer or command route. Live migration
still needs receipt-bound durable completion of narrative and Moved effects,
including the Atlas and script movement consequences, plus the boot barrier
before mutating completion callbacks. A committed location change alone is not
the completed ordinary movement contract.

## Verification

The Core tests cover derived Player/NPC images, unlocated HOME, unchanged
self-loops, inverse corruption, stale dependencies, containment and byte bounds,
and strict receipt decoding. Sim tests exercise actual command capture through
the registered provider, stage and final take, including late audit changes,
unrelated windows, exposed attachments and inconsistent suppressed occupants.

Production-composition Store tests cover walking and HOME across lost
acknowledgements and cold boot, exact receipt retry, acknowledged installation
without invoking Moved observers, and a carrier-to-carrier transfer without
creating a placement record. Boot tests reject either missing side of the
receipt/capture pair and mismatched historical facts.
