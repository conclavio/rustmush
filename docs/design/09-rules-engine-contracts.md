# Cross-ruleset mechanical contracts and proving gates

**Status:** Implementing 0.11 — K0/K1 formal exit criteria landed. K2 has strict atomic boot
hydration, bounded typed transactions, final guard revalidation, provider registration, and a
versioned split-gateway request custody protocol. Same-owner causal triggers now execute against
one root scratch transaction and persist their complete event graph in the same bundle and compact
receipt. Transport-authenticated coordinator entry and the remaining ownership fault matrix remain open. K3's
persistence kernel and durability-audit closeout are implemented: exact bundle
reservation/reconciliation, pre-hydration recovery, store-owned authoritative prefixes, compact
checkpoints, watermark-first epoch retirement, a production coordinator plus first tactics provider,
and recovery-driven durable-outbox dispatch. Transport/legal-offer integration and the IPC portions
of Gate 5 remain cross-cut work with K2/K5; all five end-to-end proving gates remain open.

**Scope:** the shared contracts that must be stable before RustMUSH substantially expands character,
tactical, economic, and AI-driven rules

**Applies to:** `mush-core`, `mush-ruleset`, `mush-dice`, `mush-tactics`, `mush-sim`,
`mush-scenes`, `mush-proto`, `mush-server`, future `mush-economy`, and ruleset adapters

**Companions:** docs 06 (objects/spatial domains), 07 (NPC control/tactical AI), and 08
(economy/organizations/ownership)

This document makes twelve previously identified engine contracts and five minimum proving gates
part of the repository's design. They are not twelve new global subsystems. Together they define
one narrow **mechanical kernel** around the existing single-writer world:

```text
registered, versioned meaning
           |
           v
project legal action -> prepare against immutable truth -> validate guards
           |                                            |
           +--------------------------------------------+
                                                        v
                                   materialize one complete post-state
                                                        |
                                      queue one authoritative bundle
                                                        |
                                             exact durable acknowledgement
                                                        |
                                    install RAM state + observer projection
                                                        |
                                               external effects
```

The kernel owns identity, ordering, bounds, staleness, durability, and information-flow rules.
Rulesets and domain modules own formulas and meaning. The goal is not a universal RPG evaluator;
it is one safe route through which very different evaluators can act.

The five gates use deliberately tiny toy rules. Passing them is the condition for growing broad
weapon catalogs, effect libraries, economic instruments, or complex initiative systems.

---

## 1. Decisions and boundaries

### 1.1 Locked principles

1. The world task remains the only authoritative state writer.
2. Rules-bearing persisted references name an exact, immutable definition version.
3. A client, script, NPC, or LLM submits an **invocation**, never trusted authority evidence or a
   mutation patch.
4. Planning is read-only. A refusal, stale result, or invalid plan changes nothing and consumes no
   resource.
5. Every operation in one mechanical outcome is prepared before any of it is persisted or
   installed; live RAM installation follows exact durable confirmation.
6. Authoritative state changes exactly once; presentation is recoverable and safely repeatable,
   not falsely promised as an exactly-once network delivery.
7. Randomness is scoped to a resolution and recorded. Replay uses the committed outcome rather
   than depending on historical rules code to reproduce it.
8. Hidden truth is projected before it reaches commands, panels, errors, logs intended for
   players, NPC context, or LLM tools.
9. Cross-owner mechanics require a registered bridge. Merely sharing a room or simulation domain
   never implies rules compatibility.
10. Missing meaning or corrupt authoritative state freezes the smallest safe scope and creates a
    recoverable incident; valuable state is never silently dropped.

### 1.2 What does not belong in the kernel

The kernel does not know:

- dice-pool formulas, initiative systems, damage tables, or effect modifier order;
- whether a section is a torso, hull, cyberlimb, room, or abstract stress domain;
- what hunger, edge, heat, ammunition, nuyen, favors, or C-bills mean;
- whether an action is tactically wise or narratively appropriate;
- how a sensor finds a contact or how concealment is opposed;
- prices, recipes, careers, production times, or market behavior;
- terminal layout or prose.

It also does not expose a general JSON patch language. Opaque payloads are interpreted only by
their registered owner, and prepared mutations use closed, typed operation vocabularies such as
`UnitMutationPlan` or an economy posting plan.

### 1.3 Crate ownership

No new catch-all crate is required initially. The responsibilities fit the current dependency
direction:

| Owner | New responsibility |
|---|---|
| `mush-core` | canonical mechanical IDs and envelopes; erased global definition/action/bridge registries and registration seams; guard and revision vocabulary; transaction/cascade coordinator interfaces |
| `mush-ruleset` | typed definition-catalog helpers, action-provider helpers, parameter schemas, generic resource/effect helpers, and character-sheet adapters |
| `mush-dice` | the only authoritative RNG facade and recorded draw types |
| `mush-tactics` | tactical definitions, legal actions, typed source/target unit plans, and tactical projections |
| `mush-sim` | placement/control/lease/window revision providers; no combat formulas |
| `mush-server` | durable bundle submission, commit-pending world mode, exact acknowledgement, restart reconciliation, and after-commit delivery/effects |
| `mush-proto` | versioned projected views, action receipts, keyframe/delta sequencing, and resync |
| rulesets / future `mush-economy` | definitions, formulas, typed plan construction, migrations, visibility decisions, and bridge implementations |

`mush-core` owns only the vocabulary necessary to coordinate modules. Reusable game mechanics stay
in `mush-ruleset`; setting content stays in rulesets.

### 1.4 Two atomicity boundaries

“Atomic action” has two separate meanings and both are required:

1. **Logical atomicity:** every fragment validates against one immutable pre-state; only complete
   prepared post-states are installed. No fallible policy decision occurs after the first install.
2. **Durable atomicity:** the complete authoritative write set and a stable result are identified by
   one token. Ambiguous acknowledgement or Redis execution-time partial application is reconciled
   by that exact token before success is released.

E0's retained-seal table closes the immutable Ledger rollover hole. It intentionally does not
provide whole-bundle atomicity for mutable objects and persistence materializations. The bundle protocol in §4 is a
separate prerequisite for the gates in this document and for economy E2.

---

## 2. Common vocabulary

The examples below specify semantics, not final field spelling.

### 2.1 Owners and definitions

```rust
pub struct DefinitionKey {
    pub owner: OwnerId,          // registered module/ruleset, e.g. "tacsim"
    pub kind: DefinitionKind,    // e.g. "action", "effect", "component"
    pub id: DefinitionId,        // owner-local canonical id, e.g. "medium_laser"
}

pub struct DefinitionRef {
    pub key: DefinitionKey,
    pub version: u32,            // exact immutable version, never "latest"
    pub digest: [u8; 32],        // canonical definition digest
}

pub struct RulesValue {
    pub schema: DefinitionRef,
    pub data: serde_json::Value,
}
```

`owner`, `kind`, and `id` are structurally distinct. An owner must not be guessed by splitting a
dotted local id. Each segment uses the existing bounded canonical-id policy: trimmed lowercase
ASCII, no empty dot segment, and a hard serialized-size limit.

The digest prevents two binaries from claiming that different bytes are the same exact version.
It is provenance and drift detection, not a substitute for validating an instance.
It is SHA-256 over a version-tagged canonical encoding of the key, version, and definition payload;
object keys are sorted and numeric encoding is deterministic. A provider may not hash incidental
Rust debug/serde layout or a map with nondeterministic iteration order.

### 2.2 Revisions and guards

One `observed_revision: u64` is insufficient for a real action. A shot may depend on a source unit,
target unit, placement, contact, component, control lease, action window, and ammunition
reservation.

```rust
pub struct RevisionKey {
    pub owner: OwnerId,
    pub kind: RevisionKind,
    pub subject: SubjectKey,
    pub facet: Option<CanonicalId>,
}

pub struct RevisionStamp {
    pub key: RevisionKey,
    pub generation: u64,
}

pub struct RevisionSet(pub BTreeMap<RevisionKey, u64>);

pub enum AuthorityGuard {
    Capability { actor: ObjId, capability: CapabilityId, generation: u64 },
    ControlLease { subject: ObjId, channel: ChannelId, holder: ControllerRef, epoch: u64 },
    Reservation { provider: OwnerId, id: ReservationId, revision: u64 },
    Window { id: ActionWindowId, generation: u64 },
    Definition(DefinitionRef),
}

pub struct GuardSet {
    pub revisions: RevisionSet,
    pub authority: Vec<AuthorityGuard>,
    pub deadline: Option<ClockStamp>,
}
```

These are server-created assertions about the state planning observed. They are never bearer
tokens accepted from a network client or model. The final guard/deadline check and the atomic
transition from `Writable` to `CommitPending` are the action's serialization point. That transition
freezes every source of guard-producing truth until the store result is exact and RAM installation
finishes. There is no fallible guard check after durable commit.

### 2.3 Actions and receipts

```rust
pub struct ScopedIdempotencyKey {
    pub scope: IdempotencyScope,
    pub epoch: u64,
    pub key: CanonicalKey,
}

pub struct IngressEnvelope {
    pub request: RequestUid,
    pub principal: PrincipalId,
    pub surface: IngressSurfaceVersion,
    pub canonical_payload: CanonicalBytes,
    pub idempotency: ScopedIdempotencyKey,
}

pub struct ActionInvocation {
    pub request: RequestUid,
    pub actor: ObjId,                 // supplied by authenticated session context
    pub offered_action: ActionOfferId,
    pub subject: PerceivedRef,
    pub targets: Vec<PerceivedRef>,
    pub parameters: serde_json::Value, // validated against the bound offer's projected schema
    pub idempotency: ScopedIdempotencyKey,
}

pub struct PreparedAction {
    pub action: DefinitionRef,
    pub provider: OwnerId,
    pub ingress_hash: [u8; 32],
    pub request_hash: [u8; 32],
    pub guards: GuardSet,
    pub resolution: ResolutionRecord,
    pub plan: RulesValue,              // decodable only by provider/bridge
    pub plan_hash: [u8; 32],
}

pub struct AuthoritativeReceipt {
    pub commit: CommitToken,
    pub request: RequestUid,
    pub idempotency: ScopedIdempotencyKey,
    pub ingress_hash: [u8; 32],
    pub request_hash: [u8; 32],
    pub plan_hash: [u8; 32],
    pub outcome: AuthoritativeOutcome,
    pub eligible_witnesses: WitnessSet,
    pub committed_at: ClockStamp,
}

pub struct ProjectedActionReceipt {
    pub commit: CommitToken,
    pub request: RequestUid,
    pub status: ProjectedActionStatus,
    pub outcome: Option<ProjectedOutcome>,
    pub committed_at: ClockStamp,
}
```

The authenticated ingress assigns the principal and later resolves the actor. For interactive
commands, the gateway assigns the idempotency key from a `RequestUid` that remains stable if the
request is retried across a server reconnect; an untrusted client does not choose a new mechanical
identity after seeing a partial result. Authenticated HTTP callers may supply a bounded key scoped
to their account/API operation. Internal jobs, approvals, scheduled work, and LLM completions
supply deterministic keys in their own namespaces.

Two hashes serve different times. The gateway/server boundary can immediately compute an
`ingress_hash` from principal, surface/schema version, and canonical raw-line or structured payload.
It looks up `(ScopedIdempotencyKey, ingress_hash)` before parsing, contact resolution, offer
regeneration, planning, trigger expansion, or RNG. An exact match returns or joins the original
result. A miss is parsed into `ActionInvocation`, after which the server computes the semantic
`request_hash`. Different ingress bytes for an existing untrusted key are rejected, audited, and
rate-limited; they must not freeze the world and become a denial-of-service primitive. A collision
or byte conflict for a server-generated deterministic key is an integrity incident. `plan_hash` is
internal audit/recovery evidence, not part of retry equality and never appears in a viewer receipt.

### 2.4 Perceived references

```rust
pub enum PerceivedRef {
    KnownObject { id: ObjId, revision: u64 },
    Contact { provider: OwnerId, id: ContactId, generation: u64 },
    SelfActor,
}
```

A `ContactId` is opaque, scoped to its viewer or sharing principal and observation context, and
resolved only inside the world task. It must not encode or serialize the underlying `ObjId`.
`KnownObject` is used only when projection policy explicitly authorizes that viewer to know the
ordinary MUSH dbref; it is a deliberately disclosed public reference, not a way around contact
redaction.

### 2.5 Clocks

```rust
pub enum ClockRef {
    Utc,
    WorldIc { generation: u64 },
    SceneTurn { scene: SceneUid, generation: u64 },
    DomainTick { domain: DomainId, generation: u64 },
    Registered { owner: OwnerId, id: CanonicalId, generation: u64 },
}

pub struct ClockStamp {
    pub clock: ClockRef,
    pub at: i64,
}
```

Two stamps are comparable only when their complete clock identities match. This prevents an IC
second, UTC second, scene turn, and domain tick from accidentally sharing an integer API.

### 2.6 Canonical hashes

Every definition, request, plan, intent, and bundle hash uses a named domain plus a versioned
canonical byte encoding and SHA-256. The encoding normalizes maps, definition lists, state keys,
and replay writes; rejects duplicate or conflicting writes to one target; and never hashes Rust
debug output, incidental serde layout, or nondeterministically iterated collections. A hash field
always states its domain/encoding version in the surrounding schema. Hashes used for internal
integrity are not automatically safe to expose to observers: small hidden outcome spaces can be
dictionary-attacked.

The transport-specific ingress hash covers the principal, ingress surface/schema, and canonical raw
or structured payload available before parsing. The semantic request hash covers the authenticated
principal/actor, idempotency scope, opaque offered action, perceived subject/targets, normalized
parameters, and invocation schema version. It excludes delivery attempts, connection IDs,
timestamps, and the idempotency key itself. The plan hash covers the exact definitions, guards,
resolution, causal expansion, and prepared state keys/post-images. The bundle hash covers the
normalized durable intent described in §4. Replays on one surface have an identical ingress hash;
cross-surface semantic equivalence is claimed only after both surfaces explicitly normalize into
the same `ActionInvocation`.

---

## 3. The twelve contracts

### C1. Versioned definition registry

#### Contract

Every durable rules-bearing instance refers to an exact `DefinitionRef`. The registry is composed
and frozen before persisted world/module records load. It enforces:

- one owner for each definition key;
- immutable bytes for a `(key, version)` pair;
- duplicate rejection, including same-version/different-digest registration;
- canonical identifiers and hard count/byte limits;
- complete validation of the definition, including referenced definitions;
- an explicit, contiguous migration chain when an instance must advance versions;
- fail-closed behavior when an exact definition or required migration is unavailable.

“Current version” is registry metadata for authoring. Persisted references never mean current,
latest, or a semver range.

Self-describing instances such as `TacticalUnit` remain inspectable when their definition is
missing, but they are mechanically frozen. Self-description helps recovery; it does not authorize
continued interpretation by unknown code.

#### Implementation

