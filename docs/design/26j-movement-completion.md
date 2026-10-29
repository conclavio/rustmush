# DT.2b2b.3d — Durable ordinary movement completion

> **Superseded by the owner's 2026-10-29 correction. Do not extend this design.**
> [Doc 27 FO-16](27-ic-field-operations-design-and-survey.md) requires current
> containment at preparation and launch, with no embarkation intervals or
> movement-history certification. The text below records historical work, not
> an active migration plan. Leave merged code in place for now; the closed path
> is inert on the live command route. A later cleanup cut may delete the v2
> provider if nothing adopts it. Continue with NPC hiring contracts, ordinary
> boarding, DT.2b4 activation and DT.3 launch as described in
> [doc 26c](26c-detachment-preparation.md#current-delivery-sequence).

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

## Production Sim and Scenes composition

Production now registers the composed action with the real Sim primary and a
distinct Scenes transcript adapter. The composition root adds concrete observer
owners through an opaque builder during Sim registration, so adding Atlas will
not introduce a Sim-to-Atlas dependency. Scenes receives only its scoped reader;
the full receipt reader remains with Sim.

The composed action has its own Sim recovery operation and immutable crossing
family, with separate plan and capture digest domains. Ordinary and earlier
Sim+Scenes v1 records keep their original definitions and codecs. All crossing
families retain their uncertified coverage label. Sim still refuses any open
window, direct placement, or other unsupported physical lifecycle.

Scenes prepares the same bounded original event selection and actual Ledger
appends for both protocols. Its cold audit joins both receipt families to the
original log positions in one bounded audit, before pausing scenes or dropping
closed tails. Duplicate claims across protocols, missing events and changed
capture material refuse boot. Adding a participant to a later composition does
not reinterpret older partial manifests.

That initial composition covered only the Scenes observer. The next cut below
adds Atlas discovery and checked window no-effects. No command route or
legal-action offer selects the composed action; other callbacks and durable
Core presentation remain live-migration gates.

## Prepared Atlas discovery

The actual Atlas owner now joins both contiguous Atlas callbacks through one
participant. A covered Player arrival commits the real `known:<actor>` document
and its knowledge-history entry with movement. FIFO eviction, duplicate visits,
missing versus empty books, and self-loop discovery retain their original
meaning. NPC and unmapped arrivals retain a real checked owner observation;
duplicate visits produce neither a document overwrite nor a synthetic event.

The retained proof contains the original bounded Atlas capture and the exact
knowledge-history boundary. New knowledge names its actual original ledger
position and event digest. Read-only effects retain the observed boundary and
bind the real owner facts, including the journal tail, without fabricating a
write. All owners still bind before any object, knowledge or transcript image
is installed.

Later real legacy learning writes its actual document and history entry in the
same legacy batch. Boot checks each prepared effect at its original history
position, then advances through later journal mutations before comparing the
current owner projection. It never compares an old receipt directly to today's
book or reclassifies it using today's geometry. Missing documents or original
events, contradictory no-op observations, duplicate claims and orphaned
prepared events refuse boot before ordinary Atlas cleanup. The existing legacy
boot normalization of known places remains the projection policy. Untouched
older books acquire no fabricated history, and only open Ledger tails remain
in World state; audit indices are transient.

This adapter proves no selected window effect for its covered route. Sim's
existing refusal of any open window remains; window release, placement,
re-posing and map delivery still need the real joint placement protocol.
Production's composed manifest now covers Scenes and both Atlas callbacks,
leaving five explicit callback blockers. Personnel crossing evidence stays
uncertified, and ordinary command routing remains disabled.

## Bounded window-arrival planning

Atlas can now capture the original window arrival as a read-only plan. It shares
the actual entry-cell selector and room resolver with legacy placement: first
covering selection before the drawn check, first matching portal in either
direction, a second lookup by that portal's record key, first matching surface,
all non-custodied participants' occupied xy cells (including the actor and
other heights), and distance/y/x entry ordering. The planned pose uses a fresh
passive posture. Overlapping surfaces can project it into a different room;
the plan retains the actual resolver result rather than assuming the admitted
destination.

