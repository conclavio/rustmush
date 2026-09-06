# 12 — The economy checkpoint review (C17, 08g §12.1 item 5)

**Status:** review, 2026-09-05. Written for the economy owner's decision. Nothing here is built;
§6 is the recommendation and §7 the cuts it would take.

**Trigger.** 08g §12.1 item 5 fixed the rule: *the durability checkpoint becomes the next cut's
first item when a cold replay exceeds 5 s or 1 000 000 entries.* H3's harness
(`scale_cold_journal_replay_reports_time`, release profile) put a 100 000-entry cold replay at
**6 998 ms** — past the time line at a tenth of the entry line. The two thresholds were meant to
describe the same event; they do not. This review says where the time goes, what the line actually
measures, and what a checkpoint would have to be to change it.

**Sources, by symbol.** `crates/mush-economy/src/boot.rs` `load_boot`; `module.rs`
`on_world_loaded`, `CHECKPOINT_REVIEW_SECS`, `CHECKPOINT_REVIEW_ENTRIES`, `EconomyStore`,
`ReplayStats`; `crates/mush-core/src/ledger.rs` `scan_contiguity`, `Ledger::restore_from_kv`,
`inspect_history`, `encoded_len_bounded`, `read_cold_range`, `recent_sealed`;
`crates/mush-store/src/redis.rs` `module_json_list`; `crates/mush-server/src/modules.rs`
`load_one_namespace`; doc 08 §7.5 and §15.1; 08g §12.1 item 5 and the C17 rows;
`docs/design/11-h-implementation-plan.md` §3.3 (H3 measured).

---

## 1. What a cold boot does today

The journal is the sole authority (doc 08 §7.5); every account, asset and hold document is a
projection, and boot re-derives all of them from the whole journal. The path, in order:

1. **Fetch.** `load_one_namespace` calls `module_json_list("economy")`: one `SCAN` for
   `mod:economy:*`, then **one `JSON.GET` round trip per key, sequentially**. Every chunk of the
   journal is a key (`log:journal:{chunk}`, sealed at `DEFAULT_CHUNK_TARGET_BYTES` = 16 KiB), so
   the fetch is O(chunks) round trips, and each document is parsed from text into a
   `serde_json::Value` here. **This is outside the replay timer** (`on_world_loaded` starts its
   clock after the `ModuleBootData` exists).
2. **Contiguity scan.** `scan_contiguity(&data.kv)` → `inspect_history`. For every chunk: serialize
   the whole chunk `Value` to a counting writer (the 8 MiB cap), **clone** the `Value`, deserialize
   it to `LedgerChunk`, and serialize **every entry** again to a counting writer (the per-entry
   cap). Then walk the chunks for gaps, seals and sequence contiguity. The parsed chunks are
   dropped.
3. **Ledger restore.** `Ledger::restore_from_kv(&data.kv)` → `inspect_history` **a second time**,
   identical work, to keep the open tail (only the last chunk) in RAM.
4. **Document parse.** `load_boot` walks `data.kv` once more; for `log:journal:*` it clones and
   deserializes the chunk to `LedgerChunk` **a third time** and keeps them; account, instrument,
   vault, hold, obligation, contract, cadence, escalation, approval and notice docs are typed here
   (O(records)).
5. **Entry typing.** Every `LedgerEntry.value` is deserialized to `EconomicTransaction` and the
   list is sorted by `seq`.
6. **The fold.** One pass over the typed entries: running balances, last-touch and touch counts,
   held amounts, asset and hold post-states, custody marks, consumed approvals, the idempotency
   index, the reversed set, the debit-window tails, and `transactions.insert(tx.id, tx.clone())`
   — **every transaction is retained in RAM**.
7. **Reconciliation.** Per account, asset, hold, vault, approval, obligation, contract, cadence and
   escalation: the directional heal and the drift/ahead/conservation latch. O(records); the
   contract pass looks settlements up in the retained map by id.
8. **Heal writes and `meta`.** Changed docs are re-persisted; `meta` is always persisted.

Per journal entry, before the fold touches it, that is: three `Value`→`LedgerChunk`
deserializations (amortized over the chunk), three chunk clones, two whole-chunk serializations,
two entry serializations, one typed deserialization and one typed clone. The fold itself is
`BTreeMap` arithmetic.

## 2. Measured

Method: `scale_cold_journal_replay_phases` in `crates/mush-contract-tests/tests/scale_harness.rs`
(added with this review; the CI `scale` job runs it) synthesizes the journal from one real minted
transaction — same document shapes, same 16 KiB sealing rule as `Ledger` — and times each phase
separately by calling the same functions `load_boot` calls, then boots the module for the total
the C17 timer reports. Release profile, on the development box (Windows 11, shared with other
build jobs). H3's harness seeds the same journal through the engine at ~4.5 ms per mint; the
synthetic seeder takes milliseconds, which is why the review could run three scales:

```
SCALE_PROFILE_SCALES=25000,50000,100000 cargo test --release -p mush-contract-tests \
  --test scale_harness -- --ignored --nocapture scale_cold_journal_replay_phases
```

Phase times in milliseconds; `boot_total` is what `ReplayStats.elapsed_ms` reports (the C17
timer); `fetch_parse` is the text→`Value` parse the store does **before** that timer starts. 100
accounts; one mint per entry (two lines, no effects): 575 bytes of JSON per entry, ~28 entries per
sealed chunk.

| entries | chunks | journal JSON | fetch_parse | scan (inspect 1) | restore (inspect 2) | chunk parse (3) | entry typing | remainder (fold, reconcile, heal) | **boot_total** |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 000 | 881 | 14.0 MB | 126 | 264 | 272 | 154 | 82 | 47 | **819** |
| 50 000 | 1 774 | 28.0 MB | 263 | 576 | 559 | 310 | 181 | 90 | **1 716** |
| 100 000 | 3 560 | 56.1 MB | 548 | 1 048 | 1 156 | 651 | 396 | 1 154 | **4 405** |

Per entry at 100 000: **44 µs** in the timer (H3's engine-seeded run measured 70 µs on the same
box under different load — the two runs bracket the noise; the shares are the finding, not the
absolute). Of the 44 µs: inspection 1 ≈ 10.5, inspection 2 ≈ 11.5, chunk parse ≈ 6.5, entry
typing ≈ 4, and a remainder ≈ 11.5 that the second run below shows to be contention noise rather
than the fold. Outside the timer: text→`Value` ≈ 5.5 µs per
entry, and one sequential `JSON.GET` round trip per chunk (3 560 at 100 000; not measured here —
at a local Redis's ~0.15 ms that is ~0.5 s, over a 1 ms network hop ~3.5 s).

A second run, ordered 100 000 → 200 000 → 25 000 in one process, to check linearity and the
in-process effects (the workspace test gate was compiling on the same box during both runs):

| entries | fetch_parse | scan | restore | chunk parse | entry typing | remainder | **boot_total** | µs / entry |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 000 | 557 | 1 226 | 1 340 | 639 | 469 | 1 536 | **5 210** | 52 |
| 200 000 | 1 308 | 3 463 | 4 081 | 1 890 | 1 062 | 288 | **10 784** | 54 |
| 25 000 | 173 | 369 | 373 | 145 | 84 | ≈ 0 | **967** | 39 |

The remainder (the fold, the reconciliation, the heal writes, retention) is the boot total minus
the four serde phases measured separately, so it also absorbs the run-to-run noise between the
two measurements; its clean values (≈ 0 at 25 000, 288 ms at 200 000) put the fold at **≤ 1.5 µs
per entry**. The 1.1–1.5 s remainders at 100 000 in both runs are contention noise, not a
non-linearity (F8). Cost per entry rises gently with scale (39 → 44/52 → 54 µs).

In-RAM sizes: `EconomicTransaction` 400 B, `IdempotencyRecord` 96 B, before heap.

## 3. Findings

**F1 — The time is JSON handling, not money.** Inspection 1, inspection 2, the third chunk parse
and the entry typing are 3 251 of the 4 405 ms at 100 000 entries in the first run (74 %) and
10 496 of 10 784 at 200 000 in the second (97 %); the fold, the reconciliation, the heal writes
and the retention together cost ≤ 1.5 µs per entry. Add the text→`Value` parse that happens
before the timer and essentially the whole boot cost is serde over the journal.

**F2 — The journal is inspected three times, and two of those serialize every entry to count
bytes.** `scan_contiguity` and `Ledger::restore_from_kv` are called back to back on the same
`kv` (`boot.rs` lines 86 and 92) and each runs `inspect_history` in full — serialize the chunk,
clone it, deserialize it, serialize each entry — then discards the parsed chunks; `load_boot`
then clones and deserializes each chunk a third time (line ~362). The two inspections alone are
2 204 ms of the 4 405. The byte counting guards the *append* caps (`MAX_SAFE_CHUNK_BYTES`), which
a persisted document has already passed; on the read side the raw document length says the same
thing without a serialization.

**F3 — The timer measures a subset of the boot.** `on_world_loaded` starts its clock after the
namespace is fetched. The fetch is a `SCAN` plus one sequential `JSON.GET` per key, so it is
O(chunks) round trips, and the store parses every document from text to `Value` before the
module sees it (548 ms at 100 000 entries, +12 % on the timer). At 1 000 000 entries the
~35 000 round trips are probably the largest single term on a store that is not on the same
host. The C17 warning cannot fire on any of this.

**F4 — Retention is what couples a checkpoint to the cost.** `EconomyStore.transactions` keeps
every transaction (E4a-D11): ~600 bytes each retained (400 in the struct, the rest heap) — ~60 MB
at 100 000 entries, ~600 MB at 1 000 000 — and seven consumers read it (§5.1). While that stands,
a checkpoint that skipped the covered chunks would still have to fetch, parse and type every one
of them to fill the index, so it would save only the fold: ~10 % of the timer. During the boot
itself the journal's `Value` tree (56 MB of text at 100 000 entries; several times that as
`Value`) is also alive end to end.