1. Add canonical `OwnerId`, `DefinitionKind`, `DefinitionId`, `DefinitionKey`, `DefinitionRef`, and
   validated `RulesValue` to `mush-core`.
2. Add the erased `DefinitionRegistryBuilder`/`DefinitionRegistry`, bridge registry, and
   migration-provider registration seams to `mush-core`, beside the existing module composition
   machinery. Keep typed catalog authoring/validation helpers in `mush-ruleset`; freeze the erased
   registries before loading boot data.
3. Move/re-export the tactical definitions from `mush-tactics::unit` and the duplicate sim/tactics
   opaque rules blobs onto the shared types.
4. Change `UnitCatalogBuilder::add` to reject duplicates and run full `UnitSpec` validation.
5. Read legacy tactical `{id, version}` references through an explicit v1 adapter whose owning
   module supplies `owner` and `kind`; never infer them globally. Persist v2 only after an
   acknowledgement-gated migration.
6. Add registry fingerprint diagnostics so a deployment can compare the loaded definition set
   before enabling mechanics.

#### Failure posture

Missing optional definitions with no persisted dependents disable only the actions/content that
need them. An intact, schema-valid instance whose exact definition is temporarily unavailable is
preserved in `Frozen` state and may be thawed after the same definition returns. Malformed,
untrusted, or non-migratable bytes are preserved in `Quarantined` state for explicit repair. A
digest conflict for an already-used exact version is an owner/module integrity freeze.

The K1 runtime implements that distinction with a bounded, composition-roster readiness latch.
The first integrity incident for an owner wins for the process lifetime. Executable exact
definition, action-provider, revision-validator, authority-validator, and bridge-route lookups then
return a typed owner-not-ready error; they never disguise the incident as missing content. Recovery
can still inspect immutable definition/bridge/migration metadata through explicitly structural APIs,
but those paths do not authorize callbacks or interpretation. `mush-tactics` checks every exact
definition owner named by a unit (not only its root schema), fans one qualifying digest conflict out
to all intact dependent unit records, and leaves unrelated owners active. Malformed records remain
record-local quarantine. If a core object is already being destroyed when the post-event observer
runs, that observer cannot reverse the object's prior `GOING` transition; for a frozen owner it
removes the unit from the active index and preserves the authoritative module record instead of
emitting a delete.

### C2. One action -> plan -> commit protocol

#### Contract

Every mechanical action follows one lifecycle:

```text
authenticated IngressEnvelope + ingress hash
  -> early idempotency lookup (return/join before parsing, resolution, or RNG)
  -> on miss parse transport into UnresolvedActionRequest (no truth reads)
  -> predecessor durability barrier
  -> resolve authenticated actor and perceived references
  -> regenerate the offered legal action
  -> build canonical semantic ActionInvocation + request hash
  -> construct server-owned guards
  -> provider prepares typed post-states and a semantic outcome
  -> expand and bound same-commit triggers
  -> materialize one PreparedTxn from every post-state
  -> validate the complete PreparedTxn
  -> recheck every guard and atomically enter CommitPending
  -> submit one authoritative bundle
  -> reconcile exact durable status
  -> install the PreparedTxn's owned RAM images without further decisions or serialization
  -> release projected success and launch external effects
```

Providers plan against immutable state. Planning may return a provider-owned opaque plan, but it
cannot return arbitrary world mutations. The owning provider or registered bridge must decode it
into typed prepared replacements.

Request ownership has an explicit linearization point. Before the complete intent in §4 is durably
reserved, the server has not accepted responsibility: the gateway/client retains the same
`RequestUid` and may replay it. After exact intent reservation, recovery must drive that action to a
terminal receipt or integrity incident. The server must not acknowledge dequeue/acceptance across
the split-process IPC boundary earlier than that transfer. An API that wants an earlier asynchronous
`202` must first durably journal the invocation at the gateway; an in-memory “received” message is
not acceptance.

The IPC protocol makes disposition explicit:

```text
Rejected(uid, projected_refusal)   no intent; gateway drops only after receiving this response
Busy(uid, retry_hint)              no intent; bounded refusal/backpressure
EphemeralFinished(uid, output)     pure/replay-safe query completed; no server ownership
LegacyClaimed(uid)                 classified; server waits for GatewayReleased(uid)
Owned(uid, commit_token)           exact intent exists; server/recovery now owns completion
Finished(uid, projected_receipt)   terminal committed result
```

If `Rejected`/`Busy` is lost, replay is harmless and may be reevaluated against newer state because
no durable action was accepted. If `Owned` is lost, replay joins the exact pending intent. The
gateway's pre-intent custody table is resource-bounded by per-connection and global count/bytes;
reaching the bound stops socket reads or refuses before accepting more commands, never grows a
must-deliver queue. Age is an alert/SLA, not an eviction rule: a UID remains until disposition or an
explicit cancellation handshake proves intent reservation did not win. Raw SSH custody covers a
server crash/reconnect while the gateway remains alive. A gateway
or host crash before `Owned` can lose the line unless optional durable gateway ingress journaling is
enabled; HTTPS instead relies on caller replay of the same composite idempotency key. Gate 5 names
which process is killed and applies the corresponding promise.

Because the gateway cannot classify a raw line, the server must return one disposition for every
`Cmd`, including queries and commands not yet migrated to the kernel. Lost `EphemeralFinished`
causes a replay that is safe but may observe newer state. A legacy mutating command must either use
an exact legacy request journal or use a two-step at-most-once handoff: the server classifies and
sends `LegacyClaimed` without executing; the gateway removes the raw request from custody and sends
`GatewayReleased`; only then may the server execute. The gateway intentionally receives no false
completion promise for this compatibility lane: loss before release causes replay of classification
only; a server crash after release may lose the command, but cannot make the gateway duplicate it.
Both sides bound their short-lived classification tables. This weaker bridge avoids
duplicate legacy mutation without pretending to provide Gate-5 durability. Commands become
eligible for `Owned/Finished` only when their entire mutation has moved to the authoritative
protocol.

For initial correctness, only one authoritative action bundle may be in flight. While it awaits
resolution, the world enters `CommitPending`: receipt/order bookkeeping, status, recovery,
shutdown, and transport-only connection bookkeeping remain admitted (subject to the FIFO caveat
below), but no gameplay/world read, world
or module mutation, hook, timer, or asynchronous apply is accepted. Legacy K0 handlers have already
installed their tentative RAM post-image at this point, so world reads receive a typed `Busy`
rather than observing it; `NameFree(false)` and other fabricated defaults are forbidden. The K2
prepared-action lane instead leaves typed action state at its acknowledged pre-state and retains
the post-images privately, but the shared admission policy still refuses gameplay/world reads until
the receipt resolves. Serving pure queries from that pre-state remains a future policy choice.
Pipelined MVCC is explicitly deferred until measurements show the single-flight rule is inadequate.

`CommitPending` never means “append every later message to an unbounded queue.” In the current K0
compatibility path, retryable `Cmd`, query, `CreatePlayer`, and dump requests receive an immediate
typed/bounded refusal and do not occupy the one deferred FIFO slot. Barriers are capped at 256
waiters. Gateway `ReAnnounce` is a full replaceable roster: while a receipt is pending, the world
retains only the latest snapshot, capped at the gateway's 256 lifecycle obligations and stored as a
tight fixed-record slice. Together those operational items have an explicit 81,920-byte logical
custody ceiling. Repeated reannounce therefore cannot append another room render to
`PendingCommit.sends`; the latest snapshot is applied after the exact receipt and before its
barriers complete. Oversized rosters are protocol violations, atomically rejected before any
record is applied, and tear down the split boundary so the gateway-owned roster is replayed.

The replaceable-snapshot claim is scoped to pending custody on a freshly booted server: neither the
older nor newer pending snapshot has been applied yet. `Engine::reattach` itself is additive/update
bookkeeping; it does not remove a connection absent from a later roster. Normal operation avoids a
stale live-world roster because a boundary gap terminates that server world and individual
`ConnDown`/`SessionGone` messages own removal. If same-world repeated full reconciliation becomes a
supported protocol, it needs an explicit generation plus remove-missing semantics; do not infer
that behavior from this coalescer.

Fresh connection publication uses the same exact boundary. The gateway reserves bounded lifecycle
custody before changing its local session table, sends `ConnUp`, waits for an exact typed barrier,
and then asks the engine to prove the exact `(connection, actor)` association. Only after both
proofs does it publish `SessionInfo`, inspect the actor's input context, and enqueue the initial
room view. `Busy` remains retryable while an aborted receipt is `Unavailable`; neither is promoted
to a barrier acknowledgement. A failed fence or negative association emits a compensating
`ConnDown` (`Lost` for a revived link-dead session, `Boot` otherwise), restores local session state,
and publishes no successful connection event. The association check is necessary because an
ordering barrier alone can succeed after a hard world mode refused the preceding connection hook.

This does **not** yet make every operational lane independently live. An `Apply`, `ConnUp`,
`ConnDown`, or `SessionGone` has already transferred semantic/lifecycle custody and cannot be
dropped as retryable input. The world retains one such dequeued item and stops polling the bounded
main FIFO, so a barrier or reannounce physically behind it remains head-of-line blocked until the
current receipt resolves. Memory remains bounded by the single deferred item plus the 1,024-entry
inbound channel, whose awaited send provides backpressure, but liveness is conditional. K2 must
route semantic completions and lifecycle obligations through separately count/byte-bounded lanes
with explicit saturation/backpressure and sequence-aware barrier ordering; merely scanning around
the main FIFO would break its ordering contract.

The eventual contract remains: reconstructible timers and hooks coalesce by stable key or remain
overdue in their authoritative scheduler record, and must-retain completions use a separately
byte/count-bounded durable mailbox with an explicit saturation incident. No class may silently grow
in memory while the store is hung.

The K0 writer-to-world control seam follows the same rule now: a capacity-one wake channel points
at fixed coalescing slots for health and breach state plus an exact-resolution map capped at 1,024
seal identities and 16 MiB. Publishing never awaits the world. If the exact map is full, the writer
does not shed the resolution: its bounded poison table retains the outcome and republishes it when
the still-retained seal is re-emitted. The world-owned seal table uses the same 1,024-identity cap
and a 64 MiB encoded-payload cap. Capture preflights the complete candidate atomically; refusal
latches persistence integrity before every exact candidate seal is preserved to the quarantine
domain.

#### Prepared installation

`ActionTxn` is a scratch/clone-on-write view of only affected records. Mutation providers first
produce typed owner-validated post-states. Before submission, the coordinator consumes all of them
exactly once into one owned materialization:

```rust
pub struct PreparedRecord {
    pub key: StateKey,
    pub canonical_post_image: CanonicalBytes,
    pub install_image: Box<dyn InstallImage>,
}

pub struct PreparedTxn {
    pub token: CommitToken,
    pub records: Vec<PreparedRecord>,
    pub authoritative_outcome: AuthoritativeOutcome,
    pub durable_outbox: Vec<DurableOutboxIntent>,
}
```

That is the target K3 materialization, not the shape currently persisted. The K2 implementation is
deliberately narrower: it carries owner/provider identity, transaction counts, one store
reservation/revision pair, and the prepared state records. It has no durable commit token,
authoritative outcome, causal event set, replay writes, idempotency binding, or durable-outbox
intents yet.

The bundle writes and RAM install images are derived from these same records. Canonical bytes are
serialized and hashed before the typed values move into `PreparedTxn`; neither provider logic nor
serialization runs again after acknowledgement. `InstallImage::install` is infallible and
`InstallCtx` exposes bounded replacement/append APIs, not unrestricted `&mut World`. A bridge can
aggregate several records, but validation completes for all of them before the first store write.
A tactical attack therefore prepares both source and target `TacticalUnit` post-states;
single-unit `UnitMutationPlan::apply` alone is not cross-unit atomicity.

During migration, the action may not plan against RAM containing an earlier unacknowledged legacy
dirty write. Before reading its pre-state, the coordinator drains and exactly acknowledges every
predecessor write that could contribute to that state, or absorbs the complete dirty dependency
set into `PreparedTxn`. Once a state key/prefix is registered as authoritative-bundle-owned, the
store rejects legacy fire-and-forget writes to it. That final prefix-registration/rejection rule is
a K3 requirement. The current server reserves every `action-state/*` module key from ordinary
module boot, eager module migration, boot-time writes, and runtime legacy writes, but the store
does not yet independently register/enforce that ownership if a caller bypasses the server.

#### K2 state-ownership decision (accepted: Option 1)

Option 1 is the accepted architecture and its initial typed-state foundation is implemented. The
alternatives remain here as decision rationale:

1. **Engine-owned typed state families (accepted).** A registrar declares each bounded
   `StateFamilyId`, key codec, value schema/version, owner, persisted-value decoder, validator, and
   canonical serializer, then receives an opaque owner-stamped `StateFamilyHandle<K, V>`. The
   engine stores authoritative family records in an `ActionStateStore`. `ActionTxn` exposes only
   read/insert/replace/remove
   operations through those handles; action providers receive no unrestricted `&mut World`, raw
   `ModuleStateMap`, or arbitrary persistence operations. Finalization consumes the transaction
   into one non-cloneable `PreparedTxn`, serializes every post-image once, and creates infallible
   install images from the same typed values.
2. **Provider-owned state with prepare/install callbacks.** Providers retain their maps and return
   opaque canonical bytes plus an install closure. This requires less migration, but the engine
   cannot mechanically prove that preparation left live state untouched, that a callback did not
   access another owner's state, or that post-ack installation is infallible. Those guarantees
   become conventions and per-provider review.
3. **Clone the whole `World` for each action.** This preserves existing APIs but makes action cost
   proportional to world size, complicates concurrent operational reads, and still does not
   isolate external/module-owned state. It is rejected as a production design.

The implementation freezes registered state families with the deployment registry and includes
their stable descriptor/callback IDs in its mechanics fingerprint. Handles are owner-stamped,
typed, and bound to the exact registry composition. A provider transaction is stamped from its
registration rather than a caller-provided owner, can access only that owner's handles, and has no
`World`, `Ctx`, raw `ModuleStateMap`, or arbitrary persistence API. `insert`, `replace`, and
`remove` validate live-pre-state semantics; any ignored error poisons finalization. Mutations are
ordered by canonical `StateKey`, each key may be mutated only once in the initial implementation,
and `prepare` produces one non-cloneable materialization plus private typed install images without
running a provider or serializer after that point.

At boot, the server partitions every `action-state/*` document found in a composed module's
namespace away from legacy `ModuleBootData` and module migrations. It resolves the exact registered
family, cross-checks namespace/path/envelope/schema identity, decodes to the typed value, validates
it, reserializes it, and requires canonical equality with the persisted document. Hydration builds
a scratch map and installs nothing unless every record succeeds. It is bounded to 1,000,000 records,
256 KiB per canonical record, and 512 MiB across hydrated canonical records. Future persistence
versions fail closed rather than reaching legacy module code. This scan is presently limited by
the store API to namespaces of composed modules; K3's exhaustive cursor scan must also detect
orphaned/uncomposed-owner records.

