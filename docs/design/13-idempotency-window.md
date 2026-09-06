# 13 — The idempotency window: every key on disk, a window in RAM, a Bloom filter on the slow lookup (D-12-6)

**Status:** design, 2026-09-06, for the owner's eye before the cuts (§6). Decided points are
numbered **W-D**; open ones are in §7.

**Trigger.** Doc 12 §5.8 designed the window; J3 (doc 12 §7) could not build it: a post's
idempotency check is synchronous on the world task, and a window cannot tell a never-used key
from a key used before the window — so every fresh post would have paid the off-loop store
check. **D-12-6** (doc 12 §9) is the owner's direction: a Bloom filter over every key ever posted
gates the slow lookup; a negative is certain and posts at once; only a positive outside the window
goes to the store; the rare positive is a **transient refusal** ("try again shortly") while an
off-loop read warms the RAM index — no `post` caller grows a re-entry path. The owner confirmed
the transient refusal on 2026-09-06 (**W-D1**).

## 1. What grows today, and by how much

Two RAM structures in the economy are O(journal entries), and the checkpoint's parts carry both:

| structure | per entry | at 1 M entries | at 7 M (D-12-4's line) |
|---|---:|---:|---:|
| `EconomySnapshot.idempotency` (key → record) | ~150 B | 150 MB | 1.05 GB |
| `EconomyStore.tx_index` (id → `TxRef{seq, key, posted_at}`, J2) | ~100 B | 100 MB | 700 MB |
| checkpoint idem parts (one row per entry, J3) | ~130 B on disk | 130 MB | 910 MB |

J2 added `tx_index` after the review's table; it is the same order as the index it sits beside and
has to be windowed with it, or the RAM win is nil. Everything else in the store is O(records) or
O(window) already (the RAM tail, the debit windows, the fold's per-account maps).

## 2. The design

### 2.1 Every key on disk, forever (W-D4)

Every post also writes **`idem:{h}`**, `h` the lowercase hex SHA-256 of `namespace ‖ 0x00 ‖ key`,
carrying the `IdempotencyRecord` (namespace, key, plan hash, transaction id, posted-at, journal
seq) — in the **same write batch as the journal append**, so the key document and the entry are
one durable fact. ~150 bytes a key; never deleted (D-12-5's retention). The hash keeps caller-chosen
namespaces and keys out of the store's key grammar and bounds the key's length.

On Redis this is Redis RAM: ~150 MB per million keys. That is the cost D-12-6 noted; a key-indexed
disk tier is a separate question (§7) and not J5's chunk archive, which is not indexed by key.

### 2.2 The window (W-D2): a cache, never authority

The RAM index keeps the records posted within **`idempotency_window_secs`** (default 30 days) and
at most **`idempotency_window_max`** records (default 1 000 000, ~150 MB); whichever is smaller
binds. The sweep evicts the oldest past either bound (`evict_window`), exactly like the RAM tail,
never on the post path. `tx_index` is windowed by the same rule; **`chunk_first_ids`** (chunk →
first transaction id, maintained beside `chunk_firsts` and carried by the checkpoint) keeps an
evicted id locatable: ids and sequences advance together, so the chunk holding an old id is a
binary search over that map, and `@econ/reverse`'s cold path reads that chunk and finds the body
by id as it does today by sequence.

Why a count cap as well as a time: at D-12-1's design rate (250 entries a minute) 30 days is 10.8
million entries and 1.6 GB — past D-12-4 on its own. The window is a *performance cache*: the
system's own retry horizon (the durable outbox's dead-letter bound, K7's deadlines) is inside it,
so a crash retry or a redelivered wake never reaches the cold path; anything older takes one
transient refusal and one store read, and is still exact. Correctness never depends on the window.

### 2.3 The filter (W-D3): every key ever, ~20 bits each

A **scalable Bloom filter** over every key ever posted — a sequence of slices, each a plain Bloom
filter sized for a capacity that doubles from 65 536, at **20 bits a key, 14 hash functions**
(false-positive rate ≈ 1 in 10 000 at capacity). A lookup asks every slice; an insert goes to the
newest, and a new slice opens when it fills. No rebuild ever, no deletions ever, ~2.5 MB per
million keys. Hashing is deterministic across machines and boots: `SHA-256(namespace ‖ 0x00 ‖
key)` gives two 64-bit halves and the i-th index is `(h1 + i·h2) mod m` (Kirsch–Mitzenmacher);
the filter's bytes are a checkpoint **part** (`checkpoint:{seq}:bloom`, base64, hashed and listed
like the idem parts), rebuilt in full by a full replay (every entry inserted), maintained on every
post. A checkpointed boot seeds it from the part. The filter is written whole at every checkpoint;
at the default cadence that is a few megabytes per ten thousand posts.

### 2.4 The gate (W-D1): the check on the post path

`EconomySnapshot` gains a **cold cache** (`cold: key → Option<record>`, bounded, entries aged out by
the sweep) beside the window and the filter, and one question the planner asks instead of the
map lookup it makes today:

```
verdict(key) =
  window has key            → Known(record)        (the map, as today)
  cold cache has key        → Known(record) | Fresh (the store answered: found | absent)
  filter says "never"       → Fresh                 (certain)
  otherwise                 → Unknown               (a positive outside the window)
```

`Known` keeps today's logic exactly (same plan → `AlreadyPosted`; a different plan →
`IdempotencyConflict`). `Fresh` posts. **`Unknown` is `EconomyError::RetryShortly`**, a new
transient variant: `post_with` then asks the store for `idem:{h}` through the H1 store-read seam
(one read in flight per key, keyed by the key, not the actor — sweep-driven callers post as the
system), and the completion handler writes the answer into the cold cache: found → the record
(the next attempt is `Known`), absent → `None` (the next attempt is `Fresh`, and its post writes
the real record). Interactive callers see *"That request is being checked against the ledger's
history; try again shortly."*; the sweep's disposition maps it to `Transient` — the next fire is
the retry, by which time the cache answers. Every other `post` caller already treats an `Err` as
a refusal of this attempt, which is the right treatment; none needs to change.

The false-positive rate is the rate at which a **fresh** post takes that detour: about 1 in 10 000
at the filter's design load, which the owner accepted.

### 2.5 Boot

A **full replay** inserts every entry's key into the filter and only entries inside the window
into the map and `tx_index`. A **checkpointed boot** seeds the filter from the bloom part and the
window from the idem parts, which now carry **only window rows**: a checkpoint re-lists an
inherited part only while its newest row is inside the window (a part wholly older is dropped
and swept), so the parts are O(window) at any journal size and the boot is O(records + window) —
the number §5.8 promised.

The contract heal (`finished_transition`) reads the keys of pending transitions; those are
inside the window by the retry-horizon argument. If the server was down longer than the window,
the heal sees no record and leaves `pending` as it is (**W-D7**); the live contract driver retries
the transition, its post hits the filter, takes one transient, and converges on the store's
answer — the same path as any old retry. A test forces it with a zero window.

### 2.6 The backfill (W-D5): the first boot after the cut

Existing journals have no `idem:` documents. The sweep backfills: `meta.idem_backfill_seq` is a
cursor; each sweep writes key documents for up to 2 000 entries from it, reading the records from
the RAM map — which still holds every key, because **window eviction is gated on the backfill
being complete** (`meta.idem_backfilled`). Until then the store behaves exactly as J3 left it
(whole map, whole parts): no regression on the first boot, and the window switches on by itself
when the cursor passes the journal head (a 100 000-entry journal is 50 sweeps, 25 minutes). The
filter, by contrast, is built at that first boot from the replay and is complete at once.

### 2.7 Sizing

| journal | filter | window (1 M cap) | parts per checkpoint | Redis key docs |
|---:|---:|---:|---:|---:|
| 100 000 | 250 KB | 15 MB | idem ≤ 13 MB + bloom 250 KB | 15 MB |
| 1 000 000 | 2.5 MB | 150 MB | idem ≤ 130 MB (window) + bloom 2.5 MB | 150 MB |
| 10 000 000 | 25 MB | 150 MB (capped) | same | 1.5 GB |

## 3. What does not change

R-B2's semantics: a key is never forgotten; a same-plan retry after any interval returns the
original result; a different plan under a used key is refused. The journal is authority; the key
documents are a projection of it (rebuildable by a replay), and the filter and window are caches.
The checkpoint's trust model (doc 12 J3): the bloom part is hashed and listed like every part; a
checkpoint with a bad bloom part is rejected whole and the journal replayed.

## 4. Verify lenses

- **I** (idempotency): a same-plan retry after the window's eviction → transient, then
  `AlreadyPosted` with the original id; a different plan → transient, then `IdempotencyConflict`;
  a fresh key → posts at once (filter negative); a fresh key that is a filter positive → transient,
  then posts, and its record is the real one. A key read in flight is not re-requested.
- **B** (boot): a full replay builds the filter from every entry and the window from the recent
  ones; a checkpointed boot seeds both from parts and answers the same verdicts; an inherited
  part wholly older than the window is dropped and swept; the contract heal with a zero window
  leaves `pending` and the live driver converges.
- **S** (sweep): a sweep-driven settlement whose key is a filter positive gets `Transient` and
  posts on the next fire; the backfill writes key documents at its pace and gates eviction; the
  window evicts by count and by age, `tx_index` with it, and a reversal of an evicted id still
  reaches its chunk through `chunk_first_ids`.
- **F** (filter): the false-positive rate at 100 000 random keys is within 3× of 1e-4; a key
  inserted is always positive; the scalable growth keeps the rate as slices open; the part round
  trips through JSON and hashes stably.
- **M** (measured): the from-file boot's at-rest RSS with the window capped at 1 000 versus
  unbounded at 100 000 entries; the checkpoint parts' size with the window versus whole; the
  post path's cost with the gate (µs per post, harness).
- **C**: the cadence-1 workspace pass, again — the boot rule and the checkpoint shape change.

## 5. Config

```toml
[economy]
# idempotency_window_secs = 2592000   # 30 days: how long a key stays in RAM (a cache; every key is on disk)
# idempotency_window_max  = 1000000   # at most this many keys in RAM, whichever bound is smaller
```

## 6. Cuts

**W1 — the mechanism.** `idem:{h}` documents written with every post; the scalable Bloom filter
(`mush_economy::bloom`) maintained on every post and built by the replay; the gate in the planner
(`verdict`), `EconomyError::RetryShortly`, the keyed store read and its completion into the cold
cache, the sweep disposition; the backfill cursor. Nothing evicted yet: after W1 every verdict is
`Known` or `Fresh` in practice, and the transient path is exercised only by tests that force a
positive. Lenses I, S, F. Size: a day.

**W2 — the window.** Eviction of the map and `tx_index` by age and count, gated on the backfill;
`chunk_first_ids` and the cold reverse of an evicted id; the checkpoint's bloom part and
window-only idem parts; the boot from parts; the contract heal under a zero window; the RAM and
parts measurements. Lenses B, S, M, C. Size: a day.

## 7. Open for the owner

1. The window's count cap default: 1 000 000 records (~150 MB) — inside D-12-4 with room; say if
   you want it lower.
2. Redis RAM for the key documents grows with every key ever (§2.7): ~150 MB a million. When that
   line matters, the answer is a key-indexed disk store (not the chunk archive); not in these cuts.