**F5 — The idempotency index is the entry-linear term a checkpoint does not remove.** R-B2 keeps
every key forever (a key reused with a different plan is always an error), so the index is
O(entries), must be carried by the checkpoint (§5.2), and must be parsed back at boot. At the
measured 4 µs per entry for typing a 575-byte record, a compact ~120-byte record is ~2–3 µs:
about 3 s at 1 000 000 entries, before the tail replay. That is the checkpoint's ceiling, and
it is set by a rules decision (Q5), not by engineering.

**F6 — The two thresholds disagree by an order of magnitude.** At 44–70 µs per entry, 5 s is
70 000–110 000 entries and 1 000 000 entries is 44–70 s. The entry line was drawn against a cost
model ~10–14× more optimistic than the code. Whatever else happens, one of the two lines should
be derived from the other by a measured per-entry cost (§4).

**F7 — The cold-chunk skip is already anticipated.** `scan_contiguity`'s doc says a tail-only
`kv` "would look like a missing prefix here and this check must be revisited (e.g. keyed off a
persisted min-present marker)"; `restore_from_kv` validates from chunk 0 on the same assumption.
§5.6 is that revisit.

**F9 — A full replay holds the whole journal as `serde_json::Value` until the fold ends.** The
store parses every document to `Value` before `load_boot` runs, and `data.kv` lives through the
boot. A 575-byte entry is roughly 3 KB as a `Value` tree (fifteen keyed fields, two line objects,
and `plan_hash` as thirty-two boxed numbers), so the transient is ~330 MB at 100 000 entries and
crosses 1 GB near 330 000 — a memory line, not a time line, and one that J1's parse-once only
removes if the parse goes from text straight to typed records with the text dropped chunk by
chunk. (Estimate from the shapes; the harness does not measure peak working set.)

**F10 — The journal itself is Redis memory.** At 575 bytes per entry plus RedisJSON's overhead the
journal is ~1 GB of store RAM near 1 000 000 entries. Sealed chunks are immutable; nothing here
moves them anywhere colder. That is the other half of C17 ("cold paging") and it binds before any
boot-time line does at the rates in §9. §10 is the owner's answer: the ledger archive tier.

**F8 — The remainder column is noise, not a non-linearity.** The first run's 47 → 90 → 1 154 ms
looked like retention biting at 100 000; the second run (100 000 → 200 000 → 25 000, §2) shows
≈ 0 ms at 25 000 and 288 ms at 200 000, so the 1.1–1.5 s remainders at 100 000 are the
concurrent build on the box landing on that measurement. Cost per entry does rise gently with
scale (39 → 54 µs from 25 000 to 200 000), which is the allocator and cache, not the fold.

## 4. What the 5 s line measures

Doc 08 §7.5 step 8 is the promise the line stands for: *publish no control or spending authority
until validation completes*. What the operator experiences is the time from process start to
that publication, and F3 shows the timer covers less than half of the JSON work on the way
there once the store is remote. The line should mean **fetch plus replay**, logged as two numbers
(`fetch_ms` from `load_one_namespace`, `replay_ms` as today) with the warning firing on their
sum. That is a small change to the loader and to `ReplayStats`.

The second line should be **derived, not drawn**: the harness prints `entries at the line =
5 s ÷ measured µs per entry`, and `CHECKPOINT_REVIEW_ENTRIES` is set from that number after
each cut that moves it. Under today's code that is ~70 000–110 000; after J1 (§7) it is
~140 000–230 000.

## 5. The checkpoint, designed

Doc 08 §7.5 names the contents — *balances, active holds, allocator positions, the covered
journal sequence, and a deterministic state hash* — and the trust rule: it *accelerates boot but
does not replace journal history*. This section makes that concrete against the code, and says
what it costs. The short version: a checkpoint is a persisted image of the fold's accumulators at
a covered sequence, validated by hash, and it only saves time if the covered chunks are **not
read at all** — which today's retained transaction index forbids (F4). The design is therefore
three coupled pieces: a bounded transaction index, the checkpoint documents, and a narrower boot
fetch.

### 5.1 The retained transaction index becomes a bounded tail

`EconomyStore.transactions` holds every `EconomicTransaction` ever posted (E4a-D11, deferred in
C17 as "RAM tx index"). Six consumers read it; each can be served without full retention:

| consumer | what it needs | under a checkpoint |
|---|---|---|
| boot's contract pass (`finished_transition`, the settled-claim check at `boot.rs` ~1012) | does tx `id` exist, and is its idempotency key mine | a **reverse map** `tx id → (seq, idempotency key, posted_at)`, carried by the checkpoint (it is the idempotency index seen from the other side) |
| `title_generation` (`api.rs` ~2505): how many `Created` effects an object has had | a per-object counter | `created_count: object → n`, folded and carried (tiny) |
| the sweep's republish filter (`sweep.rs` ~1542): `posted_at` of a settlement | one field by id | the reverse map |
| `econ_audit` (newest 20 bodies) | recent bodies | the RAM tail |
| account history (`commands.rs` ~2803): bodies touching an account, newest first, capped | recent bodies, then older | the RAM tail first; older pages through the store-read seam (H1 `Ctx::read_store` + `read_cold_range`), exactly as `+timestop/log` pages a closed scene |
| `rebuild_debit_window` (`module.rs` ~854): bodies within `MAX_APPROVAL_WINDOW_SECS` | a time-bounded window | the RAM tail must cover at least that window — *superseded by J2-D1 (§7): no tail covers 90 days at the design rate; the rebuild folds from the tail and says when older spend is not counted* |
| `transaction(id)` (reversal, staff audit) | one body by id | the RAM tail, else a cold read: the reverse map gives the seq, a carried `chunk_firsts: chunk → first seq` table gives the chunk key |

The tail is bounded two ways: at least `MAX_APPROVAL_WINDOW_SECS` of entries, at most
`TX_TAIL_MAX` entries (a knob; 20 000 is a sensible default — about the last 300 chunks). Eviction
happens in the sweep, never on the post path.

### 5.2 The documents

`checkpoint:{seq}` keeps doc 08's key, with `{seq}` the **covered sequence** (the last journal seq
folded in). It is a head plus parts:

- **Head** `checkpoint:{seq}` — `covered_seq`, `covered_chunk` (the sealed chunk that contains
  `covered_seq`; a checkpoint always ends at a chunk boundary so the tail replay starts at a
  chunk), `written_at`, `format: 1`, the part list `[(key, sha256, seq_lo, seq_hi)]`, and
  `state_hash` (SHA-256 over the canonical JSON of the mutable body, domain-separated the way
  `plan_hash` is, `CHECKPOINT_HASH_DOMAIN`), plus the mutable body itself when it is small enough
  (below 1 MiB), otherwise as part 0.
- **Mutable body** — the fold's accumulators at `covered_seq`, exactly the locals of the replay
  loop in `load_boot`: per account `running`, `last_touch`, `touches`, `running_held`,
  `last_hold_touch`; `journal_assets` (post-states); `journal_holds` and `hold_terminal_at`;
  `last_custody` and `max_custody_vault`; `consumed_approval` and `max_consumed_approval`;
  `max_hold_id`, `max_txid`; the `reversed` set; `created_count`; `chunk_firsts`; the debit-window
  tails. O(records), not O(entries): with 10 000 accounts and a few thousand titles this is a
  couple of MB.
- **Immutable parts** `checkpoint:{seq}:idem:{n}` — the idempotency index and the reverse map for
  one sealed chunk range, compactly encoded (hex plan hash, no field names repeated). These are
  the checkpoint's dominant payload — the idempotency index is unbounded by contract (R-B2: a key
  reused with a different plan is always an error, however old) — but they are **append-only**:
  a part covering chunks `[a, b]` never changes, so a later checkpoint re-lists the old parts and
  writes only the new range. The head's part list is what makes them one checkpoint. At ~130
  bytes per entry: ~13 MB at 100 000 entries, ~130 MB at 1 000 000, in ≤ 4 MiB parts.

Ordering of the write: parts first, head last. A crash between them leaves parts nobody names;
the sweep's GC removes any part not listed by a surviving head, and any head older than the
newest complete one plus one predecessor.

### 5.3 Writing it

On the world task, from the economy's one sweep (D-E4-3), when
`journal.next_seq(JOURNAL_LOG) − covered_seq ≥ CHECKPOINT_EVERY_ENTRIES` (a knob; 10 000 is the
first value to try) and the open chunk has just sealed. The mutable body is assembled from the
store's live state (the same maps `apply` maintains), serialized and hashed there — at ms scale
for the sizes above — and emitted as `ModuleWrite::JsonSet` ops through the module's normal
persistence, like every other economy document. The new idempotency parts are built from the
RAM index for the seq range since the last head. **The writer runs the drift check at write
time**: every account/asset/hold projection is compared with the live state at the head, so the
checkpoint is written only when the docs agree with the journal (§5.5 says why that matters).