The state-record portion has hard ceilings of 1,024 mutation operations, 256 KiB canonical bytes
per record, and 16 MiB across those canonical state records; callers may only tighten the
per-transaction limits. The 16 MiB limit must not yet be described as a bound on the complete
authoritative bundle: outcomes, guards, causal events, replay writes, allocator changes, delivery,
and outbox work are not part of this `PreparedTxn`, and legacy output/effect construction retains
the separate K0 bounds gap described below.

`Engine::stage_prepared_action` refuses to mix a prepared action with dirty `World` state, legacy
module operations, or another staged action, and requires an explicit server-created `GuardSet`.
At the last synchronous boundary before writer custody, the single-owner world task revalidates
that guard set against immutable world truth and the exact frozen module composition. Stale or
unevaluable work aborts the complete typed image before consuming a serial or sending any write;
an ordinary stale result is a retryable action refusal, not a persistence-integrity incident. The
server sends current `ModuleJsonSet`/delete records on a separate prepared lane, retains the install
images in `PendingCommit`, and on an exact matching applied serial installs them before releasing
held output/effects. A mismatch, writer error, or dropped receipt aborts the typed images and
freezes persistence integrity. This is a useful K2
compatibility bridge, not K3 durability: the serial, store revision, and reservation sequence are
process-local, and there is no durable intent/token/idempotency witness or whole-bundle
reconciliation. A crash after store application but before RAM install now reloads a canonical
typed state record, but cannot yet prove or reconstruct the action's exact outcome, idempotency
result, causal events, allocator changes, or outbox work.

The toy `focus -> brace -> guarded` slice exercises two-record preparation, refusal, abort, stable
ordering, and install-after-ack-boundary behavior in core/contract tests. Separate restart tests
exercise strict all-or-nothing typed-record hydration. This is not Gate 1: typed clock expiry,
causal effects, early idempotency, authoritative receipt/outcome, projected offers, and real
end-to-end bundle recovery remain open. Existing module maps remain a legacy compatibility surface
and migrate one action family at a time. Authenticated ingress remains a
separate opaque server-minted type: transport JSON never gets to assert a principal, actor, or
internal deterministic idempotency key merely because it can encode the same shape.

#### After-commit work

Post-commit work is separated into:

- `on_commit`: authoritative semantic events and persistence included in the bundle;
- `after_commit_recoverable`: player output and panels reconstructed from the durable receipt and a
  current keyframe if delivery is lost;
- `after_commit_durable`: token-keyed outbox intents included in the bundle for webhooks, Discord,
  jobs, or decision wakeups that promise eventual delivery; dispatch is idempotent at-least-once
  and is forbidden until the parent token has an exact finalized witness;
- `after_commit_best_effort`: explicitly lossy telemetry or hints that may be dropped;
- `on_abort`: a bounded, non-authoritative refusal addressed to the requester.

Every durable outbox item has a deterministic ID derived from `(parent commit token, ordinal)`, a
versioned bounded payload, destination/adapter, retry policy, and `Pending | Delivered |
DeadLetter` state. Its record and independent pending-outbox index are materialized with the action;
the dispatcher checks the parent's finalized witness, scans/rebuilds its index at boot, and records
delivery/dead-letter durably. A crash after an external send but before its acknowledgement may
repeat delivery: adapters pass the stable item ID as an idempotency key when supported and otherwise
must tolerate/label duplicates. This is at-least-once, not magical exactly-once integration. An
action intent cannot be compacted while one of its durable outbox items is unresolved.

The K0 durable gate now classifies and holds output/effects until the exact writer receipt, and a
deterministic abort releases only explicitly abort-safe diagnostics. It still installs legacy RAM
mutations before acknowledgement. The K2 typed-state lane now keeps its owned post-images private
until an exact matching applied serial, but it has not migrated legacy handlers and is not the K3
exact-token/restart-reconcilable protocol.

There is a deliberate remaining K0 bound gap at construction time. `Outbox`, `Effects`, and the
initial `PendingCommit.sends/effects` are count-unbounded, and several individual items are
byte-unbounded (`OutputEvent::Panel` JSON, rich text and room/who collections,
`Effect::ModuleAsync.payload`, and its async-completion payload). Once the loop sees an overflow,
a legacy handler may already have changed authoritative RAM. A global cap imposed there would
force one of three policies:

1. refuse persistence and integrity-freeze with the already-mutated RAM post-image;
2. commit the state while truncating required presentation/effects, violating their semantic
   delivery class; or
3. roll back the legacy mutation or build it in bounded scratch state before live installation.

K0 chooses none of those silently. It exposes the held send/effect counts as metrics and prevents
later operational traffic from growing them during a hung receipt. K2 implements option 3 for
typed state records: `ActionTxn`/`PreparedTxn` preflights their operation count and canonical bytes
before any live mutation or store submission. It does not yet bound the complete action outcome,
event, presentation, or effect set. New authoritative state-family actions must use the prepared
path; legacy handlers remain a measured compatibility risk until migrated.

### C3. Legal-action projection

#### Contract

Humans, line commands, TUI/web controls, deterministic NPCs, and LLM tool calls consume the same
legal-action projection. Internal offer state and projected wire state are deliberately different
types:

```rust
pub struct InternalOffer {
    pub offer: ActionOfferId,
    pub action: DefinitionRef,
    pub provider: OwnerId,
    pub viewer_scope: ProjectionScopeId,
    pub truth_subject: SubjectKey,
    pub guards: GuardSet,
    pub binding_hash: [u8; 32],
    pub expires: Option<ClockStamp>,
}

pub struct ProjectedLegalAction {
    pub offer: ActionOfferId,             // opaque binding to InternalOffer
    pub subject: PerceivedRef,
    pub parameters: ParameterSchema,
    pub target_schema: TargetSchema,
    pub required_channels: Vec<ChannelId>,
    pub window: Option<ActionWindowRef>,
    pub expires: Option<ClockStamp>,
    pub presentation: ActionPresentation,
}
```

The projection is advisory, bounded, viewer-specific, and short-lived. It contains no hidden or
unprojected truth IDs, revision keys, raw generations, guard set, provider-internal schema,
request/plan hash, or authority evidence. An explicitly authorized `KnownObject` dbref is projected
data; an unseen object is a `Contact` or absent. The offer is never an authorization cache:
execution retrieves or regenerates the internal offer and rechecks all server-held guards. An offer
ID is opaque and bound to viewer, subject, projection generation, and expiry.

Offer IDs survive a server restart without becoming authority. Initially they are a versioned HMAC,
under a durable rotating offer secret, over the canonical internal binding and expiry. Execution
regenerates the viewer's bounded offer set and constant-time matches the token. An unchanged offer
therefore resolves identically after restart; a changed generation returns the same projected
`Stale/Unavailable` response as any other invalidated offer, not a truth-revealing “unknown ID.”
Deployments may instead persist bounded offer records, but process-local random IDs are not valid
for replayable commands or delayed decisions.

The projection must not become an oracle. Omitted actions, parameter bounds, source labels, target
lists, and refusal reasons are all observer projections and may not reveal hidden components,
contacts, identity, damage, inventory, or ownership.

#### Implementation

1. Add an object-safe `ActionProvider` registration seam beside behavior providers and separate
   internal/projected offer stores.
2. Add a bounded `ProjectedLegalActionSink`; reject duplicate action IDs and cap actions, targets, parameter
   bytes, and projection work per subject.
3. Map contextual behavior verbs to offers, so line input resolves through the same action service
   rather than invoking a second mechanical handler.
4. Generate TUI/web controls and NPC/LLM tool schemas from `ProjectedLegalAction`.
5. Require every execution path to call the same provider's `prepare`; no AI-only or UI-only action
   entry points.

### C4. Time and action windows

#### Contract

Scheduled records name a typed clock, generation, next deadline, restart policy, and maximum
catch-up. Generic action windows define eligibility and collection; rulesets define initiative,
phases, reactions, and action costs.

```rust
pub struct ActionWindow {
    pub id: ActionWindowId,
    pub clock: ClockRef,
    pub opens: ClockStamp,
    pub closes: ClockStamp,
    pub generation: u64,
    pub mode: WindowMode, // Immediate | CollectSimultaneous
    pub capacity: u32,
    pub restart: RestartPolicy,
    pub timeout: TimeoutPolicy,
    pub max_catch_up: u32,
}
```

Window boundaries are half-open: `opens <= now < closes`. Every intent is stamped with the window
generation it observed. Simultaneous intents sort by an explicit ruleset key followed by stable
engine tie-breakers, never hash iteration or LLM response time.

The final guard check records the authoritative acceptance stamp. Once the world atomically enters
`CommitPending`, crossing a wall-clock deadline while the store is resolving does not retroactively
invalidate the action. A provider must not attempt a post-commit deadline check.

For `CollectSimultaneous`, accepting, replacing, or cancelling an intent is itself an idempotent
authoritative action. The durable record contains the window generation, actor/subject, request
hash, offer binding, perceived targets, and submitted parameters, but no client-supplied guards.
Closing a window durably freezes its bounded intent set before resolution; resolution consumes
that exact deterministic set and marks it consumed in the same bundle. Restart either resumes an
open collection or resolves the already-closed set -- it never reconstructs intents from transport
messages or response arrival times.

Existing UTC scheduler jobs, world IC time, Timestop turns, domain ticks, journeys, production, and
obligations become `ClockProvider` adapters. They do not collapse into one universal initiative
model.

### C5. Deterministic randomness and recorded resolution

#### Contract

Authoritative providers cannot use ambient `thread_rng` or a long-lived mutable ruleset RNG. They
receive a bounded `ResolutionRng` scoped to one idempotent action and domain-separated draw labels.

```rust
pub struct ResolutionRecord {
    pub action: CommitToken,
    pub algorithm: RngAlgorithmVersion,
    pub definition_refs: Vec<DefinitionRef>,
    pub inputs_hash: [u8; 32],
    pub draws: Vec<RecordedDraw>,
    pub modifiers: RulesValue,
    pub outcome: RulesValue,
}
```

The per-resolution seed is derived with a versioned keyed KDF from a durable server secret,
idempotency key, and action domain. Retrying the same accepted invocation cannot reroll; separate
draw labels prevent adding a flavor draw from shifting hit-location or damage streams. Tests may
inject a known root secret. The resolution record names the secret/KDF version; rotation retains
old key material until no unresolved action can require it.

The authoritative journal records draw results and the committed outcome. Replay applies that
outcome; it does not rerun historical formulas. Visibility controls which draws players see.
Commit/reveal or third-party-verifiable randomness is a future policy layered over the same record,
not required by this contract.

Implementation enforcement is partly social and partly structural: remove direct `rand` access
from ruleset action modules, expose RNG only through `PrepareCtx`, and add a dependency/lint check
for authoritative provider crates.

### C6. Event and causality taxonomy

#### Contract

The engine distinguishes:

1. **intent** -- what was requested;
2. **resolution** -- inputs, definitions, draws, and chosen consequences;
3. **commit** -- the durable authoritative mutation and receipt;
4. **observation** -- what a particular viewer or principal learned.

```rust
pub struct CausalRef {
    pub root: CommitToken,
    pub parent: Option<EventId>,
    pub ordinal: u32,
}

pub struct SemanticEvent {
    pub id: EventId,
    pub cause: CausalRef,
    pub owner: OwnerId,
    pub kind: CanonicalId,
    pub schema: DefinitionRef,
    pub truth: RulesValue,
    pub eligible_witnesses: WitnessSet,
}
```

Presentation text is never an event API. `WorldEvent` remains useful for lightweight lifecycle
notification, but rules events that drive damage, effects, canon, economy, or AI carry identifiers
and causality.

`eligible_witnesses` means the deterministic perception/knowledge eligibility computed from the
prepared pre/post-state at event time. Post-commit delivery uses that stored fact and current
addressability; it does not rewrite history because someone moved or disconnected. Actual transport
delivery is operational metadata, not a witness claim. If gameplay needs “Alice successfully
received the message,” that is a later acknowledgement/observation action with its own cause.

#### Cascade budget

Same-commit triggers execute through one deterministic queue. Initial hard defaults are:

- maximum trigger depth: 8;
- maximum semantic events: 256;
- maximum prepared mutation operations: 1,024;
- maximum serialized plan/event bytes: 1 MiB;
- duplicate key: `(root, source_event, handler, target)` executes once;
- ordering: phase, provider registration ordinal, source-event ordinal, target key.

Dequeuing does not relinquish custody. The queue permits exactly one active work item and returns a
non-cloneable token bound to that queue instance. Derived events require that live token, and the
coordinator must return it through explicit completion after handler execution. Dropping or
forgetting dequeued work prevents successful cascade finalization; a second dequeue while work is
active poisons the action. Provider readiness is checked again at completion so a concurrent owner
freeze wins over apparent handler success. Until trigger handlers are driven through the same
`ActionTxn` and durable bundle, this is an accidental-misuse guard rather than a sandbox against
trusted in-process provider code.

Exceeding any bound aborts the entire action with zero mutation. Mechanical triggers are prepared
in the same bundle. Analytics, LLM deliberation, and other post-commit reactions are deferred as
new causally linked actions with their own idempotency keys.

### C7. Generic resources, tracks, and effects

#### Contract

The kernel standardizes safe storage and addressing, not game meaning.

```rust
pub struct ResourceRef {
    pub owner: OwnerId,
    pub subject: SubjectKey,
    pub id: CanonicalId,
}

pub struct BoundedScalar {
    pub minimum: i64,
    pub maximum: i64,
    pub current: i64,
    pub revision: u64,
}

pub struct EffectInstance {
    pub id: EffectInstanceId,
    pub definition: DefinitionRef,
    pub owner: OwnerId,
    pub source: CausalRef,
    pub target: EffectTarget,
    pub applied_at: ClockStamp,
    pub duration: EffectDuration,
    pub stacks: u16,
    pub revision: u64,
    pub state: RulesValue,
}
```

The engine enforces checked integer arithmetic, bounds, unique identity, collection limits,
provenance, definition availability, valid duration clocks, and revision changes. Rulesets own:

- what a resource or track means;
- regeneration and decay;
- marked boxes versus scalar presentation;
- effect stacking, replacement, suppression, and modifier order;
- threshold consequences and damage transfer;
- visibility.

There is no universal “+2 modifier” evaluator. An effect is inert data until its owner is asked to
interpret it through a registered query or action provider.

#### Storage adapters

Do not immediately migrate every existing WoD sheet track or tactical cell into one global table.
Add `ResourceProvider` and `EffectProvider` contracts, then adapt existing authoritative storage:

- `SheetDoc` fields remain character-sheet truth;
- `TacticalUnit::ScalarCell` remains unit truth until a second consumer proves identical storage
  semantics;
- economy accounts remain economic truth;
- the common action protocol addresses and mutates each through its owner.

The first shared extraction should be checked scalar arithmetic and operation validation, not a
forced common presentation shape.

