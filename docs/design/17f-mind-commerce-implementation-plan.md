# 17f — Mind commerce, Registry rights and IC hardware: implementation plan

Implementation continuation, 2026-10-18, on `codex/unit-minds-through-m6`.
M6 is accepted in `17097f5b` with the owner's explicit one-time parity
exception; [17e](17e-field-pilot-acceptance.md) preserves the failed raw comparison
and the passing functional evidence. The current economy checkpoint is IC14
merge `eafe0064caa86411dadb08d4c24ae17488a2bd69`, tree
`2262fd9eb65a765fbbd2ae4d69df8c3ddcd82697`: 49 stages and 1,508 tests pass,
including actual Redis recovery. On 2026-10-18, the owner requested the accepted
through-M6 work land before M7. A fresh fetch confirmed main at `a690412e`, then
a normal fast-forward push advanced it to `6fafaa11dfa6765c0b3a56ef81bc3a2f6e44e4fa`;
an independent remote-ref read verified that result. Executable tracked bytes
are unchanged from the passing IC14/M6 checkpoint; later commits only updated
the five design/acceptance documents. No M7 implementation was in that merge.
[17b §6](17b-unit-minds-implementation-plan.md#6-m6-candidate-and-acceptance-work--2026-10-17)
records the archive and exact provenance. Both implementation prerequisites are
complete. The owner has authorized progress through M7 and progressive pushes.

This note promotes the reviewed isolated design into the programme's tracked
implementation plan. The substantial prepared code remains under active
integration and testing; it is not M7 acceptance or deployment. N35 passed
compile and 215 fast tests (one ignored), then its genuine purchase story aborted
at a nested test-future stack overflow. Its independent server story was
diagnostically cancelled after sustained Registry waiting without an execution
cursor. Recovery of its disconnected runner preserved the original terminal
stage outcomes and separately recorded source verification. N36 was superseded
without execution. N37 failed compilation on a private constant; the narrow
visibility repair allowed N38 to compile.

N38 (`c4f5317d9b5af6464200870f2bd9bb2deed59207`) finished with 392 passing
tests, two failures and one ignored test. The failures were an obsolete
fork-only deletion assertion after retirement support and an unmatched expected
boot-refusal message. Both expensive stories remained unexecuted. All 13 retained
files verify, with unchanged source; archive SHA-256 is
`2a49296d82bf60eb4d29701056bce316d4f11b1ec1358d98c92797d1009d4f2d`.
The source-only F2 renewal capacity prerequisite has independent review and
seven prepared coordinator tests, but no Cargo result yet.

N39 freezes 180 reviewed packages at
`2500edb31d3b2e199d3fe4f9b13574d7a9bb147f`: the retirement assertion repair,
a bounded boot diagnostic preserving the original refusal criterion, inactive
funded-renewal groundwork, first-generation personal entitlements, and captured
retirement entry. Its first startup refused before applying input or compiling:
the host already had an unrelated server release build in
`/opt/rustmush/dev1/build`. That build remains untouched; the same frozen N39
input awaits a clear host lane. No stage result is inferred from this startup
refusal. Generated client bindings still require the actual exporter
before frontend validation. Neither parsing nor partial gates establish the
complete author/buyer story. Each further commit records its actual passing,
failing and unexecuted scope.

The detailed cuts below state the complete intended result. Existing inert
components can land with focused evidence; public paid services require their
own complete phase gates. Hardware, renewal/transfer, the house catalogue and
full acceptance are still implementation work. No Windows Cargo build is needed
for the current loop: Linux uses one exclusive build lane with two Cargo jobs.

## 1. Result and boundaries

M7 delivers a usable IC mind market: Registry conformance and rating; private
filing and public listings; titled charters and compatible lines; owner and paid
licences; pin/follow; renewal, Grace and lapse; sealed/open/open-resale forms;
royalties; resale and charter transfer; current holdings in the OOC simulator;
and titled governed cores manufactured from certified dies and fitted by real
refit work orders. The buyer must be able to buy, fly, tune and resell without
source disclosure or an unpaid/duplicated right.

M6 remains the character workflow owner. Saved source, schema, cards, rules,
drafts, checks, proving, bench and transports are extended, not replaced. New
commercial legality belongs to typed state families from birth. Ordinary
workflow JSON is not an exemption for licences, filings or sortie seats.

M8 remains separate: NPC procurement, ungoverning, greys/heirlooms, testimony and
recognition claims, black-box crime detection, seizure/fines and its zero-PC
gate. M7 must provide Registry recall/revocation and governed renewal admission,
but does not quietly enable contraband. Frontier never mounts in a hull or
authors an act; there is no new Command class.

OOC matches, proving and bench consume no IC money or seat and do not advance IC
terms. They may read committed current IC holdings at admission. Running arenas
read frozen seals, never a market, licence table or wall clock. Actual IC sortie
integration is required if that subsystem exists at implementation; otherwise
M7 must ship the real production seal/close service and exercise it through
production composition, explicitly recording the unavailable region boundary.
A test-only seat counter is not that service.

## 2. Decisions carried into implementation

### 2.1 Owner rulings and written default

**Q-UM-17 is an explicit owner ruling for the dependent owner's licence:** loss
of the current derivative charter owner's qualifying immediate-parent grant
blocks that owner's new derivative filings and owner-licence sortie seals
immediately. Existing frozen sorties finish unchanged. That owner's derivative
and its working copies remain source-sealed until qualifying parent rights are
restored; immutable source and lineage are not deleted. Loss includes lapse,
transfer away, revocation and recall as applicable to that qualifying grant and
exact parent revision. Active/Grace rights remain eligible where doc 17 permits
them; Grace ending is lapse, not an extra unpublished extension.

**Paid-customer clarification, explicit user ruling, 2026-10-17:** "Keep existing
paid licences usable through their own term; block new sales and renewals until
parent rights return." Existing paid derivative admission checks that customer's
own term, form and applicable Registry status, not the issuer's current parent
licence. New derivative sales and renewals must recheck the issuer's qualifying
parent rights before commit. Existing sealed sorties finish. Buyers never need
separate ancestor licences. No term, Grace or renewal date is extended by loss
or restoration of parent rights. This clarification preserves the dependent
owner-use/filing/source restriction above; it does not reopen Q17.

Immutable ancestry/royalties, an applicable Registry recall and the requester's
own grant remain separately named checks. There is no universal live-ancestor-
licence test hidden in the word "dependency"; the operation and licence kind
select the exact applicable check (§10).

**Q-UM-18 uses the written default under continuation authorization:** at most
one pending transfer; neither party admits a new seal while pending; settle on
last admitted sortie close or `min(term_end, accepted_sale + 3 IC days)`.
Outstanding sorties retain their old warrant and held seats after settlement.
The buyer receives only the unheld capacity. Term-end settlement delivers Grace
and a buyer-bound renewal; no seller renewal is drafted while pending.
Registry revocation cancels/refunds; recall follows the instrument with notice.
This is not relabelled as an explicit user ruling.

**Coprates Drill, explicit owner ruling, 2026-10-18:** launch standing means
current Republic membership through the existing standing service. No officer
rank or reputation score is required. Check actual admission/commerce authority
and membership loss after an eligible preview, not merely catalogue visibility.

### 2.2 Selected implementation defaults

* Use typed Roll licence authority plus the prepared participant commit in §4.
  Do not introduce title-backed licence objects as a competing holder source.
  Economy title alone owns a charter; the owner's licence derives its holder
  from that title rather than persisting another authoritative owner field.
* Filing and listing are separate. Filing gives a public Roll entry with forms
  `none` and a free, non-transferable owner's licence. `/publish` may file first
  and then list through an explicit combined service; it cannot charge twice.
  Private filing never makes a mind eligible for future NPC procurement.
* Terms are at most 90 IC days; Grace is at most 3. Renewal auto-drafts one fresh
  E4 contract per paid term, not automatic acceptance/debit. Owner renewal is
  attestation without a synthetic zero-price sale, levy or venue cut.
  A buyer may explicitly request a fresh offer after the exact preceding offer
  closes or refunds, including after parent rights return. Retain linked attempts
  and recheck eligibility; no automatic retry loop or history rewrite is allowed.
* An owner's free capacity is `OwnedDispatch`, not the paid offer's seat count:
  it covers the holder's owned hulls dispatched under current authority. Each
  actual `(unit, sortie)` still consumes one bounded open-claim record. Multiple
  owned hulls may dispatch concurrently; storage limits may refuse new work
  before admission but cannot redefine the licence as a one-seat purchased grant.
* A dependent owner's `term_end` never exceeds the qualifying immediate-parent
  grant's `term_end`; its `grace_end` never exceeds either the parent's
  `grace_end` or its own term end plus configured Grace. Apply this at initial
  filing, owner attestation, replacement qualifying grant and charter-holder
  change. If the parent is already in Grace, permitted filing creates a child
  in the remaining bounded Grace interval, not a fresh full-duration Active term;
  record its real issue time and inherited term boundary without fabricating an
  earlier filing or Active interval. No remaining eligible interval means
  refusal. Parent renewal/restoration never
  extends the child automatically: a fresh valid child attestation is required.
* Primary and renewal proceeds: Registry levy (seed 50‰), optional venue,
  parent chain, charter-title owner's remainder. Secondary licence resale uses
  the same disclosed cuts but pays the remainder to the selling licence holder.
  A separately priced charter title sale does not silently acquire mind-use
  royalties. Aggregate parent royalties are capped at 300‰ across depth ≤3,
  not 300‰ independently for every ancestor.
* Compute all non-remainder shares from gross using checked integer arithmetic
  and floor rounding; merge duplicate accounts, omit zero shares, assign the
  whole remainder once. Use E4 Exact shares, not its remainder-relative Permille
  semantics. At most Registry + venue + three ancestors + final recipient are
  distinct payees. Reject overflow, excessive cuts and invalid accounts before
  money/approval/escrow publication.
* Freeze price, payees, royalty terms, title revisions and effect identity in
  consent. A changed required title/payee binding before durable settlement
  refuses/requotes or refunds; it never redirects an accepted payout. After
  durable intent ownership, replay completes those exact images.
* Withdrawal excludes a revision from new offers and follow resolution;
  existing licensed pins and historical seals remain. Only Registry recall
  disables that revision for otherwise lawful new pinned seals. No author-side
  withdrawal is a hidden revocation power.
* A compatible qualifying grant held by the current charter owner to the same
  immediate-parent line can restore that owner's dependency; it does not erase
  exact source ancestry, clear an applicable recall, extend the child's term or
  rewrite an old seal. Traverse immutable ancestry for bounded royalties and
  integrity, not to demand a separate paid licence to every ancestor. A new
  holder's shorter qualifying term re-bounds owner eligibility immediately;
  a longer one does not lengthen the already-attested child term.
* Charter transfer preserves old frozen sorties and applies the same pending
  discipline to affected owner-licence admission. Use a charter-level pending
  record/claim aggregate, not one unbounded transaction rewriting every line.
  Generic title transfer/collateral routes must honor this policy too.
* Preserve genuinely separate pre-filing personal drafts under the OOC draft
  exception. Server-known filed/derived copies retain provenance through copy,
  save, rename, removal and source replacement. Do not infer derivation from
  behavioral similarity or offer a similarity oracle.
* Initial hardware accepts exact quantity-one certified Origin die lots.
  Generic grade/crystallization never creates a die certificate. Governed
  refits use finite content-authored compatible fitted-unit definitions, retain
  unrelated damage/ammo, and never call simulator full `refit_unit` as a repair.

These defaults are selected design choices within the authorized task. A later
departure must be recorded before its implementation and must not be presented
as a previous owner ruling.

### 2.3 Chosen provisional launch tuning

The percentages, 90-day/3-day limits, 50-permille Compact levy, Class III
restriction and Q17 policy come from doc 17. The following **new configurable
launch values are implementation choices**, not discovered prices or owner
rulings. Author them in a versioned commerce-content record with strict units
and bounds; record its digest in quotes/receipts. Missing content refuses
activation rather than silently falling back to free goods.

| Setting | Selected initial value and semantics |
| --- | --- |
| Line-tank catalogue anchor | 100,000cr for `tacsim.unit.line_tank`, a new explicit economic catalogue row. No current tank price exists. The existing 120,000cr Dray prose price and 90,000cr Co-op buyout are unrelated. |
| Finished replacement core references | I 3,000cr, II 8,000cr, III 25,000cr: exactly 3/8/25% of the tank anchor. The reference prices are for the completed core, with refit separately disclosed. |
| Die/integration split | Certified die 90% and paid integration 10% of that core reference. A customer-supplied die is not charged twice. A supplied finished core follows the ordinary asset sale path. |
| Installation service | 100cr per accepted Install/Replace work order, separate from the supplied asset. A failed/cancelled undelivered service refunds under its frozen terms. |
| Registry filing | 100cr for each newly filed revision; publish of an already filed exact revision charges no filing fee again. NPC houses use the same fee. |
| Paid launch house licence | 400cr, sealed, one seat, 90 IC days; named content may choose a different explicit form. Renewal for the same term initially costs the same 400cr, frozen in the accepted licence's renewal terms. A later price edit needs new consent, not a hidden renewal debit. |
| Renewal offer interval | Three IC days (259,200 seconds), configurable from 1 to 259,200. This is the disclosed offer interval, separate from predecessor Grace. Automatic drafting selects the unique actual personal account for the accepted instrument; absent or ambiguous accounts require buyer choice before an offer is created. |
| Paid-seat representation ceiling | `MAX_PAID_SEATS = 1,024` per paid instrument in the prepared M7.1a value codec. This is a selected implementation ceiling, not the launch seat count, an open-claim storage budget or an owner's dispatch limit. Later guarded capacity reservations may refuse new work below it. |
| Venue fee | Direct author listing 0; a house-vendor listing 50 permille. This is additional to the Compact's 50, not the Co-op's 250-permille proceeds share. Parent royalty remains an explicit listing term within the aggregate cap. New offers must explicitly freeze their downstream royalty rate, including zero. For existing v2 OpenResale grants that have no rate field, new versioned resale consent will capture an immutable 0-permille parent edge; old grant/proof bytes stay unchanged and no later listing may add a fee. This selected compatibility rule is preparation, not an implemented producer. |
| Importer allotment | Basin: I 64, II 16, III 4 exact quantity-one dies per WorldIc month, shared by the Ceres and Phobos agents. No boot, connection or per-agent reset. |
| Free content | Standard Doctrine and its six preset pages remain free. An owner's licence has zero-price attestation, not a synthetic sale. |

Currency is the existing `tacsim.credit`, scale 2 (centicredits). Convert major
content prices with checked multiplication. Core-reference percentage math
floors to centicredits and exposes that rule; these initial values divide
exactly. Use checked minor-unit share arithmetic from §2.2. Bound quotas to
1..=1,024 per class/month, venue share to 0..=1,000 with aggregate validation,
and all prices by existing positive `Amount`/quote limits. Config changes cannot
alter an accepted quote, existing renewal consent or an owned replay bundle.

The initial refit family is finite: existing `line_tank` remains the Class II
form; add explicit I and III fitted definitions differing only in the registered
turret `core` binding/capability. All unrelated weapons, sensors, section
consequences, damage and motion remain. Existing laser/thermal/ECM/ECCM lines
are different chassis variants, all II; scout/drone are I, bipeds II, heavy III.
The target and Dray have no existing mount. Do not generalize installation by
matching a label, weaken boot topology, or add owned hardware to OOC pods.

## 3. Authority, records and bounds

### 3.1 Owners and dependencies

`mush-roll` owns typed commercial records and the rights evaluator. It retains
the existing draft/revision/reference owner and takes no market or tacsim edge.
Use economy/org identifiers only where required; game semantics enter through
registered adapters. `tacsim::minds::registry` supplies rating, core/station/floor
admission, public projection, and composition. `mush-economy` owns accounts,
escrow, title and money. `mush-market` owns new entitlement listings, quotes,
capacity and settlement indexes in typed families, alongside its unchanged
legacy commodity market. Core/server own
the composed durable boundary. Tactics owns unit topology; inventory owns lots;
industry owns work-order capacity, schedule and material lifecycle.
Tacsim owns Safe Conduct role content and the bounded typed underwriting
product/quote/policy/exposure/claim families; these enter the same ActionTxn
owner-delegation discipline, not another mutable after-payment store. They use
the existing Economy and Roll-root participants where money/title/retention
needs them, without exceeding the reviewed rights lane's two legacy owners.

Source stays at the existing immutable tacsim source owner. A Roll/market DTO
never contains source simply because a server-side `HeldMind` does. Hidden org
names continue through the established renderer. Authentication AccountId for
the alt prohibition is distinct from economy AccountId for payment.

### 3.2 Typed commercial model

| Record/family | Essential authority and identity |
| --- | --- |
| Charter, line | Titled world object; bounded compatible-schema line identity; owner derived from E4 title; public name/role/house projection |
| Filed revision | Exact saved RevisionRef/source/definition digests; immutable ancestry; line sequence; withdrawal and Registry recall facts |
| Rating receipt | Exact source/envelope, ABI/engine/class-table/scenario manifests; per-profile worst operations/fees and faults; minimum certified class |
| Licence and term | Stable licence ID; paid holder or title-derived owner kind; warrant/scope; pin/follow; class/form; `PaidSeats(n)` versus `OwnedDispatch`; immutable term generation and WorldIc dates; holder-specific immediate-parent binding and bounded dependent dates where applicable |
| Pending transfer | Unique transfer generation; old licence/title revision; seller/buyer; contract; frozen consent digest; deadline; no new admission |
| Seat claim / open seal | `(unit, sortie)`; exact licence/revision/warrant; captured seal digest; open/closed receipt; survives holder change until real close |
| Listing terms | Exact line/form/seat/term/class/transferability offer, venue and frozen economic terms; no execution authority by itself |
| Entitlement quote / market capacity | Market-owned typed listing/quote identity and counters; frozen terms/effect digest, buyer/seller/contract links; bounded query indexes and reserved terminal capacity |
| Commercial operation/receipt | Permanent operation identity, phase/generation, exact effect and economic result; terminal capacity reserved before funding |
| Hardware certificates | Import quota/receipt, exact die, integration output, core asset and installation binding; distinct from a mind charter |
| Cover policy and claim | Versioned product/quote; authenticated exact IC seal/role proof; insured hull/title/value; premium and reserved liability; term/exclusions; immutable mechanical-loss/close identity and claim/wreck-title receipt |
| Inert shell reservation | Stable operation key to reserved ObjId and immutable object shape; durable acknowledgement precedes filing/manufacturing; no title, certificate or execution authority by reservation alone |
| Deployment markers | Explicit initialized-by commit shared by Roll and core shell service; checked permanent allocation/birth/consumption counts distinguish an empty deployment from lost records |

Active/Grace/Lapsed, Recall, Registry revocation, dependency and PendingTransfer
are independent facts. A dominant display refusal must not discard another
fact. New admission evaluates integrity → current principal/title → sanctions →
dependency → term → pending transfer → exact revision → class/station/floor →
IC seat. The captured seal is independent thereafter.

The dependency step names the operation and licence kind. A dependent owner-use
request checks that current owner's immediate-parent grant. Existing paid
customer admission checks their own derivative grant and applicable Registry
status, without consulting the issuer's live parent grant. New sales/renewals
check the issuer's qualifying parent rights before commit under the explicit
2026-10-17 ruling. Neither case infers ancestor purchases from the royalty chain.
Owner dispatch checks current hull title/dispatch authority;
paid seats enforce the purchased count. Both retain old open claims through
holder changes and use the same bounded idempotent close mechanism.

All families have strict codecs, explicit versions, non-reused IDs, checked
counters, bounded payloads/indexes and guarded transitions. Establish numeric
record/page/operation ceilings in the first model cut against actual encoded
worst cases; use existing action/bundle ceilings and refuse before funding.
Do not increase global limits to fit a large sweep. Large line histories and
follow roots use per-revision records/bounded pages, never one unbounded vector.
Receipt identity cannot be discarded by a rolling marker cache. Reserve terminal
receipt/root capacity with every pending obligation; closing/refunding existing
work must remain possible when new-work capacity is exhausted.

New entitlement market rows live under market-owned typed namespaces, not
mirrors in `MarketStore`. Use existing ActionTxn owner delegation and the
market's typed-family precedent (`lot.rs::DrawReceipt`), with a closed commerce
terms adapter. Market-rooted sale actions receive only the delegated Roll
commercial handles and registered participant handles they need; filing is
Roll-rooted. Composition fixes the root, grants and readiness. No client chooses
an owner or participant. Do not borrow the legacy quote allocator, create a
dummy legacy quote or add an after-ack authoritative mirror. Ordinary market
commands/rendering route by representation to the correct owner; commodity
counter/index behavior remains unchanged.

### 3.3 Retention and source rights

Commercial typed records are rights authority. Conservative external Roll roots
are collection authority. Maintain exact roots in the same commit for filed
history, pins/follow eligibility, listings/contracts, pending transfers/renewals,
lineage and open seals. Existing kept match/tape/proving/bench roots remain.
An owner/title change cannot release a still-open seal. Source-byte GC remains
deferred until every source owner is indexed.

Use distinct capabilities for execution, card read, probe, source read, memory,
fork, private derivative filing, derivative listing and charter administration.
Sealed grants execution/card/probe, never source or private memory. Open adds
source/fork/private filing while eligible; open-resale additionally permits a
listed derivative. Source access does not grant another principal's run memory.

Q17 must cover **owned derivative drafts**, not only filed licence lookup.
Persist immutable server-owned lineage and use one shared access predicate at
Read/Copy/Fork/Edit/Save/Check, SSH, web/TUI, trace, EVAL/WATCH, proving/BENCH and
export. Recheck current actor/connection and rights when queued work completes
and before reconnect delivery. Client envelope/source replacement cannot erase
lineage. Already downloaded bytes cannot be revoked; no server path may disclose
fresh bytes after rights loss. Unknown/missing/foreign source refusals remain
indistinguishable.

### 3.4 Committed facts for contracts and fronts

Register one `minds` FactProvider with these exact M7 kinds. They read the
committed commercial records/receipt projection after acknowledgement and after
boot hydration; they never grant a right or invoke a second settlement callback.

| Kind | Exact subject and temporal meaning |
| --- | --- |
| `minds.filed` | A specific immutable filed-revision receipt. True once that filing commits; withdrawal/recall does not undo the historical filing. The public Roll entry may expose its source-free reference. |
| `minds.licence.lapsed` | A specific licence term generation and its committed lapse occurrence. True once that occurrence commits; later renewal is a different generation and does not erase the event. Current admission still derives the exact WorldIc boundary without waiting for a fact-projection sweep. |
| `minds.licence.transferred` | A specific completed licence-transfer receipt and generation. PendingTransfer is not completion. Old open claims and warrants remain independently retained. |

Bound IDs to the exact record/generation and reject malformed IDs, wrong
revisions, unavailable owners and unknown kinds uniformly. Preserve the event's
actual RealUtc audit time in `FactState.at` (or 0 when unknown), never a WorldIc
term timestamp disguised as unix time; IC dates remain in the referenced record.
The event actor is audit attribution, not a principal authorization shortcut.

Public filing facts expose only the existing public Roll projection. Licence
facts are scoped to the holder/title principals and the explicitly authorized
contract/front participants; they must not reveal another holder's instrument
through guessed IDs. Current `FactProvider::resolve(&World, kind, id, revision)`
has no caller/contract context. Add a narrow opt-in scoped authorization seam:
the actual draft/offer/read/settlement paths supply current authenticated
principal and immutable contract/front scope; the `minds` provider refuses
restricted references without it. Validate condition attachment and disclosure
against that scope, and recheck changed parties/authority before offer and use.
Existing providers retain their existing behavior. Do not substitute a
client-supplied actor, an unguessable-looking ID, or a generic source-bearing
fact result for this boundary. The pure engine still receives only authorized
fact truth; the read bridge does not become a mutation owner.

`minds.seized`, `minds.unwarranted` and `minds.claimed` belong to M8 and remain
unsupported in M7. Inventory the three registered M7 kinds, provider namespace,
allowed source-free fields and scope consumers alongside commands/state owners.

## 4. One durable business transition

### 4.1 Prepared state owners and the authoritative receipt

The selected seam is detailed in `target/m7-composed-participant-sketch.md`,
subject to the independent review's corrections adopted here. This is a closed
prepared-state interface, not an arbitrary plugin callback:

1. A registered typed `ParticipantHandle<Request,Output>` is bound to one
   composition, provider and owned module slot. Owner-private preparation returns
   a non-cloneable patch and exact bounded replay images. The calling provider
   gets only prepared result facts. No client supplies writes or owner identity.
2. `PreparedComposedTxn` contains the normal typed transaction and, for rights,
   Economy and Roll retention owner images. Registry completion needs three
   distinct installed slots: Roll retention, Tacsim BulkStore and Tacsim Artifacts.
   The prepared core bound is four registered slots, at most three per invocation
   and at most two distinct module owners. These are fixed composition bounds;
   requests cannot select or expand them.
   New market/rights records are in that same typed transaction and consume no
   additional legacy slot. Hardware has its separate bounded image matrix below.
   A registered read-only preparation context supplies owner state/authority;
   current action providers do not otherwise receive World. No Ctx, outbox,
   live mutation or I/O is available during preparation.
3. Canonicalize the complete union once; reject duplicate physical targets,
   including LedgerChunkSet/JSON aliases, and retain exact equality with the
   authoritative bundle. Preserve dirty-world/module-op and predecessor checks.
4. `StagedAction` transfers one `ComposedInstallation` into `PendingCommit`.
   The writer sees the bundle only. Exact token/hash final acknowledgement is
   the only live installation boundary; ownership acknowledgement is not success.
5. Bind typed installation and all distinct owner slots before the first write.
   Validate composition, installed-slot incarnation, complete pre-state fences
   and generations. Bound installers mutate only their held state/fields and
   cannot fail, serialize, call another owner or queue persistence. The consumed
   erased capsule moves its patch and any immutable `Arc` adapter/config into
   `BoundOwnerInstall<'a>`; the returned installer borrows only the supplied
   owner state/fields, never the destroyed capsule or a temporary `&self`.
   Keep the typed reservation guard outstanding until the last apply returns;
   only then settle it and release notification-only output.
6. Before custody, stale/refused work changes nothing. After an indeterminate
   durable result, freeze; do not replan or undo money locally. Restart first
   replays retained exact images, then hydrates typed/Economy/Roll state and
   audits references. No retained Rust callback or fresh commercial admission
   runs during replay.

Core changes are real scope: `action_state`, participant registration,
`ModuleStateMap` identity/disjoint binding, engine staging/install, coordinator
assembly and world-task pending receipt. Reuse ReplayWrite and store recovery;
no new persistence format is justified merely to carry an in-memory callback.
Factor E4's complete prepared delta/image construction and Roll/Ledger
check/bind/apply helpers. Ledger tail checks happen before any apply; E4 approval,
meta/key/index facts and serialized writes are prepared before any RAM change.
Current `install_commit`, `apply_prepared`, typed `install` (which settles its
guard immediately), and Roll `publish_with` are not already this bridge.
Installed-slot incarnation changes on replacement and clone, not just owner
generation, and is process-local rather than a persisted Rust TypeId.

Every effect-bearing contract uses closed `Accept/Activate/Settle/Cancel/Expire/
Forfeit` phases. Partial acceptance grants no execution right. Zero money still
commits contract/typed phase/root images; it may have no economic transaction ID.
Do not bypass the seam through direct accept saves, empty-refund saves, pending
unwind saves or a settlement-then-licence callback. Wire commands and internal
sweeps before funding these contracts. Ordinary E4 contracts retain their path.

### 4.2 Notifications cannot invoke legacy settlement mutation

Every authority-bearing payment, right, market capacity/quote/index and retention
effect is prepared before submission. New composed effects **do not dispatch
the existing EconomyObserver list after acknowledgement**. That list mutates:
MarketObserver stamps quotes/delivers/settles IC10 proceeds; CanonEvidenceBridge
commits Canon and an Economy marker; CoopObserver persists enrolment changes.
The receipt path's no-write/debug assertion is not an atomicity mechanism.

Emit prepared notification-only facts through the coordinator's durable receipt
or bounded outbox. Persistent derived Canon/inbox projections use explicit
idempotent follow-up actions keyed to the retained committed cause and retried
until acknowledged. They cannot be the action that grants a licence, marks
payment successful, consumes capacity or roots paid content. Reserve bounded
outbox/terminal capacity before funding. Preserve the ordinary observer fan-out
for ordinary commodity and Co-op contracts; identify the new closed effect type
explicitly so it is neither processed twice nor accidentally swallowed.

### 4.3 Durable inert objects precede business activation

Charters and manufactured cores need real ObjIds. Give them a separate durable,
idempotent `ReserveShell` step with operation-key-to-ObjId mapping, immutable
shape, owner/module purpose and bounded reservation capacity. Coordinator
idempotency preflight runs before allocating another identity. Wait for its
durable acknowledgement, then submit filing/manufacturing against that exact
existing shell. Never `World::create` and stage commerce in the same dirty turn.

Implement reservation as a narrow core-owned prepared object creation with
guarded allocator/object/container images and the reservation record in one
durable union. This is not permission for arbitrary World replacement. If an
equivalent acknowledged creation operation exists at M7.0, reuse it; otherwise
this explicit small core addition is part of M7.2. It keeps creation out of the
later two-owner rights transaction and survives a crash between the two steps.

For filing/integration, activation is **derived** from committed typed records
plus the E4 title/certificate. Pre-reserve the required immutable shell shape;
activation does not clear a flag, set an attribute, move an object or change its
World owner after the business receipt. Public rendering checks the typed
authority. Failed/retried work leaves an unusable shell, not a free asset.
Bounded idempotent cleanup checks retained intents/reservations, committed
titles/certificates and live rights before reclaiming an abandoned shell.

Initial installation is explicit. `Initialize` composition registers the typed
families without reserve/filing/rating admission and remains Closed. A God-only
self action proves all actual commerce/rating/shell families and complete E4
filing history empty, with a successful source audit and no commercial roots
or quarantine. It writes both Roll and core shell deployment markers in one
acknowledged transaction; it does not open commerce. Restart in `Active` runs a
synchronous final composition audit after every ordinary loaded hook. Missing
either marker, missing all reservations, absent roots or incomplete historical
E4 proof refuses recovery. Active boot never falls back to initialization.
Permanent shell birth/consumption counts and Roll count/max/next-ID invariants
detect lost families even when every business row has disappeared. The empty
checks run through the actual typed transaction and its revision fence; deleting
staged rows cannot manufacture an empty pre-state.

### 4.4 Hardware's additional prepared images

The rights-phase two-slot use is not a claim that all M7 state fits
Economy+Roll. Use this closed matrix and refuse undeclared owner expansion:

| Transition | One typed transaction | Legacy owner images | Core built-in image |
| --- | --- | --- | --- |
| Reserve charter/output shell | Reservation key and purpose | None | Guarded allocator plus inert object/container creation |
| File / purchase / renewal / resale | Roll commercial + new market records | Economy + Roll retention | None; shell usability is derived |
| Registry completion | Roll rating job/receipt/pages/completion/status | Roll retention + Tacsim BulkStore + Tacsim Artifacts | None |
| Exact-die integration | Inventory/industry/work-order/certificate/payment phase | Economy; another owner only if a real legacy write is proven necessary | None if output activation is derived |
| Core Install/Replace | Installation/certificate/inventory facts | Economy + Tactics | Closed bounded core/object/container exchange |

Do not combine refit with mind selection or sortie checkout: those are separate
business actions and would otherwise add Roll roots/another owner unnoticed.
Hardware's object image includes both moved cores and every affected container's
contents/timestamps/derived index image, merged to one image per physical target.
Tactics includes active lifecycle records and payload digest as well as unit
cells. A core-private field-split binder borrows the object/index fields alongside
disjoint Economy/Tactics slots, completes **all** checks, then applies prebuilt
images without mutable hooks, serialization, fresh lookup or dirty writes.

Keep the existing legacy single-`CustodyEffect` arity limit unchanged. A new
closed prepared Install/Replace exchange is not two sequential legacy moves
hidden in one provider. Its bounded multiple moves are validated and installed
as one object-image set. A later unsupported container/index operation refuses
before custody rather than falling back to `World::move_to` after payment.

Final guards cover all economic facts used, not only transaction allocator and
ledger tail. Roll reference installation advances the same generation as M6
prune: prepared prune then new root refuses stale prune; prune first refuses the
old root preparation. CommitPending prevents interleaving while awaiting receipt.
Malformed/unreadable retained rights at boot block collection and unprovable
rights, without exposing which foreign owner/source caused the refusal.

These bindings are a trusted owner implementation contract, not a safety proof
from returning `()`. Compile and fault-test the erased ownership/lifetime seam
before E4 business adaptation: two heterogeneous patches, alias/missing/foreign/
cloned slots, second bind failure after first bind, guard lifetime, drop and
panic. A bind failure changes nothing; an unexpected apply panic freezes/poisons
the owner and requires recovery rather than pretending rollback occurred.

## 5. Ordered implementation cuts

Each cut ends with a reviewable diff, focused correctness gates and an evidence
entry. A model or inert adapter may land before its command; no partial paid
service is exposed. The cuts below are deliberately smaller than the original
M7 row.

| Acceptance phase | Cuts | Required exit evidence before enabling the next authority |
| --- | --- | --- |
| A: prerequisites and commit substrate | M7.0–2 | Accepted M6 and fresh economy checkpoint; strict owner/readiness inventory; real memory/Redis bind/replay/failure matrix; ordinary contracts/observers unchanged; durable inert reservation proved. No paid mind entry point yet. |
| B: certified ownership and use | M7.3–5 | Deterministic per-card rating/promotion; fee/title/owner-right/root atomicity; source ACL including derivatives and delayed work; real IC claim/close service and unchanged OOC seats/money/time. Enable private filing/use only after this phase. |
| C: commercial rights and catalogue | M7.6–8 | Typed market and mixed legacy compatibility; exact payouts; all transfer/renewal/parent-loss races; scoped committed facts; named house source/filing/eligibility and actual Safe Conduct role/underwriting services. Individual entries may activate after their gates; absent IC-host live acceptance alone follows the explicit host condition, while missing role/services remains incomplete M7. |
| D: hardware | M7.9–11 | Certified bounded supply and real manufacture; titled-core sale; all-owner object/Tactics exchange matrix; current Class III permission; preserved hull state and no simulator ownership dependency. No live refit before this phase passes. |
| E: full acceptance | M7.12 | All required stories, catalogue dependencies, compatibility and operational gates on one exact clean revision; retained artifacts and truthful deployment record. |

Pure content/adapters may be developed in parallel after A, but paid service
activation follows the phase gates. A partial phase is a checkpoint, not M7
acceptance. Do not defer an explicit M7 requirement merely to keep one diff small.

### M7.0 — Accept the prerequisites and freeze shared interfaces

Record M6 acceptance separately. Fetch/reconcile current main and record the
merged SHA, actual in-flight trade owner interfaces and checkpoint results in
17d §7. Review E4/title/issuer/lease/market hooks and the M6 source/holdings seam.
Run the checkpoint commands in §7; no arbitrary quiet period or requirement to
finish all IC13 work substitutes for that evidence.

Track IC10's 4,096 inventory-marker deployment blocker and its other documented
delivery/boot residuals as existing trade constraints. IC12 did not repair them.
IC12 also makes Treasury and Bids readiness close Co-op enrolment, including a
house bid that cannot fund one unit (Q-18-60); Sky remains reported/paged but
ungated. Record this existing trade policy and its normal-funds-depletion effect
without silently weakening it or making M7 own that unrelated policy change.
Do not copy the marker scheme or silently make this cut unrelated trade cleanup.
If those paths are required by the chosen deployment, activation waits for their
tracked repair. Coordinate `contract_live`, market APIs and tacsim composition;
preserve IC13 buyout/end-lease RetryShortly/Retrying behavior and inflight orders.

### M7.1 — Typed records, access evaluator and retention reservations

Implement Roll's §3 records in `mush-roll`, market-owned entitlement families in
`mush-market`, and game adapters in tacsim; Roll retains no market edge.
Add pure guarded transition plans, bounded IDs/indexes, capacity reservation,
readiness/quarantine and derived term/source admission. Persist derivative draft
lineage through the existing workflow without permitting client edits to it.
Make `PaidSeats`/`OwnedDispatch` and the holder-specific immediate-parent binding
distinct typed variants. Validate dependent term and Grace bounds at every plan
that creates, renews, restores or changes the owner-derived grant. Implement
§3.4's event references, scoped fact bridge and exact M7 kind inventory; no
unscoped licence lookup may be exposed through the generic Fact condition.
Register the fixed action-root/delegation matrix, separately named typed market
quote/listing counters and bounded seller/buyer/contract indexes. Add strict
versioned §2.3 commerce content and reject invalid shares/quotas/price units.

Gate: model/codec bounds; independent status combinations; exact deadline edges;
depth/cycle/parent recall; no source oracle; complete boot cross-references; full
capacity still permits close/refund; both stale-root/prune orderings;
initial/renewed child term never outruns parent, parent-Grace filing and changed
qualifying-holder bounds; owner capacity distinct from paid seats; foreign and
missing restricted fact references indistinguishable; market owner/delegation
refusal and no legacy counter consumption. No payable
command is enabled by this cut.

First filing additionally requires explicit deployment setup/restart as §4.3
describes. Real Engine tests must prove both markers share one commit, prepare
and dropped custody publish neither, setup remains Closed, retained acknowledged
images recover after a lost installation, and either missing marker or repeated
initialization refuses. No test-only readiness setter proves this boundary.

### M7.2 — Core composed commit and owner adapters

Implement §4 first with one minimal typed operation plus real E4 and Roll root
participants. Preserve typed-only providers through a zero-participant wrapper.
Add slot incarnation and bind-all-before-apply; no fallible publication loop.
Use real memory and Redis authoritative stores.

Split this cut into reviewable commits: (a) compiler-checked erased core binding
and zero-participant compatibility; (b) Ledger/Economy/Roll prepared images and
all closed effect phases, including zero-money terminals and sweeps;
(c) notification-only new-effect routing plus durable derived-projection retry;
(d) the narrow ReserveShell lifecycle. Do not enable effect-bearing funding
before all phases are wired. No arbitrary new callback or owner count is hidden
behind a generic `FnOnce`.

Gate: the complete fault matrix in §8, zero-money phases, exact equality and
collision checks, all-or-none binding, lost-install freeze, replay past expiry,
no observer persistence and unchanged ordinary action/economy paths. Use the real
observer composition to prove a new settlement emits no legacy module ops,
World dirtiness, delivery callback or second grant; ordinary IC10 commodity
settlement and Co-op lease termination still project exactly once. Kill between
shell reservation acknowledgement and business submit, and retry/cleanup across
restart without allocating another shell or granting title prematurely. Independent
review must follow this cut before business services depend on it. Fact reads
remain false/unknown before receipt, become exact after publication, and match
fresh boot hydration; lost acknowledgements/replay cannot duplicate an event or
expose a fact ahead of the authoritative state.

### M7.3 — Registry conformance, rating and re-rating

Reuse M6's isolated bounded worker for a separately identified fixed Registry
scenario manifest `combat.v1`. Compile current ABI; validate exact schema, ≤8 presets,
profiles/rules, declared class, station vocabulary, floor and memory limits;
exercise armed and unarmed roles. Measure actual operations plus kernel fees
and faults for every declared profile/preset; bind source/envelope, engine,
class table, scenario/chassis/ground/seeds and result digest into a certified
receipt. A user `/check`, win rate, source declaration or fixture hint is not a
Registry rating. Non-budget correctness faults refuse certification; ordinary
tactical refusals (cycling weapon, vanished contact) are reported outcomes,
not a failure merely because a valid request could not land.

Reserve exact terminal keys before admission. The durable admission CommitToken
keys the receipt, status and ordinal pages; the mandatory RatingRef carries that
job key, the measured certificate-content RatingId and the full immutable receipt
digest. Pages bind job, rating and result. Pending retention roots belong to the
actual queued/running job and are exchanged atomically for its exact final rating
root on completion. Failure/cancellation removes only the pending root; the
auditable job remains. A retry of one job accepts only identical complete bytes.
A new attempt gets a new job, preserving prior results and recall status even
when the measured certificate content is identical. Neither a planned job ID nor
an arbitrary structurally valid receipt constitutes certification.

The initial versioned manifest is concrete:

* Freeze the exact class table, floor, ABI/engine identity, source/card/rule and
  ground/chassis digests, 2,000ms startup and 600,000ms maximum duration. Use the
  existing Proving Ground v5 and Duel/Muster/Hold layouts, seeds 1 and 2 and both
  side orientations; run Duel 1v1, Muster 2v2, Hold 2v2 against pinned Standard/Line.
  Retain the current explicit proving chassis mapping: line_tank at I/II and
  heavy_mk1 at III, with the candidate's exact tested class schedule; freeze the
  applicable unit/weapon/sensor definitions rather than inferring them from role.
  This is 12 whole trials per distinct canonical card/profile, with real memory,
  schedule, sample lease and station landing.
* Exercise every unique preset/card and authored profile, not merely the first
  profile that the existing six-picture check executes. Map aliases to identical
  measured canonical cards. At most 8 presets plus the class's 2/3/4 profiles bound
  the set. Each card gets Empty/Hostile450/HurtTwo on armed and unarmed public
  pictures and a deterministic multi-think order/lock-loss/miss/hurt/dry/profile
  transition sequence. Promote those fixture definitions into production
  content; do not import `tests/` code.
* Add capped public-picture probes with the class's full 12/16/24 contact rows,
  allowed odds rows, rule slots and memory limits. A 2v2 match cannot measure
  the maximum-row case. Include the shipped ford/motor route and disabled-motion
  boundary; use existing public observations only. Unarmed mobile drone-style
  probes remain unarmed, and no absent target/Dray core mount is invented.
* Use a small fixed-manifest factory over the existing pure scenario/custody/tape
  builders. The current owner-selected PROVE factory's 10..100-game request
  bounds and mutable user choice are not Registry authority. Keep Registry jobs
  separately identified, bounded and scheduled fairly alongside live work.

The manifest is sampled conformance, not a theorem about all future branches.
Runtime class limits remain hard. Worst observed ops plus fees determine the
minimum complete passing candidate class; wins remain reported quality. The
bounded maximum is 12 cards x 12 full trials x 3 candidate classes = 432 trials
plus explicitly capped probes. Emit progress/cancel results without source or
freshly altered operator settings changing an in-flight receipt.

The measured minimum may exceed the author's declaration, as doc 17 requires.
Keep the saved source/envelope immutable: add an explicit certified execution
requirement to filed custody and the shared seal adapter. Do not silently rewrite
`CardDefinition.declared_class` or pass an arbitrary rated hint. The current M6
`seal.rs` equality between declared and rated class is a known integration change.
Try permitted core classes from the declared minimum upward against the same
manifest, requiring a passing exact candidate; the receipt authenticates any
promotion. Hardware refusal still precedes optional class caps, and exact final
card/rule capacity is checked at the class actually sealed.

On a Rhai/ABI/class/scenario compatibility change, queue bounded re-conformance
for live revisions. New admissions require a current valid certification;
Registry recalls failures with private holder notice. Existing frozen sorties
remain intact. Gate: repeat/cross-host deterministic identity; class-boundary
promotion; unknown/stale/tampered receipts; cancellation/restart; worker resource
isolation; no fee or live-cache pollution on failure.

### M7.4 — Atomic filing, charter and owner's licence

Register the Registry issuer and fee/levy sink. File one exact saved revision
only after certified conformance, canonical duplicate check, lineage authority,
capacity and payer/title authorization. Use a durably acknowledged M7.2 shell
with bounded idempotent cleanup; derived activation, title, fee, typed records, owner
licence, references and receipt share the composed commit. An inert reservation
has no execution or commercial authority. Do not call `issue_title` and then
fallibly save a licence. Do not mutate the shell's World fields to activate it.

Support compatible line updates, explicit new majors, pin/follow resolution,
withdraw and Registry recall; followers choose the newest eligible revision
filed before the seal. Cross-charter normalized duplicates refuse generically
without naming a charter; clean-room similar behavior remains legal. An ABI
change cannot make another owner's retained identity disappear from duplicate
or receipt checks.

Use a separate canonical duplicate identity: normalized retained source SHA plus
exact schema, presets, profiles, programs/rules and declared class, excluding only
the runtime `mind_abi` identity. Exact revision and rating identities still bind
that ABI. Persist the duplicate identity in filed and index rows; boot derives
it again from the actual source-audited revision. Keep compatible-line schema
identity separate from the Registry execution-compatibility manifest. ABI-only
refiling collides; changed source/schema/program inputs change duplicate identity.

Gate: file/retry/reboot exactly once; failed fee/title/conformance leaves no
active right; filed-unlisted owner use; public forms `none`; parent authority
lost after rating but before file; line compatibility; old pins/seals on update;
withdraw versus recall; hidden-org renderer; no procurement eligibility.

### M7.5 — Shared holdings, source ACL and real seal/close service

Extend `match_/holdings`, the shared workshop access predicate and sealed origin
with exact committed filed licence/rating facts. Selection may show prospective
state; checkout resolves/rechecks once and captures immutable custody. A stale
selection is not authority. IC seal atomically claims `(unit, sortie)` and roots;
close is an idempotent production operation and never clears someone else's
claim. OOC checkout reads holdings with no claim, payment or licence-time write.
For `OwnedDispatch`, authorize each actual owned hull and dispatcher rather than
subtracting from the paid launch offer's one seat. Concurrent owned hulls each
get an exact open claim. A charter transfer changes new owner admission, not
old claim identity or warrant; global bounded capacity refuses before sealing.

Loss at a new seal follows the established Doctrine fallback/refusal policy,
with the reason only to the unit's own principals. Claim and close remain valid
after lapse, transfer, revocation, recall or source-right loss. Current author
access may be narrower than historical replay execution; separate the two.

Gate: sealed holder flies and tunes/probes with denied source/memory through
all transports; Q17 owned-draft and queued/reconnect matrix; class/floor/station
recheck; old seal/hash unchanged; OOC journal/seat/time invariants; duplicate,
cross-unit and stale closes; restart with open claims; two or more owned hulls
dispatch under the free owner's licence while an otherwise equivalent one-seat
paid grant refuses its second concurrent claim. Include wrong-hull/title and
charter-transfer races and no capacity loss on an idempotent close. Integrate real region
sorties if available. If the fresh checkpoint proves the region driver absent,
deliver the production IC claim/close action provider and real composition
story now, name the exact missing driver/call site, and make attaching it an
explicit dependent integration task. Do not claim that OOC seats exercised IC
execution or mark the unavailable region end-to-end story as passed.

### M7.6 — Typed market routing, primary sale and the launch catalogue

Enable M7.1's separately scoped entitlement families/terms adapter, with actor
seller distinct from optional venue. Existing commodity `SaleTerms` and
`MAX_PROCEEDS_CUTS=1` remain unchanged. Bind dynamic Roll listings through a
bounded versioned catalogue seam; `CatalogSource::pack` is currently static and
cannot simply read live Roll state. Preserve existing catalogue ceilings. Use
the typed listing/quote/capacity indexes throughout acceptance and settlement;
no legacy MarketStore quote, counter or delivery marker is part of this path.

Expose sealed/open/open-resale and transferability, exact licence capacity/class/
term, rating and role. Purchase drafts an E4 escrow contract whose effect goes
through §4; the current after-payment `DeliveryProvider` cannot be the right's
authority. Approvals and queueing return pending, never granted. Payment does
not edit a loadout. Add the launch house vendor through the same actor/issuer/
fee/levy path as a player, without a privileged free grant.

Deliver the four named house entries in bounded subcuts through the same
principal/fee/title/licence path as player publication:

1. Stickney Yards Doctrine Library publishes the existing free Line/Sentry/
   Sniper/Skirmisher/Hunter/Rearguard teaching pages, keeping ROM access free.
2. Meridiani Field Series gets a real house org/account/vendor and staff-authored
   sealed combat source/presets with Registry receipts; no fixture identity or
   free privileged grant. Use §2.3's launch licence terms unless content explicitly
   differs. Existing Stickney/Tycho capital is not authority to debit for another
   house; create and fund the actual scoped principals through normal services.
3. Territorial Force Coprates Drill gets a real filed combat mind and a
   Republic-standing AvailabilityProvider with the same current transaction
   check. Standing preview alone does not authorize settlement.
4. Tycho Mutual Safe Conduct includes real `Escort` convoy-defence and `Transit`
   haul-under-escort profiles, lawful explicit ground-role hull/core binding and
   a deterministic role scenario. A cargo carrier needs actual capacity/custody
   content; the unarmed drone is only a public-picture probe. Exercise public
   commander objectives/plots, route completion and cargo conservation, not a
   renamed combat preset. Doc 17 permits this regional ground role; it does not
   require a Dray autopilot. Existing ship departure requires a pilot aboard and
   has no mind station. Do not relax that contract, expose another unit's private
   pose, or repurpose ground `goto` as an IC ship command.

   Add real production underwriting services: versioned product and exact quote,
   policy, reserved insurer exposure, mechanical-loss claim and expiry/close.
   Discount eligibility binds an exact certified Safe Conduct role and trusted
   IC seal proof, never a name, purchased licence alone, caller boolean or OOC
   seal. Premium, coverage/exposure, roots and receipt share §4's prepared commit;
   deterministic accepted losses pay once after the explicit delay/exclusions,
   including the paid wreck's title transfer to Tycho. Selected provisional
   configurable tuning is a 10‰ base premium on bounded agreed insured value,
   100‰ reduction of that premium, 800‰ covered-value payout and 1 IC-day delay:
   10,000cr insured value quotes 100cr ordinarily and 90cr with eligible cover.
   These are new launch choices, not existing policy/prices; reserve real insurer
   liability before accepting a premium. Existing RecoveryConfig cooldown/loaner
   coverage is not this product, and doc 18's later underwriting is not proof it
   already exists. Follow the concrete service/claim design in the Tycho note.

   The real-IC-host condition in doc 17's M7 gate applies here too. If the fresh
   checkpoint confirms no persistent IC sortie host, M7 still delivers and tests
   the actual role source/certification and production underwriting services.
   Authenticated composition tests exercise their same trusted mechanical proof
   interfaces. Public activation without such a host/proof refuses with no
   charge; quote/readback names the unavailable IC activation. Actual player
   convoy departure, live loss/claim and return acceptance remains an explicitly
   conditional integration, not a passed test or a requirement to build a new
   persistent host for M7. New carried shapes/stations, if genuinely needed by
   an available host, receive the declared ABI/frame/hash/inventory review.

This does not waive the named Safe Conduct role or priced-cover services. It
separates those required M7 implementations from the already-written host
condition. Later unnamed role doctrines and autonomous interplanetary ship
operation are not silently added. An actual conflict in the product contract is
reported precisely, not converted into fictional hauling or an advertised
discount with no real quote/premium/claim implementation.

Gate: actual second-account purchase, balanced §2 split and exactly one licence;
wrong/missing account, org scope and unknown identity; self/alt refusal; stale
venue/title/terms; tiny amounts/max chain/duplicate recipients; no source in
quotes/events/errors; author versus venue payout; ordinary IC10 proceeds intact.
Add mixed legacy/new sales in one process across sweeps/restart, plus all four
real catalogue reads and purchases. Safe Conduct's gate must demonstrate the
actual eligible cover quote difference and its real role behavior, not only a
label or static card. Prove counterfeit/OOC proof refusal, no-host zero-charge,
premium/exposure conservation, duplicate claim/reboot, excluded loss and wreck
title; distinguish service-composition evidence from the conditional live convoy
story. Coprates must refuse the ineligible principal at commit.

### M7.7 — Renewal, lapse, resale, charter title and PendingTransfer

Implement paid renewal and free owner attestation with one contract/receipt per
term generation. Use WorldIc for terms/Grace/deadlines and RealUtc only for
ordinary quote TTL. At exact boundaries: derive sanctions/dependency/term,
process close/refund/settlement, then admit a new seal. A deadline is a positive
settlement trigger, not `Before(deadline)` that becomes false and refunds.

Implement licence and charter resale, non-transferability and account checks
through every relevant generic E4 title/contract route, including lien/collateral
restrictions. Apply §2 Q18 and preserve held seats; charter ownership changes
derive the new free-license holder without duplicate owner authority. A pending
title sale blocks owner-licence admission across its lines through one bounded
charter pending record/aggregate. Never clear a lease on `LeaseEnd::Retrying`.
Owner attestation and charter-holder change re-bind the qualifying parent grant
and enforce §2.2's term/Grace cap; an absent/shorter parent cannot inherit a
fresh 90-day owner term. Publish §3.4's generation-specific lapse and completed
transfer facts only with their authoritative transition. A renewed licence does
not erase the earlier lapse event, and PendingTransfer is never a transfer fact.

Gate: immediate/no-claim sale; two seats with one open claim; last close and
deadline racing; term-end buyer Grace; duplicate close/settle; no seller renewal;
revocation refund and recall carried; holder/source revocation at committed
transfer; generic title bypass; stale renewal versus resale; frozen old warrants;
terminal completion at capacity and after restart.
Add dependent-parent Grace/term boundaries, restoration and shorter/longer
replacement grants, plus exact scoped fact values before/after receipt and
restart, no pending/failed-transfer fact, and no guessed-licence oracle through
contract conditions, readback or front consumers.

### M7.8 — Open forks, dependency and parent-chain settlement

Expose server-authorized fork/private-file/list operations using the persisted
lineage from M7.1. Open permits private filing; open-resale permits listing.
Every derivative settlement carries the bounded immutable royalty chain.
Renewal/reacquisition can restore current eligibility, not rewrite ancestry.
Keep four checks distinct: immutable ancestry/integrity and royalties; applicable
exact-revision Registry action; the requester's own grant/form; and the current
charter owner's qualifying immediate-parent grant for owner use/filing. A paid
customer of Child need not buy Parent. If that customer forks Child, their new
owner's licence depends on their own immediate-parent Child grant. Do not infer
automatic recall of every descendant from an ancestor recall.

Implement the explicit 2026-10-17 downstream ruling from §10: existing paid
licences remain usable under their own term and Registry restrictions; issuer
parent-right loss blocks new sales and renewals, with the qualifying grant
rechecked before commit. Do not add that issuer dependency to an existing paid
customer's new-seal or source/form checks. No date is extended on restoration.
At each applicable file/attestation/restore/charter-transfer transition, the
owner's child term and Grace are bounded by that holder's qualifying grant.
Restoration never silently extends a child or rewrites an existing sortie.

Gate: all Q17 loss causes for dependent owner use through depth three; source
sealing across owned-copy/replacement; old seals finish; initial and renewed
term caps, parent-Grace filing, restoration with expired child term, changed
qualifying grant and charter holder; applicable recall cannot be laundered;
private open fork not listed; open-resale exact full-chain money; same-account
charter/licence transfer refusal. Separately test Ada's dependent owner use,
Bo's existing paid Child grant, fresh Child issuance/renewal and Bo's own fork
under the explicit downstream ruling; no case charges Bo for ancestor licences.

### M7.9 — Certified die inputs, integration orders and titled cores

Add Basin importer Origin receipts and a persistent bounded WorldIc-month
allotment using §2.3's 64/16/4 launch quantities. Create the actual chartered
importer principal, Luna depot and Ceres/Phobos agent records; both agents debit
the same importer/month/class counter. Public labels are `die.c1|c2|c3` on
`tacsim.electronics`; numeric Grade
alone is never authority. Exact quantity-one certified lots are the initial
input. Register the separate `core_integration` facility charter and Stickney
`core_integrate@1` recipe with exact die/certificate, facility/policy/capacity,
paid service, requester and the durably acknowledged reserved output identity.
Use the explicit die/integration price split and actual scoped payer/payee
accounts; a read-only listing is not authority to mint or consume a die.

Extend industry exact-input/output semantics deliberately: current commodity
recipe lines and Grade(0) outputs cannot certify or title a core. Reserve exact
input, complete one durable manufacturing certificate/consumption/title/work-order
transition, and release once on cancellation/refusal. Reuse M7.2's separate
inert-shell reservation rather than allocating during completion. No active
core exists until its certificate and title commit; abandoned shells are bounded
and non-authorizing. The work-order typed transition and Economy participant
must be in one exact union; current commodity Grade(0) output/delivery callbacks
are neither a certified core nor an atomic substitute.

Gate: quota/month rollover/replay; forged Origin, caller-chosen grade and generic
crystallization refused; actual material conservation; repeated completion/
restart creates one output and payment; facility/charter drift; cancelled exact
input reuse; no untitled/double-titled output; capacity full after reservation.

### M7.10 — M32 component installation and hardware commit extension

Add typed `UnitMutationOp::InstallComponent` with expected unit revision and a
registered compatible fitted-unit definition. Start with §2.3's finite
line-tank I/II/III family; existing II stays canonical and the two new definitions
change only the allowed turret core binding. Pure scratch application replaces
only the designated core component/cells/capability, updates exact schema ref,
preserves all unrelated state, and validates exact post-topology before one
revision/journal increment. Empty mount and replacement are explicit; no second
core or rogue capability. Keep strict boot shape validation.

Add a tactics prepared owner image and a core-owned prepared object/custody image
where installation physically moves the replacement/displaced core. This is a
**second reviewed extension** of §4, not a claim that the initial E4+Roll pair
already publishes World objects. The hardware action can use Economy+Tactics
participants alongside typed inventory/industry/certificates, with built-in
object images bound before any apply. Tactics preparation must include its
active lifecycle projection and payload digest, not only `UnitStore.units`.
Canonicalize the incoming/outgoing core and all affected containers into one
guarded post-image each, including locations, contents, timestamps and derived
indexes. Bind these core-owned fields alongside the disjoint module slots using
the private World field-split seam. Resolve carry/destination/authority checks
and exact object revisions before custody; after receipt install prebuilt images
without mutable hooks or a fallible `World::move`. Preserve the existing legacy
single-custody-effect restriction. This new closed prepared Install/Replace
operation permits its bounded exchange, including old-core return, only because
the entire set binds before any apply. It is not two moves hidden in one mutable
provider. Finish this core seam before enabling refits; do not leave dirty
post-ack writes for a later flush. Mind selection/checkout is a separate action.

Require authority over hull and core, actual custody, certificate/title health,
unencumbered permitted title, facility charter and non-deployed/non-voyaging
state. A typed reverse installation index prevents one asset in two hulls.
Return displaced core intact to authorized custody; installation is not repair
of unrelated systems. Refuse M8 ungoverned outcomes.

Gate: install/replace/reboot; topology and capability forgery; stale unit/object/
facility; double install; deployed and port-under-way refusal; preserved damage,
ammo, heat and mobility; old-core return; destroy/scrub reconciliation; failure
between every prepared hardware leg; exact write collision; no half-installed
asset after lost receipt/restart.

### M7.11 — Restricted core sales, prices and operator surfaces

Register `tacsim.licence.core3_end_user` Entitlement and validate the current
purchasing principal/scope at actual Class III transfer, including resale and
service completion. `Legality::Restricted` and shop availability are labels/
previews, not the transaction guard. Unknown/wrong/revoked entitlement refuses
before charge; an org grant is scoped to the org. Revocation does not rewrite a
previously installed paid core; later seal rules are separate.

Author §2.3's new 100,000cr line-tank catalogue baseline, the 3,000/8,000/25,000cr
finished-core references and separate 100cr fitting quote. No existing Dray
price is used as a tank anchor. Validate the 3/8/25 percent relationship and
checked integer rounding in content tests. Ship importer,
facility, recipe and market readback explaining certified provenance. Expose
mind filing/listing/licence/renewal/resale/fork and hardware order flows through
existing command and typed request services. No text interpolation bypass, actor
field supplied by a client, unbounded source reply or alternative source oracle.

The shared service inventory must include `@mind/file <owned>@<rev>`,
`@mind/publish <owned>@<rev> ...`, current holdings/licence readback, pin/follow,
fork, withdraw, renewal, licence resale and Registry recall/revoke. Purchases
and title trades use the normal market/contract confirmation path; hardware
uses normal industry/refit orders. Select the remaining short spellings with
the existing command grammar in this cut, then expose the same typed actions
in web/TUI without requiring clients to synthesize command text. Reuse
connection/actor freshness and exact draft EditBase where source is involved;
commercial operations bind their own record/terms generations.

Gate: end-user right revoked between quote/accept/settle; direct title/service
bypass; scope/issuer failures; prices and lawful rounding; real hardware story;
command/help/type inventories; bounded narrow/wide rendering and typed stale-edit
errors. OOC pods and no-IC-posting assertions remain unchanged.

### M7.12 — Full author/buyer/hardware acceptance and release record

Run §9 on one exact clean revision with real composition/transports and retained
artifacts. Close compatibility, boot/migration, source visibility, retention and
performance reports. Update docs 17/17b/17d, help and inventories from observed
results; do not mark M7 complete on pure owner tests alone. Any unavailable IC
region integration is named, with its production service and substitute gate
evidence; it cannot be recorded as a passed real region story. The four-house
catalogue, actual Safe Conduct role and underwriting services must pass M7.6;
only the live host-dependent flight/convoy/loss story follows doc 17's explicit
IC-host condition. The resolved §10 downstream paid-derivative ruling must have
its chosen behavior tested before full M7 acceptance.
M8 remains blocked on its own IC-sortie/evidence prerequisites.

## 6. Files and compatibility discipline

Expected owners: `mush-core/{action_state,action,engine,module,ledger}` and a
participant module; `mush-server/{action_coordinator,world_task,persist,modules}`;
`mush-economy/{api,contract,contract_live,engine,module}`; market's separately
scoped entitlement terms/admission; new Roll commercial modules; tacsim Registry,
access, composition/content and existing holdings/workshop/transport services;
tactics unit/module; inventory exact-lot and industry work-order adapters.
Prefer no edits to port co-op behavior; any actual shared seam is coordinated.

Retain M6 source ABI, canonical card and runtime behavior unless a real carried
shape changes. Certified rating/provenance or new seal fields require one explicit
format/hash review with old reader rejection and resume fixtures. Bump the next
actual versions, not a number copied from these notes. No silent fixture repins,
declared/rated rewrite, observer-rule change, new private arena read or resource
budget raise. Update module roster, dependencies, ownership/delegation, mutation
writers, command/help/typed-client, state-family and persistence inventories.
Include the three M7 fact kinds and their scoped consumers; keep M8 kinds
unsupported. At documentation promotion, align doc 17's stale D-UM-57 row and
older M7-gate pending/default wording with the explicit owner-use Q17 ruling
and separately recorded downstream answer. Do not alter those production docs
as part of this preparation note.

## 7. Economy checkpoint evidence required before implementation

Run against the accepted M6 tree plus freshly reconciled main; record complete
commands, SHA, host and counts, including disabled/ignored cases and reasons:

```text
cargo test -p mush-economy --test e4b2 --test e4c --test e3a --test e3b
cargo test -p mush-market --test ic7 --test ic8 --test ic10 --test e7r2
cargo test -p tacsim --test coop --test commerce --test sale --test orders --test ports --test voyage
cargo test -p mush-server --test market_issuer --test ic_slice_loop --test boot_parity
```

IC12 moved `ic_slice_loop` from tacsim to mush-server and now boots the
production root at IC rate 24 before its controlled acceleration; do not retain
the deleted tacsim test target or substitute the older hand-built fixture. Its
new five-story target is memory-backed evidence, not the still-owed Redis smoke.

Also run current lease/end-lease RetryShortly/Retrying/reboot tests, registered
issuer/account/title and composition inventories, inventory retention refusal,
and actual merged IC13 buyout/re-lease/inflight-order cases when present. Verify
money+title atomicity, lost-title refusal, every contract state after reboot and
journal-tail eviction, approval not acceptance, refunds/expiry/idempotence,
market delivery/quote/capacity and IC10's unchanged commodity split. These command
names are a starting inventory, not permission to ignore renamed/current tests.
The completed IC14 checkpoint above satisfies this prerequisite. Historical
preparation notes alone do not supply passing evidence; repeat affected checks
when integration changes their inputs.

## 8. Mandatory composed-commit fault matrix

Run with real memory and Redis authoritative persistence; after each injected
failure, compare both live state and a fresh boot, plus actual public holdings.

| Injection point | Required outcome |
| --- | --- |
| Validation, source/rating, account authority, encoding, capacity or participant prepare | No money/title/right/root mutation; inert reservation at most, bounded and unusable |
| Old generation, slot replacement, foreign composition or second-owner bind failure | No owner installed; pre-custody refusal or post-receipt integrity freeze as applicable |
| Erased patch consumed, cloned owner slot, first owner bound then second bind fails | Returned installers own patches/config and borrow only valid owner fields; all guards release without partial apply; no generation-only alias acceptance |
| Duplicate key, Ledger/JSON collision, oversized union, legacy dirt/predecessor | No submission; no normalization/deduplication hides a conflict |
| Writer send before custody / definitive refused intent | Complete abort; ordinary retry follows durable idempotency preflight |
| Owned intent, process kill before any write / after each replay write | Restart completes exact retained bundle once; no fresh price/rights/clock evaluation |
| Final receipt lost, wrong token/hash, install capsule dropped | No success; integrity freeze/recovery; no second charge or locally inferred commit |
| Crash after RAM install but before player notification | Durable retry returns original receipt; no duplicate notice-dependent grant |
| Duplicate/conflicting operation key, delayed former callback | Exact retry returns same outcome; different request refuses; old callback grants nothing |
| Expiry/parent/title change around ownership boundary | Before ownership revalidate/refuse; after ownership replay exact images then process a separate new transition |
| Full capacity after accepted funding | Reserved terminal records permit settle/refund/close; no stranded escrow or seat |
| Prune then retain / retain then prune / unreadable retained boot state | Stale operation refuses; unknown roots cannot be reclaimed; generic privacy-preserving refusal |
| Zero consideration, zero refund, no journal append | Contract/rights/pending/root phase still atomic and replayable |
| Shell acknowledgement lost / crash between shell ack and file or manufacturing submit | Same operation resolves the same shell; no second allocation, title or active certificate; cleanup cannot delete an intent-owned shell |
| New-effect notifications with real observers; derived projection retry after restart | No legacy persistence/delivery runs inside receipt; paid rights already committed; idempotent derived projection retry cannot grant twice; ordinary IC10/Co-op observers unchanged |
| Replacement shares an old/new holder or container; later hardware bind fails | Canonical one-image-per-object exchange with complete containers/indexes; no first move, missing old core, dirty write or partial Tactics lifecycle update |

Assertions compare account balances/holds, journal sequence/key, title, contract
phase, licence/term/holder, pending state, operation receipt, seat claims and
revision/source roots. A callback counter, an observer mock or a final balance
alone is insufficient. Hardware repeats this matrix for exact lot consumption,
output certificate/title, custody, complete object/container/index state, unit
state/revision/lifecycle and work-order result. Compare hydrated RAM with live
indexes/counters, not only serialized documents, after every retained replay.

## 9. Final acceptance stories and gates

1. **Commercial author story:** real character A files and publishes a sealed
   mind; character B on another account buys IC, sees it in current sim holdings,
   selects/seals/releases and flies it. B reads cards and permitted trace, never
   source/memory through line, typed web/TUI/SSH, export or bench. The OOC run
   posts no IC transaction, takes no seat and changes no licence date. B resells;
   the old holding is unavailable at the next sim seal, with one balanced split.
2. **Private/derivative story:** filed-unlisted owner use has no offered form;
   ordinary open fork files privately but cannot list; open-resale derivative
   lists and pays the exact bounded parent chain. Parent lapse/transfer/revoke/
   applicable recall seals the dependent owner's source and blocks their new
   file/owner-use seal, while an already-running sortie and hash sequence stay
   unchanged. Restoration preserves ancestry and rechecks bounded child term/
   Grace without silently extending it. An existing paid customer's new sorties
   remain available through their own lawful term; new sales/renewals refuse
   until the issuer's qualifying parent rights return. Recheck parent loss
   between quote/approval and commit, preserve independent Registry restrictions
   and unchanged dates, and never require that customer to buy ancestors.
3. **Seat/term story:** two-seat licence, one live IC claim; sale goes pending;
   both parties refused new admission; last-close and deadline variants settle
   once. Remaining claim still consumes capacity, old warrant survives, duplicate
   close is harmless; term-end buyer gets Grace/renewal. Revocation refunds,
   recall travels, and lapse fallback is visible only to own principals. A free
   owner's parallel dispatches use `OwnedDispatch`, not that purchased seat
   counter; title transfer preserves old open claims and changes new admission.
4. **Hardware story:** real certified importer allotment → exact die custody →
   Stickney integration → titled core sale → refit → restart → actual core read.
   Class III permission is tested at transfer, replacement retains damage/ammo
   elsewhere, displaced core remains owned, and a second install cannot reuse it.
   Existing OOC pods show no owned-hardware dependency.
5. **Authority/adversarial story:** same-account licence and charter transfers,
   unscoped org acts, missing account oracle, forged receipt/grade/lineage,
   generic-title bypass, stale callbacks, cross-character source selectors,
   malformed retained records, queue saturation and cancellation all fail with
   their required no-mutation/privacy outcome.
6. **Launch catalogue story:** read and acquire the four named house entries
   through real owner/catalogue services. Free Doctrine stays free; paid houses
   pay filing/levy and obey budgets. Coprates refuses an ineligible principal
   even after an eligible preview. Safe Conduct demonstrates its real role and
   priced cover discount through the actual services and fixed role scenario;
   a renamed Standard card or static marketing sentence is insufficient. Include
   actual composition premium/exposure/claim/wreck-title records, and zero-charge
   refusal without trusted IC-host proof. Record live convoy/loss/return as
   conditional when its real host is absent; never label the service rig a live
   IC journey.
7. **Fact-interface story:** contracts/fronts observe exact committed filing,
   lapse-generation and transfer-receipt facts with proper scope; before receipt,
   after failed transition and PendingTransfer do not claim a completed event.
   Restart gives the same facts, renewal does not erase historical lapse, and a
   foreign caller cannot distinguish unknown from private licence references.

Every story includes restart and durable artifact inspection. Use configured
real public services, not direct fixture grants, for its successful path. For
unimplemented IC regions use the same production claim/close APIs the region
will call, never simulated OOC seats. Prove unlisted lines are excluded from
procurement eligibility without implementing M8 procurement.

Run workspace tests, relevant integration/ignored acceptance targets, formatting,
strict Clippy/restriction inventories, strict rustdoc, web checks and external
bot contracts. Preserve existing core/arena determinism and observer batteries;
run cross-host deterministic cases for changed execution/receipt codecs. Benchmark
Registry work alongside live arenas and report bounded queue/fairness/memory,
peak operations/fees and latency distributions with exact hardware/config.
The owner's report-only timing ruling remains: timing misses are reported and
explained, not silently made correctness gates. Resource ceilings, operation
caps, data isolation and deterministic replay are still hard requirements.

## 10. Paid-derivative ruling and remaining integration facts

Q17 is ruled, Q18 has its authorized written default, §2.3 selects provisional
launch tuning, M7.3 fixes the combat manifest, and M7.9–11 choose the finite
hardware catalogue. These are no longer generic open questions. Ordinary M7
implementation remains authorized after the existing M6/checkpoint gates.

**Explicit user ruling, 2026-10-17:** "Keep existing paid licences usable through
their own term; block new sales and renewals until parent rights return."
Ada holds Parent open-resale, files Child and sells Bo a paid Child licence.
If Ada, or Child's current charter owner/issuer, loses the qualifying Parent
grant, the resulting checks are:

| Operation | Required behavior |
| --- | --- |
| Ada's dependent owner-use/new filing | Q17 still blocks new owner-licence seals and filings; dependent owned source stays sealed until qualifying rights return. |
| Bo's already-paid Child grant | New admission uses Bo's own term/form/seat/warrant and applicable Registry status. Ada's missing parent grant is not an additional use predicate. |
| New Child sale or renewal | Refuse until the issuer's qualifying parent rights return; recheck them in the guarded preparation/commit path, including loss after quote or approval. |
| Existing sealed sorties | Finish unchanged for both owner and customer; no mutable licence lookup enters the running arena. |
| Restoration | Re-enables otherwise eligible fresh commerce; it neither extends Bo's term nor renews Ada's bounded owner term automatically. |
| Bo forks Child under a lawful open grant | Bo's new owner-use dependency is Bo's own immediate-parent Child grant. Bo does not purchase Parent or other ancestors separately. |

These are independent predicates. Applicable recall/revocation and the holder's
own term/form still apply; parent loss is not an automatic Registry recall of
Child. Preserve immutable ancestry and royalty payouts. After durable intent
ownership, replay completes the exact accepted images rather than applying a
new eligibility decision. Source/privacy projections and delayed responses use
the same operation-specific rights evaluator; they must not accidentally revoke
Bo's lawful existing paid form because Ada lost Parent.
See `target/m7-parent-dependency-interpretation.md` for the historical question,
evidence and exact resolved matrix. No policy answer is pending for this edge.

The genuinely unresolved facts are checked at M7.0 and recorded against code:

| Fact to establish | Required disposition |
| --- | --- |
| Actual merged E4/market/IC13 interfaces and trade deployment constraints | Fresh fetch/reconcile plus the exact checkpoint; adapt the scoped provider/phase boundaries without changing ordinary commodity/Co-op behavior. A known deployment blocker remains visible until its owning repair passes. |
| IC region sortie driver and production call sites | Integrate current driver with the new claim/close service if present. If absent, finish the production service/composition gates and name the precise missing driver as a dependent integration, never a passed live-region story. |
| Safe Conduct role and insurer service | Implement the actual ground Escort/Transit source/content and deterministic role scenario, plus production quote/premium/policy/exposure/claim/close services in M7.6. If no IC host supplies trusted seal/loss proof, public activation refuses without charge; production-composition gates still run and live convoy/loss acceptance remains conditional under the existing doc17 M7 clause. No Dray autopilot or unrelated persistent-host build is implied. |

No new ruling is needed for the prepared-object reservation/exchange: it is
necessary implementation work selected in §4. Its exact safe API must pass the
compiler/failure gates. Do not transform a feasibility failure into two mutable
calls or an after-payment callback. Likewise, no launch balance is asserted as
historically authored: changeable defaults are plainly labelled and all accepted
terms remain immutable. Resolve routine details autonomously. No new policy
request is created by these notes;
report an actual newly discovered contradiction rather than inventing one.

## 11. Acceptance record template

For each cut record: source and fetched-main SHA; exact scoped files; new or
changed contracts/formats; commands and passing/failing/skipped counts; memory
and Redis fault cases; cross-host comparisons where applicable; retained story
artifacts; independent review findings/dispositions; performance report; known
residual owner and deployment status. Mark M7 accepted only when all applicable
stories and hard gates pass, every explicit deferral is truthful, and the ledger
describes implemented behavior rather than this plan.
