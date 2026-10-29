# DT.2b2b.3d — Durable ordinary movement completion

The closed transaction in [26i](26i-locomotion-transition.md) commits location and
crossing evidence. Ordinary movement also renders departure/arrival output and
invokes Moved observers. Their effects must survive an acknowledgement loss
without running a mutable callback twice or classifying the event from a later
world state. Live command migration remains blocked until this contract is met.

## First slice: actual observer identity and event planning

Each production Moved subscription now has a stable owner-stamped, versioned
definition attached to the actual callback registration. The two Atlas handlers
and two Tacsim handlers have separate identities. Dispatch order and callback
behavior are unchanged. Anonymous subscriptions remain visible in the bounded
roster and block completion coverage; an inspection cannot silently omit one.

The roster includes exact definition references and execution mode in dispatch
order. Its digest therefore detects changes to handler identity, version and
ordering. Historical definitions are passive metadata, not a claim that a
current callback can implement a historical pending operation. Every current
entry remains a legacy post-mutation observer and a completion blocker.

Scenes now shares movement-event selection between its existing observer and a
bounded pure planner. The planned events preserve destination Joined before
source Left, original UTC and scene-local time, and the original witness list.
Self-loops produce no events. The strict plan codec accepts at most two events,
256 witnesses per event and 32 KiB total. Planning captures no authority and
publishes no state; later integration must bind it to Core's actual movement
intent and retain the owner preimage through final validation.

## Atomic Scenes completion

The separately named `sim_locomotion_with_scenes` action prepares the actual Sim
transition and Scenes transcript together. It accepts only Core's captured
ordinary command admission. The original `sim_locomotion` action and its v1
receipt and crossing formats retain their existing meaning.

Core supplies the same actor, route, original UTC and world-IC stamp to both
owners. Scenes derives the original Joined/Left events from its own state and
prepares their real Ledger writes. All object and owner bindings must succeed
before any image is installed. Both scene-log tails bind together; a stale
second log cannot publish the first append. A missing acknowledgement installs
nothing in memory, while restart restores the single committed movement and
its transcript.

An empty capture retains the actual Scenes slot and event-selection fence.
Only this closed protocol permits an owner image without writes. New scene
selection, membership or scene-local time can invalidate the capture through
final validation and binding; no synthetic log entry stands in for a read.
Self-loops and rooms without active scenes therefore produce no transcript
writes. Ordinary callbacks retain their previous behavior.

Before Scenes pauses live scenes on boot, it audits the joint receipts against
the original ledger positions, including closed-scene history. Missing or
changed events, duplicate position claims, and capture or event digest
mismatches refuse boot. The audit retains original UTC, scene-local time and
witness ordering; it does not reconstruct events from current scene state.
One transient ledger inspection serves both this audit and normal restoration.

Production composition registers the paired Sim and Scenes adapters. The joint
action has no command route or legal-action offer yet. It completes only the
Scenes observer; Atlas, NPC, weather, Tacsim and script effects remain blockers
to switching ordinary walking to the prepared path. Personnel crossing evidence
remains explicitly uncertified.

## Atlas discovery and window capture

Atlas has a separate bounded, pure movement capture for its two actual observers.
Discovery shares the legacy known-place planner: only Players learn, duplicate
visits preserve order without a write, and a new place evicts the oldest entries
above the existing 512-place limit. A self-loop still discovers its neighborhood.
The capture retains absent versus empty known-place records and the original
actor, route and UTC; it produces no state or persistence work.

Window capture records the actual direct-placement and ordered covering-window
observations. It preserves the first matching window rule, including an undrawn
window masking a later drawn one. The result distinguishes no selected legacy
effect from work needing a Sim owner join: release, placement, or re-posing.
It does not select a free cell, promise placement success, or execute map output.
Missing Atlas and Sim slots remain explicit observations, not owner readiness.