### C8. Authority, leases, reservations, and stale work

#### Contract

Four concepts remain distinct:

1. **Capability:** non-exclusive permission to attempt a category of action.
2. **Lease:** time/version-bounded exclusive control of a subject channel.
3. **Reservation:** a claim on consumable value or capacity that prevents double allocation.
4. **Revision:** evidence that all other observed state is still current.

The server-side `PendingDecision` retains the exact `GuardSet` and internal offer it observed. The
LLM/pathfinder/client-facing prompt receives only projected choices and an opaque decision token;
its result cannot echo or modify guards. Completion resolves that token, enters the ordinary action
protocol, and revalidates server-held state; it never resumes from the middle of an old handler.

Physical assignment to a station grants lease eligibility, not necessarily the active lease. This
allows a linkdead human to remain seated while deterministic fallback acquires the movement lease.
Primary leases are keyed by `(subject, channel)` and carry a checked epoch. Lease handoff occurs at
a declared safe boundary.

Command-subject priority is channel-specific. Unioning all seat channels and assigning the maximum
seat priority to the union can promote a low-priority movement grant to weapons/command priority.
The projection unit is
`ChannelGrant { actor, subject, owner, station, channels, priority, epoch }`: one grant per occupied
station, with no union across seats. Resolution may select the highest-priority grant that carries
the requested channel, but channel eligibility and priority must come from that same grant.
`epoch` is captured by delayed work and checked against the exact station grant at re-entry;
revocation, republish, or channel narrowing invalidates it. Unchannelled contextual commands may
retain a separate relationship priority. A station grant is lease eligibility, not the active
exclusive lease.

Reservations have a small durable state machine:

```rust
pub struct ReservationRecord {
    pub id: ReservationId,
    pub owner: OwnerId,
    pub resource: ResourceRef,
    pub holder: ScopedIdempotencyKey,
    pub amount: u64,
    pub revision: u64,
    pub expires: ClockStamp,
    pub state: ReservationState, // Held | Consumed(commit) | Released | Expired
}
```

Acquire, replace, release, and expiry are idempotent authoritative actions. Availability subtracts
all live holds using checked arithmetic. The consuming action verifies the reservation holder and
revision, changes `Held -> Consumed`, and applies the resource post-image in the same bundle;
refusal leaves it held until explicit release/expiry policy. Expiry is a causally linked scheduled
action and survives restart. Owners define which resources may be reserved and maximum hold time;
the engine enforces identity, amount, clock, revision, count, and byte bounds. A slow LLM does not
implicitly hold scarce value merely because it is deliberating.

#### Idempotency

Every action that can be retried after leaving the world loop has a bounded idempotency key and
request hash. This includes:

- gateway replay and API retries;
- LLM/NPC decisions;
- pathfinding results;
- approvals;
- reactions;
- quotes and contract settlement;
- production, upkeep, and scheduled obligations.

The result for a committed key is immutable. Conflicting untrusted reuse is refused, audited, and
rate-limited. A conflict in a server-generated deterministic key, commit-token binding, or durable
stored bytes is an integrity incident and freezes the smallest safe scope.

### C9. Truth versus observer projection

#### Contract

Authoritative state and wire state use different types. A projection provider takes truth plus a
viewer/knowledge context and returns a bounded DTO containing only what that viewer may know.

Projection covers:

- identity and target references;
- location, bearing, range, and route;
- sections, components, equipment, damage, and status;
- economic title, custody, and account visibility;
- faction membership and standing;
- legal actions and parameter/target choices;
- action outcomes and refusal detail.

`ContactId` maps remain private authoritative state. Two viewers need not receive the same contact
identifier for the same object. Losing contact increments or invalidates its generation; an old
action becomes stale rather than revealing whether the object moved, hid, or ceased to exist.

Errors follow the same projection rules as success. “Target not available” is preferable to an
error that distinguishes hidden, destroyed, out-of-range, wrong-faction, or unknown underlying
objects when the viewer cannot know the distinction.

LLM prompts receive projected DTOs and legal actions, never raw world objects. Staff truth views
are explicit privileged projections and are audited; they are not a bypass sprinkled into normal
rendering.

#### Implementation

1. Add `ProjectionProvider` and `PerceivedRef` resolution seams.
2. Replace direct serialization of `TacticalUnit`, sheets, and future economic records with
   purpose-built view DTOs.
3. Key caches by viewer/sharing-principal, truth revisions, perception revision, and contact
   generation. Bound cache size and lifetime.
4. Add leak tests that scan legal actions, panel JSON, errors, line output, and LLM snapshots for
   forbidden IDs/names/fields.
5. Capture deterministic witness eligibility during preparation; never recalculate a historical
   audience from post-commit location or connectivity. Track actual delivery separately.

### C10. Ruleset ownership and cross-ruleset interaction

#### Contract

Every definition, rules-bearing instance, action, effect, scene resolver, economic catalog, and
domain driver has exactly one registered owner. A provider may mutate only state it owns through
its public typed plan interface.

Mechanical interaction between different owners is refused unless a versioned bridge is
registered:

```rust
pub struct BridgeKey {
    pub source_owner: OwnerId,
    pub target_owner: OwnerId,
    pub interaction: DefinitionRef, // exact action/effect interaction version
}

pub trait InteractionBridge {
    fn definition(&self) -> DefinitionRef;
    fn prepare(
        &self,
        ctx: &PrepareCtx,
        invocation: &ResolvedInvocation,
    ) -> Result<Vec<Box<dyn PreparedMutation>>, ActionError>;
}
```

Bridge selection must be unique and deterministic. The bridge owns the translation, but both state
owners validate the mutations it proposes. Missing bridges disable that interaction rather than
guessing which ruleset wins.

Examples:

- a WoD character and tactical vehicle can share space and observe one another without a combat
  bridge;
- installing Shadowrun equipment into a tactical unit requires an explicit equipment bridge;
- an effect from one owner cannot alter another owner's sheet merely because their field names
  resemble one another;
- a setting adapter can bridge its character rules to generic `mush-tactics` without teaching the
  tactical container that setting's formulas.

### C11. Lifecycle and recovery

#### Contract

All valuable rules state has an explicit lifecycle:

```rust
pub enum MechanicalState {
    Active,
    Frozen { incident: IncidentId },
    Migrating { from: DefinitionRef, to: DefinitionRef },
    Quarantined { incident: IncidentId },
}
```

Boot order is normative:

1. open and validate integrity/quarantine stores;
2. compose and freeze definition/action/bridge registries;
3. exhaustively discover and reconcile/freeze every pending authoritative intent;
4. load raw object and module records from the now-reconciled store without firing mechanics;
5. validate schemas, digests, references, arithmetic, and ownership;
6. run explicit migrations as acknowledgement-gated plans, then reload their affected records;
7. quarantine failures without deleting their raw records;
8. rebuild derived indexes, command links, projections, and schedules;
9. resume only records whose restart policy and readiness gates allow it;
10. accept mutating ingress.

Intent discovery may be page- and per-tick-work-bounded, but it is logically exhaustive. The world
remains `Recovering` until the pending index and cursor scan reach a verified end; timeout, cursor
failure, or an intent beyond the first page can never fall through to writable boot.

The K0 implementation opens `QuarantineSpool` before migrations or boot writes and parse/hash
verifies every bounded versioned record. A crash-left temp containing one complete exact record is
promoted by deterministic no-overwrite recovery; a partial, malformed, or conflicting temp, an
unreadable directory, or a malformed final record blocks boot as unresolved integrity state.
Production paths have no log-only poison-preservation fallback. Exact-target reconciliation and a
durable resolution receipt remain required before a valid incident can clear automatically.

Use a structural world mode rather than overlapping booleans:

```rust
pub enum WorldMode {
    Booting,
    Recovering { incident: Option<IncidentId> },
    Writable,
    CommitPending { token: CommitToken },
    StoreReadOnly { incident: IncidentId },
    IntegrityFrozen { incident: IncidentId },
}
```

Mutating ingress is not attached until `Booting`/`Recovering` completes; if a transport is already
available for status, its action path returns a bounded not-ready response. Recovery writes use the
same exact-token protocol but cannot invoke ordinary module hooks or presentation while the world
is not writable.

`IntegrityFrozen` permits pure queries, transport-only attachment bookkeeping, barrier/status,
recovery, and shutdown. It returns bounded refusals for commands, input-context mutations, async
applies, timers, mutating connection hooks, external effects, and staff mutation; only explicitly
reconstructible work may coalesce by bounded stable key. `StoreReadOnly` may use a less restrictive
command classification, but an integrity freeze never relies on a verb string allowlist or an
unbounded deferred queue.

#### Recovery incidents

```rust
pub struct RecoveryIncident {
    pub id: IncidentId,
    pub owner: OwnerId,
    pub scope: RecoveryScope, // Record | Object | Module | World
    pub severity: RecoverySeverity,
    pub reason: RecoveryReason,
    pub references: Vec<SubjectKey>,
    pub payload_digest: Option<[u8; 32]>,
    pub discovered_at: ClockStamp,
    pub state: IncidentState,
}
```

Freeze the smallest scope that preserves invariants. A missing cosmetic definition may disable one
view; a corrupt unit freezes one unit; an unresolved cross-account journal break freezes economy;
an authoritative world-ledger or bundle conflict freezes the world.

Recovery is previewed, revision-bound, attributable, audited, and acknowledgement-gated. A spool
record moves to resolved state only after reading back the exact target bytes/hash. “Delete the
file” is not a recovery protocol.

#### Automated recovery before human escalation

Paging is the terminal branch of a bounded recovery state machine, not the first reaction to an
incident. For every recognized failure class the owner registers deterministic probes and repairs,
each with preconditions, an exact before/after digest, an attempt limit, and a verification step.
The recovery coordinator must, in order:

1. re-read and re-verify the authoritative bytes so a transient or stale observation cannot page;
2. retry only operations declared retry-safe, using the same identity/token;
3. search trusted installed providers and retained versions for the exact referenced definition and
   its closed dependency set;
4. dry-run any registered migration or reconstruction and require its guards to still match;
5. apply a repair only through the acknowledgement-gated recovery transaction, then read back the
   exact result;
6. otherwise freeze the smallest scope that preserves invariants, keep unrelated scopes live, and
   create or update one deduplicated incident;
7. page staff only when no registered operation can prove a safe result, or when bounded automated
   attempts are exhausted.

An LLM may summarize diagnostics, rank already-registered runbook choices, and explain a proposed
repair. It is never the authority that invents a migration, chooses between ambiguous truths, or
writes authoritative state. Successful recovery closes the incident and cancels pending pages;
acknowledging a page merely records human custody and does not unfreeze anything.

#### On-call staff paging

`page staff` means dispatch through a first-class protected operational service. It is not a log
line, in-game page, or broadcast to every Wizard. The initial model is:

```rust
pub struct OnCallPolicy {
    pub rotation: Vec<OnCallMemberId>,
    pub acknowledgement_timeout: Duration,
    pub retry_schedule: Vec<Duration>,
    pub escalation_limit: u16,
}

pub struct OnCallMember {
    pub staff: ObjId,
    pub active: bool,
    pub endpoints: Vec<VerifiedContactRef>,
}

pub enum ContactKind { Sms, Email, Discord }
```

Each participating Wizard supplies a cell-phone number for SMS and an email address; once Discord
integration exists, the Wizard also links and verifies a Discord account rather than entering an
unverified display name. Endpoint ownership is verified out of band (SMS/email challenge or
Discord OAuth/challenge), periodically re-verified, revocable, and referenced by opaque IDs.
Numbers, addresses, verification tokens, and provider credentials live encrypted in an operations
store with narrowly scoped access. They never live on MUSH objects, enter gameplay persistence,
appear in the canon/epistemic ledger, reach LLM context, or appear in ordinary staff diagnostics.

An alert contains only a stable incident ID, severity, affected scope/owner, safe bounded summary,
service/world identity, start time, and a runbook/status link. Quarantined payloads, player secrets,
and credentials are fetched separately after authorization. Critical incidents page the active
primary over SMS plus email (and verified Discord when available); lack of acknowledgement advances
through the rotation and retry schedule, then escalates to the configured fallback. Lower severity
can remain in the staff incident feed or email. Repeated signals coalesce by incident ID, delivery
attempts and acknowledgements are audited, quiet hours never suppress critical pages, and rate
limits prevent one poison condition from becoming an alert storm.

#### Staff runbooks and the LLM incident copilot

Runbooks are staff-authored, reviewed, versioned operational definitions rather than free-form
prompts. Initial scenario families should include player safety/harassment, database and durability,
security/privacy, transport/provider outage, and a combined abuse-of-service class—informally,
“database harassment”—where hostile gameplay input deliberately creates load, queue pressure,
idempotency conflicts, pathological records, or repeated recovery incidents.

Each runbook declares its exact version, applicability predicates, required staff role, evidence
sources, privacy class, typed steps, safe automatic actions, confirmation/dual-approval boundaries,
stop conditions, verification, rollback or containment, communication templates, attempt/time
bounds, and escalation target. Steps call registered diagnostic or recovery adapters; they do not
embed arbitrary shell commands. Every observation records provenance, every execution records its
inputs and result, and an incident stays bound to the runbook version that was selected. Staff can
amend a runbook only through the normal review path, never midway through an incident without a new
version and an explicit handoff.

A minimal implementation model is:

```rust
pub struct RunbookRef { pub id: RunbookId, pub version: u32, pub digest: DefinitionDigest }

pub struct RunbookDefinition {
    pub reference: RunbookRef,
    pub applies_when: Vec<IncidentPredicateId>,
    pub required_role: StaffRole,
    pub steps: Vec<RunbookStep>,
    pub escalation: EscalationPolicyId,
}

pub enum RunbookStep {
    Observe { adapter: DiagnosticId, evidence_class: PrivacyClass },
    Automatic { adapter: RecoveryId, attempts: u16, timeout_ms: u32 },
    Approval { roles: Vec<StaffRole>, quorum: u8, expires_ms: u32 },
    Contain { adapter: ContainmentId, reversible: bool },
    Verify { predicate: VerificationId },
    Communicate { template: MessageTemplateId },
    Escalate,
}

pub struct IncidentRun {
    pub id: IncidentId,
    pub runbook: RunbookRef,
    pub tracks: Vec<IncidentTrackRef>,
    pub next_step: u32,
    pub evidence: Vec<EvidenceRef>,
    pub actions: Vec<ActionReceipt>,
    pub state: IncidentRunState,
}
```

Every collection and diagnostic field above has a hard count/byte bound. `RunbookRef.digest`
prevents reviewed prose or executable adapter bindings from changing underneath a live incident;
an upgrade creates a new version and an audited handoff. `ActionReceipt` records the exact adapter,
inputs hash, approval witnesses, start/end times, result, and postcondition witness, so staff and a
later incident review can distinguish advice from actions that actually ran.

