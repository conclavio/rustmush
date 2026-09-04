# E7 implementation plan — requisition and production vertical slice

**Status:** Draft 0.3, 2026-09-04. The two early kernel dependencies are pulled forward as K4a/K5a
in doc 09 and implemented on the K branch. K4b's checked scalar, reservation-record, and typed
effect-instance primitives are also implemented and exercised by the restart-safe Gate-1 toy.
This plan deliberately keeps E7's first journal-backed records on the existing economy durability
path; it does not pretend the remaining kernel migration gates are complete.

## 1. Outcome

Prove one generic production flow through the whole game server:

1. an organization member requests a ruleset-defined item or unit from a real facility;
2. deterministic policy prices it and checks a division budget, material vault, facility capacity,
   destination, and approval threshold;
3. accepted value/material/capacity reservations survive restart and make the work unavailable to
   competing orders;
4. production advances on a typed clock without teleporting through interruptions or rate changes;
5. completion creates or modifies the output, assigns title/maintenance/custody, and places it at
   the declared destination without a partially usable intermediate state;
6. line SSH and one structured panel expose the same legal actions and projected state;
7. duplicate input, duplicate scheduler fire, restart, cancellation, capture, and stale async work
   converge without double spend, double output, or leaked private records.

The foundation is rules-neutral. A ruleset supplies recipes, names, costs, durations, requirements,
and output builders. Nothing in the engine assumes mechs, tanks, factories, nuyen, C-bills, or a
particular campaign economy.

## 2. Dependency decisions

### 2.1 Pull forward now

- **K4a versioned world timeline:** shipped shape is an append-only sequence of
  `{ real_anchor, ic_anchor, rate, generation }`, strict `ClockRef`/`ClockStamp`, atomic epoch/head
  persistence, boot hydration before modules, and synchronous module-context reads. Configuration
  seeds generation one only; persisted truth wins thereafter. Exact module-owned registered/domain
  clocks now use the composition-frozen provider registry: the provider validates the requested
  generation and returns only its scalar value, while the kernel owns stamp identity and readiness.
  The durable schedule cursor now supplies bounded oldest/latest catch-up, explicit skipped ranges,
  resume/skip/suspend restart transitions, and deterministic same-owner coordinator ingress.
  Composition-frozen sources now bind exact owner-local schedule families to exact actions and
  support bounded canonical restart/live discovery directly from hydrated typed state. A
  provider-specific read-only adapter builds domain addressing and guards only after vacant store
  preflight; the kernel re-evaluates and advances the exact cursor in the provider transaction.
  The production world loop drives one candidate at a time through the K3 coordinator, preserving
  legacy/retained predecessors, retrying transient failures, and rebasing retired server
  idempotency epochs without confusing them with gameplay schedule generations. E7 can register a
  production schedule source directly; the scene-clock adapter is still outstanding.
- **K5a station channel grants:** one
  `ChannelGrant { actor, subject, owner, station, channels, priority, epoch }` per occupied station.
  Channel and priority are selected from the same grant; no cross-seat union/max promotion. Exact
  station/channel/epoch revalidation is available to delayed work.

These primitives unblock E7 record design and early gameplay without waiting for every K4/K5
feature.

K5's bounded internal/projected offer types, signed server-held bindings, crash-safe durable private
key provisioning/rotation, canonical registered-provider enumeration, opaque resolution, and
Gate-1 coordinator seam are now implemented. The next enabling slice is the first public line/panel
routes through that boundary, followed by E7a's recipe/facility/work-order definitions.
E7b may use the existing economy journal for its first real holds, but it must use K4b's
state-machine semantics and typed expiry shape so the later kernel migration is a storage-boundary
change rather than a model rewrite.

### 2.2 Already satisfied

- K3 checkpointing re-reads exact post-images before releasing a full intent.
- K3 allocator admission accounts for unresolved intent custody by refusing a second pending
  authoritative intent.
- E3 separates title, custody, physical location, operation, and maintenance.
- Organizations are first-class principals with scoped roles/capabilities, wallets, vaults, and
  audit history.
- Sim/tactics already provide generic unit, placement, station, channel, and control-generation
  foundations.

### 2.3 Does not block the first E7 slice, but gates kernel migration

Before production reservations or settlement move from the economy journal to authoritative K3
state families, all three must land:

1. an authoritative bundle can carry the mixed owner write set, including the Ledger append;
2. ordinary module code can synchronously read its typed state families outside an action;
3. the Ledger seal is folded into the same exact bundle rather than acting as an external
   predecessor barrier.