The plan separately retains any release selected before the placement attempt.
A full destination does not erase that release or imply it succeeded. Missing
slots, undrawn or inconsistent window lookups, absent geometry, absent room
indices and no free cells remain distinct observations or attempt results.

Rechecking recaptures the original actor, route and UTC. Selected geometry is
hashed from actual bounded surfaces, cell data, room tables and footprint
inputs, not a cached composite digest or revision alone. Portal captures
include the real scanned prefix and second geometry lookup. Occupancy captures
copy only ids, revisions, xy and the actual arena-exclusion result; opaque
participant postures and World are never cloned. Nonfinite footprint and
portal coordinates retain their original float bits in the fingerprints,
including when legacy selection falls back to the footprint or grid centre.

Explicit planning refuses oversized or unsafe inputs before copying or
encoding them: eight surfaces, 65,536 cells and 4,096 room ids per surface,
4,096 placement rows, 4,096 scanned portals with 64 KiB of aggregate keys,
the existing footprint point bound, and checked grid dimensions/coordinates.
Selector and occupancy encodings have separate 8 MiB and 1 MiB caps; the final
plan is at most 256 KiB. These are explicit planning bounds, not new limits on
ordinary legacy placement.

This cut supplies observations and an attempted entry pose only. It has no
decoder, write capability, authority definition or command route. Genuine Sim
placement admission, compound Core projection, owner binds, output custody and
new versioned historical verifiers remain prerequisites for window completion.
Existing v1 definitions and the refusal of open-window composed movement remain.

## Bounded Core movement preview

A genuine locomotion admission can now produce an owned, read-only Core object
preview for the admitted move and one hypothetical subsequent room projection.
The ordinary image path shares its row builder. The projected preview reads
the first move's bounded postimages over original tracked World facts; it never
clones World or accepts caller object rows. Only captured objects are readable,
and unobserved scope is an error. Original admission object facts remain fenced
independently from the image's bounded dependency census.

Both steps share the existing 512-object/1 MiB image-read budget and the
256-object carried-subtree cap. The second step repeats the actual containment,
inverse-membership, live-object, cycle and depth checks over that overlay. At
most four objects are touched: actor, original source, admitted room and final
projection. Returning to the source does not collapse into a no-op: the actor
can move to the end of its contents, and the intermediate room still receives
the original preparation UTC. Genuine self-loops preserve their bytes.

The preview is neither a Sim placement decision nor an installation capability.
It exposes no decoder, receipt, write list or publication method. Existing v1
images still accept only the admitted move. A joint window protocol must bind
its real Sim/Atlas projection and any participant-behavior attachment, supply
new historical verifiers, and retain durable map output before selecting these
effects in a live route.

## Bounded Sim window movement capture

Sim can now derive release and direct-placement candidate phases from a genuine
Core preview and hypothetical passive entry coordinates. It shares the actual
transition planner: release keeps the admitted Core location, carries the
original UTC into unplacement metadata, and advances the real placement
revision. A subsequent placement sees that released state and advances the
revision again. Reposing within one domain has one phase. Missing placement
and an explicit unplaced record remain different observations.

This initial scope requires an already attached Sim participant for placement,
with no physical dependents or structural/control host records on the actor.
Interior/attached source parents, held state, closed/non-window targets,
inconsistent indexes or domain aliases, and exhausted revisions refuse. Every
selected phase must plan successfully; a failed second phase publishes neither
the earlier release nor any Core move. Real journey-seal and IC-exclusion
checks remain in the transition path. The candidate uses the Core-validated
hypothetical final room rather than invoking a resolver over the pre-move
World; the actual Atlas owner must attest that room in the future protocol.

The existing resident Sim census/4 MiB byte bound precedes its temporary state
copy. Selected original records are capped at 16 KiB and the final capture at
128 KiB. Opaque placement, anchor, egress and station payloads receive an
iterative preflight (64 levels, 65,536 aggregate nodes and 4 MiB text) before
recursive resident encoding. Rechecking retains the observed IC stamp, requires
the same clock generation and nondecreasing time, keeps the Core preview's UTC,
and compares the actual bounded native facts and phase data.