The LLM is a copilot over those definitions. It may correlate sanitized signals, retrieve and rank
applicable approved runbooks, summarize evidence with links to its sources, fill non-secret step
parameters, explain risks, draft player/staff communications, and guide the operator to the next
permitted choice. The deterministic coordinator—not the model—executes pre-approved automatic
steps and verifies their postconditions. The model cannot invent an executable step, widen scope,
read paging credentials, make a final harassment finding, impose irreversible discipline, delete
evidence, choose between ambiguous authoritative histories, or declare an incident repaired.

Mixed incidents keep linked but access-controlled tracks: a moderation track preserves the minimum
necessary player evidence and handles immediate reversible safety containment, while an operations
track rate-limits the vector, freezes the smallest affected scope, and repairs exact durable state.
This avoids copying sensitive conversation into database diagnostics or exposing infrastructure
secrets to moderators who do not need them. Closing either track requires its own typed verification;
the parent incident closes only when both are resolved. Runbooks should have fixture-driven drills
and post-incident review so staff experience, including novel failure modes, becomes reviewed policy
instead of disappearing into chat history.

Make authoritative Ledger writes preparatory: `prepare_append` validates entry/chunk byte limits
and checked sequence/chunk arithmetic without mutation; `apply_prepared` runs inside `ActionTxn`.
Authoritative restore returns a checked report/error instead of logging corruption and continuing.

### C12. Structured presentation contract

#### Contract

Rules providers emit projected semantic records, never terminal-shaped authoritative events.

```rust
pub struct ViewEnvelope {
    pub view: ViewId,                    // e.g. "unit.status"
    pub schema_version: u32,
    pub viewer_scope: ProjectionScopeId,
    pub subject: PerceivedRef,
    pub state_revision: u64,
    pub projection_revision: u64,
    pub sequence: u64,
    pub base_sequence: Option<u64>,
    pub commit: Option<CommitToken>,
    pub data: serde_json::Value,
}
```

Keyframes replace a view. A delta names the exact base sequence it extends; a gap, stale projection
generation, or unknown schema forces a keyframe request. Payload and delta sizes are bounded.

Initial stable view names include:

```text
character.sheet
legal.actions
resolution.outcome
unit.status
unit.sections
unit.components
account.summary
contract.status
recovery.incident
```

Redaction occurs before constructing `ViewEnvelope`; transports never receive truth and decide what
to hide. Registered renderers turn the same view into line SSH text or TUI widgets. HTTPS consumes
the semantic data directly.

Success presentation is emitted only after durable acknowledgement. It carries a commit token so
structured clients can deduplicate and acknowledge it. Raw line SSH has no client acknowledgement,
so exactly-once text cannot be guaranteed across a crash: it receives best-effort/at-least-once
rendering with the stable transaction ID, while `action/status <idempotency-key>` and current
keyframes recover the semantic result.

`action/status` authenticates and authorizes the requesting principal before lookup, then projects
the stored authoritative outcome through C9 into `ProjectedActionReceipt`. A guessed commit or
idempotency token reveals neither existence nor truth to an unauthorized viewer. Internal request,
plan, bundle, and outcome hashes never appear in ordinary receipts, panels, errors, or LLM context.

---

## 4. Authoritative bundle protocol

This protocol is the persistence half of C2 and the core of Gate 5.

### 4.1 Bundle shape

```rust
pub struct AuthoritativeBundle {
    pub schema_version: u16,
    pub token: CommitToken,
    pub request: RequestUid,
    pub idempotency: ScopedIdempotencyKey,
    pub ingress_hash: [u8; 32],
    pub request_hash: [u8; 32],
    pub plan_hash: [u8; 32],
    pub bundle_hash: [u8; 32],
    pub cause: CausalRef,
    pub definitions: Vec<DefinitionRef>,
    pub writes: Vec<ReplayWrite>,
    pub outcome: AuthoritativeOutcome,
    pub semantic_events: Vec<SemanticEvent>,
    pub eligible_witnesses: WitnessSet,
    pub durable_outbox: Vec<DurableOutboxIntent>,
}

pub enum ReplayWrite {
    ObjectSet { id: ObjId, post_image: Obj },
    ObjectDelete { id: ObjId },
    ObjectAllocatorSet { next_id: ObjId }, // core-owned; not arbitrary metadata
    ModuleJsonSet { namespace: OwnerId, key: CanonicalKey, value: Value },
    ModuleDelete { namespace: OwnerId, key: CanonicalKey },
}
```

Every write is an idempotent exact post-image. `StreamAppend`, increment operations, server-time
reads, and generated Redis IDs are forbidden in an authoritative bundle. Append-only history uses
deterministic keyed Ledger chunks/commit records. Creating an object must include the exact
core-owned allocator post-image in the same bundle; until a store implements that variant, object
creation through the action protocol is refused. Arbitrary core metadata writes remain forbidden.

Definitions and writes are normalized into stable key order before hashing. Duplicate writes to a
state key are rejected even when their bytes agree; conflicting writes are an invalid plan. The
versioned, domain-separated `bundle_hash` covers every field except itself, including the complete
authoritative outcome, event-time witness facts, and durable outbox intents. Observer-specific
redaction and prose are not stored in this bundle.

Bundle v2 adds the contiguous `semantic_events` graph. Event zero must exactly match the root cause,
outcome, and outcome witnesses; every later event names an earlier parent and all events remain
owned by the bundle provider in the current same-owner transaction model. Bundle v1 remains
strictly readable without inventing events. Compact receipt v3 retains the event graph after replay
writes are checkpointed; receipt v2 remains strictly readable for legacy v1 actions.

Initial bounds are configurable with conservative hard defaults:

- one authoritative bundle in flight;
- 256 KiB per prepared record;
- 16 MiB total encoded bundle;
- 256 retained/recovery records;
- 64 MiB aggregate encoded full-intent retention until checkpoint compaction;
- 1,024 replay writes;
- all arithmetic and encoded sizes checked before RAM installation or store submission.

Reaching a bound refuses before mutation. It never merely logs and sends a likely poison record.

### 4.2 Store contract

Extend `WriteBatch`/`WriteReceipt` with an authoritative class and exact token/hash. Add:

```rust
pub trait ObjectStore {
    async fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> Result<IntentReceipt, StoreError>;

    async fn reconcile_authoritative(
        &self,
        token: &CommitToken,
        expected_bundle_hash: [u8; 32],
    ) -> Result<AuthoritativeReceipt, StoreError>;

    async fn authoritative_status(
        &self,
        token: &CommitToken,
        expected_bundle_hash: [u8; 32],
    ) -> Result<BundleStatus, StoreError>;
}
```

`BundleStatus` is `Absent`, `PendingExact`, `FinalizedExact(AuthoritativeReceipt)`, or `Conflict`;
it never infers success from a world-loop serial. A status API must receive the expected bundle
hash or return enough stored witness metadata for the caller to compare it; token equality alone
cannot establish exactness.

The backend protocol has two immutable records: a full **intent** and a compact **finalized
witness**. It proceeds as follows:

1. Atomically reserve the scoped idempotency binding, `SET NX` the versioned canonical intent, and
   add its token to a pending index. The intent contains the full bundle/replay set and hash.
   Existing exact bytes mean resume; a different ingress/request/token/hash follows the conflict policy in
   §2.3. This is implemented as one checked Lua/CAS operation in Redis.
   Once `IntentReceipt` is exact, the server may send the gateway's IPC ownership acknowledgement;
   losing that acknowledgement is harmless because replay finds the same pending binding.
2. Apply the deterministic persistence materializations from the intent. These are recovery
   post-images, not observer projections, and may be replayed any number of times.
3. Establish a compare-and-finalize barrier, read back every touched key, and verify its canonical
   bytes. The backend must prevent any touched value/version from changing between readback and
   finalization. Redis uses `WATCH` before readback followed by checked `MULTI/EXEC`, or one bounded
   Lua operation that rechecks every post-image/digest and finalizes atomically. An intervening write
   aborts and retries reconciliation. Wrong types, partial execution, disconnects, and timeouts
   remain `PendingExact`; they never erase the intent.
4. In that atomic barrier, verify the intent and bundle hashes plus every touched post-image, create
   the immutable finalized witness/authoritative receipt, persist the idempotency result binding,
   and remove the pending index entry. This transition is the durable commit point.
5. Read back or probe that witness before returning exact acknowledgement.

Persisting intent before any materialization guarantees that no partial authoritative post-state
exists without boot-discoverable recovery evidence. Boot reconciles both the pending index and a
bounded cursor scan of the intent keyspace before loading any authoritative world/object/module
snapshot; if one was read earlier it is discarded and reloaded. The scan repairs an index update
interrupted by backend/host failure. An exact pending intent is replayed and finalized. An
unreplayable intent or token/hash conflict quarantines the complete bundle and integrity-freezes.
The protocol is a store contract; the in-memory `FaultStore` must implement the same state machine,
not a simpler happy-path approximation.

The writer halts subsequent mutating jobs at the first unresolved deterministic or ambiguous
failure. It may service status probes and recovery, but it must not let a later FIFO job commit past
an unresolved predecessor. Before an authoritative action is accepted, earlier fire-and-forget
writes that contributed to its observed RAM state must drain through an exact writer barrier or be
absorbed into its bundle. Merely declaring them “irrelevant” is insufficient without a registered,
disjoint persistence owner/keyspace and dependency proof.

Registration assigns owned persistence prefixes a class such as `Authoritative`, `LegacyDurable`,
or `Rebuildable`. After a prefix migrates to `Authoritative`, the store/coordinator rejects legacy
dirty or fire-and-forget writes to it. A mixed batch may not bypass that check.
Production Redis credentials/ACLs also grant mutation of authoritative prefixes only to this store
writer; bypass clients and modules are unsupported corruption, detected by exact verification.

Finalization and retention are distinct. Pending full intents are byte/count bounded but are never
silently evicted. After exact readback, the replay payload may be compacted only when a durable
checkpoint proves the authoritative post-images and leaves an immutable tombstone containing the
scope/key, request hash, token, bundle/outcome digest, and terminal status. Presentation/delta
history may use ordinary bounded retention. Idempotency tombstones remain for the declared retry
horizon; permanent/internal operation keys require permanent ledger/checkpoint evidence. Tombstones
are compacted only as a complete epoch after a durable checkpoint. The per-scope
`minimum_accepted_epoch` is advanced atomically with compaction or durably **before** individual
tombstones are deleted; deletion must never precede the watermark. A composite key below that
watermark returns `Expired` even when its individual tombstone is gone, so absence is never mistaken
for new work. Gateway request UIDs and internal keys carry their epoch; HTTP clients that want retry
guarantees must echo the issued composite key rather than a bare reusable string. If the API
promises historical outcome replay, the protected authoritative outcome is retained for that same
horizon; afterward status may return only authorized terminal metadata plus a current keyframe.

The receipt policy must also state what “durable” means. A successful Redis finalization reply
proves server application, not survival of Redis/host power loss. Process-restart gates may use the
applied receipt; deployments promising host-crash zero loss require an explicit Redis
AOF/`WAITAOF` policy such as `appendfsync always` and a corresponding receipt mode.

### 4.3 World-task state machine

```text
Writable
  -> prepare complete + final guards
  -> CommitPending(token)             serialization point; no mutating ingress
       -> reserve exact durable intent (request ownership transfers; IPC ack may follow)
       -> replay/read back materializations
       -> finalize exact witness + ack
       -> install prepared RAM state
       -> Writable
       -> publish recoverable/best-effort work; dispatch durable outbox

CommitPending(token)
       -> transient/ambiguous          remain pending; probe intent/witness and retry
       -> deterministic poison         spool whole bundle; IntegrityFrozen
       -> token conflict               IntegrityFrozen
```

Installing RAM after exact acknowledgement is the simplest initial model. If the process dies
after store commit but before RAM installation, boot loads/reconciles the committed post-images. If
the writer reports failure, no tentative state needs rollback. This intentionally trades one
Redis-round-trip of mutation latency for a much smaller correctness surface.

While an intent is pending, allowed gameplay queries read the authoritative world task's pre-state
and may report that a transaction is pending. They never read partially materialized Redis keys.
Redis is the durability/recovery authority, not a concurrent gameplay read model; any HTTPS or
module path that bypasses the world to read those keys violates the contract.

The existing fire-and-forget path remains available for explicitly non-authoritative cosmetic or
rebuildable work, but rules actions, economy posts, irreversible approvals, and mechanical clocks
use the authoritative path. Ledger seals produced inside an authoritative action belong to that
bundle and are not independently cleared by E0's global high-water mechanism.

---

## 5. The five minimum proving gates

The gates live in a dedicated top-level integration-test package (working name
`mush-contract-tests`) so it can depend on core, ruleset, tactics, sim, NPC/LLM mocks, server, store,
proto, and testkit without creating production dependency cycles. Its `ToyRulesModule` contains
only the definitions needed below.

Each gate must pass in the in-memory store suite. Persistence/restart portions also run against a
disposable Redis instance when `REDIS_URL` is configured. Golden wire fixtures cover line and
structured output from the same projected DTOs.

### Gate 1. Resource spend plus temporary effect

Toy definitions:

- character resource `focus`, bounded `0..3`;
- action `brace`, costing one focus;
- effect `guarded`, lasting two scene turns.

The action spends exactly one focus, emits `Braced`, and a registered same-commit trigger creates
exactly one `guarded` effect in one bundle. Tests prove:

1. insufficient focus, overflow, unavailable definition, or closed window produces zero mutation;
2. resource and effect cannot commit separately;
3. duplicate idempotency returns one receipt without replanning, even after the spend takes focus to
   zero or unrelated state later changes;
4. two contenders for the final focus exercise reservation acquire/contention/consume/release/
   expiry, including restart at each state;
5. a looping or over-budget trigger aborts the spend, event, and effect with byte-identical
   pre-state;
6. effect expiry is a causally linked action on the declared clock and is restart-safe;
7. legal actions disappear/reappear with current resource, effect, authority, and window state;
8. a missing but intact effect definition freezes and preserves its instance; malformed bytes
   quarantine it; restoring the exact definition permits an audited thaw;
9. line/TUI/web views render the same projected state, and dropped/out-of-order deltas, unknown
   schemas, and changed projection generations force an authorized keyframe.

This gate primarily proves C1, C2, C3, C4, C6, C7, C8, C11, and C12.

### Gate 2. Component action plus sectional damage

Toy unit definitions contain three opaque sections, one weapon component, ammunition and heat
cells, and a component-provided `fire` action. The action requires the weapons-channel lease,
consumes ammunition, adds heat, records to-hit/location/damage draws, and adjusts a target section.

Tests prove:

1. source and target post-states are both prepared before either installs;
2. ammunition, heat, target damage, resolution event, and receipt share one bundle;
3. stale source, target, placement, component, lease, window, or definition revisions refuse with
   zero mutation;
4. removing/disabling the component removes the legal action;
5. retry after ammunition reaches zero, and after unrelated later mutations, returns the recorded
   resolution rather than regenerating eligibility or drawing again;
6. accepting/replacing/cancelling simultaneous intents survives restart, and two equal intents from
   the durably closed set resolve with stable explicit ordering;
