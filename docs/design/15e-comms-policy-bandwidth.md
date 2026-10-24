# Communication policy and bandwidth follow-up

Authorized scope: retrospective **RR-5 through RR-7**, after availability/privacy
commit `749e0f9808171314941f43fabec7792718b822e9`. Implement, integration test on
conclav.io dev1, commit/push to origin/main, then pause. No additional consumer gate
is included. This document reconciles design 15's original hub wording.

## Admission and physical capacity

Each traversed pair retains its own bounded queue and shared bidirectional physical
capacity. There is no aggregate Luna bucket. An envelope pays Standard/Deferred
admission delay once per distinct operator, at the first serialized unit for that
operator. Later spokes waive that delay, including after a route replan. Different
operators impose their own first-link admission. The history is bounded by the
existing four-hop ceiling; reaching the bound refuses another operator rather than
silently forgetting admission. One Exchange retail purchase still covers all
Exchange hops; price, payer and class contracts are unchanged.

A Deferred window remains a physical-link scheduling constraint on every serialization
pass, including subsequent spokes. Class/FIFO rank and aging govern eligible work.
An eligible partial message can consume the remaining token budget and continues in
that rank on later passes. Closed windows, jammed destinations and stale tickets do
not block other eligible work. This replaces the old "skip a message that cannot
fit this second" rule, which would starve messages on narrow hardware.

The route planner remains a bounded physical-path planner, not an ETA optimizer.
It filters each pair's maximum message size and access policy. It may select another
pair with conventional ingress/egress when one pair's size cap excludes a message.
It does not price routes or predict future queue load.

## Mutable, attributed per-link schedules

`mush_propagation::schedule::set` is the checked typed ingress. It requires current
pair discovery and `relay.manage`, an expected revision, validated `Settings` and
a plain reason. The command adapter is `+relay/schedule pair=key:value,...;reason`;
`expect:N` supplies an optional compare-and-set revision. Without settings the
command reads the observer-safe current schedule. The public board includes it.
A scheduling mind can later call the same API; no new mind is introduced here.

| Setting | Bound / seeded behavior |
|---|---|
| Standard admission | 0–86,400 IC seconds; seed 60 |
| Deferred admission | 0–604,800 IC seconds; seed 7,200 |
| Deferred period | 1–604,800 IC seconds; seed 86,400 |
| Window start/duration | nonnegative start, positive duration, entirely within period; seed 0/3,600 |
| Aging interval | 1–86,400 IC seconds; seed 300 |
| Maximum accounted message bytes | 128–1,024, multiples of 128; seed 1,024 |
| Refill rate | 1–1,000,000 units/IC second, at most installed physical capacity |
| Burst | 1–1,000,000 units; seed minimum of legacy burst and physical rate |

Each update records revision, actor, IC time, reason and complete settings. The
bounded audit retains the same 128-row limit, with up to 512 bytes per action to
include those values. A single transport post-image contains the revision, audit
and reconfigured capacity, and acknowledgement waits for durable commit. Invalid,
unauthorized or stale updates install nothing. There is at most one current
schedule per pair (the registry's existing bound).

Admission tickets snapshot revision, operator, delay/window/aging/size settings and
whether the operator was previously admitted. Schedule edits preserve accepted
tickets. A newly issued ticket at the next pair uses that pair's current settings,
with admission waived if that operator is already recorded. Live authority, receiver
throttling and billing restrictions still revalidate; changes in effective class
use the frozen ticket's settings. This does not freeze authorization forever.

## Token capacity and serialization

One unit remains each started 128 accounted content bytes. Message size is independent
of rate and burst: a seven-unit message can use a one-unit/second, one-unit-burst
pair. Public links seed eight/eight; manufactured links seed one/one.

A token bucket persists its timestamp, spent-at-that-time counter and remaining
balance. Refill is elapsed whole IC seconds times rate, capped at burst. Repeated
passes or restarts at the same IC time do not replenish it. Clock rollback gives
no capacity. Policy changes first materialize under the old rate/burst, then clamp
to the new burst, so raising a limit or toggling settings cannot mint tokens. Current
rate/burst govern physical capacity, including work with older accepted tickets.

Partial progress persists total units, units sent, first-unit time and last-unit
time with the envelope and capacity. First-unit payment capture uses the existing
strict durable commerce batch; failed capture installs neither capacity nor progress.
No completed hop, service receipt or recipient payload is exposed until all units
for that pair finish. Final receipt records both serialization start and completion.
The ordinary remaining route and source-owner delivery gate still govern arrival.
A partially consumed courtesy copy is no longer purchasable, including after replan.

Destination changes, revoked authority/coverage or registry revisions can replan a
route and discard unfinished pair serialization. Spent units and consumed fees stay
spent; the operator admission history survives. A registry change can conservatively
replan even when unrelated to the current pair; selective invalidation is not added
in this unit. Complete physical flights retain their existing frozen timing.

## Persistence and compatibility

No reseed or destructive migration is required. Missing schedule tables mean seeded
revision 0. Legacy tickets retain their old delay/window behavior until consumed;
new tickets use the explicit snapshots and operator history. A legacy spent bucket
converts conservatively to remaining balance and then refills normally. Old receipts
need not have a serialization-start field. New records validate settings, physical
rate bindings, bounded admission history, partial-progress ordering, and accounted
unit consistency. Unmounting a reusable mobile pair leaves its bounded schedule
and capacity dormant even though its registry entry disappears; refit reuses them.
Rate bindings are checked when hardware is present, and runtime rate is capped by
that hardware. Corrupt state refuses boot without resetting spent resources.

The current receipt/board privacy boundaries remain in force. Policy is public only
for discoverable links; actual capacity and unfinished routes remain operator/staff
information. Serialization progress is available through staff `@prop/route`.

## Integration gate

Production relay tests cover authorization/CAS/bounds, frozen delay, two-spoke
admission through restart, paid first-unit capture and capacity conservation, Deferred
windows during partial transmission, real one-unit hardware fixtures, revocation
fallback, and corruption refusing boot. Existing mixed-traffic coverage now checks
FIFO serialization and channel receipt membership. A token test covers refill,
burst changes, repeated passes, rollback, long elapsed time and legacy buckets.

The live harness `scripts/relay-policy-live.mjs` uses TLS/WebSocket commands and
read-only durable state assertions on dev1. The fixture saves the prior database
and config privately, restores the genuine six-payment R9b checkpoint through
Redis RESTORE while AOF stays enabled, and verifies dev2 is untouched. It explicitly
upgrades the public star while preserving the funded private hardware, demonstrates
one admission across two spokes, paid multi-unit capture/restart/arrival, then buys
real voyage supplies and sails the commissioned carrier to separate coverage for
an actual one-unit private-link multi-unit delivery across a crash. No payments,
industrial receipts, installed hardware or clock anchors are fabricated.

The gate passed: **2,086 passing automated tests, 3 pre-existing ignored tests, 93 checked live commands and 6 SIGKILL/restart boundaries**. Full formatting, strict Clippy, release build and harness syntax checks passed. All 1,495 build/help/harness inputs match. The saved dev1 baseline/config was restored and verified healthy; dev2 was unchanged.

[Evidence, retained attempts and exact source identities](evidence/relay-policy-bandwidth-2026-10-24/README.md) close RR-5–RR-7. GitHub CI for the new push remains separate.