This is candidate data only. It creates no participant grant, Core or Sim
installation, audit append, receipt, decoder, output or eligibility evidence.
Participant-behavior attachment, actual Atlas attestation, compound object and
ledger publication, historical verifiers and the remaining owner joins still
belong to a new versioned completion protocol. Existing v1 routing and window
refusals are unchanged.

## Versioned window composition kernel

Core's separate version 2 of `sim_composed_locomotion` accepts the real Sim
primary's immutable window proposal. Only Core can mint its captured-command
intent and bounded read seed. The proposal names the admitted room's subsequent
projection, passive metric target, and actual Atlas/Sim capture digests. It is
bound into every observer intent, owner capture and proof; proposal data alone
grants no geometry or placement authority.

The kernel shares the bounded Core row builder for both steps. It retains the
intermediate room's timestamp and the source contents ordering on a return,
with at most four touched objects and the existing aggregate read/image bounds.
Every actual owner and Core preimage must bind before publication. Late callback
roster changes, stale projection rooms, owner refusal and a final bind failure
publish no movement or earlier owner image. When both Core steps are no-ops,
there are no object rows; an absent placement target remains explicit.

The new `window_composed_locomotion` receipt family wraps its exact versioned
movement record and proposal. Its strict canonical decoder verifies the full
proposal binding for every owner; old v1 action definitions, digest domains and
receipt bytes retain their meaning. Sim receives the full receipt while an
observer receives only its own proof and the common immutable proposal.

This slice supplies the Core kernel and real registered test composition only.
Production Sim, Atlas and Scenes window adapters, original-history verifiers,
participant attachment and durable map output still require subsequent slices.
Window-arrival notifications remain an explicit completion requirement even
after placement can commit. Partial manifests remain partial, crossing evidence
remains uncertified, and no command route or offer selects version 2.

## Atlas window selector attestation

An optional Atlas adapter now joins version 2 through its real `AtlasState`
participant. It verifies the primary's exact metric target and projected room
against the original bounded Atlas plan, including the resolver's admitted-room
fallback. The complete plan digest joins the common Core proposal. Failed
selected placement attempts refuse this profile; they cannot be relabeled as a
successful release-only arrival while their output is still unimplemented.

Discovery uses the same real known-document and knowledge-journal preparation
as v1. Its new proof retains the original window plan and exact journal boundary.
Final binding recomputes Atlas's actual geometry, portal prefix/lookup, footprint
and resolver inputs without trusting revisions or a cached composite digest.
The Sim owner must independently retain occupancy and physical topology. The
Atlas slot check cannot substitute for that joint native fence.

Historical decoding preflights 32 levels and 65,536 nodes before recursive
encoding or cloning, then accepts bounded, strict evidence data without
manufacturing an executable window plan. Boot audits both configured profiles against
one original knowledge history, so duplicate position claims across profiles
also refuse recovery. It does not re-run old selectors over current geometry.
Existing v1 proof and journal bytes retain their meaning.

The new group covers only `travel_discovery`. The actual `window_arrival`
callback remains an explicit blocker until durable map delivery joins. Current
production composition still selects the earlier v1 adapters; the real Sim
window primary, v2 Scenes join and end-to-end physical recovery follow. The
registration test uses an execution-refusing primary and establishes no Sim
placement evidence. Live routing and personnel eligibility remain disabled.

## Ordered window movement observations

Sim now has a distinct v2 candidate capture for the admitted Core step followed
by each selected native release/place phase. Each stage retains its actual
before/after ancestry digests and ordered crossing facts at the original IC
stamp. Adjacent ancestry digests must agree. A return to the original room
keeps the intermediate step, and a brief carrier visit retains both entry and
exit instead of collapsing to an empty net result.

One aggregate 256-crossing and 512-dependency bound covers all stages. The
128 KiB capture limit remains. Tests exercise a genuine Core preview with 128
people entering and leaving (256 crossings); one additional person refuses the
whole capture. No Core object, native placement, audit or module marker is
published. Rechecking preserves original UTC while requiring nondecreasing IC
time in the same clock generation.