7. a cross-owner variant refuses with no bridge, succeeds with exactly one versioned bridge, and
   aborts atomically when either owner rejects; duplicate/ambiguous bridge registration fails
   composition before boot.

This is the first proof that a `UnitMutationPlan` is a fragment inside a multi-record action rather
than the whole transaction protocol.

### Gate 3. Hidden contact targeting without truth leakage

The Gate-2 action targets only `PerceivedRef::Contact`. Two observers receive different opaque
contact IDs for the same target.

Tests prove:

1. no underlying target `ObjId`, real name, hidden component, exact damage, or forbidden action is
   present in legal-action JSON, parameters, outcome, panels, line output, errors, or LLM context;
2. the world resolves the contact privately and validates its generation at execution;
3. losing or changing contact makes the invocation stale without explaining hidden truth;
4. observer-specific outcomes may legitimately differ while referencing one authoritative event;
5. reconnect/keyframe refresh preserves only currently knowable contacts;
6. the cross-owner bridge exposes no truth IDs or owner-private fields through actions, errors, or
   outcomes;
7. moving or disconnecting a witness after commit does not rewrite stored event-time eligibility,
   while actual delivery remains separate operational state.

This gate primarily proves C3, C6, C9, C10, and C12.

### Gate 4. Stale asynchronous NPC/LLM action

A mock deliberator receives the same legal-action projection as a human, then pauses. While it is
paused, tests separately change the control lease, unit revision, contact generation, order
revision, action window, and dynamic provider/action-enablement generation. A separate restart case
loads with the formerly referenced exact definition unavailable.

On completion, the proposal re-enters as an ordinary invocation. Tests prove:

1. the server-held `PendingDecision`, not the model prompt/result, retains guards; every changed
   guard returns a typed stale/unavailable refusal with zero mutation, no RNG draws, and no
   mechanical events;
2. duplicate, late, out-of-order, malformed, and provider-down completions are idempotently ignored
   or refused;
3. the LLM cannot add an action/tool not present in its offer set;
4. a deterministic fallback controller remains available without waiting for the LLM;
5. a still-current completion uses exactly the same prepare/commit route as human input.

This gate primarily proves C2, C3, C4, C8, C9, and C11.

### Gate 5. Restart and ambiguous-commit matrix

This is a matrix, not one happy-path restart test. Inject process termination or store failure at:

1. after the gateway sends the request but before planning, while the gateway still owns replay;
2. after resolution/plan, before durable intent reservation;
3. after exact intent reservation, before/after (or with loss of) the IPC ownership
   acknowledgement, and before any persistence materialization;
4. after some persistence materializations apply, before readback/finalization;
5. after the exact finalized witness commits, before the writer receives or reports its
   acknowledgement;
6. after acknowledgement, before RAM installation;
7. after RAM installation/ack, before presentation or durable-outbox dispatch;
8. during or after presentation/dispatch, followed by duplicate request/delivery;
9. exactly at Ledger chunk rollover;
10. while a poison bundle or quarantine record is being written;
11. after an object post-image materializes but before its paired allocator post-image applies;
12. on both sides of watermark-first or atomic finalized-intent/tombstone compaction (and the
    prohibited delete-before-watermark order as a negative test);
13. with an unindexed pending intent beyond the first boot-scan cursor page.

Add `FaultStore` modes for `BeforeIntent`, `AfterIntent`, `AfterNMaterializationOps`,
`AckLostAfterFinalize`, `TokenConflict`, `QuarantineWriteInterrupted`, and `RestartAfterAck`. Add a
live Redis case with a wrong-typed middle materialization and split-process cases on both sides of
the IPC ownership acknowledgement.

Server-process kill cases keep the gateway alive and prove its bounded custody/reannounce path.
A separate gateway-process kill before `Owned` expects loss for raw SSH unless durable ingress
journaling is configured; with journaling, it must replay. HTTPS cases kill either process and use
caller replay. Also lose a `Rejected` response and prove replay causes no duplicate mutation, then
saturate custody limits and prove read backpressure/bounded refusal rather than queue growth. A
mixed raw-command case loses `LegacyClaimed` and `GatewayReleased` frames and proves legacy code
never executes before release and never executes twice.

For one stable idempotency key, every cut point must converge on:

- before request-ownership transfer, no commit is allowed and the gateway/client still owns replay;
- after ownership transfer, one terminal receipt or explicit integrity incident is required;
- every partial materialization has either an exact finalized witness or a complete exact pending
  intent from which recovery finishes;
- boot remains recovering through every discovery page and never misses an unindexed intent;
- exactly one final state transition when a witness exists;
- one immutable receipt and recorded RNG outcome;
- no reuse of a conflicting idempotency key;
- no object ID reuse and an allocator value consistent with all committed objects;
- a compacted old key returns `Expired` below the durable scope watermark and never re-executes;
- no silent auto-lift of integrity quarantine;
- an authorized, viewer-projected semantic result and current keyframe;
- no durable-outbox dispatch before finalization; post-finalize restart rediscovers it independently,
  send/ack loss may duplicate only under the documented adapter contract, and recovery reaches one
  durable `Delivered` or `DeadLetter` state;
- duplicate receipts are deduplicated, and missing/out-of-order deltas, unknown schemas, and stale
  projection generations trigger resync rather than corrupt a view.

The gate promises exactly-once authoritative state. Structured clients deduplicate commit tokens;
line SSH may repeat or miss a transient rendering across a crash and recovers via transaction ID,
`action/status`, and state keyframes. It does not promise impossible exactly-once raw text delivery.

### 5.1 Gate coverage

| Contract | Primary proving gates |
|---|---|
| C1 definition registry | 1, 2, 4 |
| C2 action/plan/commit | 1, 2, 4, 5 |
| C3 legal actions | 1, 2, 3, 4 |
| C4 clocks/windows | 1, 2, 4 |
| C5 recorded RNG | 2, 5 |
| C6 event/causality | 1, 2, 3, 5 |
| C7 resources/effects | 1, 2 |
| C8 authority/staleness | 1, 2, 4, 5 |
| C9 observer projection | 1, 2, 3, 4 |
| C10 ownership/bridges | 2, 3 |
| C11 lifecycle/recovery | 1, 2, 4, 5 |
| C12 structured presentation | 1, 2, 3, 5 |

No contract is accepted solely because its unit tests pass. At least one end-to-end gate must
exercise it through real registration, world dispatch, persistence, acknowledgement, projection,
and presentation seams.

---

## 6. Current implementation map

The repository already contains useful local proofs. They should be adapted, not discarded.

| Contract | Current foundation | Missing shared contract |
|---|---|---|
| C1 definitions | shared canonical owner/kind/id/digest identities; bounded `RulesValue`; immutable global definition registry; exact-reference closure; one provider semantic validator per owner/kind; bounded monotonic owner-readiness latch; tactics exact-digest owner fan-out; dry-run migration registry; tactics/sim adapters | owner-freeze discovery in remaining rules-bearing adapters, executing one migration through K3 durability, broader production provider coverage |
| C2 plan/commit | `mush-sim::Transition`, `UnitMutationPlan`, world single writer, `WriteBatch`, K0 exact receipt gate, typed output/effect custody and one `CommitPending` batch; K2 action invocation/provider metadata; engine-owned typed state families; owner-scoped bounded multi-record `ActionTxn`; one-shot `PreparedTxn`; same-owner trigger execution in the root scratch transaction; exact staged pairing between typed post-images and an immutable K3 bundle; FIFO reserve/reconcile custody; exact 128-bit token/hash receipt validation before RAM install; canonical bounded K3 bundle/token/replay/event/outbox models; in-memory exact intent protocol and cut-point matrix; Redis checked-Lua intent/idempotency reservation plus replay/readback/finalization; object-plus-allocator admission; hash-aware status; live Redis 8.10 fault execution; exhaustive pre-hydration server recovery; bounded runtime deterministic retries and whole-bundle dead-letter; event-preserving compact finalized checkpoints; watermark-first epoch retirement; typestate early-idempotency coordinator and first real tactics provider; recovery-driven durable-outbox worker with stable-ID at-least-once delivery; independent bounded authoritative/legacy/control writer lanes; versioned epoch-scoped gateway request UIDs, bounded per-connection/global count+byte custody, explicit dispositions, legacy claim/release, and post-reattach FIFO replay | transport/parser/legal-offer coordinator wiring; remaining ownership-ack process/IPC fault matrix; cross-owner transaction/bridge execution; broader provider and legacy migration |
| C3 legal actions | contextual behavior commands, required channels, exact per-actor/per-station `ChannelGrant` projection with paired channel/priority and epoch revalidation, LLM tool manifests, exact owner-stamped action-provider registry | separate internal/projected legal actions, bounded offer store, grant-bound offer witnesses, one submission path for human/UI/AI/LLM |
| C4 time/windows | scheduler; strict `ClockRef`/`ClockStamp`; bounded append-only world-IC epoch history `{ real_anchor, ic_anchor, rate, generation }`; continuous rate changes; atomic epoch/head persistence; boot hydration before modules; synchronous `(ic_now, generation)` module-context read; Timestop turns, passive domain clock, Journey timing, mobility switching | registered provider lookup, typed deadline validation, generic window generations, simultaneous intent collection/order, restart/catch-up policy |
| C5 RNG | injectable `mush-dice::RollCtx`; test-seeded ChaCha in rulesets | idempotency-derived resolution streams, draw transcript, algorithm/version and durable outcome record |
| C6 causality | `WorldEvent`, scene/canon ledgers, module observers; stable pre-commit request/event IDs and causal references; bounded deterministic trigger queue with duplicate suppression and non-droppable active-work custody; queue execution inside one root `ActionTxn`; v2 bundles and v3 compact receipts retaining ordered root/derived semantic events and event-time witnesses | public commit-token-bound event IDs, intent-resolution-commit-observation integration, cross-owner trigger execution through validated bridges |
| C7 tracks/effects | sheet tracks, tactical `ScalarCell`, ruleset-specific damage/effect state | common checked operations, resource/effect addressing, provenance/duration/revision contract and adapters |
| C8 guards | per-station channel grants (no cross-seat union/max promotion), persisted unit `control_gen` carried as each grant's epoch, exact station/channel/epoch stale-intent revalidation, expected unit revision, economy plan draft; canonical `RevisionSet`, owner-stamped validator registries, explicit server-created `GuardSet` on every staged typed action, final fail-closed revalidation at the synchronous writer-custody transition, durable exact-bundle reservation, and coordinator-owned early idempotency preflight | active per-channel leases, safe-boundary handoff, and wiring every delayed result/offer through the exact grant plus lease guards |
| C9 projection | per-observer perception and delivery, witness-aware power events | opaque contact references, dedicated view DTOs, shared legal-action/error/LLM projection |
| C10 ownership | registrar-stamped definition/action/state-family owners; composition-bound opaque typed state handles; provider-stamped transactions that reject cross-owner handles; fail-closed cross-owner tactical references; immutable directional exact-definition bridge registry; shared owner-readiness gates on executable definitions/providers/validators/bridge routes | ownership migration for state outside `ActionStateStore`, executable bridge preparation across typed transactions, freeze aggregation from remaining owner adapters |
| C11 lifecycle | pre-boot validated unified poison spool; v1 Ledger read compatibility plus tagged v2 Ledger/bundle records sharing one lock, capacity gate, and evidence archive; exact seal retention/isolation; structural world modes; bounded recovery incidents/reports; tactical Active/Frozen/Migrating/Quarantined records; tactics owner-scoped digest-conflict freeze; prepared typed images install or abort on the matching legacy serial or K3 token/hash receipt; strict atomic state-family decode/validate/canonical boot hydration; legacy isolation of `action-state/*`; store-enforced ownership of authoritative module prefixes; bounded in-memory and Redis pending indexes plus independent paged full-intent discovery; exhaustive bounded boot reconciliation before world hydration or writable bootstrap; exact retry repair of stale pending-index membership; canonical retained-intent storage validation; atomic reconstruction of derived Redis count/byte counters from exhaustive boot discovery; exact bundle quarantine recovery with live Redis WRONGTYPE proof; coordinator abort of typed reservations on pre-stage assembly failure; exact checkpoint/epoch/outbox cleanup and retry | exhaustive orphan-owner discovery; full C11 resolution receipts; owner freezes in remaining lifecycle adapters; authoritative account ingress; automated runbook |
| C12 presentation | `OutputEvent::Panel`, registered panel feeds, keyframe/delta kind, line fallback renderers | schema/projection/state revisions, delta base/sequence, commit receipt, resync/deduplication |

None of the five gates currently traverses all of these through one action pipeline. The first real
`tactics:brace` slice now crosses early idempotency, provider preparation, whole-bundle durability,
compact retry and fresh-engine typed-state hydration directly through the coordinator/store APIs.
It does not yet cross transport custody, legal offers, clock expiry, projection, or presentation.
That is the reason for using toy vertical slices rather than declaring the contracts finished from
their local precursors.

---

## 7. Implementation plan

Cuts use `K` (“kernel”) labels to avoid colliding with repository `M` milestones and economy `E`
milestones. Every cut must keep existing WoD/Shadowrun behavior and the current tactics tests green.
New paths are opt-in until a gate passes; there must never be two authoritative implementations of
the same migrated action.

```text
K0 integrity -> K1 definitions -> K2 logical transactions -> K3 durable bundles
                                                            |\
                                                            | +-> K5 legal views
                                                            +---> K4 clocks/RNG/effects
                                                                       |
                                                               K4 + K5 complete Gate 1
                                                                       |
                                                      K6 tactical/contact Gates 2-3
                                                                       |
                                                         K7 async decision Gate 4
                                                                       |
                                                  K8 full crash matrix Gate 5
```

The synthetic Gate-5 fault harness begins in K3 and is rerun/expanded at every later cut; it is not
deferred until the end.

### Execution sequence after the K3 durability audit

The remaining milestones are delivered as vertical proofs, not as independent piles of framework
types. Each phase leaves one executable path that the next phase extends:

1. **Durability closeout (K3.1e-f).** Run one authoritative conformance suite unchanged against
   memory and Redis. Preserve the complete canonical legacy batch—not only sealed Ledger members—on
   deterministic failure, and give authoritative, legacy, and recovery/control work independently
   bounded lanes with guaranteed service. Exit only when poison re-emission cannot wedge or starve
   unrelated authoritative persistence.
2. **Execution closeout (K2).** Execute the bounded causal trigger queue against the root scratch
   transaction and materialize one bundle or no mutation. Route transport-authenticated request UIDs
   into the coordinator, then prove claim/release/ownership convergence across dropped frames,
   gateway restart, and server kill. This is the sole new mutating ingress path.
3. **Mechanical Gate-1 slice (K4).** Add only the clocks, deterministic RNG, checked resource
   reservation, and temporary-effect lifecycle needed by `focus`, `brace`, and two-turn `guarded`.
   Prove spend + effect + expiry + receipt as one restart-safe causal history before generalizing the
   provider APIs.