### 5.4 Booting from it

1. From the fetched documents pick the highest `checkpoint:{seq}` whose head parses, whose parts
   are all present with matching hashes, whose `state_hash` matches, whose `covered_seq` is
   ≤ the journal head, and whose `covered_chunk` exists sealed. Anything else is **ignored** — a
   log line and an escalation, then the full replay exactly as today. A checkpoint never fails
   the boot and never freezes anything: it is a projection under the same directional rule as an
   account document — never authority.
2. Contiguity and restore run over the tail only: `scan_contiguity` and `Ledger::restore_from_kv`
   gain a *min-present* chunk (the doc-comment on `scan_contiguity` already names this as the
   thing a cold-chunk boot-skip must change): chunks below `covered_chunk + 1` are absent by
   design, not a gap.
3. Seed the fold's locals from the mutable body and the idempotency index from the parts, then
   run the existing replay loop from `covered_seq + 1`, then the existing reconciliation unchanged.
4. `meta` is persisted at the end as today; the freeze latch is untouched by any of this (§5.5).

### 5.5 What the drift detector loses, and the latch

Today drift is judged **at the sequence a document claims**: the fold records each account's
running balance when it passes `last_journal_seq`, and a stored balance that disagrees there
latches the whole economy (`snapshot_at_stored`; the same for `asset_at_stored`, `hold_at_stored`,
`held_at_stored`, and E4b1's B5 "claims a sequence it never appears at"). A boot from a
checkpoint cannot visit sequences ≤ `covered_seq`, so for a document claiming `X ≤ covered_seq`:

- `X == last_touch(account)` as of the checkpoint → compare with the checkpoint's balance: the
  drift verdict is preserved (this is every document that was at head when the checkpoint was
  written, i.e. the normal case after a clean run);
- `X < last_touch` → the document **lags**: it is healed to the journal-derived value as today,
  but **without** the drift or B5 verdict, because the balance at `X` is unknown. Log at `info`.

The loss is bounded by §5.3's write-time check: every document version that existed at a
checkpoint or a full boot was verified then; the only unverified heal is a version written and
superseded *between* checkpoints that resurfaces after a lost companion write. It is still a
weakening of the money-safety story and it is the owner's call (Q2 in §8). The alternative —
carry per-account balance history in the checkpoint — is the journal again.

The freeze latch is unaffected: `meta.frozen` stays doc authority, a frozen economy boots and
heals from a checkpoint exactly as it does from a full replay, and a checkpoint may be written
while frozen (it records journal-derived state, not the latch).

### 5.6 The fetch

`load_one_namespace` lists the whole namespace and `module_json_list` issues one `JSON.GET` per
key. A checkpoint boot must not fetch cold chunks, and no module-specific knowledge belongs in
the generic loader. The mechanism: a per-namespace **boot manifest** document (`boot-manifest`,
written by the module) that says `skip_log_chunks_below: { "journal": k }`; the loader reads it
first, lists keys, and fetches everything except `log:{id}:{n}` with `n < k`, recording the
omission in a new `ModuleBootData.omitted` field the ledger checks consume as the min-present
marker. The economy writes the manifest right after each head and clears it when a head is
GC'd. If the checkpoint then proves invalid at boot, the module cannot full-replay from what it
has, so `on_world_loaded` returns a typed `ModError::RetryFullFetch`, and the loader refetches the
namespace whole (manifest ignored) and calls once more. That retry path is the design's main API
cost and gets its own test.

Independently of the checkpoint, the per-key `JSON.GET` loop should become a pipelined
`JSON.MGET` in pages — it is the fetch cost of every module's boot, not just the economy's.

### 5.8 The idempotency index: forever on disk, a window in RAM (D-12-5)

R-B2's semantics stay: a key is never forgotten, and a retry of the same plan after any interval
must find its original result rather than post again. What changes is the mechanism, because an
O(entries) RAM index crosses the economy's 1 GB budget in weeks at the §9 design rate:

- **On disk, forever:** every post also writes `idempotency:{namespace}:{key_hash}` — the key doc
  08 §15.1 already names and nobody writes — carrying the `IdempotencyRecord` (transaction id,
  plan hash, posted-at). ~100 bytes per entry in the store; a one-time backfill from the journal
  when the cut lands.
- **In RAM, a window:** the index keeps keys posted within `IDEMPOTENCY_WINDOW_SECS` (30 days to
  start). The window's floor is the system's own retry horizon — the durable outbox's dead-letter
  bound and K7's deadlines — so a crash retry or a redelivered wake never reaches the cold path.
- **The check:** a key inside the window is checked synchronously on the world task, as today. A
  key outside it is checked **off-loop** through the store-read seam (H1) before the post
  re-enters with expected revisions, the way quotes and approvals already re-enter (doc 08 §7.4).
  A hit returns the original result or refuses a different plan, as today; a miss posts. No
  probabilistic structure: a false "reused key" refusal on a money path is not acceptable, and a
  round trip on a month-old key is. *Amended by D-12-6: a Bloom filter gates the slow lookup —
  a negative is certain and posts synchronously; a positive is never a refusal on the filter
  alone, it is what sends the key to the off-loop read.*
- **The checkpoint** (§5.2) carries only the window's keys, so its parts are O(window), not
  O(entries), and boot from a checkpoint is O(records + window) at any journal size. The full
  replay still rebuilds the window from the journal (an entry older than the window contributes
  no key).

### 5.7 Sizing

| journal | chunks | checkpoint parts (idempotency + reverse map) | mutable body | cold boot reads |
|---|---|---|---|---|
| 100 000 entries | ~1 500 | ~13 MB in ~4 parts | O(records) | head + parts + tail chunks (≤ `CHECKPOINT_EVERY_ENTRIES` worth) |
| 1 000 000 entries | ~15 000 | ~130 MB in ~35 parts | O(records) | same |

The parts are read and parsed at boot too — they are the entries, compacted — so the checkpoint
boot is not free: it trades typed deserialization of full bodies plus three inspections for one
parse of a compact index. The measured per-entry costs in §2 give the expected ratio (F1).

## 6. Recommendation

**Do not start with the checkpoint. Start with parsing the journal once (J1).** Half of what a
checkpoint would save is available with no new persisted format, no new trust surface and no
change to the money-safety story: at 100 000 entries J1 takes the replay from ~4.4–7.0 s to an
estimated ~2.2–3.5 s (inspection 2 gone, inspection 1 reduced to one deserialization, the third
chunk parse gone; the typing and the fold stay). J1 also makes the number cheap to produce (the
synthetic seeder) and fixes the line (§4) so the next decision is taken against a measurement
that includes the fetch.

**Then decide the checkpoint on growth, not on 100 000.** The cost model, per entry, with the
entries at the 5 s line each stage implies:

| stage | replay cost per entry | entries at 5 s | notes |
|---|---:|---:|---|
| today | 44–70 µs | 70 000–110 000 | plus the fetch (F3) |
| after J1 | ~22–35 µs | ~140 000–230 000 | no format change |
| after J1 + J2 | ~18–30 µs | ~170 000–280 000 | RAM bounded; the tail's consumers off-loop |
| after J3 (checkpoint, full fetch) | ~9 µs (fetch parse 5.5 + parts ~3) + tail | ~500 000 | the fetch round trips dominate on a remote store |
| after J4 (narrow fetch) | ~3 µs (parts) + tail | ~1 200 000–1 600 000 | bounded by the idempotency index (F5); Q5 moves it |

> *Superseded for the J1 rows by the measurement in §7's J1 as-built block (2026-09-06): after
> J1 the replay is ~3.3 µs per entry and the 5 s line is ~1.5 million entries.*

With G entries a day, the line is `entries at 5 s ÷ G` days away. At a few thousand entries a day
J1 alone buys years; at fifty thousand a day the checkpoint is due within months of J1 and J2–J4
should be planned now. The owner has the G (Q1).

**If the checkpoint is taken:** J2 → J3 → J4, in that order, with Q2 and Q5 answered first. J2
is worth taking on its own for RAM (~600 MB retained at 1 000 000 entries) whatever happens to
J3. J3 without J4 is an intermediate that already halves the boot; J4 is what makes 1 000 000
entries boot in seconds and it is where the loader and the store change.

**Keep the entry line honest either way:** derive `CHECKPOINT_REVIEW_ENTRIES` from the measured
per-entry cost after each cut (§4) instead of leaving a number that today's code cannot reach
inside the time line by an order of magnitude.

> **Owner decisions of 2026-09-05 (§9)** set a design rate of 250 entries a minute and a 1 GB RAM
> budget, which make J2 mandatory, add streaming to J1, and fold the windowed idempotency index
> (§5.8) into J3. The order above stands; the reasons are stronger.

## 7. Cuts

Each cut lands under the house discipline (confirm by symbol, build, clippy `-D warnings`,
workspace tests, fmt, an adversarial verify with the named lenses, findings folded as tests, the
as-built block recorded, the verify outcome in the commit message).

**J1 — parse once (and make the number cheap).** `mush-core`: `inspect_history` returns the
parsed chunks it built, `scan_contiguity` and `Ledger::restore_from_kv` share one inspection
(a `LedgerHistory` value the caller passes to both), the boot-side byte counting checks the raw
document length instead of re-serializing every entry, and no chunk `Value` is cloned (deserialize
through `&Value`). `mush-economy`: `load_boot` consumes the parsed chunks and types each entry
once. Under D-12-4 the parse **streams**: the loader hands ledger chunks to the module as text
(or typed), chunk by chunk, and drops each after typing, so the `Value` tree of F9 never exists
whole — this is the part of J1 that touches `ModuleBootData`. J1 also adds the journal rate
metric and the `@stats` projection of D-12-1 and the `fetch_ms` number of D-12-3.
`mush-contract-tests`: the synthetic seeder landed with this review; `SCALE_JOURNAL_ENTRIES` on
the engine-driven seeder stays as the shape check at a small default. `CHECKPOINT_REVIEW_ENTRIES`
is re-derived from the measured per-entry cost so the two lines agree (§4). *Lenses:* **E** (every breach kind
`inspect_history` reports is still reported from the shared inspection — the existing
`ledger.rs` breach tests run unchanged), **R** (restore semantics: sealed-tail-becomes-empty-open
and the unsealed-tail byte count are byte-identical to today), **M** (the harness numbers
before/after, same box, same profile). No persisted format changes. Size: one to two days.

> **J1 as built — 2026-09-06.**
>
> 1. **The ledger's boot form is text.** `ModuleBootData` gained `ledger: Vec<LedgerDoc>`
>    (`(key, raw JSON text)` for every `log:`-prefixed key) beside `kv`, and `fetch_ms`; the
>    store gained `ObjectStore::module_docs` (a split listing with a default derived from
>    `module_json_list`; Redis fetches ledger text through `RawValue` without a decode, the
>    memory store hands over the text it holds), and `load_one_namespace` times the fetch. A
>    rig that still puts `log:` values in `kv` is served by `ModuleBootData::normalize`, which
>    the engine runs before every `on_world_loaded` — no test rig changed shape.
> 2. **One inspection.** `inspect_ledger(docs, target) -> LedgerHistory` decodes each chunk's
>    facts (index, seal, sequence numbers) from its text with the payloads skipped
>    (`IgnoredAny`), walks the contiguity rules unchanged, and decodes only each log's
>    **unsealed tail** in full. `Ledger::inspect`, `restore_from_history`,
>    `restore_logs_from_history(keep)` and the text forms of `scan_contiguity`,
>    `restore_from_kv`, `restore_from_kv_lossy` and `recent_sealed` sit on it; `typed_entries::<T>`
>    is the consumer's one typed decode from text (`LedgerChunkOf<T>`, strict). `EntryTooLarge`
>    is no longer produced on the read side — a chunk under the cap bounds its entries — and the
>    chunk cap is judged on the stored text's length. The seal rule for an open tail is applied by
>    the **restoring** ledger's target whatever target the inspection was made under, so a scan
>    made with `None` and a restore under a target agree with the old two-pass behaviour exactly.
> 3. **The economy types once and retains by move.** `load_boot` inspects, restores from the
>    inspection, walks `chunk_docs("journal")` decoding each chunk straight to
>    `EconomicTransaction` and dropping its text, and the fold moves each record into the
>    retained map instead of cloning it. Scenes validate every log and restore live tails from
>    the same inspection; canon, org, sim and the toy module switched to the text form.
> 4. **The numbers the decision keys on.** `EconomyStore::posts_since_boot` and `journal_len`;
>    `ReplayStats.fetch_ms`; the world sample carries `mush_economy_journal_entries`,
>    `mush_economy_journal_per_hour`, `mush_economy_replay_us_per_entry`, `mush_economy_replay_ms`,
>    `mush_economy_fetch_ms`; `@stats` renders the `Economy journal:` line with the entry line
>    derived from the measured cost and the projected days to it. The harness's phase test
>    reports `inspect | restore | typed | boot_total | us_per_entry | entries_at_5s`.
>
> **Measured (M), release profile, the same box and seeder as §2:**
>
> | entries | chunks | journal JSON | inspect | restore | typed | **boot_total** | µs / entry | entries at 5 s |
> |---:|---:|---:|---:|---:|---:|---:|---:|---:|
> | 25 000 | 881 | 14.0 MB | 13 | 0 | 39 | **74** | 3.0 | ~1 700 000 |
> | 50 000 | 1 774 | 28.0 MB | 30 | 0 | 77 | **156** | 3.1 | ~1 600 000 |
> | 100 000 | 3 560 | 56.1 MB | 63 | 0 | 148 | **313** | 3.1 | ~1 600 000 |
> | 200 000 | 7 131 | 112.7 MB | 105 | 0 | 282 | **652** | 3.3 | ~1 500 000 |
>
> Against §2's 4 405–5 210 ms at 100 000 entries (H3's engine-seeded run: 6 998 ms) that is
> **14–22× faster**, linear through 200 000, and it puts the 5 s line at **~1.5 million
> entries** — not the ~140 000–230 000 §6 and §9 estimated for J1. The estimate assumed the
> typed decode would still cost what it cost from a `Value` (4 µs per entry) and that the fetch
> side's text→`Value` parse (5.5 µs) would stay; in fact both were the same expense — building
> and walking `Value` trees — and going from text straight to typed records removed it
> everywhere. Of the 3.3 µs per entry now: inspection ≈ 0.5 (payloads skipped), typed decode
> ≈ 1.4, the fold, the reconciliation and retention ≈ 1.3. The restore is one chunk.
>
> **The lines after J1.** `CHECKPOINT_REVIEW_SECS` stays 5 s. The derived entry line is
> ~1.5 million; `CHECKPOINT_REVIEW_ENTRIES` **stays at 1 000 000** as the more conservative of
> the two, now consistent with the time line for the first time (a million entries replay in
> ~3.3 s). At the §9 design rate (250 a minute) the million is ~2.8 days of journal and the 5 s
> line ~4 days — but D-12-4's RAM lines bind first and unchanged (retained transactions at
> ~1.7 million; the boot transient is now text plus typed, ~1.2 KB per entry, 1 GB near
> 800 000 entries), which is why J2 is next. §6's and §9's "after J1" rows are superseded by
> this table; their "after J3/J4" rows stand, since the checkpoint's payoff was always the
> idempotency index and the fetch, not the decode.
>
> **Verify (lenses E, R, M).** E: every `BreachKind` but `EntryTooLarge` is produced by the new
> inspection and the existing breach tests run unchanged over the text form; new tests cover the
> shared inspection's order and counts, the strict typed decode, the chunk cap on text, and
> `recent_sealed` over text. R: the one-shot restore and the restore-from-history are equal
> (`Ledger: PartialEq`), the seal rule follows the restoring ledger, a filtered restore keeps only
> the selected logs and still refuses on a breach elsewhere; the economy boots to the same state
> from text as from values (`e2b::j1_boot_from_ledger_text_equals_boot_from_values`); the store
> suites prove `module_docs` and `module_json_list` agree on both backends. M: the table above.
> Findings folded: **J1-F1** the literal sweep that added `..Default::default()` to every
> `ModuleBootData { kv, streams }` rig also rewrote one fn returning `ModuleBootData` and missed two
> literals with nested braces — caught by clippy over all targets, fixed by brace matching;
> **J1-F2** clippy's `manual_checked_div` on the harness's entry-line derivation.