Native phase plans stay typed and private. The original v1 candidate and its
digest domain retain their exact bytes; v2 wraps that evidence with the ordered
observations and an explicit uncertified coverage label. These are still
hypothetical candidates. The actual Sim participant, same-commit recovery and
crossing records, historical verifier, joint Scenes adapter and durable map
output remain required before a window movement can be installed or routed.

## Native owner read preimages

The ordered window candidate also retains a selected-domain metric occupancy
fingerprint. Atlas selection and Sim's callback-free occupancy recheck now share
the same bounded scalar collector: map key, stored object id, revision and XY.
Coordinate changes without revision bumps, insertion/removal and domain changes
invalidate it. Z, heading and opaque posture are not occupancy inputs. The
collector retains the 4096-placement scan bound and the existing coordinate
limits; Atlas keeps its original six-field evidence and custody exclusion bit.

The fingerprint is only the native placement portion. Actual custody exclusion
preimages, topology, Core rows, Atlas selectors and owner readiness still need
their own checks. It grants no admission, receipt, personnel interval or route.

The existing owner-read service can now retain one pure, bounded fingerprint of
selected fields in its actual legacy state slot. Core checks those fields after
the service returns, at final writer custody and before post-acknowledgement
publication. The actual service runs once. Slot identity, requester/owner
readiness and typed-family read guards keep their existing checks and limits.

This closes the gap where a slot's contents could change while its incarnation
remained the same. Unrelated fields may change when the selected projection is
unchanged. Failed, repeated or incorrectly typed guards poison the read even
when service/provider code catches the error. A stale field snapshot refuses
all prepared typed and owner images before any apply.

Sim now registers a real, self-granted read service over its existing JourneyBook
slot. The structural, ordinary, joint Scenes and composed locomotion providers
retain record presence and the shared under-way predicate for their actual
dependency/ancestor roots, plus the absence of held journey records. A held
record or changed selected predicate after writer custody refuses publication
before any typed, Core or owner apply. Allocator and tick-cursor progress do not
invalidate an otherwise unchanged read. The service snapshots at most 512 roots
and never clones or serializes routes, anomalies or the entire book.

The same providers also retain an independently registered read over the actual
ephemeral FocusState slot. Its bounded projection checks the existing
`has_focus_on` and `focus_of_actor` presence predicates for the affected closure.
New actor or unit focus after custody refuses before publication; unrelated
focus activity remains valid. This is an exclusion preimage, not a control
lease, grant or proxy-seat capability. Both handles share immutable heap storage
in providers; neither legacy slot is copied or written by the read services.

The same preparations retain the actual Sim IC exclusion slot's loaded state,
owner generation and selected identity-presence predicates. The selected lookup
is shared with the existing exclusion helper; an inconsistent active index
refuses. Retaining the owner's existing global generation conservatively refuses
any intervening custody update, including an open/close cycle. This reads at most
512 ids and does not copy historical custody records or construct a seal, Open,
Close or four-owner admission capability.

These read joins add no owner image, callback coverage or authority mirror. The
native window participant still needs complete placement/topology fences and
same-commit records before it can publish a window transition.

Tactics now offers an opt-in, Sim-only owner-read service over its real UnitStore.
It observes active-unit presence, held/miskeyed/duplicate record claims and OOC/IC
custody, retaining the selected metadata through Core's state guard. It reads
Tactics' own markers and IC owner generation even when the Sim projection is
empty. Frozen records stay held. Unit cells, checkout snapshots and historical
custody bodies are neither copied nor serialized by the service.

The data contract bounds requests to 512 objects, the global record scan to 4096
rows, selected record metadata to 512 rows and the retained image to 128 KiB.
Selected keys/arena names have a 1024-byte individual and shared 64 KiB text
bound. These facts are not custody, physical admission or title capabilities.
The production root now configures both this real Tactics service and Sim's
typed read join. Structural, ordinary, joint Scenes and composed locomotion
providers retain that actual read before preparing owner publication. Held
records or OOC/IC custody refuse; the response must cover exactly the requested
ids in order. Tactics readiness and its state preimage remain checked after Store
ACK. Root configuration selects a typed registered service with the fixed
Tactics descriptor, rather than supplying a callback that claims a check ran.
Standalone Sim compositions keep their existing configuration. This adds no
movement route or callback-completion claim.