4. **Projected Gate-1 slice (K5).** Expose that same action through one internal offer and one
   bounded projected DTO used by line, TUI/web, deterministic NPC, and mock LLM callers. Add
   keyframe/gap recovery and authorized action status. No interface receives a second authorization
   or execution implementation.
5. **First production migration.** Move one modest WoD resource/effect action through the complete
   K2-K5 path, compare its old/new semantics with golden tests, then remove its legacy mutation
   handler. Economy remains domain-model work until its Ledger append/seal and all mixed post-images
   can join one authoritative bundle.
6. **Tactical and asynchronous slices (K6-K7).** Extend the proven offer/transaction path to atomic
   source/target damage and hidden contacts, then make delayed NPC/LLM decisions return only opaque
   projected offers through ordinary coordinator re-entry.
7. **Production closure (K8).** Run the complete crash matrix against the toy and migrated WoD,
   Shadowrun, and tactics paths; publish compatibility rules only after all three exercise the same
   protocol.

Until K8 closes, new work follows four anti-debt rules: no new mutating legacy handlers, no direct
LLM mechanical mutation, no trigger side effect in a later commit, and no backend-specific
authoritative behavior without a shared conformance assertion. Existing legacy outcome/effect
construction remains compatibility debt and may not gain new unbounded item or payload shapes.

### K0. Integrity prerequisites and contract harness

**Purpose:** make the existing durability boundary safe enough to build the action kernel on it.

Work:

1. Ratify this document and create the `mush-contract-tests` workspace package with a minimal
   `ToyRulesModule`, deterministic definitions, and shared fixture builders.
2. Replace `spool_unresolved_count` with a typed `QuarantineSpool::open` performed before
   `boot_modules`, migrations, timers, or any startup write. Validate record version, SHA-256,
   filename identity, sealed payload, `.tmp` artifacts, and directory readability.
3. Require a durable spool implementation in persistent-server construction. Tests use an
   explicit in-memory spool; `None -> log and claim preserved` is removed.
4. Introduce `WorldMode` and central structural ingress classification. Test every `WorldMsg`,
   timer, hook, observer, input context, async apply, effect launch, and staff operation in
   `Booting`, `Recovering`, and `IntegrityFrozen`.
5. Add `Ledger::prepare_append`/`apply_prepared` and checked authoritative restore. Keep the current
   `append` temporarily deprecated for non-authoritative consumers.
6. Split current effects/output into `on_commit`, recoverable, durable-outbox, best-effort, and
   `on_abort` classes; never launch async effects before authoritative acknowledgement or emit
   success on deterministic error.
7. Make child-process integration tests set their own log filter rather than inheriting ambient
   `RUST_LOG`.

Exit criteria:

- a malformed/unreadable/partial quarantine store prevents all boot mutation;
- mutating ingress remains unattached/refused throughout every `Booting`/`Recovering` phase;
- every mutating ingress path is mechanically refused while integrity-frozen;
- an oversized or overflowing Ledger append changes nothing;
- deterministic write failure emits no success and launches no external effect;
- workspace tests and all-target clippy are green without ambient environment assumptions.

### K1. Shared identity, definitions, ownership, and lifecycle

**Contracts:** C1, C10, C11 foundation.

Work:

1. Add shared canonical IDs, `DefinitionRef`, `RulesValue`, mechanical lifecycle, and recovery reason
   types.
2. Add frozen definition and bridge builders with duplicate/digest/reference validation and hard
   size/count limits.
3. Add versioned migration registration, dry-run plans, and boot reports. K1 reads legacy through an
   explicit adapter and writes only the new form; it does not rewrite authoritative legacy bytes
   before K3 supplies exact-token durability.
4. Move new `mush-tactics` definitions and both tactics/sim rules-value shapes to the shared types.
   Tighten unit catalog full validation, canonical persistence keys, and behavior-binding
   reconciliation.
5. Make tactical boot freeze intact records with temporarily missing definitions/providers and
   quarantine malformed, unknown-schema, or non-migratable records instead of deleting/activating
   them.
6. Expose registry fingerprint and incident summaries through staff diagnostics.

Exit criteria:

- duplicate or conflicting definition registration fails composition deterministically;
- legacy tactical records decode/validate through an explicit v1 adapter while new records use v2;
- an exact dry-run migration plan is available but no in-place rewrite occurs before K3;
- a missing definition preserves but mechanically freezes its instance;
- rules owned by different providers cannot interact without one unambiguous bridge.

### K2. Logical action transaction, guards, and causality

**Contracts:** C2, C6, C8 foundation.

Work:

1. Add action invocation/resolution/prepared/receipt/error types and provider registration.
2. Add `RevisionSet`, registered revision validators, server-created `GuardSet`, and final
   revalidation immediately before atomically entering `CommitPending` and submitting the bundle.
3. Add `ActionTxn` scratch state and one-shot `PreparedTxn` materialization tying canonical durable
   bytes to owned infallible RAM install images.
4. Add bounded causal trigger expansion with deterministic ordering and duplicate suppression.
5. Add early idempotency lookup by namespace/key and canonical ingress hash before parsing, offer
   resolution, planning, or RNG. On a miss, parse without truth reads, pass the predecessor barrier,
   then resolve/bind the semantic invocation and request hash. Retain plan hashes only for pending-
   bundle integrity/audit.
6. Add `CommitPending`; reject retryable mutations, coalesce reconstructible work, and backpressure
   bounded must-retain mailboxes until the current result is exact.
7. Assign gateway requests stable UIDs; add explicit rejected/busy/owned/finished dispositions,
   bounded custody/backpressure, and ownership acknowledgement/replay across the split-process
   boundary.

Implementation status: item 1's shared identities/provider registry and item 3's state-record
transaction plus atomic typed boot hydration are implemented. Item 4 now executes the bounded
deterministic queue against that same root scratch transaction: each handler produces one ordered
derived semantic event, handler failure or any cascade bound discards the root scratch, and bundle
v2 plus compact receipt v3 retain the complete graph while continuing to read legacy bundle v1 and
receipt v2. Trigger dispatch is deliberately same-owner until K6's bridge-backed multi-owner
transaction proof. The
revision/authority registries, mandatory staged-action `GuardSet`, final synchronous guard
revalidation, and central `CommitPending` admission foundation cover parts of items 2 and 6. A
provider can now prepare the toy two-record action without mutable-world access, and the server can
persist those records on a compatibility lane and install/abort their typed images on the matching
process-local applied receipt. A provider that supplies a complete K3 bundle can instead stage it
only when its owner and replay writes exactly match those typed post-images; the server then retains
the images across FIFO reserve/reconcile and installs them only after an exact token/hash receipt.
The `action-state/*` keyspace is isolated from legacy module boot, migration, and server write paths.
The production coordinator now performs early exact-idempotency preflight and assembles the bounded
semantic causal/outcome/outbox portion of a complete bundle; the first `mush-tactics` provider uses
it directly. IPC version 2 wraps every split-mode command/query in an epoch-scoped stable UID;
gateway custody is independently bounded per connection and globally by count and logical bytes,
survives server restart, and replays in FIFO order only after the reattachment barrier. Queries end
with `EphemeralFinished`; legacy commands use the at-most-once `LegacyClaimed`/`GatewayReleased`
handoff, and both peers independently bound retained classifications. The store writer now reserves
independent bounded authoritative, legacy, and recovery/control lanes while retaining global
accepted-order execution. What remains at K2 is transport-authenticated entry into the coordinator
and the remaining deliberate frame-loss/process-kill matrix. Recovery-driven durable-outbox delivery is
implemented; viewer projection and keyframe recovery remain C9/C12 work.

Exit criteria:

- a two-record toy action can fail any validation with byte-identical pre-state afterward;
- changing any registered revision or authority guard makes a prepared action stale;
- trigger order is invariant under map/hash insertion order and bounds abort the whole action;
- identical retry after eligibility or unrelated state changes returns one result without planning;
- untrusted conflicting-key reuse is refused/audited, while an internal deterministic-key conflict
  is an incident;
- mixed query/kernel/legacy raw commands leave bounded gateway custody through the correct
  disposition; lost legacy claim/release frames cannot execute twice.

### K3. Exact-token authoritative bundle durability

**Contracts:** C2, C6, C8, C11; foundational portion of Gate 5.

**Implementation note (0.11):** the core canonical bundle/token/replay/outbox types and the separate
`AuthoritativeStore` contract now implement exact intent reservation, early idempotency lookup,
exact replay/readback/finalization, hash-aware status, bounded retained-intent capacity, object
allocator admission, a pending accelerator, and an independent bounded full-intent scan in memory.
The memory harness covers loss before/after intent, every materialization prefix (including object
before allocator), token conflict, and acknowledgement loss after finalization. Redis now has
checked Lua reservation and finalization barriers, immutable intent/idempotency/receipt records,
retention counters, pending-index and cursor discovery, and live-gated tests for exact retry,
missing accelerator membership, conflicts, and wrong-type partial execution. Retained intent reads
now require the exact canonical storage form, and exclusive boot atomically rebuilds the derivative
count/byte pair from exhaustive validated discovery, including over wrong-typed keys. The full
Redis suite and server recovery integration test pass against disposable Redis 8.10.1. Server startup now
connects without ordinary bootstrap writes, exhaustively reconciles every retained exact intent,
repairs stale pending membership, refuses conflicting or incomplete recovery, and only then creates
indexes, migrates, imports, or hydrates the world. The opt-in action lane now rejects a bundle whose
provider/scope/replay writes do not exactly match its prepared typed transaction, transfers the
complete bundle through FIFO writer custody, classifies pre-intent refusal separately from ambiguous
integrity failure, and validates the exact token/hash receipt before RAM installation. The unified
quarantine spool now keeps v1 Ledger records read-compatible while writing tagged v2 Ledger/bundle
records under one lock and capacity budget. Runtime authoritative poison is retried three times,
then the whole immutable bundle is durably dead-lettered and the writer integrity-freezes. Pre-boot
recovery performs exact token/hash status and pending reconciliation and archives a bundle only
after an exact finalized receipt. Finalized receipts are now compact, self-hashed semantic
witnesses (legacy full-bundle receipts remain read-compatible). The single writer and boot recovery
atomically checkpoint finalized actions without unresolved durable-outbox work, release their full
replay intents and capacity charges, and preserve exact token/idempotency retry through the compact
witness; the transition is idempotent after acknowledgement loss and is covered in memory and live
Redis. A production coordinator now makes early durable idempotency preflight a typestate boundary,
centrally derives tokens, normalized plan hashes, causality and complete bundles, and aborts typed
reservations on every pre-stage assembly failure. `mush-tactics` owns the first migrated provider:
`brace` writes a typed `stance` family only through that authoritative lane; exact retries return
the compact outcome without provider execution, and restart hydration is proven before a later
action reads revision 1 and prepares revision 2. Linux CI already supplies Redis 8 for the
live-gated suite. Compact witnesses now retire by durable per-scope epoch watermark: pending or
uncheckpointed work blocks advancement, deletion follows the watermark, and an interrupted purge
resumes without reopening an expired key. Finalization materializes an exact per-parent outbox
manifest; terminal records survive checkpointing and are removed with their retired parent. A
bounded production worker rediscovers finalized outbox items, persists the attempt before delivery,
reuses stable item IDs for ambiguous send/ack loss, records retry or terminal state, and checkpoints
the parent only after all items are terminal. Missing adapters retain work and cannot starve
installed adapters. The remaining cross-cut work includes transport/parser/legal-offer entry into
the coordinator, the remaining gateway ownership process/IPC fault cases, recoverable
projection/keyframes, and broader provider/legacy migration. Those depend
respectively on unfinished K2 trigger/transport work and K5/C9/C12 projection. K3.1 is closed for
bounded accepted work; K0's explicitly documented legacy pre-mutation construction gap is retired
incrementally by migrating handlers onto the bounded authoritative path.

#### K3.1. Durability-audit closure (closed for bounded accepted work)

Post-implementation memory-store probes found two correctness violations and several liveness,
parity, and observability gaps. They are tracked here rather than hidden inside later economy or
Gate-5 work:

1. **K3.1a — checkpoint proof (closed):** memory and Redis atomically verify every exact replay
   post-image before deleting the full intent. A mismatch retains the intent and fails closed.
2. **K3.1b — store-owned admission (closed):** the store enforces the initial single unresolved
   authoritative reservation bound (or a future equivalent per-target claim protocol), so two
   pending bundles cannot claim the same fresh object ID.
3. **K3.1c — poison release valve (closed):** an audited exact token/hash/evidence transition is
   legal only after the unified quarantine spool durably proves custody. It releases only a
   never-finalized full intent, keeps a non-replayable idempotency tombstone, and remains recoverable:
   a later boot can atomically restore the exact spooled bundle, retry it, and rediscard it if the
   deterministic fault remains. Finalized intents can never use this discard path.
4. **K3.1d — unroutable outbox policy (closed):** missing adapters advance a separate durable,
   bounded discovery-pass counter without consuming real delivery attempts, persist a terminal
   dead-letter outcome with diagnostics, and then permit parent checkpoint/epoch progress. Installed
   adapters must remain independently dispatchable.
5. **K3.1e — Redis concurrency/parity (closed):** Redis-gated tests are explicit ignored
   tests locally and an explicit Linux CI job, so skipped Lua coverage cannot count as passing.
   Retention-counter repair now uses the generation captured before exhaustive scanning, and the
   complete-bundle recovery lookup reports `Retired` from the scope watermark after compact records
   are purged. One shared suite now proves exact lifecycle/discovery, store-owned admission,
   quarantine restore, durable outbox, epoch retirement, conflict handling, and retention-generation
   CAS semantics unchanged against memory and Redis; backend-specific corruption probes remain in
   their native harnesses.
6. **K3.1f — K0/K3 integration (closed for bounded accepted work):** deterministic E0
   drops now preserve the complete canonical legacy batch in the unified quarantine spool before
   seal isolation, including object post-images, deletes, module operations, allocator state, and
   original FIFO serial. Boot recovery replays only provably idempotent absolute batches in serial
   order; ambiguous stream appends remain active and fail closed. Retained poison-seal re-emission
   reuses exact existing custody rather than creating serial-dependent duplicate records.
   Authoritative, legacy, and recovery/control work now have separate admission capacities and
   gauges; accepted jobs carry their lane permit through merge buffering and completion. A global
   acceptance sequence preserves cross-lane FIFO, while the finite legacy capacity bounds how much
   older seal traffic can precede newly accepted authoritative/control work. Boot recovery runs
   before these runtime lanes exist. This closure inherits K0's explicitly documented pre-mutation
   construction gap: an outcome too large for any finite quarantine budget must be refused before
   live installation as legacy handlers are migrated, never "fixed" by an unbounded spool.

Economy obligations/contracts remain on their legacy durable path until mixed authoritative write
sets can include the Ledger append, modules can read their own typed families outside an action, and
the Ledger seal participates in the same bundle instead of blocking it. That migration is a later
provider milestone, not part of K3.1's kernel repair.

