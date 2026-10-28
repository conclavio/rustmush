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

## Remaining completion protocol

A new locomotion version must commit each covered owner's actual prepared
effects with movement, or create exact durable obligations for effects that
cannot share that commit. Earlier v1 history receives no inferred completion.
Deferred completion must atomically commit its real effects and handled marker,
with exact parent identity and idempotency. Notifications are only wake hints;
canonical pending rows retain custody across restart.

Scenes is the first planned adapter: its existing event Ledger can prepare the
captured events without publishing them and include those writes in the movement
commit. An empty capture still needs a retained owner preimage, without a fake
ledger append. Lost acknowledgements then recover the committed transcript with
the movement, without replaying an observer after restart.

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