Bounds apply before copying known keys or window observations; the legacy
callbacks retain their existing behavior even when explicit capture refuses an
oversized input. The strict capture codec and real-command parity tests establish
selection behavior only. An atomic adapter still needs the actual owner slots,
exact observer references, final read fences, and one coordinated Sim placement
patch. Atlas remains a live-migration blocker.

## Composition-bound completion manifest

Each covered action now has a frozen manifest of the complete actual Moved
callback roster. Every original callback ordinal remains represented as either
a registered prepared owner group or an explicit legacy blocker; anonymous
callbacks cannot acquire prepared coverage. Registration resolves the concrete
adapter type, actual owned state slot, current participant and callback
definitions, and the action provider's exact request and owner grant.

The existing Sim+Scenes service requires the real Scenes completion group.
Production inspection identifies that one group and all seven remaining callback
blockers. Several contiguous callbacks may share one owner image; reordered,
overlapping or interleaved groups refuse composition. Binding registration order
does not affect the manifest, while actual callback order and added coverage
change its digest. Generic transaction limits remain unchanged.
Inspection rechecks the actual roster digest; a late legacy subscription makes
the frozen coverage unavailable instead of hiding the new callback.

The bounded canonical snapshot retains exact bindings and observer identities.
Historical decoding checks its retained definitions rather than today's roster,
so an older partial snapshot remains partial when another adapter is added.
Snapshots are inspection data, not executable capabilities. The existing v1
movement receipt formats retain their meaning; a future generic completion
consumer must include the snapshot and each owner's actual effect proof in its
own commit. Registry coverage alone never enables an offer or command route.

## Closed composed movement kernel

Core now supplies a separate `sim_composed_locomotion` protocol for the actual
Sim primary and up to seven registered observer owner slots. The composition
adds concrete typed adapters before freeze; the resulting opaque service retains
their real handles and the exact prepared subset of the frozen manifest. Each
owner receives a Core-minted route, original clocks and group binding. Core
hashes the actual collected writes and retains each bounded owner proof with the
historical manifest in the same receipt.

Only this closed action can prepare eight slots and retain empty observer
images. Ordinary transaction limits remain unchanged. All captured slots,
incarnations, owner fences and object images bind before any apply. A refusal
poisons preparation even if a provider catches it. A late Moved subscription
invalidates every manifest lease, including images already taken for install.

The Sim reader can inspect the full receipt; each observer's read capability
projects only its own exact participant proof and immutable movement header.
Read authorization uses the engine's module stamp, not a mutable public context
field. Unknown historical participant profiles for an owner refuse its audit
rather than silently disappearing. Snapshot decoding validates retained
definitions and exact historical bindings independently of current coverage.

Kernel tests exercise eight simultaneous slots, an empty observation, a final
bind refusal, caught preparation failure, stale slot incarnations, late roster
drift, receipt corruption and owner-scoped reads. This slice registers no
production action or command route. Production Sim/Scenes integration and the
Atlas owner adapter follow; durable Core presentation and remaining callbacks
are still required before ordinary dispatch can select the protocol.

## Remaining completion protocol

A new locomotion version must commit each covered owner's actual prepared
effects with movement, or create exact durable obligations for effects that
cannot share that commit. Earlier v1 history receives no inferred completion.
Deferred completion must atomically commit its real effects and handled marker,
with exact parent identity and idempotency. Notifications are only wake hints;
canonical pending rows retain custody across restart.

Owners requiring deferred completion need a recovery barrier before migrations,
mutating boot callbacks, ticks or session ingress. Scenes' current boot loader,
for example, pauses scenes and persists changes; it cannot serve as a pure
recovery hydrator. Unknown handler versions or incomplete owner availability
must hold any pending recovery.

Atlas discovery and window placement, NPC memory and work queues, Tacsim return
markers and tutorial state, weather output, and scripted entry/exit effects each
need their own real adapter. Nested movement and script timers cannot be treated
as acknowledged merely because their callbacks returned. No broad World clone,
post-ACK callback replay, or legacy write batch establishes durable completion.
