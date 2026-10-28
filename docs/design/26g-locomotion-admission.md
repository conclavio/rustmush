# DT.2b2b.3a — Bounded ordinary movement admission

Ordinary exit traversal and `HOME` need their own admission before they can join
the prepared personnel transition. The structural movement seam in
[26d](26d-prepared-physical-transitions.md) is not a substitute: it asks Carry
hooks and uses a closed Sim projection exemption. A character walking through
an exit must present that actual exit to MoveAttempt hooks and retain the
ordinary placement gate.

This slice supplies an inert Core admission. It does not change dispatch,
relocate an object, create a Sim placement, publish a receipt, or certify the
captures in [26e](26e-personnel-crossing-capture.md).

## Actual context and route

Only an actual composed `PrepareCtx` supplies the invocation, action, commit,
registered transaction preimage and installed MoveAttempt hook chain. Requests
select a bounded exit name or `HOME`; they cannot name a substitute actor or
destination. Admission requires the actual Player/NPC actor, an object principal
equal to that actor, a self subject and no explicit targets.

Exit selection shares the existing room, zone and Limbo search order, including
aliases, case matching and first-match precedence. Admission binds the selected
exit and its actual destination. Home uses the actor's actual home, with a
distinct `AlreadyHome` decision that carries no movement admission. The actual
Basic lock, containment feasibility and MoveAttempt hooks still decide whether
the proposed route is usable. No teleport ownership, JUMP_OK or Teleport-lock
rule is added to walking.

The opaque admission binds the complete invocation, action and commit. Rechecking
it compares those identities and its object observations, then reruns route,
lock, feasibility and current hooks with the live owner-scoped typed lens. A
caught refusal still poisons both preparation and transaction.

## Bounded observations

A private reader records full object preimages and missing references. This
includes scanned exits that did not match, their ordered holder lists, parent
attributes and indirect lock references. Creating a previously missing object
or inserting an earlier matching exit invalidates the observation. Unrelated
object creation does not.

Each trace allows 512 distinct references, 4,096 read visits, 1 MiB of encoded
preimages and 4 Mi work units. Repeated reads consume work. Iterative depth and
size checks precede recursive serialization, cloning and final comparison.
Exhaustion is sticky: catching an error cannot produce a usable trace.
These are bounds on Core's proof work. Registered hooks retain their own
execution contract; their arbitrary reads are checked by rerunning them, not
charged to this private reader.

The tracked and ordinary readers share attribute and lock evaluation. Existing
inheritance cutoffs, indirect-lock depth behavior, missing-reference results,
short-circuiting and unreadable-attribute poisoning remain unchanged. The new
proof-resource error propagates separately through boolean operations, so
negation cannot turn capacity exhaustion into permission.

## Remaining command migration

This admission grants no publication capability. A later consumer must bind the
canonical command ingress and revalidate the existing CommandGate checks before
using it. The final prepared image must retain these route and hook checks
through the single writer's custody and installation; arbitrary module facts
read by hooks are not represented by the object trace alone.

Actual movement also triggers narrative output, room rendering and mutating
Moved observers. Their completion needs a receipt-bound durable protocol before
ordinary dispatch can migrate. Emitting raw Moved callbacks after an ACK would
not make their writes atomic or retries safe. Existing ordinary movement stays
on its current path until those prerequisites and the Sim participant are joined.

## Verification

The Core suite passes 806 unit tests and 240 integration tests. Fourteen new
composed-provider tests exercise actual invocation binding, live foreign-owner
hooks, stale routes, missing-reference creation and caught-error poisoning.
Successful fixtures prepare only their explicit test marker and abort it;
admission creates no Core object write, other-owner write or movement output.
Thirteen private-reader tests cover strict bounds, sticky errors, lock and
inheritance parity, short-circuiting, oversized/deep values and live comparison.
The existing lock evaluator tests retain their fixed 2 MiB stack, and the lock
construction, property, ordinary command and macro regressions pass.
The 13 Core doctests, 78 Sim unit tests, 17 production DT integration tests and
32 server composition tests also pass. Core Clippy passes for all targets with
warnings denied, alongside formatting and source-policy checks.
