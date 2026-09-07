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

> **W1 as built — 2026-09-06.** §2.1, §2.3, §2.4 and §2.6, and the checkpoint's bloom part
> from §2.5 — everything but the eviction, so that a checkpointed boot can never gate on a
> filter with a key missing: the filter is complete from the first boot after the cut.
>
> 1. **Key documents** (`mush_economy::keys`): `idem:{h}` written by `install_commit` beside
>    the journal append, carrying namespace, key, plan hash, transaction, posted-at and
>    sequence; ignored at boot; the boot manifest (J4) grew **`skip_key_prefixes`**
>    (`mush_store::BootSkip` replaces the floors alone; the J4 retry now rewrites the manifest
>    with its floors cleared and its prefixes kept, instead of deleting it), so the loader
>    fetches nothing under `idem:` even on a whole fetch.
> 2. **The filter** (`mush_economy::bloom::ScalableBloom`, W-D3 as designed but the bytes as
>    **hex**, not base64 — no base64 dependency in the workspace; 5 MB per million keys in the
>    document, 2.5 MB in RAM): built by the full replay from every entry, maintained by
>    `index_transaction`, carried as `checkpoint:{seq}:bloom` — a hashed part the head lists
>    beside the idem parts and the state hash covers (**checkpoint format 2**; a format-1 head
>    is rejected and replayed). A checkpointed boot seeds from the part and the replayed tail
>    extends it; a tampered or missing bloom part rejects the checkpoint whole.
> 3. **The gate** (`KeyGate` / `KeyVerdict` in the planner's `ValidateEnv`, the store's
>    `StoreKeyGate`): the window's map first, then the cold cache, then the filter. **W-D9:**
>    the environment's `keys` is an `Option` in W1 — `post_with` and `reverse` hand the store's
>    gate in; the sweep- and contract-driven `validate` calls and the test rigs pass `None`
>    (the map alone, today's semantics), which W2 makes mandatory when it switches the window
>    on. **`KeyMode::Whole`** is the only production mode in W1: the map holds every key, so a
>    miss is `Fresh`; `Windowed` decides as W2 will and is reachable through test hooks.
> 4. **The transient** (`EconomyError::RetryShortly`, W-D1): a verdict of `Unknown` refuses the
>    attempt with *"that request is being checked against the ledger's history; try again
>    shortly"* and asks the store for the key's document through the H1 seam — one read in
>    flight per key, keyed by the key; **W-D8:** the read is issued under the economy's own
>    module identity whichever module's command posted, so the completion routes to the
>    economy's handler (`Ctx.module` is swapped for the call and restored). The completion
>    warms the cold cache — the record, or a confirmed absence — and the same request is then
>    decided from it; a failed or oversized read is forgotten and asked again. The cold cache
>    ages out after ten minutes and stale reads after the store-read bound, on the sweep. The
>    sweep's disposition maps the transient to `Transient` — the next fire retries.
> 5. **The backfill** (W-D5): `meta.idem_backfill_txid` / `idem_backfilled`; the sweep writes
>    up to 2 000 key documents a fire from the RAM index and flags done when the cursor passes
>    the newest id; a fresh world is done at boot. W2's eviction waits for it.
> 6. **W1-F1 (owner-facing, pre-existing):** `apply` **clones the whole `EconomySnapshot` for
>    every post** (`let mut next = snapshot.clone()`), which is every account, title and
>    reservation and the entire idempotency map — O(records + keys) per post today. The window
>    (W2) shrinks the map's share; the rest is a cost this programme did not create and does
>    not fix. Worth its own look before the design rate is real.
>
> **Verify (lenses I, B, S, F, C).** I: with the gate deciding as W2 will, a key evicted from
> the RAM map that the filter has seen is a transient — the store is asked once (one read in
> flight per key, a second attempt asks nothing twice), the answer warms the cold cache, and
> the same request is then decided from it: the original id for the same plan, a conflict for
> a different one; a never-posted key posts at once; a never-posted key the filter is wrong
> about takes the transient, the store answers "absent", and the next attempt posts the real
> record, which supersedes the cold answer; a failed read is forgotten and asked again; the
> cold cache ages out on the sweep. B: the filter knows every key posted and never a fresh one;
> a checkpoint carries it as a hashed part; a checkpointed boot seeds from it and the replayed
> tail extends it; a tampered or missing bloom part rejects the checkpoint whole and the full
> replay rebuilds the filter; the key documents handed over by a rig are ignored at boot. S:
> every post writes its key document beside its entry, under the hash of its key, with the
> record and the sequence; a fresh world needs no backfill; a journal older than the cut is
> backfilled by the sweep and flagged done, and a world booted from the result writes nothing
> more; the sweep's disposition maps the transient to `Transient`. F: a key inserted is always
> positive; the false-positive rate over 100 000 never-inserted keys against 100 000 inserted
> (two slices) is at most 3 in 10 000 (asserted ≤ 30 hits); the filter round-trips through
> JSON with the same hashes; a malformed filter is rejected. The loader's and the store suite's
> narrowed listings skip a key prefix on both fetch shapes and report it, the J4 retry keeps
> the prefixes, and the J4 end-to-end drive is unchanged. C: the cadence-1 workspace pass,
> repeated for the checkpoint's new shape. Not measured in W1: the post path's added cost (a
> SHA-256, a filter insert and one more document a post); W2's RAM measurement is the number
> this design is for. The Redis suite is `REDIS_URL`-gated and was not run on this box.

**W2 — the window.** Eviction of the map and `tx_index` by age and count, gated on the backfill;
`chunk_first_ids` and the cold reverse of an evicted id; the checkpoint's bloom part and
window-only idem parts; the boot from parts; the contract heal under a zero window; the RAM and
parts measurements. Lenses B, S, M, C. Size: a day.

> **W2 as built — 2026-09-06.** §2.2, the rest of §2.5, and W-D6, W-D7, W-D9.
>
> 1. **The window** (`[economy] idempotency_window_secs`, default 30 days;
>    `idempotency_window_max`, default 1 000 000, zero meaning the default): the sweep evicts
>    the idempotency map and the transaction index **together**, oldest first, past either
>    bound (`evict_window`), after the garbage collector and never on the post path — and never
>    before every key has its document (W-D5): the backfilling sweep flags done and switches
>    the gate to `Windowed`; the next sweep evicts. A boot from a checkpoint or a full replay
>    keeps only the window's keys in RAM (the replay inserts only entries inside the age bound
>    once the store is backfilled; the count bound is applied when the store is up) while the
>    filter takes every key. A fresh world is backfilled at birth and windowed at once.
> 2. **The chunk map** (W-D6): `chunk_first_ids` beside `chunk_firsts`, maintained at install
>    and at boot, carried by the checkpoint body (**checkpoint format 3**), never evicted. An id
>    the window evicted is still one the allocator minted (`id_allocated`) and still locatable
>    (`chunk_of_tx`): `@econ/reverse` takes the same cold path as a body that left the RAM
>    tail, reads that chunk, finds the body by id, and `reverse_original` accepts it when the
>    chunk map and the body's sequence name the same chunk.
> 3. **Window-only parts** (§2.5): a checkpoint's new rows are the window's (the index is
>    windowed), and an inherited part wholly below the window's floor — its highest id below
>    the index's first — is not re-listed and is swept; the parts are O(window) at any journal
>    size, and a checkpointed boot seeds the window from them and trims it to the count bound.
> 4. **The gate is mandatory** (W-D9): `ValidateEnv.keys` is `&dyn KeyGate`; every live
>    validation — the post path, reversals, the sweep's and the contract driver's
>    environments — hands the store's gate in; the rigs that validate against a bare snapshot
>    pass `MapOnlyKeys`. `idempotency_resolved` answers through the gate (a key outside the
>    window the filter has seen is not known to be resolved; the post decides).
> 5. **Transients in the drivers** (W-D1): a settlement whose post answers `RetryShortly` is
>    `NotReady` (no attempt counted), and a pending terminal transition whose post answers it
>    waits for the next fire (nothing counted, nothing paged) — the key read is in flight and
>    the next fire finishes on the original transaction.
> 6. **The boot heal under the window** (W-D7, as built — see W2-F1 below): boot's
>    `finished_transition` asks the gate; a contract whose escrow the journal no longer holds
>    and whose terminal keys are outside the window (the filter has seen one) is neither
>    finished nor quarantined at boot but queued (`heal_wanted`), and the sweep's contract step
>    resolves it through keyed reads — finishing it as boot would, or disputing it as boot would
>    have quarantined it once the store has answered "absent" for every key.
>
> **Measured (lens M).** The from-file boot (`scale_cold_journal_replay_from_file`, release,
> this dev box) at **1 000 000 synthetic entries** (35 702 chunks, a 643 MB document file),
> sampled from outside for its peak and its at-rest working set, the world held and leaked so
> the last sample is the store and not its teardown, the RAM tail at its default 20 000:
>
> | window (`idempotency_window_max`) | keys in RAM | at rest | peak |
> |---:|---:|---:|---:|
> | 1 000 000 (whole) | 1 000 000 | 422 MB | 1 218 MB |
> | 100 000 | 100 000 | 82 MB | 1 218 MB |
> | 1 000 | 1 000 | 41 MB | 1 218 MB |
>
> The window is the RAM line D-12-4 wanted: at a million entries the economy at rest goes from
> 422 MB to 41 MB, about 380 bytes a key across the map, the index and the snapshot — above
> the review's 150-byte estimate, which counted the map alone. The peak is the harness's own
> transient (the whole document file parsed into memory before the boot) and is the same in
> every row; a production boot reads the store in pages and never holds that. The replay took
> ~3.1 s at a million entries either way (~3 µs an entry, as J1 measured). At 100 000 entries
> the difference (24 MB versus 33 MB at rest) is inside the sampler's noise.
>
> **Verify (lenses S, B, I, C).** S: once backfilled, the sweep evicts the map and the index
> together — the newest three of five stay, an evicted id is still locatable, the filter keeps
> its key, and a repeat of an evicted key takes the transient and then the original id; the age
> bound leaves only the post inside the window; an old journal is not evicted until its
> documents exist (the backfilling sweep switches the mode, the next sweep evicts). A reversal
> of a transaction both the tail and the window evicted reads its chunk through the chunk map
> and posts; an id nobody minted is still nobody's. B: with the window on, a checkpoint's parts
> carry only the window's rows, an inherited part wholly below the floor is dropped and the new
> head's part is numbered from zero, a checkpointed boot seeds the window from the parts and
> trims it, evicted ids stay locatable after a boot, and a full replay keeps only the window's
> keys in RAM while the filter takes every key. **W-D7, as found (W2-F1):** the lost-transition
> test with the window at zero showed that boot would have *quarantined* a finished contract
> whose keys were outside the window — `finished_transition` found no record and the escrow was
> not held — and that a lost *settle* doc write leaves nothing pending for the live driver to
> retry, since the restored record predates the attestation. Boot now asks the gate: a key the
> filter has seen but the window lacks is `Unknown`, the contract goes to the store's queue
> instead of quarantine, and the sweep resolves it through keyed reads and finishes it exactly
> as boot would have (or disputes it, as boot would have quarantined it, once the store has
> answered "absent" for every key). Verified for settle and refund: boot leaves it, the first
> fire asks for exactly one key (the others are certain negatives), nothing is paged or frozen,
> and the next fire finishes it on the original transaction with the balances untouched. I: the
> settled-contract identity check at boot accepts a settlement older than the window when the
> allocator minted its id and the filter has seen the contract's own settle key, and still
> freezes on one naming another transaction. C: the cadence-1 workspace pass, again.

## 7. Open for the owner

1. The window's count cap default: 1 000 000 records (~150 MB) — inside D-12-4 with room; say if
   you want it lower.
2. Redis RAM for the key documents grows with every key ever (§2.7): ~150 MB a million. When that
   line matters, the answer is a key-indexed disk store (not the chunk archive); not in these cuts.
