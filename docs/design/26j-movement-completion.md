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