The existing Sim deployment projection also has its own retained read. Its
complete, bounded index is fingerprinted because domain exclusion and occupancy
may consult markers outside the moving subtree. Any index drift conservatively
refuses a pending closed movement. Limits are 4096 marks, 1 MiB of encoded image,
1024 bytes per arena name and 64 KiB of shared name text, checked before encoding.
This remains a projection guard: the independent Tactics read is still the
custody source, and neither read supplies a checkout or movement capability.

The ordered window candidate now retains a separate native-slot preimage over
the complete canonical placement, domain, space, mount, control, held-row and
allocator fields. A callback-free recheck accepts only the original state and
consistent rebuilt indexes. Unknown placement identities refuse. This is
deliberately conservative about unrelated native edits: the existing 4096-row,
4 MiB resident envelope and opaque-value preflight run before encoding, and the
new fingerprint has a 4 MiB bound. No World or SpatialState is copied to validate
the bind. The original v1 candidate bytes remain unchanged.

This guard supplies no installation or authority. Core rows, actual owner reads,
Atlas selectors and the actual recovery-ledger append still bind independently;
native primary publication and same-commit records remain unfinished.

Ordered candidates now reuse the actual locomotion projection-dependency walk
for the complete Core inventory. Ordinary carried things and people remain
supported and join the Core read dependencies. Additional native placements,
spaces, mounts, control surfaces and held assignments refuse this actor-only
profile; a hosted domain refuses even when empty or retired. The shared actual
physical exclusion helper checks the complete captured dependency set, including
carried people. Both the 512-dependency and 256-crossing budgets remain shared.
The original v1 candidate is unchanged. Compound hosts need their own real owner
join before this scope can widen; no refused case becomes a partial publication.

Ordered window evidence now has a separate, strict historical decoder. It
checks exact required/unknown fields, original clocks, ordered dependency ids,
linked stage ancestry and shared phase digests, the aggregate crossing budget,
and the exact release/place revision, projection, audit and final-record chain.
The full capture digest must match the supplied retained digest. Input depth,
nodes, text and encoded bytes are bounded before recursive decoding or cloning.

Decoded evidence remains passive: it cannot create a Core preview, native Plan,
owner patch, installation or eligibility interval. A later recovery auditor
must bind its actor, route, clock and commit to genuine same-commit owner receipts
and records. It never reruns current geometry or classifies history from today's
World. Native participant publication and recovery joins remain unfinished.

Atlas can now join its actual window plan to a genuine Core projection preview
and Sim's ordered capture as one read-only candidate. The selected room and
metric target are available before Sim capture; the shared proposal is then
built from the actual Atlas and Sim digests without a placeholder digest. The
join checks actor, source, admitted/projected rooms and original UTC, recaptures
the Atlas selectors, and retains both owners' independent evidence for recheck.
It still grants no receipt, native patch, installation or personnel eligibility.

Sim's existing structural, ordinary, Scenes and composed typed movement
histories are now audited at the start of its world-load hook, before its IC,
relay, spatial-repair or journey loaders can mutate state. The final composition
audit remains in place. Missing or mismatched receipt/crossing pairs therefore
cannot first move an unrelated object through Sim's projection repair. This is
a Sim-local typed-history barrier; the cross-owner recovery gate and native
window journal/receipt joins below remain required.

The server now hydrates and audits the reserved typed partition before module
schema migrations or module load callbacks. Core dispatches a read-only,
owner-stamped typed-history audit to every actual registered module; Sim uses
its existing receipt/crossing checks. The ordinary boot fetch must match the
same immutable partition after migrations, so a reserved-tree rewrite or
manifest omission refuses before loading. Direct `Engine::modules_loaded`
also audits all modules before the first load callback. Per-module drivers must
invoke the audit explicitly, as the server does. Native owner journals and
foreign observer recovery still require their own pre-mutation validators;
this typed-only phase does not certify those unfinished joins.