Until then, the economy journal remains authority and E7 conventional documents are projections or
workflow records. Every record is shaped for a mechanical migration: canonical whole-document
post-images, one mutation per key per transition, explicit revisions, deterministic idempotency
keys, and no hidden side effects after the final fallible preflight.

### 2.4 Still required before E7 control handoff

K5a grants establish eligibility and stale-intent witnesses; they are not active exclusive leases.
Direct/proxy control changes require K6's per-channel lease record and safe-boundary handoff. The
first production slice may assign title, maintenance, and custody, but it must not silently evict a
live operator or transfer an active movement/weapons lease.

## 3. Engine and ruleset boundary

The engine enforces:

- bounded canonical IDs, counts, byte sizes, quantities, and checked arithmetic;
- work-order revision and idempotent transition shape;
- typed clock identity/generation and refusal to compare incomparable stamps;
- reservation holder, amount, revision, expiry, and state-machine invariants;
- exact authorization witnesses and final stale-state revalidation;
- atomic persistence/ack custody, restart hydration, deterministic timer keys, and causal events;
- destination existence and owner-adapter participation;
- projection boundaries and opaque offer/decision IDs.

Rulesets/campaign modules provide:

- recipe catalogs and immutable recipe versions;
- currencies/instruments, baseline costs, material kinds, quantities, and substitutions;
- eligible facility kinds, skills, tooling, quality, duration, and interruption policy;
- approval policy and which organization scopes may request, start, cancel, reprioritize, or claim;
- output construction/refit/repair logic and tactical definitions;
- clock choice and explicit response to a world-timeline generation change;
- player-facing names, descriptions, help, and projected detail.

LLMs may explain options or select one currently offered action. They never invent a recipe,
reservation, price, facility capability, destination, grant, or completion result.

## 4. Core models

Illustrative Rust shapes; exact field types reuse the canonical K/economy/org IDs.

```rust
struct RecipeRef {
    owner: OwnerId,
    id: CanonicalId,
    version: u32,
    digest: DefinitionDigest,
}

struct FacilityRecord {
    object: ObjId,
    owner: PrincipalId,
    capabilities: BTreeSet<CanonicalId>,
    capacity_revision: u64,
    policy_revision: u64,
    active_slots: u32,
    maximum_slots: u32,
}

struct WorkOrder {
    id: WorkOrderId,
    revision: u64,
    requester: PrincipalId,
    beneficiary: PrincipalId,
    facility: ObjId,
    recipe: RecipeRef,
    quantity: u32,
    budget_account: AccountId,
    material_source: Option<AssetContainerRef>,
    destination: DeliveryTarget,
    reservations: Vec<ReservationId>,
    schedule: Option<ProductionSchedule>,
    state: WorkOrderState,
    visibility: EconomicVisibility,
}

struct ProductionSchedule {
    started: ClockStamp,
    due: ClockStamp,
    clock_policy: CanonicalId,
    progress_units: u64,
    total_units: u64,
}

enum WorkOrderState {
    Draft,
    AwaitingApproval { request: ApprovalRequestId },
    Ready,
    Running,
    Interrupted { cause: InterruptionCause },
    AwaitingDelivery,
    Completed { commit: CommitToken },
    Cancelled,
    Failed,
}
```

Reservations remain separate records, not embedded counters. At minimum an accepted order holds:

- money/requisition value;
- each conserved material input;
- one facility slot or measured capacity;
- any unique source asset being repaired/refit;
- an optional bounded destination capacity claim.

Every reservation uses the work order's deterministic holder key. Completion consumes all required
holds in the same transition that makes output available; cancellation/rejection releases them
idempotently. A failed delivery may keep a completed output in `AwaitingDelivery`, but may not
re-run production or consume the inputs twice.

## 5. Clock semantics

Starting work captures `started` and `due` on one exact clock generation. Scheduler comparisons are
legal only while the current provider returns that same identity. A world-time rate change never
silently reinterprets an old deadline.

On a generation mismatch the production provider performs one explicit campaign-selected action:

- restamp while preserving remaining IC duration;
- restamp while preserving the previously computed real due instant;
- pause as `Interrupted { TimelineChanged }` pending campaign resolution.

The first slice uses the third, safest policy unless the campaign module registers one of the first
two. The restamp is a durable attributed work-order transition. Merely observing a new generation
never edits the order in place.

## 6. Control and ownership semantics

- Title transfer does not move the unit, assign crew, or grant control.
- Custody/destination does not imply title or a station assignment.
- A station assignment projects one K5a grant per seat and carries the current control epoch.
- Starting, cancelling, repairing, claiming, or delivering a work order revalidates organization
  authority independently from vehicle control.