Work:

1. Add `AuthoritativeBundle`, normalized replay-write and allocator validation, `CommitToken`, exact
   intent/authoritative receipts, and reserve/reconcile/hash-aware status methods to store
   interfaces.
2. Implement the in-memory store contract and fault modes first.
3. Implement Redis atomic intent/idempotency reservation, exhaustive paged boot discovery using the
   pending index plus cursor scan, idempotent materialization, compare-and-finalize readback,
   finalized witness, and conflict detection.
4. Retain/quarantine the entire bundle on deterministic poison. Track encoded bytes and count;
   never clear a mutable bundle through the sealed-chunk global high-water shortcut.
5. Reconcile pending bundles before loading any authoritative world/object/module snapshot (or
   discard and reload a snapshot already read). Install prepared RAM state and publish
   post-commit work only after exact success. Materialize a separate durable-outbox index/state,
   recover it at boot, and dispatch with stable item IDs and at-least-once semantics.
6. Add authorized semantic `action/status`; separate bounded presentation retention from compact
   idempotency tombstones with an explicit retry horizon/checkpoint rule.
7. Register persistence classes/prefixes, enforce the predecessor durability barrier, and reject
   legacy writes once a prefix migrates to authoritative ownership.
8. Execute one K1 migration through the bundle protocol and prove restart on v2; never mutate raw v1
   state before the durable intent exists.
9. Document/test applied versus host-power-loss receipt policy and validate production Redis AOF
   configuration when a stronger policy is selected.

Exit criteria:

- the synthetic Gate-5 fault matrix converges for in-memory store;
- every partial materialization has a complete exact pending intent; live Redis wrong-type partial
  execution is healed or freezes with that bundle record;
- IPC never transfers request ownership before intent reservation, and a lost ownership ack
  converges by replaying the same request UID;
- acknowledgement loss cannot duplicate state or rerun RNG;
- restart after commit/before RAM install loads the committed post-state;
- restart after ack/before output recovers the receipt and keyframe;
- an action-created object and allocator restart without ID reuse;
- legacy pre-state dependencies cannot be lost beneath a later committed action.

#### E7-enabling priority cut (K4a/K5a)

E7's first production/requisition cuts must not mint temporary clock or control models.  Pull these
two kernel primitives forward before finishing the rest of K4/K5:

1. **K4a — versioned world timeline (implemented):** `ClockRef`/`ClockStamp`, a bounded
   append-only `{ real_anchor, ic_anchor, rate, generation }` epoch history, continuous rate
   changes, immutable generation documents plus an atomically updated head, fail-closed boot
   validation, configuration-as-genesis-only behavior, and synchronous `Ctx::world_ic_now()`.
   Campaign policy decides who may open a generation; the kernel only enforces shape, continuity,
   generation, bounds, persistence, and comparison rules.
2. **K5a — station channel grants (implemented):** project one
   `ChannelGrant { actor, subject, owner, station, channels, priority, epoch }` per occupied station.
   Command resolution selects priority from the same grant that supplies the required channel, and
   delayed intents can revalidate the exact station/channel/epoch. The existing persisted unit
   `control_gen` is a conservative grant epoch: any control-surface change invalidates every grant
   on that unit. Per-channel active leases and safe-boundary transfer remain K6 work.

The K3.1 checkpoint-readback and pending-allocator findings are already closed in both memory and
Redis and therefore are not prerequisites. Mixed authoritative write sets, synchronous typed-family
module reads, and folding a Ledger seal into an authoritative bundle remain the migration gate
before E7 reservations or production settlement move from the economy journal onto kernel
authority; they do not block E7 records and policy from using the two primitives above.

### K4. Typed clocks, resolution RNG, resources, and effects

**Contracts:** C4, C5, C7; **passes Gate 1 mechanically**.

Work:

1. Add `ClockRef`, `ClockStamp`, provider registration, generation validation, typed deadlines,
   restart policy, and bounded catch-up.
2. Add `ActionWindow` with durable idempotent simultaneous acceptance/replacement/cancellation,
   close/consume state, and deterministic resolution order.
3. Add `ResolutionRng`, durable root-secret/version handling, domain-separated streams, draw caps,
   and structured resolution records; adapt `mush-dice` to it.
4. Add checked scalar operations, reservation state machine, and resource/effect provider contracts
   without moving existing owner truth prematurely.
5. Implement toy `focus`, `brace`, and two-turn `guarded` definitions/actions/expiry.
6. Run Gate 1 through idempotency and restart cases.

Exit criteria:

- Gate 1 state, causal events, RNG record (if used), expiry, and receipt are atomic and restart-safe;
- same idempotency key produces the same draws/outcome;
- restart preserves open and durably closed simultaneous intent sets without double resolution;
- reservation contention/consume/release/expiry is checked and restart-safe;
- incomparable clocks cannot compile through typed APIs or pass runtime deserialization;
- no authoritative provider action module can access ambient RNG.

### K5. Legal actions, projection, and semantic views

**Contracts:** C3, C9, C12; **completes Gate 1 on every interface**.

Work:

1. Add separate internal/projected action offers, parameter/target schemas, bounded enumeration,
   restart-stable HMAC offer bindings, durable secret rotation, and provider routing.
2. Route a toy line command, panel/TUI control, deterministic NPC choice, and mock LLM tool through
   the same offer and prepare callback.
3. Add `ViewEnvelope`, versioned keyframes/deltas, base sequence, schema/generation/gap detection,
   projected receipt frames, authorized status, and resync query.
4. Register line renderers over projected DTOs; structured transports use the same DTOs.
5. Add leak-test helpers that recursively inspect all player/AI-facing serializations and text.
6. Migrate the Gate-1 action from any temporary direct handler to action offers only.

Exit criteria:

- all four callers receive equivalent action semantics and cannot submit an unoffered action;
- legal actions revalidate rather than authorize stale execution;
- an unchanged pre-restart offer resolves after restart while a changed binding returns projected
  stale, and no internal guard/revision/hash is serialized;
- dropped/out-of-order delta forces a correct keyframe;
- Gate 1 presents consistently over line, TUI-shaped, web-shaped, and LLM snapshots.

### K6. Tactical source/target action and hidden contacts

**Contracts:** C1-C10 and C12; **passes Gates 2 and 3**.

Work:

1. Add component-provided tactical action definitions and a tactical action provider.
2. Add durable per-channel control leases/epochs, channel-specific command priority, and lease
   handoff at a declared domain/scene boundary.
3. Prepare source and target unit post-states plus placement/window/RNG/event guards in one action.
4. Add tactical `ContactId` ownership, observer/team sharing scopes, generations, private truth
   mapping, and contact lifecycle.
5. Add projected unit, section, component, contact, legal-action, and outcome DTOs. Stop serializing
   full tactical truth to ordinary viewers.
6. Exercise disabled components, lost contacts, durable simultaneous fire, and a toy bridge's
   unique/missing/ambiguous selection plus validation by both owners.

Exit criteria:

- Gates 2 and 3 pass, including recursive leak checks;
- source and target never land on opposite sides of an action;
- lease loss or any source/target/contact revision change invalidates the plan;
- bridge selection is unique, both owners validate, and cross-owner projection leaks no private
  state;
- component removal removes the action from every interface.

### K7. Asynchronous decision re-entry

**Contracts:** C2-C6, C8, C9, C11; **passes Gate 4**.

Work:

1. Define durable server-side `PendingDecision` with internal offer/guards/deadline/cause/key and a
   separate LLM-facing `DecisionPrompt` containing projected legal actions and an opaque token.
2. Make NPC/LLM tasks hold no world references or guards and return only an offered action ID,
   perceived targets, bounded parameters, and the decision token.
3. Re-enter through the ordinary action invocation service; remove bespoke mechanical LLM ability
   mutation paths as representative actions migrate.
4. Add per-controller cancellation/coalescing, deadlines, malformed-output limits, and deterministic
   fallback independent of provider availability.
5. Run Gate 4 for every guard dimension and duplicate/out-of-order completion.

Exit criteria:

- Gate 4 passes with zero mutation/draw/event on every stale case;
- a valid delayed result is indistinguishable from a human invocation after ingress;
- restart preserves bounded pending decisions without exposing or accepting returned guard data;
- provider latency changes neither initiative ordering nor authority;
- provider outage cannot prevent deterministic fallback control.

### K8. Full restart matrix and production migrations

**Contracts:** all; **passes Gate 5 using Gates 1-4 actions**.

Work:

1. Run every Gate-5 cut point against each toy vertical slice, not only a synthetic bundle.
2. Add process-kill tests with the split gateway/server and a disposable Redis database; include
   Ledger rollover and quarantine-spool interruption.
3. Add structured-client token dedupe/ack and line `action/status`/keyframe recovery journeys.
4. Migrate one existing WoD resource/roll action and one existing Shadowrun action through the
   kernel. Compare old/new semantic outcomes with golden tests, then delete the old mutation path.
5. Publish compatibility rules for modules and scripts. Do not promise a stable third-party ABI
   until two setting rulesets plus tactics have exercised the contract.
6. Add CI jobs for contract tests, Redis fault tests, definition/wire fixtures, and direct-rand/
   truth-serialization policy checks.

Exit criteria:

- all five gates and the full crash matrix pass in CI-appropriate and Redis-gated suites;
- representative WoD, Shadowrun, and tactics actions share one coordinator;
- no action can emit pre-ack success or launch a pre-ack external effect;
- boot reaches writable mode only after definitions, recovery, bundles, indexes, and schedules
  validate;
- content expansion may proceed without adding a second execution protocol.

---

## 8. Required tests by layer

### 8.1 Pure/property tests

- canonical IDs and domain-separated definition/request/plan/bundle hash stability, including
  equivalent JSON, numeric normalization, ordering, and cross-restart fixtures;
- duplicate/conflicting definitions and bridge ambiguity;
- revision-set ordering and stale detection;
- checked scalar/sequence/chunk arithmetic;
- clock comparability, window boundary, pause, and catch-up behavior;
- RNG stream separation and retry identity;
- reservation arithmetic, lifecycle transitions, expiry, and holder identity;
- trigger ordering, deduplication, depth/operation/byte caps;
- projection DTOs cannot deserialize as truth types;
- prepared-record serialization and install-image fixtures remain stable.

### 8.2 In-process integration tests

- every gate through `TestWorld` plus the real action coordinator;
- refusal means no dirty objects, module ops, causal events, RNG consumption, or after-commit work;
- one action touching multiple module states installs all or none;
- world modes classify every message/timer/effect path, including prolonged `CommitPending`
  saturation without unbounded queues;
- missing owners/definitions/bridges and failed behavior attachment preserve the specified frozen
  or quarantine state and raw record;
- retry after self-invalidating and unrelated mutations bypasses offer/planning/RNG;
- restart-stable opaque offers and private server-held pending-decision guards;
- every raw command receives a query/kernel/legacy disposition; lost legacy claim/release frames
  cannot execute before release or more than once;
- keyframe/delta schema/generation/gap recovery and viewer-specific contact maps;
- event-time witness eligibility remains stable across post-commit movement/connectivity changes.

### 8.3 Store and crash tests

- complete in-memory `FaultStore` matrix;
- Redis intent-CAS refusal, execution-time wrong type, acknowledgement loss, reconnect, and token/
  hash conflict;
- process termination at all Gate-5 cut points;
- IPC crash on both sides of request-ownership acknowledgement, lost refusal response, and bounded
  gateway-custody saturation;
- every partial materialization has an exact complete pending intent and boot discovery finds it;
- boot discovery remains recovering through an unindexed intent beyond the first cursor page;
- object-plus-allocator recovery never reuses an ID;
- epoch compaction/checkpoint failure and retry below the minimum accepted epoch;
- durable-outbox post-finalize boot rediscovery, duplicate send/ack loss, and terminal delivered/
  dead-letter state;
- Redis process/host durability tests separately gated by the configured AOF receipt policy;
- quarantine file partial write, rename, directory-fsync failure, malformed file, permission denial,
  and restart;
- repeated recovery is idempotent and never auto-unfreezes an unresolved incident.

### 8.4 Information-flow tests

- two viewers receive distinct contact projections;
- target loss, concealment, and nonexistence are indistinguishable when required;
- no hidden truth in action offers, errors, text renderer output, structured frames, audit surfaces
  available to players, hashes, status queries, or LLM prompts;
- guessed action/idempotency/decision/offer tokens neither reveal existence nor authorize lookup;
- staff truth projection requires explicit capability and produces an audit event;
- delta and action availability changes do not act as forbidden side channels.

---

## 9. Bounds and observability

All user-, ruleset-, script-, store-, and model-influenced collections require explicit limits.
The initial configuration surface should include:

- definitions per owner and encoded definition bytes;
- legal actions/targets and parameter-schema bytes per projection;
- plan fragments, operations, events, trigger depth, and total action bytes;
- revision and authority guards per action;
- RNG draws and recorded bytes per resolution;
- resources/effects per subject;
- reservations per resource/holder and maximum hold duration;
- simultaneous intents per window;
- pending decisions per controller;
- gateway pre-intent custody per connection/global by count and bytes, plus oldest-age SLA/alert;
- authoritative pending-intent/recovery count and bytes;
- must-retain mailboxes and durable-outbox attempts/age;
- idempotency horizon/checkpoint evidence, presentation receipts, and view-delta history.

Metrics and staff diagnostics include:

- action requests by provider/outcome (`committed`, `refused`, `stale`, `invalid`, `frozen`);
- prepare, commit-pending, reconciliation, and after-commit latency;
- guard failures by kind without player-secret labels;
- idempotent replays and conflicting-key incidents;
- RNG draws, trigger depth/ops, and bound refusals;
- bundle status, pending reconciliation, quarantine count/bytes, and world mode;
- gateway custody depth/bytes/age, ownership-ack loss/replay, and input backpressure;
- commit-pending refusals/coalesces/mailbox pressure and oldest durable-outbox age;
- projection/keyframe/delta counts and resyncs;
- asynchronous decision latency, staleness, cancellation, and fallback.

Logs carry commit/action/incident IDs and owners. Player-hidden truth is confined to protected staff
audit records, never ordinary application log fields.

---

## 10. Deferred work and explicit non-shortcuts

Deferred until the five gates create evidence that it is needed:

- pipelining multiple authoritative bundles or general MVCC;
- a universal effect-modifier language;
- event-sourcing the entire `World`;
- public dynamic/native rules plugins;
- cross-world/distributed transactions;
- cryptographic public commit/reveal dice;
- one universal clock or initiative algorithm;
- automatic migration across missing definition versions;
- exactly-once raw terminal text;
- broad catalogs of tactical, economic, or character content.

The implementation should remain boring at the boundary: exact IDs, immutable versions, checked
integers, bounded collections, prepared post-images, one commit token, explicit guards, and
observer-specific DTOs. Complexity belongs in ruleset formulas after the five gates prove that the
boundary can contain it.