Atlas participates in the early typed phase with the same strict owner-proof
decoder used by its native knowledge-history audit. Both ordinary and window
profiles bind the original actor, route, UTC, capture digest and knowledge
effect claims; two prepared events cannot claim one knowledge position across
the readers. Multiple no-op observations at a boundary remain valid. This pass
uses no current geometry, callback or loaded Atlas projection. The later audit
still requires the real journal events and current knowledge projection.

Scenes also runs its actual typed receipt/proof readers in that early phase.
Atomic and composed claims share one bounded position index, so duplicate
transcript claims refuse before loading, and no-event captures already require
their exact route/UTC digest. This does not hydrate, pause or reconstruct a live
scene. Nonempty captures still require the later original-transcript join;
typed claims alone never stand in for committed native events.

Core and the server now also expose a read-only native-history phase. Every
configured Atlas knowledge and Scenes transcript join runs before module
migrations or the first load callback. A narrowed history gets at most one
whole-fetch retry without writing a manifest or running a load. The server
rechecks the fresh post-migration snapshot globally before loading; direct
`Engine::modules_loaded` likewise validates every owner first. Native audits
retain their existing bounds and later restoration checks, with extra boot-only
validation passes rather than cached authority or a reconstructed World. Sim's
future native window journal still needs its own exact receipt/record join.

The window record wire format now binds ordered Sim evidence to the original
Core v2 header, projection, final-placement digest and a claimed native journal
position/body digest. Its genuine-receipt constructor and matcher retain the
actual action, participant, commit, exit, clocks and plan/capture digests. The
decoder checks bounds before cloning and rejects missing/unknown fields or
rebound inconsistent headers and native final images. These are passive data:
decoding does not prove a journal append or create an installation. The native
publisher, same-commit family append and cold original-position join remain
required before enabling the profile.

The optional window-history API now registers the separate
`sim.window_locomotion_crossings` family only through a Sim-owned registrar.
Its append checks the genuine Core receipt, ordered capture, and exact prepared
`window_movement` journal entry before staging a unique commit row. Typed audits
join receipts and records in both directions; native audits additionally require
each original journal position/body and reject duplicate commits or orphan rows.
The dedicated history is bounded to 8 MiB and 4096 chunks before copying; other
Sim logs are neither borrowed as authority nor inspected through this profile.
Registration alone enables no provider or live route. Native state binding and
producer integration remain required, and none of this certifies an aboard
interval or reconstructs current placement from historical evidence.

Until the actual native publisher and recovery join are configured, Sim rejects
the dedicated window journal during the global read-only boot phase, before
any module migration or load callback. Positive omitted-chunk floors cannot
hide an unsupported history. The local load hook repeats this guard for drivers
that call it directly. Passive history registration alone does not opt into a
publisher, and unrelated legacy Sim logs retain their existing loading behavior.

The actual Sim window participant now accepts only Core's opaque window intent.
It derives the ordered native plans itself, prepares only the final actor
placement and a bounded dedicated journal append, and retains the original
context, selector, native and occupancy preimages. Binding builds the bounded
placement/index postimage before borrowing the selected ledger tail; application
then installs only those prepared images. Dropping a binding or refusing a stale
tail changes nothing. Planning snapshots exclude recovery logs entirely, and
binding preserves unrelated log tails without cloning them.

Production composition now registers the separate closed v2 producer, actual
Sim participant and actual Atlas window-discovery adapter. The selector returns
proposal data; Atlas independently attests the shared header. Preparation keeps
the real Journey, Focus, IC exclusion, deployment projection and Tactics custody
reads through final validation. The genuine Core receipt, ordered capture,
prepared native journal append and window family row share one commit. Both
typed directions and original native journal positions are audited before
migrations or load callbacks. Generic Sim compositions remain opt-in.

V1 remains the authoring default and its receipts/readers remain registered.
Completion registration requires an enabled provider for the exact action,
alongside the current actual adapter/callback, owned slot, request and grant;
retained action metadata alone cannot enable a profile. Core exposes only a
read-only preview of an already retained captured admission to the producer.