- Evicting an operator or changing an active lease is a separate legal action. It captures the
  exact grant and lease epoch and commits only at the declared scene/domain safe boundary.
- A proxy seat receives only its granted channels. Revocation bumps the control epoch; queued human,
  NPC, pathfinder, or LLM intents issued under the old grant fail stale.

## 7. First vertical scenario

Use a generic `tactics.scout_vehicle@1` recipe at a two-slot motor-pool facility:

1. A division member with requisition permission opens a draft for one vehicle, selecting the
   division wallet, material vault, motor pool, and hangar destination.
2. The server returns deterministic price/duration/requirements and an opaque legal offer.
3. Submission revalidates membership, role scope, account/vault authority, recipe digest, facility
   revision, destination capacity, and offer binding.
4. Below-threshold requests reserve and start. Above-threshold requests enter existing org approval
   and reserve only after approval.
5. A duplicate submit joins the same order; a competing order cannot spend the held funds/material
   or overbook the second slot.
6. Restart during `Running` hydrates the same holds, schedule generation, progress, and timer key.
7. Destroying/losing facility access interrupts rather than completing remotely. A world-timeline
   generation change pauses or explicitly restamps according to registered campaign policy.
8. Completion consumes holds once, constructs one generic tactical unit, assigns title and
   maintenance liability, places it in the hangar, and emits one causal completion event.
9. A designated operator may then receive a station assignment/grant; title alone grants none.
10. Line SSH and a structured panel show equivalent legal transitions and projected status.

## 8. Milestones

### E7a — definitions, facilities, and drafts

- Register versioned recipe/facility definition kinds and validation.
- Add bounded facility/work-order records, indexes, revisions, and boot quarantine.
- Add create/edit/submit/cancel legal actions; draft mutation has no economic effect.
- Project a private line view and structured DTO from the same internal view.

**Gate:** recipe/facility drift makes a draft stale; malformed or oversized records fail closed;
hidden org/division/account facts do not leak.

### E7b — authorization and reservation

- Resolve org capabilities, division wallet/vault access, and approval thresholds.
- Acquire money, material, source-asset, and facility-capacity holds under one deterministic order
  key; release/replace idempotently.
- Start only after every hold and final revision witness is current.

**Gate:** contention, duplicate submission, approval replay, cancellation, and restart never
overspend or overbook.

### E7c — typed progress and interruption

- Drive progress from `ClockStamp`, bounded catch-up, and deterministic scheduler keys.
- Detect clock-generation, facility, recipe, source asset, and policy revision changes.
- Implement pause/resume/restamp/cancel as explicit causal transitions.

**Gate:** duplicate/out-of-order timer fires and large time jumps converge; a timeline change never
silently changes a due date.

### E7d — output, delivery, and recovery

- Add ruleset output builders for create, repair, and refit.
- Consume reservations and materialize title/maintenance/custody/placement/output truth without a
  usable partial output.
- Make delivery failure recoverable as `AwaitingDelivery`; add capture, salvage, and write-off
  transitions as compensating/audited operations.

**Gate:** every injected cut point either retains the pre-state or converges to exactly one output;
allocator restart cannot reuse its object ID.

### E7e — offers and interfaces

- Bind every mutation to K5 legal offers and exact projection generation.
- Add line commands and one web/TUI-shaped panel using the same DTOs.
- Add deterministic NPC use; LLM use selects only opaque offered actions.

**Gate:** stale offer/grant/lease completion performs no spend, draw, progress, event, or output;
recursive leak tests pass.

### E7k — authoritative-kernel migration

After the three §2.3 gates land, migrate reservations, work-order transitions, Ledger append, and
cross-owner output materialization into complete K3 bundles. Remove the conventional handler only
after memory/Redis restart and acknowledgement-loss matrices prove equivalence.

## 9. Review checklist

- No command or timer can confirm before its durable acknowledgement.
- No `?` or other fallible operation remains after the first irreversible live mutation.
- Every retryable entry has a bounded scoped idempotency key and semantic request hash.
- Holds subtract from availability and facility capacity; embedded advisory counters are not
  authority.
- Rate changes create generations; old facts retain old generations.
- Channels and priorities are never unioned across station grants.
- Title/custody/location/station/grant/lease/maintenance remain separate facts.
- Completion cannot expose an object before all required owner projections agree.
- All lists, catch-up loops, recipes, inputs, outputs, reservations, events, and DTO bytes are
  bounded.
- Staff intervention is the last resort after deterministic reconciliation and scoped recovery have
  been exhausted; any page includes the exact order, failed invariant, and recommended runbook.
