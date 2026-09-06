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
| `rebuild_debit_window` (`module.rs` ~854): bodies within `MAX_APPROVAL_WINDOW_SECS` | a time-bounded window | the RAM tail must cover at least that window |
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
  round trip on a month-old key is.
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

**J4 — the narrow fetch.** §5.6: the boot manifest, `ModuleBootData.omitted`, the min-present
marker through `scan_contiguity`/`restore_from_kv`, the `RetryFullFetch` path, and the pipelined
`JSON.MGET` (which every module gets). *Lenses:* **G** (a manifest naming a chunk the store no
longer has; a manifest with no checkpoint; an invalid checkpoint under a manifest → one retry,
full replay, and the manifest rewritten), **S** (the store suites: `module_json_list` with and
without omission returns the same documents for the kept keys, both backends). Size: two days,
plus the Redis suite time.

J1 is the first cut. The order after the owner's decisions (§9) is J2 → J5 (§10.6, the
archive tier) → J3 → J4; J2 and J5 can each be taken alone.

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
an outage, where a slow boot is a slow restart.

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
*Lenses:* **H** (a hole: a chunk in neither tier fails boot exactly as today; a chunk in the
archive only boots and pages), **V** (verify: a tampered segment reads as absent; a chunk
present in both tiers with different hashes is reported and the primary wins), **T** (torn
ETL: crash after segment, before index; crash mid-purge), **W** (the window: nothing runs
outside it; a pass that overruns the window finishes its chunk and stops), **O** (the open
tail is never copied, never purged; a chunk sealed during the pass is picked up next night),
**S** (the store suites: `module_json_get`/`module_json_list` through the tiered store return
the same documents as the flat store for every key that exists, both backends). Size: three to
four days.