Production tests prove unplaced entry, empty native phases, original journal
matching, lost acknowledgements, cold typed/native refusals, and failure before
any owner image is installed when Core preimages drift after acknowledgement.
Production lifecycle tests now follow actual Player and NPC actors through entry,
same-room re-positioning, departure and return, joining all four original native
journal positions after restart. A same-room move keeps Core rows unchanged while
the native placement revision and journal advance. Departure retains the previous
placement in its genuine release record; lost acknowledgement restores that
release exactly once across repeated boots. Store acknowledgement alone installs
none of the participating owners.

Sim's existing movement gate already permits ordinary exits from window domains,
so these routes use the actual hooks without a new admission exemption. Departure
from a non-window domain still receives the actual Core movement-hook veto and
receives no exemption from this profile.

The production window composition also joins the actual Scenes transcript owner
through a separate exact participant and scoped reader. Original source-departure
and destination-entry events, including scene clock stamps and witnesses, use
bounded prepared ledger tails.
Empty captures retain a genuine owner observation. All configured movement
profiles share one cold transcript-position census, so two receipts cannot claim
the same original event across profiles. Its actual version 2 adapter captures
both Core room legs and binds up to four original events, including repeated
appends to one transcript. Version 1's exact participant and completion binding
remain retained metadata for original single-leg receipts. Older window
manifests without a Scenes group retain their original partial coverage.

The ledger now supplies a separate bounded four-entry ordered preparation for
the two-leg transcript adapter. It copies only selected tails, retains
original per-event sequence numbers even when a log repeats, and emits one final
write per touched chunk, including sealed rollover chunks. Every selected
original tail and seal policy is checked before a binding can publish any final
tail; discarding the binding is inert. This primitive supplies no owner identity
or movement completion, and the existing two-distinct-log batch is unchanged.

Scenes also supplies a bounded passive capture of the original admitted leg
and optional projected leg. It retains each leg's original join-before-leave
events, local clock stamps and witness order, including a return to the source
and explicit no-op legs. Its strict canonical decoder validates route continuity
and per-leg grammar; decoded observations never become an append or movement
capability. Tests compare a two-leg loop with the actual registered legacy
observer. The actual adapter consumes this capture, and its original-history
audit reconstructs each leg only from the genuine Core header, exact original
positions and native event bodies. Missing second-leg claims, changed events,
repeated positions and nonconsecutive appends refuse before migration or load.

An actual composition owner can now retain an explicit bounded list of older
versions of its same participant key in its scoped receipt reader. The current
typed owner must still be registered in the composition. Foreign keys, current
or future versions, duplicates and excessive profiles refuse; each historical
receipt still validates its original manifest and exact retained definitions.
The typed owner stamp continues to govern access, and retaining a reader grants
no execution to an old adapter. The ordered transcript adapter uses this scope
to keep the earlier single-leg receipts readable.

Atlas's current discovery adapter still captures only the admitted-room leg.
It now refuses a different final projected room during both preparation and
historical audit, so the closed root cannot publish that route until actual
second-leg discovery and its knowledge journal join are implemented. Completing
Scenes does not complete Atlas discovery for the second Core leg.

Atlas now supplies a separate bounded passive discovery capture for both legs.
The projected-room observation uses the admitted leg's proposed knowledge
postimage, so a FIFO eviction can require learning the source again on a return
loop. Duplicate visits retain their existing order and produce no new document.
The capture shares the single-leg discovery policy, checks exact route continuity
and original known-book preimages, and distinguishes missing from empty books.
Its canonical decoder grants no movement, window selection or publication
authority. Tests compare both observations with the actual registered legacy
observer; the native publisher and historical knowledge join still need to
consume this capture before the closed root can admit a second room leg.

V2 registers no offer or live command route. Its manifest remains partial:
the bounded original journal refuses a 257th commit until a real checkpoint or
archival protocol is implemented, rather than accepting an unrecoverable tail.
Atlas window map output and other observer effects are not completed, and
personnel evidence remains uncertified. Incomplete history never reconstructs
present placement or an aboard interval.

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

Atlas window placement, NPC memory and work queues, Tacsim return
markers and tutorial state, weather output, and scripted entry/exit effects each
need their own real adapter. Nested movement and script timers cannot be treated
as acknowledged merely because their callbacks returned. No broad World clone,
post-ACK callback replay, or legacy write batch establishes durable completion.