**J2 — the bounded transaction tail.** `EconomyStore.transactions` becomes a tail bounded by
window and count (§5.1); the reverse map and `created_count` are folded at boot and maintained by
`apply`; the seven consumers are re-pointed; account history past the tail pages through the
store-read seam (H1). *Lenses:* **C** (every consumer answers identically for entries inside the
tail; the contract boot pass and `title_generation` answer identically for entries outside it —
tested by booting the same journal with the tail at 10 and at ∞), **B** (eviction never runs on
the post path; the window floor holds under a clock that jumps), **P** (`@account/history`
past the tail is a page-bounded off-loop read, the world loop never blocking). RAM measured
before/after at 100 000 entries. Independently valuable whatever happens to J3. Size: two to three
days.

> **J2 as built — 2026-09-06.**
>
> 1. **The tail.** `EconomyStore.transactions` is the newest `tx_tail_max` bodies (`[economy]
>    tx_tail_max`, default `DEFAULT_TX_TAIL_MAX` = 20 000, ~300 chunks). A post only appends;
>    the sweep evicts oldest-first down to the limit (`evict_tail`, beside the debit-window
>    prune). Boot retains only the newest `tx_tail_max` while folding every entry, and the
>    typed pass now **streams**: each chunk is decoded and folded at once and its text dropped,
>    so no more than one chunk's records exist outside the tail at any moment (the rest of F9).
> 2. **The index.** `tx_index: id → TxRef { seq, idempotency, posted_at }` for every transaction
>    ever posted, `created_count: object → n` (the title generation), and `chunk_firsts: chunk
>    → first seq` (what a cold read by sequence needs) — folded at boot, maintained by
>    `index_transaction` and the post path's `note_chunk`. Consumers re-pointed: the boot
>    contract pass (`finished_transition` and the settled-claim check read the index),
>    `title_generation` (the counter), the sweep's republish filter (`posted_at` from the index),
>    `econ_audit` and the warm half of `@econ/history` (the tail), `emit_post_events` and the
>    hold-id lookups after a post (a just-posted body is always in the tail).
> 3. **Past the tail, through the seam.** `@econ/history` renders the tail's rows at once and,
>    when they did not fill the request and the journal goes further back, reads the sealed
>    chunks below the tail off the world loop (`Ctx::read_store`, `DEFAULT_ARCHIVE_PAGE_CHUNKS`
>    per page, at most `HISTORY_MAX_PAGES` = 8 pages per command) and renders the rest through
>    the same rows via `EconomyArchiveRead`, the economy's `StoreReadCompletion`. `@econ/reverse`
>    of a body that left the tail (`EconomyError::TransactionNotInMemory`, distinct from
>    `NoSuchTransaction`) reads the chunk the index names and reverses on re-entry through
>    `api::reverse_original`, which checks the fetched record against the index (seq and
>    idempotency) before planning; standing is re-checked in the completion, one read per
>    actor is in flight, a stale completion is dropped. An absent chunk is reported, never
>    papered over (J5's purge will only ever remove chunks the archive holds).
> 4. **J2-D1 — the debit window's horizon.** `rebuild_debit_window` folds from the tail; when the
>    tail starts after the window floor it says so at `warn` and older spend is not counted. An
>    approval window is a spend control, not a conservation rule; at the §9 design rate the
>    90-day floor is 32 million entries and no RAM tail covers it. §5.1's "the tail must cover
>    the window" row is superseded by this decision, which the owner should confirm.
>
> **Measured (M), release profile.** The economy booted **from a file** of split boot data (the
> production loader's shape: ledger chunks as text) in a process that did nothing else
> (`scale_cold_journal_replay_from_file`, `SCALE_JOURNAL_FILE`, sampled from outside for its peak
> working set), with the tail at the default and at "unbounded" (`SCALE_TX_TAIL_MAX` past the
> journal's length — what J1 retained):
>
> | entries | tail | peak working set | replay |
> |---:|---:|---:|---:|
> | 100 000 | 20 000 (bounded) | **126 MB** | 309 ms |
> | 100 000 | 100 000 (unbounded) | 150 MB | 301 ms |
> | 200 000 | 20 000 (bounded) | **247 MB** | 537 ms |
> | 200 000 | 200 000 (unbounded) | 291 MB | 971 ms |
>
> Retention measured at **~0.25 KB per body** (24 MB over 80 000 evicted bodies; 44 MB over
> 180 000) on two-line mints — below the ~0.6 KB the review estimated from `size_of` plus heap,
> because the peak counts pages, not live bytes, and a mint's heap is small; a settlement with
> effects is bigger. The rest of the peak is the process holding the journal's text twice (the
> file it read and the parsed chunk strings, ~1.2 KB per entry together) — in production the store
> hands the text over once, so the boot transient is about half of that, and D-12-4's 1 GB line
> for a full-replay boot moves out to ~1.5 million entries. The bounded fold is also faster at
> 200 000 (537 vs 971 ms: fewer map inserts, less allocation). The at-rest working set could not
> be sampled reliably from outside (the process exits as its world drops); the peak is the number.
>
> **Verify (lenses C, B, P).** C: `e4b2::j2_every_contract_state_survives_a_reboot_with_an_evicted_tail`
> (every state, the tail capped at one body, the settlement resolved through the index);
> `e4c::j2_a_reissue_after_the_tail_evicted_the_first_title_mints_the_next_generation`;
> `e2b::j2_the_tail_is_bounded_by_the_sweep_and_the_index_outlives_it` (reboot under the limit:
> same tail, same index, same balances). B: the same test — six posts leave six bodies until the
> sweep runs, then three; the index keeps all six ids and their chunk. P: `@econ/history` past
> the tail is one `StoreRead` effect, rows rendered on the completion newest-first and capped;
> `@econ/reverse` past the tail is one effect, the reversal posted on re-entry, a replayed
> completion posts nothing, an unknown id refused with no read. Findings folded: **J2-F1** —
> the first cold history page rendered the tail's two rows again, because the open chunk holds
> the tail's bodies too; the completion now renders only entries below the tail floor captured
> when the command ran, and the test asserts the rows are not repeated. **J2-F2** — the replay
> stats counted the RAM tail (`transactions().count()`), so with the tail bounded the C17 line,
> the per-entry cost and the harness all read 20 000 whatever the journal held; they count
> `journal_len` now, and the tail test asserts it.

**J3 — the checkpoint documents and the checkpoint boot.** §5.2–§5.5 and §5.8: the head, the
mutable body, the immutable parts (now the window's keys only), the `idempotency:` point docs
with their one-time backfill and the off-loop cold check, the sweep-time writer with the
write-time drift check, GC, and `load_boot` seeding from a validated checkpoint with the tail
replay — **with the full fetch still in place** (the cold chunks arrive and are ignored; the
loader is untouched). *Lenses:*
**V** (validation: a head with a missing part, a part with a wrong hash, a wrong `state_hash`, a
`covered_seq` past the journal head, an unsealed `covered_chunk` — each is ignored and the boot
full-replays, never fails, never freezes), **D** (drift: every existing boot drift/ahead/B5 test
re-run from a checkpoint, plus the §5.5 cases: a document at head as of the checkpoint that
drifts is caught; a pre-checkpoint lagging document heals and logs), **I** (idempotency: a key
posted before the checkpoint is refused after a checkpoint boot; a key reused with a different
plan is refused), **T** (torn writes: crash between parts and head; crash mid-GC), **F** (frozen
economy checkpointed and booted stays frozen). Size: four to five days.

> **J3 as built — 2026-09-06.** Built after J1, J2 and J5, which changed what a checkpoint is
> for: with the replay at ~3.3 µs per entry, the tail bounded and cold chunks archived, the
> checkpoint's remaining value is the boot **skipping every covered chunk's decode**, the
> runtime **drift shadow** it needs, and the trust model its documents carry — not the boot
> time the §6/§9 rows once forecast. §5.8's windowed idempotency index is **not** in this cut
> (J3-D1, below).
>
> 1. **The shared fold** (`mush_economy::fold::FoldState`): every accumulator the boot replay
>    derives from the journal — running balances and held amounts, last sequence and touch
>    count per account, title and reservation post-states, terminal times, custody marks,
>    consumed approvals, the reversed set, the title generation, the allocator high-water
>    marks — as one value folded by one function, `fold_entry`, which boot replays with and
>    `index_transaction` calls on every post. The store's three journal-derived maps moved into
>    it. The live store therefore carries a **journal-authority shadow** of its projections.
> 2. **The documents** (`mush_economy::checkpoint`): the head `checkpoint:{seq}` (format,
>    covered sequence and chunk, written-at, the part list with each part's SHA-256 and
>    sequence range, the `state_hash`, the body: the fold plus the debit-window tails and the
>    chunk-to-first-sequence map) and the immutable parts `checkpoint:{seq}:idem:{n}` (the
>    idempotency records and transaction refs for one range, ≤ 25 000 rows each). A later
>    checkpoint re-lists the parts it inherits and writes only the new range. Hashes are over
>    canonical JSON, domain-separated.
> 3. **The writer** runs from the sweep after GC, every `checkpoint_every_entries` past the last
>    covered sequence, never on the post path. **The write-time drift check** compares every
>    account, title and reservation projection with the shadow first; a mismatch writes nothing,
>    says so at `error`, and **freezes the economy** — boot's money-safety rule at runtime, a
>    property the boot-only fold never gave. Parts are persisted before the head; two heads are
>    kept and the next write sweeps what neither names. Nothing is written while frozen or
>    with the cadence at 0.
> 4. **The boot** selects the newest checkpoint whose key agrees with its covered sequence,
>    whose sequence is at or below the journal head, whose covered chunk exists, whose parts
>    are all present and hash to the head, decode and cover what they claim, and whose state
>    hash recomputes; every rejected one is said at `error` and ignored. It seeds the fold,
>    the idempotency index, the transaction refs, the reversed set and the two maps, decodes
>    **no chunk below the covered one except those holding the RAM tail** (J3-F4), keeps the
>    tail's bodies without folding them, skips every other entry at or below the covered
>    sequence, and folds the rest. `ReplayStats` gained `from_checkpoint`, `replayed` and
>    `checkpoint_ms`.
> 5. **§5.5 as decided (D-12-2):** a document claiming a sequence at or before the covered one
>    is judged when that sequence is the last the fold touched for it (the checkpoint's value is
>    the value at the claim), and otherwise lags — healed unverified, at `info`, exempt from the
>    drift and "never appears at" verdicts. Drift at the head still freezes.
> 6. **J3-D1 — the windowed idempotency index (§5.8) is deferred to its own cut.** Building it
>    exposed a hole the review glossed over: a post's idempotency check is synchronous on the
>    world task, and a window cannot tell a never-used key from a key used before the window —
>    so either every post with a fresh key pays an off-loop store check (a re-entry the dozens
>    of `post` callers do not have), or a probabilistic fast-negative (a Bloom filter over every
>    key, carried by the checkpoint) sits in front of the cold check. The boot's contract heal
>    (`finished_transition`) also reads keys that may be older than any window. That needs its
>    own design page and the owner's eye; this cut carries the whole index in the parts, so J3's
>    RAM story is J2's, and D-12-4's ~7 million-entry line for the index stands. *Decided the
>    same day: D-12-6, a Bloom filter gate on the slow lookup.*
> 7. **J3-D2:** the covered sequence is the journal head at write time, not a chunk boundary
>    — the boot skips whole chunks below the covered chunk and entries at or below the covered
>    sequence within it, which is simpler than aligning writes to seals and loses nothing.
>
> **Measured (lens M).** The scale harness's phases test grew a checkpointed boot beside the
> full replay (`SCALE_CHECKPOINT_EVERY`): boot once, let the sweep write a checkpoint at the
> head, overlay its documents, boot again. Release build, this dev box, 100 accounts:
>
> | entries | chunks | full boot | of which typed decode | checkpointed boot | of which select + seed | replayed |
> |---:|---:|---:|---:|---:|---:|---:|
> | 25 000 | 881 | 95 ms | 39 ms | 127 ms | 64 ms | 0 |
> | 100 000 | 3 560 | 307 ms | 159 ms | 326 ms | 204 ms | 0 |
>
> (The default 20 000-entry RAM tail is decoded either way — J3-F4 — which is most of the
> 25 000-entry journal.) **The checkpoint as built is boot-time neutral to slightly worse.** It
> skips the typed decode of every covered chunk outside the tail and pays about the same to
> decode and hash the parts, because the parts carry **one row per journal entry** (the whole
> idempotency index, per D-12-4 and J3-D1) — the checkpoint moved the per-entry cost from the
> journal to its own documents, and the inspection pass (the text decode of every chunk, ~59 ms
> at 100 000) is paid either way. The boot-time and RAM win
> the review forecast lives entirely in §5.8's window, deferred (J3-D1). What this cut delivers
> on its own is the runtime drift shadow, the write-time freeze, and the checkpoint documents'
> trust model. `ReplayStats` and the boot log line carry `checkpoint_ms` so the split stays
> visible.
>
> **J3-F2 (owner-facing, pre-existing).** The measurement first booted a *frozen* economy: the
> synthetic journal's documents were captured before the template mint, so the faucet and
> account 1 carried `last_journal_seq == 0` — the never-touched sentinel — while the journal's
> entry 0 touched them, and the boot read that as drift at seq 0 rather than lag. Sequence 0 is
> both a real journal sequence and the sentinel, so a document lost between the **first**
> journal append and its own write reads as drift and freezes instead of healing. A one-entry
> exposure, untouched by J3, and the J1/J2 measurements ran on that frozen boot without
> noticing (their timings were unaffected). The fix is a 1-based journal or an `Option` on the
> anchor, both schema changes — flagged, not taken here. The harness now captures the documents
> as the mint left them.
>
> **Verify (lenses V, D, I, T, F).** V: a missing part, a tampered part, a wrong state hash, a
> head not covering what its key says — each ignored, the journal replayed whole, unfrozen,
> balances right; a garbage newer head does not shadow a valid older one. D: drift at the
> claimed head sequence still freezes after a checkpoint boot; a pre-checkpoint lag heals
> without a verdict; the write-time check names every disagreement (unit). I: a key posted
> before the checkpoint is refused as already posted after a checkpoint boot (a test-only
> `@testpost` command posts under a chosen key); a fresh key posts. T: parts with no head and a
> head with no parts are both ignored. F: a frozen economy writes no checkpoint; a zero cadence
> writes none. The round trip: the sweep writes at its cadence, a boot seeds and replays only
> the tail, a later checkpoint inherits the earlier part, and the third write sweeps the first
> head while keeping the part it introduced. Findings folded: **J3-F1** — the round-trip test
> first expected a checkpoint from fewer new entries than the cadence; the cadence is what the
> test now asserts.
>
> **The cadence-1 pass (lens C).** The default cadence means no ordinary test ever reaches the
> writer, so the write-time drift check and the checkpointed boot were exercised by the five
> J3 tests alone. The whole workspace was therefore run once with the cadence forced to 1 —
> every sweep in every economy-touching test writes a checkpoint and every reboot seeds from
> one. Twelve tests failed, from two causes, both folded:
>
> - **J3-F3 — the sentinel at the checkpoint.** The §5.5 rule excluded every claim of sequence
>   0 as the never-touched sentinel, so an account whose only touch was the journal's first
>   entry was left to the head verdict, which read its balance against nothing and froze; and a
>   hold or asset the rule could not place lagged where B5 should have frozen. The fold's last
>   touch now decides what the stored sequence alone cannot: a claim at the touch is judged at
>   the value (a real first touch at 0 included), a touch past the claim lags (the sentinel on
>   an account the journal did touch included), and no touch or an earlier one is left to the
>   verdicts — an untouched sentinel is nothing, a claim the journal never made is still B5.
> - **J3-F4 — the empty tail.** A checkpointed boot decoded nothing below the covered chunk, so
>   J2's RAM tail came up empty: every history page and reversal went cold, and the boot heals
>   that read a transaction's body (a settlement's stamp, a lost transition) found none. The
>   boot now decodes from the chunk holding the tail's floor even below the checkpoint's, keeping
>   those entries' bodies without folding them; the tail is full after any boot.
>
> With both folded the cadence-1 pass is clean across the workspace, and it is the run to
> repeat whenever the fold, the writer or the boot rule changes.

**J4 — the narrow fetch.** §5.6: the boot manifest, `ModuleBootData.omitted`, the min-present
marker through `scan_contiguity`/`restore_from_kv`, the `RetryFullFetch` path, and the pipelined
`JSON.MGET` (which every module gets). *Lenses:* **G** (a manifest naming a chunk the store no
longer has; a manifest with no checkpoint; an invalid checkpoint under a manifest → one retry,
full replay, and the manifest rewritten), **S** (the store suites: `module_json_list` with and
without omission returns the same documents for the kept keys, both backends). Size: two days,
plus the Redis suite time.

> **J4 as built — 2026-09-06.** §5.6 as designed, with the retry made a typed error and the
> omission a marker rather than a key list.
>
> 1. **The manifest** (`mush_core::BootManifest` under `BOOT_MANIFEST_KEY`, `boot-manifest`):
>    `{ format: 1, skip_log_chunks_below: { "journal": k } }`. The economy writes it beside every
>    head, with *k* the chunk holding the RAM tail's floor at that head (`covered_seq + 1 −
>    tx_tail_max`), so everything below is both covered and outside the tail. Loader-owned
>    protocol: the loader reads it first, honours a format it knows (an unreadable or unknown
>    one is a whole fetch at `warn`), and strips it from the module's documents.
> 2. **The narrowed listing** (`ObjectStore::module_docs_from(ns, &floors)`): a chunk below its
>    log's floor is neither fetched from the primary nor merged from the archive (J5's
>    `merge_archived` takes the floors), and the floors come back in `ModuleDocs::omitted`. Both
>    backends skip the fetch itself; the trait default lists whole and drops. **The Redis fetch
>    is now pipelined**: one `JSON.MGET` per page of 256 keys instead of one `JSON.GET` per key,
>    for every module's boot; a page that trips on a non-JSON key falls back to the per-key
>    path, which still classifies each key positively before skipping it.
> 3. **The marker** (`ModuleBootData.omitted`, log id → chunks below not fetched) reaches the
>    inspection as `ChunkFloors`: `inspect_ledger_from` / `scan_contiguity_from` /
>    `Ledger::inspect_from`. Below a floor absence is not a hole; the floor's own chunk must be
>    present when a higher one is (a manifest naming a chunk the store no longer has is a
>    `MissingChunk` at the floor); the sequence run starts at the first present chunk; a floor
>    permits absence and never demands it. The restore is unchanged — it only reads the tail.
> 4. **The retry** (`ModError::RetryFullFetch { module, reason }`): a module handed a narrowed
>    namespace it cannot boot from returns it; `Engine::module_loaded` delivers one module, and
>    `boot_modules` walks the modules in order, and on the retry **clears the manifest** (it is
>    the loader's, and it stood for nothing), fetches that namespace whole, reloads its streams,
>    and delivers it once more to that module alone; a second refusal fails the boot with the
>    module's reason. The economy returns the retry when the fetch was narrowed and no valid
>    checkpoint's covered chunk reaches the floor — the head invalid, gone, or below it.
> 5. **J4-D1:** the omission is a marker, not the omitted keys — the ledger only needs the floor,
>    and a key list would be thousands of strings for nothing. **J4-D2:** a floor above the RAM
>    tail's chunk (the tail lengthened by config since the manifest was written) shortens the
>    tail until posts refill it, at `info`; never a retry — the money is all there.
>
> **Measured (lens M).** The scale harness's checkpointed boot grew a narrowed twin: the same
> boot with the chunks below the manifest's floor never handed over. Release build, this dev
> box quiet, 100 accounts, the default 20 000-entry tail:
>
> | entries | chunks | full boot | checkpointed boot | narrowed boot | floor | chunks handed over |
> |---:|---:|---:|---:|---:|---:|---:|
> | 25 000 | 881 | 81 ms | 104 ms | 115 ms | 172 | 709 |
> | 100 000 | 3 560 | 327 ms | 334 ms | 269 ms | 2 844 | 716 |
>
> At 100 000 entries the narrowed boot hands over a fifth of the chunks and is the first boot
> shape faster than a full replay (269 vs 327 ms): what it saves is the inspection's text
> decode of the 2 844 omitted chunks. At 25 000 the tail is most of the journal, the floor
> omits a fifth, and the three shapes are within noise of each other. **The store-side saving
> is the point of J4 and is not measurable in process**: on Redis the narrowed listing issues
> no `JSON.MGET` for the omitted keys at all, and every listing is now one round trip per 256
> keys rather than one per key; that number needs the Redis suite on a real instance (D-12-3).
>
> **Verify (lenses G, S, L, E, C).** G: through `boot_modules` over the in-memory store, a
> module that refuses its narrowed namespace is delivered the whole one, itself alone, the
> manifest is cleared, and the boot goes on; a module that refuses the whole one too fails the
> boot with its reason after exactly one retry. In the economy: a tampered head under a floor,
> no checkpoint documents under a floor, and a floor above the covered chunk (the manifest
> naming a chunk the store no longer has) each answer the retry, and the same store fetched
> whole boots by a full replay with the money right; a floor at the covered chunk boots with a
> shorter tail (J4-D2). S: the store suite's narrowed listing returns the whole listing's text
> for every key it keeps, lists nothing below a floor, leaves other logs and the ordinary
> documents alone, echoes the floors, and narrows nothing on empty floors — the in-memory
> backend on this box; the Redis suite is `REDIS_URL`-gated and was **not** run here. L: the
> ledger accepts an omitted prefix and restores the tail past it, still flags the floor's own
> chunk missing when a higher one exists, a gap above the floor, and a sequence gap above it,
> inspects a log the floors do not name whole, and takes a present chunk below a floor as
> present. E: end to end through the driver — the sweep writes head and manifest; a fresh
> engine boots narrowed from the checkpoint with the tail full; the head tampered, the next
> boot recovers whole with the manifest cleared; the recovered world's next checkpoint writes
> a fresh manifest and the boot after it is narrowed again. C: the cadence-1 workspace pass
> repeated for the boot rule's change. Findings folded: **J4-F1** — the RAM tail's floor was
> computed from the inspection's entry count, which under a narrowed fetch counts only the
> present chunks, so a narrowed boot retained every present body (31 for a tail of 3); the
> journal's length is the restored tail's next sequence, and the floor comes from that.

J1 is the first cut. The order after the owner's decisions (§9) is J2 → J5 (§10.6, the
archive tier) → J3 → J4; J2 and J5 can each be taken alone. *All five are built (§7's as-built
blocks); the windowed index (D-12-6) remains, as its own cut.*

## 8. Open questions for the owner

1. **Growth rate.** How many journal entries a day does the game post at the scale you plan for?
   The line in §4 is a date once that number exists; without it the review can only say "at 100k".
2. **The drift verdict for pre-checkpoint documents (§5.5).** Is healing a lagging document
   older than the checkpoint *without* a drift or B5 verdict acceptable, given the write-time
   check? If not, the checkpoint is off the table and only J1/J2 remain.
3. **What the 5 s line means.** Replay only (today's timer), or process start to spending
   authority (fetch and text parse included)? §4 argues for the latter and a timer that says both.
4. **RAM budget for the economy.** The bounded tail's ceiling (`TX_TAIL_MAX`) and whether
   ~600 B per retained transaction at 1 000 000 entries (~600 MB) is the number that decides J2
   regardless of boot time.
5. **Idempotency key retention.** R-B2 keeps keys forever; the checkpoint's parts are ~130 B per
   entry because of it. A bounded retention (say, keys older than a year may be re-used) would
   shrink the checkpoint to O(records) and is a rules change, not an engineering one.

## 9. Owner decisions (2026-09-05) and what they change

The owner answered §8 the same day. Recorded as decisions with the arithmetic they imply.

**D-12-1 — Growth rate: unknown, so a design rate and a measured one.** The optimistic edge the
owner drew — 1 000 players, half online, five entries a minute each — is 2 500 entries a minute,
3.6 million a day, 2 GB of journal a day: beyond every line in this review and beyond a VPS's
Redis in days (F10). It is a ceiling, not a design point. The review therefore takes **250
entries a minute** (a tenth of the edge; 360 000 a day) as the design rate, keeps the edge as the
stress case, and makes the real rate observable: J1 adds `mush_economy_journal_entries_total` and
an `@stats` line with the current entries-per-minute and the projected days to the 5 s line at
the measured per-entry cost. Time to each line, at three rates:

| stage | entries at 5 s | 2 500/min (edge) | 250/min (design) | 25/min |
|---|---:|---:|---:|---:|
| today | 70k–110k | 28–44 min | 5–7 h | 2–3 days |
| after J1 | 140k–230k | 1–1.5 h | 9–15 h | 4–6 days |
| after J1 + J2 | 170k–280k | 1–2 h | 11–19 h | 5–8 days |
| after J3 (checkpoint) | ~500k | 3 h | 33 h | 14 days |
| after J4 (narrow fetch), index O(entries) | 1.2M–1.6M | 8–11 h | 3–4 days | 33–44 days |
| after J4 with §5.8's window | no entry line (O(records + window)) | — | — | — |

*The "after J1" rows above were estimates; J1 as built (§7, 2026-09-06) measured ~3.3 µs per
entry, a 5 s line of ~1.5 million entries — 10 hours at the edge, 4 days at the design rate,
40 days at 25 a minute — and leaves D-12-4's RAM lines as the binding ones.*

**D-12-2 — Healing a lagging document older than the checkpoint without a drift verdict is
acceptable**, with §5.3's write-time check as the condition. §5.5 stands as designed.

**D-12-3 — The 5 s line means replay with the store local to the server**; it was replay-only
originally and stays so as the spec. The fetch and text-parse time is logged as a second number
(`fetch_ms`, from `load_one_namespace`) beside `replay_ms` so a run on the VPS shows both, and a
separate line for it can be drawn when there is a reason to. The CI `scale` job runs on GitHub
runners; the VPS number comes from running the harness on the VPS by hand
(`SCALE_PROFILE_SCALES=… cargo test --release … scale_cold_journal_replay_phases`).

**D-12-4 — The economy's RAM budget is 1 GB.** Three structures grow with entries; the budget
dates each:

| structure | bytes per entry | 1 GB at | at 250/min | at 25/min |
|---|---:|---:|---:|---:|
| retained transactions (F4) | ~600 | ~1.7M entries | 5 days | 47 days |
| the idempotency index in RAM (F5) | ~150 | ~7M entries | 19 days | 190 days |
| the journal's `Value` tree during a full-replay boot (F9) | ~3 000 | ~330k entries | 22 h | 9 days |

So **J2 is mandatory on the budget alone**, whatever the boot time; J1's parse must stream text to
typed records (F9), or J4 must land, before the journal passes ~330 000 entries; and the RAM
idempotency index must be bounded (D-12-5) before ~7 000 000.

**D-12-5 — Idempotency keys are retained forever** (the owner's reason: consistency with anything
that still points at old accounts; the review adds the stronger one — a same-plan retry after any
interval must not post twice). Retention is a semantics the review keeps; the O(entries) **RAM**
index is a mechanism it replaces with §5.8 (a window in RAM, every key on disk, the cold check
off-loop). "Until we observe it trending towards an issue" is honoured by D-12-1's metric — at the
design rate the trend is visible in weeks, which is why the mechanism is designed now and cut
when the metric says so.

**Revised order.** J1 (parse once, streaming; the rate metric; `fetch_ms`) → J2 (the bounded
tail; mandatory on D-12-4) → **J5 (the ledger archive tier, §10)** → J3 with §5.8 folded in
(the checkpoint carries the window; the `idempotency:` docs and their backfill) → J4 (the narrow
fetch). After J4 the boot is O(records + window); after J5 the journal's residence in Redis
(F10) is bounded too. J5 goes before the checkpoint because the store running out of memory is
an outage, where a slow boot is a slow restart. *As built, J3 landed without §5.8 (J3-D1); the
window is its own cut after J4, under D-12-6.*

**D-12-6 — The §5.8 window is gated by a Bloom filter on the slow lookup (owner, 2026-09-06).**
Resolves J3-D1. §5.8 as written could not tell a never-used key from a key used before the
window, so every fresh post would have paid the off-loop store check. The owner's direction: the
RAM index keeps the window's keys; a **Bloom filter over every key ever posted** rides in the
checkpoint body, is maintained on every post and rebuilt by a full replay; a post's key is looked
up in RAM first, and a miss is put to the filter — a **negative is certain**, so the post proceeds
synchronously as today, and only a positive outside the window gates the slow lookup, the
off-loop read of the key's `idempotency:{namespace}:{key_hash}` document. Sizing: ~20 bits a key
(about 2.5 MB per million keys, a false-positive rate near 1 in 10 000), a 30-day window to start.
The review's default for the rare positive is a **transient refusal** ("try again shortly") while
the off-loop read warms the RAM index with the answer, so no `post` caller grows a re-entry path
and the retry posts synchronously; the one sub-point left open is whether that visible retry is
acceptable on interactive money commands or whether the rare positive should re-enter instead,
the §5.8 letter. Two notes for the design page: on Redis the "forever" key documents live in
Redis RAM (~150 bytes a key), so the window moves that cost to the store rather than to disk —
a key-indexed disk tier is a separate question, not J5's chunk archive; and the boot's contract
heal reads keys older than any window, so it reads them through the same gate. The cut gets its
own design page and follows J4; D-12-1's metric is the trigger, well before D-12-4's ~7 million
line.

## 10. The ledger archive tier (owner direction, 2026-09-05; J5)

The owner's direction: a nightly ETL job inside a maintenance window defined in the TOML copies
journal entries to a compressed disk-based store; Redis is purged at a high watermark, keeping
everything under 48 hours old so a chunk gets two nightly chances to be copied; a read that
misses the primary store checks the secondary. Against the code, three details change and each
one makes the direction safer.

### 10.1 The unit is the sealed chunk

The ledger's persisted unit is `log:{id}:{chunk}` — rewritten on every append until it seals at
`DEFAULT_CHUNK_TARGET_BYTES`, then immutable (`LedgerChunk.sealed`). The ETL copies **sealed
chunks only**; the open tail is never eligible, so there is no torn state to reason about, and
copying is idempotent: a chunk the archive already holds is hash-checked, not re-copied. The
facility is generic over the `log:` key family per namespace, so scene logs (`log:{scene_uid}:*`,
paged by H1 after a scene closes) and canon's log ride the same tier.

### 10.2 The purge gate is the verified copy; 48 hours is the hot floor

A chunk is purgeable when it is **sealed**, the `posted_at` of its newest entry is older than
`retain_hours` (48), **and the archive holds it with a matching SHA-256**. Two nightly chances are
not the safety mechanism — one failed night plus one purge would lose money history — the
verified copy is; the 48 hours keeps recent history hot in Redis for account views and audit,
with two nights of grace on top. Purging runs at a **high** watermark down to a **low** one,
oldest eligible first, bounded per pass, measured as ledger-family bytes resident in Redis
(what the purge can actually reduce), not instance memory. A pass that finds nothing eligible
above the high mark logs and escalates rather than purging unverified chunks.

### 10.3 Reads fall through at the store, and boot reads through it too

Today a purge breaks boot: `scan_contiguity` requires the complete chunk history and refuses on
a hole ("journal contiguity breach; refusing to boot"). The fix is a tiered `ObjectStore` for
the `log:` family: `module_json_get` on a primary miss consults the archive index;
`module_json_list` merges the primary's keys with the archive's for that namespace (a key in
both is served from the primary and hash-compared). Then a full-replay boot, H1's cold reads,
the scene archives and canon all read the archive without knowing it exists, the contiguity
check runs over both tiers unchanged, and **the journal's authority spans both tiers** — which
is what keeps the checkpoint a projection (§5.4 rule 1): a full replay is always still possible.
§5.6's min-present marker is then needed only for the checkpoint's boot-skip, not for the
purge. The archive is read-only to the server; only the ETL writes it.

### 10.4 The archive on disk

Segments of `segment_chunks` (256) sealed chunks each, `<data_dir>/archive/<ns>/<log>/
{first}-{last}.seg`, each chunk a separate zstd frame so one cold read decompresses one chunk
(index: chunk → segment, offset, length, sha256, newest `posted_at`); the index is a
rebuildable side file, and every segment is self-describing. Written temp → fsync → rename,
index appended after, so a crash mid-copy leaves nothing half-present. JSON compresses ~6–8×:
at the §9 design rate (360 000 entries, ~200 MB raw a day) that is ~30 MB a day, ~11 GB a
year; Redis holds the floor's two days plus the open tail. **The archive is money and belongs
in the same backup set as Redis**; a hash mismatch on read is treated as absent, which at boot
is a contiguity breach and a fail-closed boot — the right failure.

### 10.5 Configuration

```toml
[store.archive]                    # (J5) the ledger archive tier — sealed log chunks to disk
enabled = false
dir = "archive"                    # under [server].data_dir
window = "03:00-04:00"             # UTC, daily; the ETL and the purge run only inside it
retain_hours = 48                  # never purge a chunk whose newest entry is younger
high_watermark_mb = 512            # ledger-family bytes resident in Redis that start a purge
low_watermark_mb = 256             # … and where it stops
segment_chunks = 256
```

### 10.6 Cut and lenses

**J5 — the ledger archive tier.** `mush-store`: the archive writer/reader and the tiered
`ObjectStore`; `mushd`: the windowed job (a tokio task beside the writer, never on the world
task), the two-phase pass (copy and verify every new sealed chunk; then purge if above the
mark), the config; `mush-config`: `[store.archive]`; ops: the backup note and the metrics
(`mush_ledger_archive_chunks_total`, `mush_ledger_resident_bytes`, last pass outcome).

> **J5 as built — 2026-09-06.**
>
> 1. **The archive** (`mush_store::LedgerArchive`, `crates/mush-store/src/archive.rs`): segments
>    of up to `segment_chunks` sealed chunks under `<data_dir>/<dir>/<ns>/<log>/`, each chunk a
>    gzip member at a recorded offset, a `.idx.json` per segment (offset, length, raw length,
>    SHA-256, archived-at, purged), written temp → fsync → rename, segment before index; an
>    orphan segment or a stray temp file is swept at open; `store_chunks` reads every copy back
>    and compares its hash before the index admits it; `read_chunk` treats a member that no
>    longer inflates or hashes as **absent** and says so at `error`. **J5-D1:** the compression
>    is gzip (`flate2`, already in the tree) rather than the zstd §10.4 named — one member per
>    chunk, the same shape, no new native dependency.
> 2. **The tier is inside the backends**, not a wrapper: `ObjectStore` gained `module_keys`
>    (what is resident), `module_doc_text` (text, primary first, then the archive for a `log:`
>    key), `module_purge_archived` (ledger chunks only, refused for anything else) and
>    `ledger_archive`; `RedisStore::with_archive` / `InMemoryStore::with_archive` attach it;
>    `module_json_get` falls through for a `log:` miss and `module_docs` merges every archived
>    chunk the primary lacks (a chunk in both is served from the primary and compared —
>    `merge_archived`). Boot, H1's cold reads, scenes and canon are unchanged.
> 3. **The pass** (`mush_server::archive::run_pass`): list every namespace's resident keys; for
>    each `log:` key the archive lacks, read its text and copy it if it is a sealed, index-matched,
>    non-empty, contiguous chunk (an open tail is counted and left alone); then, only above the
>    high watermark of resident ledger bytes, purge chunks the archive holds, unpurged and copied
>    at or before `now − retain_hours`, oldest copies first, down to the low watermark and at most
>    `max_purge_per_pass`; a copy or verify error ends the pass before the purge; a purge call
>    that fails ends the purge and is reported. **J5-D2:** the retention floor is measured from
>    when the archive copied a chunk — the ledger is payload-agnostic and carries no entry
>    timestamp — which is at or after the seal, so it is at least the 48 h the direction asked.
>    `spawn_archive_job` runs the pass once a UTC day inside `[store.archive] window`, beside the
>    store writer, and publishes the five gauges; `mushd` opens the archive under the data dir,
>    attaches it before the store is shared, and schedules the job over every composed module's
>    namespace.
>
> **Verify (lenses H, V, T, W, O, S).** H: after a purge, `module_docs` merges both tiers and
> the merged ledger is contiguous; a damaged archived chunk is absent and the merged ledger is a
> breach (`the_primary_wins_and_damage_is_a_breach`). V: a chunk in both tiers with different
> bytes is served from the primary. T: an index dropped after its segment (the crash between
> the two writes) is swept at open and the chunk copied again next pass. W: the window parses,
> may cross midnight, and gates by UTC minute and day. O: the open tail is never copied and never
> purged; a second pass copies nothing; below the high watermark nothing is purged; above it
> nothing younger than the floor is; the purge is bounded per pass and stops at the low
> watermark. S: `module_keys`, `module_doc_text` and `module_purge_archived` behave the same on
> both backends (`suite::module_keys_text_and_purge`, memory + Redis-gated). Findings folded:
> **J5-F1** — the archive's read-back treated a gzip member that failed to inflate as an I/O
> error rather than damage, so a bit-flipped segment errored where the design says "absent";
> `read_chunk` now inflates lossily and the damaged-segment test asserts absence. **J5-F2** — two
> pass tests assumed a copy and its purge happen on different nights; with a zero retention floor
> a chunk is eligible the pass it is copied in, which is the design (the floor is the only
> separation), and the tests now say so.
>
> **Measured (M):** the 100 000-entry synthetic journal (56.4 MB in 3 560 chunks of ~16.6 KB)
> gzips at level 6 to **3.3 MB, 17×** (958 bytes a chunk) — mints are repetitive; a settlement-heavy
> journal will compress less, and §10.4's 6–8× stays the planning figure.
*Lenses:* **H** (a hole: a chunk in neither tier fails boot exactly as today; a chunk in the
archive only boots and pages), **V** (verify: a tampered segment reads as absent; a chunk
present in both tiers with different hashes is reported and the primary wins), **T** (torn
ETL: crash after segment, before index; crash mid-purge), **W** (the window: nothing runs
outside it; a pass that overruns the window finishes its chunk and stops), **O** (the open
tail is never copied, never purged; a chunk sealed during the pass is picked up next night),
**S** (the store suites: `module_json_get`/`module_json_list` through the tiered store return
the same documents as the flat store for every key that exists, both backends). Size: three to
four days.
