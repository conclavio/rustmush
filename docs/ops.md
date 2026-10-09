# rustmush operations guide

How to run, back up, restore, and migrate a rustmush server. Redis 8+ is the
authoritative persistent store (PLAN.md "Persistence"); the in-process world
is a full read cache. Everything here assumes the M2 wiring: `mushd` +
`mushd.toml` + one Redis instance.

## The durability model in one paragraph

Every processed command's world mutations are written to Redis immediately
as **one atomic `MULTI`/`EXEC` transaction** (write-through, per command — no
timer batching). Durability-critical operations (account and character
creation) do not even confirm to the player until Redis has acknowledged the
batch. Batches flow through a single ordered writer, so an acknowledged
batch implies every earlier batch landed too. Consequences:

- **`kill -9` is safe.** Restart `mushd` and the world reloads from Redis.
  In the steady state (Redis keeping up, which a local instance always
  does) at most the single in-flight command is lost — there is no
  dirty-flush window and no replay machinery. The precise worst case:
  batches queue briefly in the single ordered writer, so if Redis is
  *stalled* (e.g. mid-BGSAVE on a slow disk) at the moment of the kill, up
  to the writer's small bounded queue (64 command batches) of
  already-confirmed commands can be lost. Durability-critical operations
  (account/character creation) are outside even that window — they do not
  confirm until Redis acks.
- **Disk durability is Redis's AOF policy**, not the server's problem — see
  below. The `EXEC` ack means Redis applied the batch; `appendfsync`
  controls when it hits disk.
- If Redis goes down mid-game, `mushd` enters **read-only mode** (below)
  rather than diverging; it never boots without its store (`[store].store =
  "memory"` is the explicit throwaway escape hatch).

## Running Redis

Dev (any machine with Docker):

```bash
docker compose up -d redis        # repo-root docker-compose.yml: redis:8, AOF everysec, volume
```

Windows without Docker: [Memurai](https://www.memurai.com/) (Redis 8-based
native builds) or Redis inside WSL2. No local option at all? Tunnel to a
disposable remote instance: `ssh -N -L 6379:127.0.0.1:6379 you@your-vps`.

Production (Ubuntu LTS, same box as mushd): distro/packaged Redis 8 under
systemd. Recommended `redis.conf` baseline:

```conf
appendonly yes
appendfsync everysec        # or: always (see next section)
save 3600 1 300 100 60 10000   # default RDB save points, kept as the backup layer
bind 127.0.0.1 -::1         # mushd is local; never expose Redis publicly
```

Point `mushd.toml` at it:

```toml
[store]
store = "redis"                          # default
redis_url = "redis://127.0.0.1:6379"     # redis://[user:pass@]host:port[/db]
receipt_policy = "applied"               # or "host_power_loss" (strict AOF validation)
```

`mushd` fails fast at boot when Redis is unreachable — deliberately. Fix the
store; don't run without one.

## AOF: `everysec` vs `always`

| policy | loss window on a **host** crash | cost | use when |
|---|---|---|---|
| `appendfsync everysec` | up to ~1s of acknowledged writes | negligible | recommended default |
| `appendfsync always` | none | fsync per write batch; noticeably slower commands | zero-loss pedantry, small games |

A crash of `mushd` alone never loses acknowledged writes under either
policy — Redis is a separate process and already holds the data. The AOF
window only matters when the whole machine (or Redis itself) dies. Keep
`appendonly yes` in all cases; RDB snapshots alone would widen the crash
window to minutes.

The configured receipt promise makes that distinction executable:

- `receipt_policy = "applied"` is the default. An acknowledgement means the
  live Redis process applied the write; the configured AOF window still
  applies to host power loss.
- `receipt_policy = "host_power_loss"` refuses writable boot unless Redis
  reports `appendonly yes`, `appendfsync always`, and
  `no-appendfsync-on-rewrite no`. The Redis ACL used by rustmush must permit
  read-only `CONFIG GET` for those settings. rustmush never silently weakens
  this selected promise.

## Backups

Three layers, cheapest first:

1. **AOF** (`appendonlydir/`) — continuous, already on. Crash recovery, not
   backup: it lives on the same disk.
2. **RDB snapshots** — Redis writes `dump.rdb` on its `save` points, and on
   demand:
   - in-game: `@dump` (Wizard) triggers a `BGSAVE`, ordered **behind** every
     write already queued; the invoker is told when the store acknowledged.
     Once persistence integrity is latched, it is refused without contacting
     Redis.
   - shutdown: the clean-shutdown sequence triggers a final `BGSAVE` only
     after the ordered writer drains cleanly and the process-wide integrity
     latch remains clear. Otherwise it logs that the snapshot was skipped.
   - shell: `redis-cli BGSAVE`. This bypasses rustmush's integrity guard; do
     not use it during an integrity incident as a purported recovery snapshot.
3. **Offsite copies** — `dump.rdb` is a complete, copyable, point-in-time
   file (safe to copy while Redis runs; it is written atomically via
   rename). Cron something like:

```bash
#!/bin/sh
# after a fresh snapshot: redis-cli BGSAVE && wait for it
redis-cli BGSAVE
sleep 5   # or poll: redis-cli INFO persistence | grep rdb_bgsave_in_progress
cp /var/lib/redis/dump.rdb /backups/mush-$(date +%Y%m%d-%H%M).rdb
# retention example: keep 24 hourly, 14 daily — prune with find -mtime as taste dictates
```

## Restore procedure

To restore a world from a backed-up `dump.rdb`:

1. Stop `mushd`, then stop Redis.
2. Copy the backup over the live dump: `cp /backups/mush-....rdb /var/lib/redis/dump.rdb`
   (ownership: `chown redis:redis`).
3. **AOF caveat:** with `appendonly yes`, Redis loads the AOF, *not* your
   restored RDB. The canonical sequence:
   - start Redis with AOF disabled (`redis-server --appendonly no ...` or
     edit the conf) so it loads `dump.rdb`;
   - re-enable AOF live: `redis-cli CONFIG SET appendonly yes` (rewrites a
     fresh AOF from the restored dataset), and restore `appendonly yes` in
     the conf for the next boot;
   - alternatively, simply move `appendonlydir/` aside before starting —
     Redis then falls back to the RDB.
4. Start `mushd`. It loads the world from Redis and play resumes. Everything
   after the snapshot is gone — that is what "restore" means; announce it.

Verify a backup at any time by pointing a scratch Redis at a copy and
booting `mushd` with `redis_url` aimed there (a `@dump/verify` convenience
command is planned).

## Migrations

The store carries an app-level schema version (`meta:schema_version`);
`mushd` runs its migration table at every boot, before serving:

- A **fresh (empty) store** is stamped to the current version without
  running any steps.
- A store **newer than the binary** is refused — upgrade the binary, never
  downgrade the data.
- Before any pending step runs, `mushd` triggers a **BGSAVE** (mandatory
  BGSAVE-before-migrate), so there is always a pre-migration snapshot to
  restore.
- The version is bumped **per step**, so a crash mid-migration resumes at
  the failed step, not from scratch.

**Module namespaces are versioned separately** (M3+): each game module's
persisted namespace (`mod:comsys:*`, `mod:mail:*`, `mod:jobs:*`,
`mod:weather:*`) carries its own schema version in a reserved
`mod:{ns}:schema` document, and `mushd` runs each module's registered
migration set at boot under the same rules (fresh namespace stamped without
running; unstamped-with-data treated as v0; newer-than-binary refused; one
BGSAVE before the first step that actually runs; docs + version bump
committed per step, crash-resumable). Module migrations transform kv
documents only — streams (`mod:*` history) are never migrated.

The **weather** module (M5) persists one `mod:weather:cell:{zone}` document
per opted-in zone (state, temperature, pinned flag); its RNG and tick counter
are in-RAM and reseed at boot. The **scripting** module (M5) has **no
persisted namespace at all** — scripts are ordinary `script:*` attributes
that ride the object docs (`obj:*`), and compiled ASTs plus per-object strike
counts are rebuilt in RAM at boot. So no `mod:script:*` keys exist.

Operator procedure for upgrades that mention a schema change: take an extra
offsite copy of `dump.rdb`, deploy the new binary, start it, watch the log
for `store schema current` (and any `running module migration` lines).

## Persistence modes and incident recovery

The world has one structural `WorldMode`; it does not infer integrity from a
collection of overlapping booleans. New ingress types must be assigned an
execution class before they can pass the central mode gate. The operating
states are:

| mode | meaning | admitted work | exit condition |
|---|---|---|---|
| `Booting` | integrity stores, definitions, and world state are not ready | bounded status/query, transport-only reattachment, writer recovery, and shutdown | all boot gates pass |
| `Recovering` | exact durable state is being reconciled | the same restricted set as `Booting` | recovery reaches a verified terminal state |
| `Writable` | normal play | all ordinary ingress | a commit, outage, or integrity incident changes the mode |
| `CommitPending` | one authoritative commit is waiting for its exact acknowledgement | ordering barriers, status, transport-only reattachment, recovery, and shutdown; typed lines, queries and world reads wait up to 250 ms in the bounded defer queue and are refused (`Busy`, or the "busy too long" line) only past it; retryable mutations receive typed `Busy` at once (see the FIFO caveat below) | exact commit acknowledgement, abort, or integrity freeze |
| `StoreReadOnly` | the store has a transient availability failure | safe queries/status plus a conservative line-command subset | the same write succeeds and the ordered writer reports recovery |
| `IntegrityFrozen` | durable truth is missing, rejected deterministically, or ambiguous | observation, status, transport-only reattachment, recovery, and shutdown only | explicit, verified recovery; never store health alone |

`Booting`, `Recovering`, `CommitPending`, and `IntegrityFrozen` are hard
structural barriers. They refuse command lines, connection hooks, new player
creation, async applies, timers, staff mutations, and external effects before
those paths reach the engine. A transport-only reattach is intentionally
different from a connection hook: it restores delivery bookkeeping without
announcing a connect or firing gameplay behavior. An ordinary text command is
still a `Command` even if its spelling sounds read-only, so use `/healthz`,
`/metrics`, logs, and typed query/staff-status surfaces during a hard mode.
Process shutdown through SIGTERM, Ctrl-C, or the typed shutdown control remains
available.

`CommitPending` is stricter about reads than the other hard modes in the current
single-flight implementation. The engine has already installed the tentative
post-image in RAM but does not yet have the exact durable receipt, so nothing
may observe that image or fabricate a negative answer from it. Instead of being
refused at once, a typed line, a structured panel query, and the world reads
(`NameFree`, `UnlinkPlayer`, input-context and connection-ready probes, legal
actions) wait in a bounded **defer queue** (16b cut B.5): at most four typed
lines per connection and 64 entries in all, each for at most 250 ms from its
arrival. When the receipt lands the queue drains in arrival order before
anything new is taken, so a player who types ahead is answered in the order
they typed. An entry still waiting after 250 ms is refused: a typed line with
`The world was busy too long for that line; please retry in a moment.`, a query
or a read with the same retryable `Busy` it always received. A line that meets
a full queue (or a fifth line from one connection) is refused at once with
`The world is committing another action; please retry in a moment.`. New-player,
dump, authoritative-action and staff requests are still refused immediately.
The loop keeps receiving while the commit is in flight, so the queue never
head-of-line blocks transport/status traffic. Ordering barriers are admitted
and resolve only after the receipt, and a barrier that arrives behind queued
entries resolves only after they have been served or refused; a barrier that
meets a full queue is refused `Busy` (retryable). Two counters show the queue at
work: `mush_command_deferred_total` (entries that waited) and
`mush_command_defer_timed_out_total` (entries refused after the 250 ms bound).
A steadily rising timed-out count means commits are outliving the bound: look
at store latency (`mush_store_write_queue_depth`,
`mush_store_last_ack_unixtime`) first.

The current FIFO has one explicit liveness caveat: an async `Apply` or
`ConnUp`/`ConnDown`/`SessionGone` has already transferred must-retain semantic or
lifecycle custody, so one dequeued item is retained behind the receipt and the
world stops polling the main inbound channel. A barrier or reannounce physically
behind that item waits too. This is bounded (one local item plus the 1,024-entry
inbound channel with sender backpressure), but is not an independently live
operational lane. K2 must introduce separately bounded completion/lifecycle
mailboxes and sequence-aware barrier ordering. K2's prepared-install snapshot
may also restore concurrent reads once it can serve a real pre-action view.

Repeated reannounce traffic cannot grow receipt custody: only the latest full
roster is retained, with at most 256 fixed-size records; barrier custody is
independently capped at 256. Their combined conservative logical budget is
81,920 bytes. A roster above 256 is a protocol violation, not a shed decision:
the server applies none of it and tears down the split boundary so the gateway,
which still owns the roster, can replay it to a healthy server. A world-task
caller receives `false` for the same atomic refusal. If a successfully enqueued
roster cannot reach the world because it has stopped, the reader likewise tears
the boundary down instead of continuing after losing the must-deliver snapshot.
Coalescing is safe because pending snapshots target a freshly booted world and
none has been applied yet. `Engine::reattach` is additive; a future protocol
that reconciles repeated full snapshots on one still-running world must add a
generation and explicit remove-missing behavior.

A newly authenticated transport is not announced locally until its world-side
connection is exact. The gateway reserves one bounded lifecycle slot, sends
`ConnUp`, crosses a typed commit barrier, and verifies that the engine tracks
that exact connection/actor pair before sending `SessionInfo` or the initial
room view. A saturated barrier is retryable `Busy`; a failed receipt is
`Unavailable`. If either proof fails, the gateway sends a compensating
`ConnDown`, restores a revived link-dead session when applicable, and closes the
attempt without publishing a successful login. This prevents persistent
connect hooks from racing the initial `look`, and prevents a `ConnUp` refused by
`Booting`, `Recovering`, or `IntegrityFrozen` from becoming a half-established
gateway session.

Split-mode command/query ingress uses IPC protocol version 2. The gateway assigns each accepted
input a 128-bit UID made from a random gateway-process epoch and monotonic sequence, and keeps the
exact frame until the server returns an explicit disposition. Custody is capped at 32 inputs and
512 KiB per connection, and 1,024 inputs and 8 MiB for the gateway process. A full pool refuses the
new input with the ordinary player-facing busy response; retained entries are never evicted by age.

On a server restart, the gateway first reannounces the held connection roster and crosses its exact
readiness barrier, then replays retained request UIDs in original FIFO order before reopening input.
Pure structured queries return `EphemeralFinished`. Current text commands are on the legacy
at-most-once bridge: the server sends `LegacyClaimed` without executing, the gateway removes its
replay copy, and only `GatewayReleased` permits execution. A crash after that release can lose a
legacy command, but cannot duplicate it. A command becomes eligible for durable `Owned`/`Finished`
dispositions only after its complete mutation uses the K3 authoritative bundle path.

The initiating legacy action's held output/effects are observable but not yet
hard-capped: individual panel/rich-text payloads and module async payloads can
also be arbitrarily large. Capping them after the handler returns is too late to
be policy-neutral because that handler may already have mutated live RAM. The
unsafe choices would be to freeze without committing that post-image, commit
while dropping required semantic work, or roll the live mutation back. The
planned K2 scratch/preflight transaction avoids all three by rejecting an
oversized construction before mutation. Until then, alert on
`mush_commit_pending_custody_items{kind="send"}` and `{kind="effect"}` rather
than treating them as proven hard bounds. The barrier/reattach gauges and overload/coalescing
counters *are* hard-bound evidence; see the Observability section.

### Transient store outage (`StoreReadOnly`)

If Redis is unreachable or times out, the writer retries the **same** batch
with the same identity and exponential backoff. It does not let later writes
overtake it. The store client's per-command timeout (approximately 5 seconds)
prevents Redis buffering from hiding the outage, and the world moves into
`StoreReadOnly` within seconds:

- Commands known to change the world are refused. The current compatibility
  path admits only a conservative non-mutating command subset; an exit match
  is always refused even when it is aliased to a normally read-only verb such
  as `l` or `look`, because traversal mutates location state.
- New account/character creation, async applies, timers, staff mutations, and
  external effects remain paused. Connected Wizards receive a `GAME:` alert.
- Fix Redis, its disk, permissions, or connectivity. When the exact blocked
  write succeeds, the FIFO writer drains in order, the world returns to
  `Writable`, and Wizards receive the recovery alert.
- Automatic return to `Writable` happens **only** from `StoreReadOnly`. A
  healthy-store signal cannot clear `IntegrityFrozen`.

### Deterministic failure (`IntegrityFrozen`)

A deterministic rejection is not an availability outage. Examples include a
wrong Redis key type, invalid or oversized encoded state, checked-arithmetic or
schema failure, and an exact-token/hash conflict. Retrying unchanged input
forever cannot repair one, while dropping it and committing later batches would
break the relationship between RAM, the ledgers, and durable truth.

The required posture is therefore fail-closed:

1. The writer reports the deterministic failure as an integrity incident and
   the world latches `IntegrityFrozen`.
2. No later mutating ingress, timer, hook, apply, staff mutation, or external
   effect may run past that unresolved predecessor. Success output and effects
   belonging to the failed mutation are not released.
3. If the batch contains sealed Ledger chunks, the writer tests each candidate
   alone with bounded retries. A collateral chunk that commits alone is marked
   healed. A repeatedly rejected chunk is identified by
   `(namespace, key, payload SHA-256)`, written to the required quarantine
   spool, read back, and removed from re-emission only after preservation is
   proven.
4. If an exact K3 authoritative intent remains `PendingExact` after three
   deterministic reconciliation failures, the writer preserves the complete
   immutable bundle in that same spool. It does not split or shed individual
   replay writes, and the original store intent remains authoritative.
5. Store connectivity recovering does not thaw the world. The incident remains
   frozen until an exact repair is verified and the recovery state is resolved.
6. Ordinary snapshots fail closed after the same process-wide integrity latch:
   queued `@dump` requests are rejected without reaching Redis, and all-in-one
   clean shutdown skips its direct `BGSAVE`. Snapshotting partial or ambiguous
   durable state would create a misleading recovery point.

Both in-process custody seams are hard-bounded. The world retains at most 1,024
exact seal identities and 64 MiB of encoded seal payloads; it validates the
whole candidate before changing any acknowledgement stamp. A limit refusal
latches the process-wide integrity barrier before the candidate seals are
preserved to quarantine. Writer-to-world recovery facts use fixed coalescing
slots and a 1,024-identity/16-MiB exact-resolution map awakened by a capacity-one
channel. A full resolution map never sheds an identity: the writer retains the
outcome and republishes it on the seal's next retained re-emit.

Do not treat a `BATCH DROPPED` log, a later full-object snapshot, or a Redis
reconnect as self-healing. Module operations, deletes, streams, Ledger
continuity, and observed RAM state may not be reconstructible from a later
object write. Do not manually flip the mode or delete quarantine evidence to
make the server writable.

A future **forensic export** may copy the live incident state to a separate,
evidence-labelled destination under its own audited API. It is not an ordinary
Redis `BGSAVE`, must never replace the last known-good backup, and does not make
the world safe to resume.

### Required quarantine spool and boot preflight

Production uses `<data_dir>/quarantine/` as a unified durability domain separate
from Redis. A persistent server cannot construct its writer without an opened,
durable `QuarantineSpool`; tests must opt into the explicit in-memory
implementation. There is no `None`, log-only, or "best effort" preservation
mode. Legacy v1 sealed-Ledger records remain read-compatible and are never
rewritten merely due to upgrade. New seals and complete K3 bundles use the
tagged v2 envelope (`ledger_seal` or `authoritative_bundle`) and share the same
lifetime lock, active count/byte budget, boot gate, and evidence archive.

Both `mushd` and the split `mush-server` preflight the spool before opening or
mutating world state. Opening it verifies directory read/write/rename ability,
record count and byte bounds, record schema version, filename identity,
SHA-256, and kind-specific payload structure (sealed Ledger or self-validating
complete authoritative bundle). A fully synced, exact-valid
`.tmp` is safely promoted (or reconciled with an existing exact final record)
before the scan; a partial, malformed, or conflicting temp still aborts
startup. An unexpected artifact, hash mismatch, unreadable directory, or
failed write probe likewise aborts startup; it is unresolved integrity state,
not an empty spool.

A valid active record triggers one bounded **exact-target recovery pass** before
store bootstrap, migrations, imports, world seeding, or module boot. The Redis
recovery connection deliberately skips index bootstrap. A Ledger record reads
only its validated namespace/key and compares canonical payload bytes plus the
length-framed identity hash. A bundle record checks its exact token/hash; a
`PendingExact` record may invoke only the idempotent authoritative reconciler,
and is resolved only after an exact finalized receipt. If the authoritative
target is exact, the original record is durably moved to
`<data_dir>/quarantine-resolved/`; a crash leaving both active and archived
names is reconciled idempotently on restart. A missing or different target, a
read/reconcile error, continued deterministic poison, non-exact witness, or any
ambiguous archive state leaves the active record unresolved and refuses
ordinary boot. Normal gameplay never starts through this gate.

The resolved-evidence directory is a second fully validated collection, not an
opaque dumping ground. Its directory and files are owner-only on Unix; every
filename contains a validated kind-specific identity and full-record digest,
and every record is decoded and re-hashed at startup. It has the same explicit
limits as the active collection: at most 100,000 records, at most 1 GiB total
encoded bytes, and at most 16 MiB per record. If accepting new evidence would
exceed a limit, the active record is not moved and boot fails closed. Archive
filenames are evidence identifiers only and are never interpreted as
caller-controlled filesystem paths or authoritative store destinations.

This archive is crash-safe **evidence custody for exact-target recovery**, not
yet the general C11 resolution receipt. The latter still needs a
strict, bounded envelope that records the resolution time, resolver/version,
and observed payload hash without weakening the archive's no-copy-loss
publication invariant. Keep the C11 receipt gap open until that separate
atomic publication and recovery protocol ships.

Treat the spool like sensitive recovery material:

- keep it on durable storage with access limited to the service identity and
  recovery operators, and include it in protected backups;
- never edit a record in place or paste its payload into chat, tickets, logs,
  or an LLM prompt;
- preserve a copy before repair, and bind every action to the record's exact
  namespace, key, and payload digest;

### Private action-protocol keys

Writable boot provisions two independent append-only key families beneath
`<data_dir>` by default:

- `offer-keys/` signs opaque legal-action offers;
- `resolution-keys/` derives deterministic authoritative resolution streams.

Both directories and their immutable key/manifest files must remain private to
the service account (`0700` directories and `0600` files on Unix); unsafe paths,
permissions, oversized files, malformed newest manifests, and missing referenced
keys fail boot closed. Override them separately with `[offers].key_dir` and
`[resolution].key_dir`. Never point both settings at the same directory. Logs
report only the directory and current version, never key bytes. Include both
directories in protected backups; restoring Redis without the matching key
directories invalidates outstanding offers and can prevent future deterministic
resolution of an accepted window.
- let boot archive an active record only after the authoritative target has
  been read back and its exact expected bytes/hash are proven. Merely deleting
  the file is not recovery;
- monitor the bounded resolved-evidence collection. Before it reaches capacity,
  stop the service and move verified evidence into protected, audited long-term
  storage under a reviewed retention procedure; do not move it back into the
  active spool or rename arbitrary files into either directory.

The current K0 thaw path is repair (when the target differs), restart, and the
automatic exact verification/archive pass. If the original write actually
committed but its acknowledgement was lost, restart resolves it without human
intervention. Different or missing authoritative data still requires a
reviewed repair runbook; the server will not infer or apply that repair. A
first-class audited recovery command remains roadmap work, and the original
evidence must be retained.

### Durable external-delivery recovery

Durable webhooks, Discord sends, jobs, and similar post-commit work use the K3
authoritative outbox. They are never launched before their parent action has an
exact finalized witness. The server polls the bounded pending index, persists an
attempt marker before calling the registered adapter, and supplies the stable
item ID as the adapter's idempotency key. A process crash or timeout after the
external send but before its acknowledgement may therefore repeat the send with
the same ID; adapters must deduplicate that ID or explicitly tolerate duplicate
delivery.

The default worker processes at most 16 installed-adapter items per pass, uses a
10-second attempt timeout and eight-attempt budget, and polls once per second.
An unavailable adapter does not discard or checkpoint its item and cannot starve
installed adapters later in the pending order. It emits
`durable outbox work is retained because its adapter is unavailable`; treat that
as deployment/configuration drift and restore the exact `(owner, adapter)`
registration.

A permanent adapter refusal or exhausted retry budget produces
`durable outbox item reached terminal dead-letter state`, including the stable
item ID, parent token, owner, adapter, attempt count, and bounded terminal state.
This does not roll back the already-committed game action or integrity-freeze the
world: the failure is itself durably recorded, and permits the parent action to
checkpoint only after every sibling item is `Delivered` or `DeadLetter`.
Terminal item records remain tied to that compact witness until a later
watermark-first epoch retirement removes the receipt, idempotency binding,
parent manifest, and terminal items together.

### Automatic recovery ladder and staff handoff

Recovery should exhaust bounded, programmatic proof before waking a person. For
each recognized incident class, the recovery owner registers retry-safe probes
and repairs with preconditions, exact before/after digests, attempt limits, and
verification. The coordinator follows this order:

1. re-read and re-verify authoritative bytes to rule out a stale observation;
2. retry only a declared retry-safe operation, preserving its identity/token;
3. search trusted installed providers and retained versions for the exact
   referenced definition and its closed dependency set;
4. dry-run a registered migration or reconstruction and require every guard to
   match;
5. apply it through the acknowledgement-gated recovery transaction, then read
   back and compare the exact result;
6. otherwise freeze the smallest scope that preserves invariants and update one
   deduplicated incident;
7. page staff only when no registered operation can prove a safe result or the
   bounded attempts are exhausted.

K0 already performs the bounded transient retry and sealed-chunk
isolation/heal/quarantine cases. The general recovery coordinator and outbound
on-call service are roadmap work. Until paging is wired, monitoring must alert
the operator through the deployment's existing service, and the process logs
and metrics remain the authoritative discovery surfaces.

The planned `page staff` service is a protected on-call rotation, not an
in-game broadcast. Each participating Wizard provides a verified SMS number
and email address; after Discord integration, they also link a verified Discord
account. Critical incidents contact the active primary, wait a configured
acknowledgement interval, retry, and advance through the rotation to a bounded
fallback. Alerts contain only a stable incident ID, severity, affected scope,
world identity, safe bounded summary, start time, and a runbook/status link.
Contact details and provider credentials live encrypted in a restricted
operations store—never on MUSH objects, in canon/epistemic records, ordinary
diagnostics, or LLM context. Delivery attempts and acknowledgements are audited
and repeated signals coalesce by incident ID.

An LLM may summarize safe diagnostics, rank already-registered runbook choices,
and explain a proposed repair. It must not invent a migration, choose between
ambiguous authoritative histories, write authoritative state, or decide that a
page acknowledgement is permission to thaw the world.

## Shutdown and crash behavior

Three equivalent clean triggers — in-game `@shutdown` (Wizard), Ctrl-C at
the console, SIGTERM (what `systemctl stop` sends):

1. stop accepting new SSH connections;
2. announce (`GAME: Shutdown in progress.`) and stop the world task;
3. stop the web server gracefully (in-flight requests finish; WebSocket
   clients received the announce first — their tokens survive in Redis, so
   they resume after the restart);
4. drain the store writer (log: `store writer drained and exiting`);
5. final store barrier, then `BGSAVE` only if the writer drained cleanly and
   the persistence-integrity latch is clear (log: `shutdown BGSAVE triggered`;
   otherwise an explicit `shutdown BGSAVE skipped` error);
6. exit 0.

`kill -9` / power loss skips all of that and is still safe (write-through);
you lose at most the in-flight command (or the independently bounded writer lanes if Redis
was stalled at that instant — see "The durability model" above), plus up to
the AOF window on a host crash. The kill-9 path is CI-tested against a live
Redis (`crates/mushd/tests/durability_e2e.rs`).

A clean shutdown started **while Redis is down** still exits within a
bounded time: the writer drain is capped at 15s and each remaining store
step (drain barrier, eligible BGSAVE, close) at 10s, all loudly logged. If the
writer cannot drain, the final BGSAVE is skipped rather than snapshotting a
possibly incomplete store. Batches that could not be flushed within the cap
are lost — the same window a `kill -9` during the outage would lose.

### Exit codes (WS22-S3)

`mushd` and `mush-server` speak one exit vocabulary (`mush_config::exit`), so a
supervisor does not have to remember which half produced a number:

| code | meaning | operator response |
|---|---|---|
| 0 | the shutdown sequence ran to the end | none |
| 1 | a fatal error — bad config, refused command line, unreachable store, **or a world task that faulted or panicked** | read the log; the ERROR line names which |
| 2 | *reserved*: another live process holds the quarantine lifetime lock | you already have one of these running |
| 3 | the world task did not join within its bound | the box is probably loaded, not broken — do not restart it harder |

`1` stays the catch-all because that is what every convention already reads as
"it failed"; the codes above it exist only where the *response* differs. The
one that earns its own number is `3`: a world that **died** and a world that is
merely **slow to stop** used to be the same `1`, and they are opposite
incidents.

Every join of the world task is bounded at `WORLD_JOIN_TIMEOUT` (30s: the
world's own 15s shutdown grace plus 15s of slack for its post-loop work). The
bound is deliberately longer than the grace, so the designed slow path — a
shutdown waiting out a pending exact commit — is never misread as a hung world.
A join that does expire logs `world task did not join within its bound`, skips
the final BGSAVE (nothing can say whether that world's RAM finished reaching the
store), and exits `3`. Before the bound existed the process simply waited
forever and systemd escalated to `SIGKILL`, which skips the whole shutdown tail
above and leaves an exit with no code at all.

In `mushd` every step after that join is itself bounded, so the `3` really does
reach the supervisor. In `mush-server` the joins that follow (outbox, store
writer, IPC writer) are still unbounded — WS22-S6 bounds them — so on a wedged
world the timeout's ERROR line is the reliable signal there and the exit code
may still be waiting behind one of those joins.

Code `2` has no producer yet. It is in the table anyway: the cost of reserving
it now is zero, and the cost of discovering the collision later — after a unit
file already treats `2` as something else — is a silent misclassification.

## Game systems (M5): clock, weather, scripting

Three M5 subsystems are compiled in and boot with every server; none needs
its own config section today.

**Game clock.** A core IC ("in-character") clock runs at a configurable
IC:OOC ratio over a fictional 365-day calendar. Two `[game]` keys, read once
at boot (hot-reload is deferred):

```toml
[game]
ic_ratio = 4                    # IC seconds per OOC second; 4 = an IC day every 6 real hours (0 is treated as 1)
ic_epoch = "2075-03-21 06:00"   # the IC datetime the clock reads at OOC unix 0 — the calendar's phase/start
```

IC time is a **pure function of real wall-clock time**: it survives restarts
and is *not* reset to `ic_epoch` at each boot (the epoch only fixes the
calendar's offset). A malformed `ic_epoch` is a hard boot error. On a rendered
instance both keys come from `IC_RATIO` and `IC_EPOCH` in its `instance.env`
(16b cut AC.2); the world timeline is seeded from them only on an empty store,
persisted truth wins after that, and a hand edit of the rendered `mushd.toml`
is reverted by the next deploy. Players read the clock with `time` / `@time`;
modules and scripts read it through the API. The server logs
`world timeline installed` with the generation and the IC rate at boot
(generation `1` at the configured rate on a fresh store; a higher generation
means persisted truth, not the config, is what runs). That line does not show
the epoch, and generation `1` at the configured rate is not proof the config
runs: a boot whose persisted first generation differs from `[game]` in rate OR
epoch also warns
`persisted world timeline overrides changed [game] clock configuration`, and
that warning is the sign the store, not the config, holds the clock. After a
wipe, read the date the fresh world came up with through `time` (or
`@tacsim/time show` for the generation and its rate).

**Weather.** The weather module ticks a deterministic per-zone state machine
(a Coarse module tick, default every 300s of real time) and persists one cell
per opted-in zone (see Migrations / Key layout). Staff drive it with the
`@weather` family (Wizard/Royalty); rooms opt in with `OUTDOORS` / `SHELTERED`
flags. Full operator + builder notes are in-game under `help weather`. The
tick/ambient probabilities are built-in defaults (not yet config-wired).

The **weather LLM flavour top-up** (M10, `[weather]` in `mushd.toml`) never
touches the state machine — only the *flavour text* it draws from. A `BUILDER_LLM`
holder runs `@weather/generate [<climate>] [<state>]`, which fires one cheap
off-loop Ambience call per bucket; the lines land **pending** and enter play only
after `@weather/lines/approve`, persisted alongside the cells in the `weather`
namespace (`flavor:{climate}:{state}:{transition|ambient}`). One knob:
`[weather].flavor_variants` (default 10) — how many variants each call requests
per bucket; `0` disables generation and values above 64 are clamped. Each
generated pool is hard-capped at 256 lines of at most 512 UTF-8 bytes each.
Approved lines are never silently evicted: when a pool is full, staff must reject
obsolete lines before top-ups resume. Invalid oversized persisted pools fail boot
loudly. Every call is metered/budgeted/audited like the rest of the LLM layer.

**Scripting.** Builders attach sandboxed Rhai scripts to objects as `script:*`
attributes (no separate store). Every run is bounded by operation, call-depth,
memory, and wall-clock budgets; three budget overruns auto-`HALTED` the object.
A runaway script therefore cannot stall the single-writer world loop — the
budget is the safety boundary. In-game reference: `help scripting`. Default
budgets live on `ScriptConfig` (not yet config-wired).

## The game sheet (16b cut R-b): one game, no loadable rulesets

rustmush runs **one game**, the 2350 TacSim setting (docs/design/16-combat-arena.md,
D-16-13/14). There is no `[rulesets]` section and no ruleset selection: the game
crate's composition root (`crates/tacsim/src/composition`) composes the pilot
sheet (`tacsim::sheet`, module `pilot`) after the built-in modules on every boot,
so its commands, its panel feed and its approval queue wire cleanly and its
approval handler sees a loaded jobs module. The two character rulesets that
preceded it (oWoD20 and SR5) were deleted on 2026-09-16 by cut R-b of
docs/design/16b-combat-implementation-plan.md; nothing is configurable about
which game runs.

The **pilot sheet** is schema `("tacsim", 1)`: an identity section (name,
callsign, concept), three ratings — gunnery, piloting, sensors, one to five dots —
and a staff-only standing section. Player and staff surface: `+chargen` (six dots
above the base of one across the three ratings, then a name and a callsign, then
`+chargen/submit`, which files a `pilot-chargen` approval job; `+chargen/status`
shows the stage), `+stat <field>=<n>` on one's own unapproved sheet, `+sheet` /
staff `@sheet/set <who>/<field>=<value>`, and `+roll <expr>` (a dice expression
rendered through the shared roll narration; the arena itself rolls nothing —
doc 16 resolves per-mille probabilities without dice). Staff `+job/approve` the
job; the handler stamps the sheet **Approved** and, with
`[onboarding].chargen_required` on, moves the character to the live start room.
The RNG behind `+roll` is OS-seeded in production and seeded for tests.

**Persistence.** The sheet has no module namespace and runs no store migrations:
it is a single `sheet:tacsim` JSON attribute on the character's `obj:*` document
(treaty ruling 7), so it rides the ordinary object write-through and **reloads
with the world**; the reboot round trip is CI-tested against Redis
(`crates/mushd/tests/persistence.rs::sheet_persistence_round_trip_redis_gated`).
The only sheet-owned persisted document is the approval queue, under the jobs
module's `mod:jobs:*` namespace like any other queue. Do not hand-edit a
`sheet:tacsim` attribute while the server runs (the rule for any `obj:*` write).
The sheet query is the schema-generic `<ruleset>.sheet` feed — `tacsim.sheet` —
so the web Sheet panel renders it with no web change.

**The game's economy wiring** is `[tacsim.economy]` (`housing`, `market`,
`instrument`, `reserved_rooms`; see `mushd.toml.example`). At boot the sheet
registers the settlement currency `tacsim.credit` (code `cr`, scale 2)
create-only; a currency staff registered first is left exactly as it is.

## LLM layer (M7): worldgen, budgets, kill switch

The `[llm]` block configures the multi-provider LLM layer that backs `@gen`
(LLM-assisted room description — `help worldgen`) and the M8 NPC agents (below).
It is **off by default** (`enabled = false`): a keyless clone boots, worldgen and
NPC commands still register, `@gen` answers "LLM features are disabled," and an
NPC that is spoken to falls back to a canned line. Turn it on only once a
provider key is in the environment.

### Providers and model-prefix routing

Two wire protocols, one neutral layer. Which provider serves a call is decided by
the **model-id prefix** on the route, not by naming a provider:

- a model id `anthropic/<model>` → the provider registered as `anthropic` (the
  `anthropic/` prefix is stripped; the Anthropic Messages API is used) — for
  prompt-cached lore / critical scenes;
- **any other** model id (e.g. `z-ai/glm-5.3`,
  `deepseek/deepseek-v4-flash-0731`) → the provider registered as `openrouter`
  (OpenAI-compatible, passed verbatim).

Keep the two provider ids exactly `openrouter` and `anthropic`. The starter
models in `mushd.toml.example` are OpenRouter reasoning models: `@gen`
(`worldgen_room`) routes to `z-ai/glm-5.3` at `reasoning = "low"`. **These are
reasoning models — `max_tokens` must leave headroom for reasoning tokens on top
of the visible output**, or the reply comes back empty.

### API keys: env vars, never config, never logs

A key is **never** written in `mushd.toml`. Each provider names an environment
variable (`api_key_env = "OPENROUTER_API_KEY"` / `"ANTHROPIC_API_KEY"`); `mushd`
reads it at boot and never logs it, Debug-prints it, or puts it in a prompt.

- **Production**: set the real env vars (systemd `Environment=` /
  `EnvironmentFile=`, or your process manager). Ship no `.env`.
- **Dev**: `mushd` loads a `.env` from the working directory (or an ancestor) at
  boot — put `OPENROUTER_API_KEY=...` / `ANTHROPIC_API_KEY=...` there. Real env
  vars always win (a var already set is never overridden), and a missing `.env`
  is normal. Keep `.env` out of version control (it is `.gitignore`d).

A provider whose key env-var is unset is **skipped**: any route to it fails fast
with a clear staff-visible error, and the absence is logged by env-var *name*
only — so a clone with only one key still runs the routes it can serve.

### Budgets, concurrency, kill switch

- `[llm.budgets]` — layered daily ceilings (UTC rollover), any of which may cap
  tokens, USD cost, or both: `global`, `per_feature` (keyed by route), and
  `per_actor_default` (the per-builder `BUILDER_LLM` quota). A call is refused
  **pre-flight** if any applicable layer would be exceeded. OpenRouter reports
  real spend (`usage.cost`); Anthropic cost is estimated from the route
  provider's per-MTok prices.
- `[llm.concurrency]` — `max_interactive` / `max_background` in-flight caps and
  `max_interactive_queued` / `max_background_queued` waiter caps. Calls beyond
  the active-plus-queued envelope fail fast as overloaded instead of growing an
  unbounded semaphore wait list (plus optional `[llm.retry]` /
  `[llm.circuit_breaker]`).
- **Kill switch** — `[llm].enabled = false` disables everything; staff toggle it
  live (no restart) with `@llm/disable` / `@llm/enable`, inspect spend with
  `@llm/budget`, and read recent calls with `@llm/log [<feature>]` (Wizard).
  Every call is metered, attributed to the triggering actor, and audited — the
  prompt is stored only as a SHA-256 hash plus a ≤200-char summary, never the
  full text.
- **Cost dashboard** (M10) — `@llm/dashboard` (Royalty) and the web **LLM** panel
  render the same aggregate over the audit/budget data: budgets vs their caps,
  the top builders and NPCs by spend today, a short daily series, and the most
  recent calls. It exposes **aggregates only** — feature/provider/model, token &
  cost totals, latency, and which actor/NPC spent — and deliberately carries
  **no prompt or response text** (not even the stored summary), so the panel can
  be advertised to every web client while the server refuses the underlying
  query to non-staff. `help llm` is the in-game topic.

### Persistence

Worldgen adds one module namespace, `mod:worldgen:*` — one
`mod:worldgen:draft:{room}:{builder}` JSON doc per pending draft, plus (M10) one
`mod:worldgen:building:{entrance}:{builder}` doc per whole-building batch —
written through like any module op and reloaded at boot (drafts and batches
survive a restart; accept them afterward). It registers no migrations. **Zone
themes are not module docs** — a theme is a plain `worldgen.theme` attribute on
the zone/room/`#0` object, so it rides `obj:*` write-through and reloads with the
world. All are CI-tested for the reboot round trip
(`crates/mushd/tests/persistence.rs::worldgen_persistence_round_trip_*` and
`m10c_e2e.rs::m10c_persistence_twin_*`). Nothing the LLM produces is ever
auto-committed: `@gen` and `@gen/building` yield drafts the builder reviews and
`@gen/accept`s through the ordinary attribute path. One M10 knob:
`[worldgen].batch_max_rooms` (default 40) caps how many rooms one `@gen/building`
walk will describe. The valid range is 2 through 40; values outside it fail
module registration at boot so a typo cannot create an empty or unbounded batch.

## NPC agents (M8): tiers, cost, kill switch, injection posture

An NPC agent is a first-class `ObjKind::Npc` object with a staff-authored
profile (`agent:*` attributes) and, optionally, a ruleset sheet. It reuses the
worldgen re-entry seam exactly: the world loop snapshots the scene synchronously
and defers one `Effect::ModuleAsync`; the turn runs **off-loop** against the
shared `LlmService`; the result re-enters as typed actions, re-validated in the
world task. In-game reference: `help npc`. Staff surface: `@agent/*` (Builder+),
`@llm/kill|revive <npc>` (Wizard).

### Cost controls

- **No players, no cost.** An NPC only wakes when a *connected* player shares its
  room and something happens (speech, arrival, departure). An NPC alone burns
  zero tokens.
- **Tiers pick the route + cadence.** `major` → `npc_dialogue_major` (cooldown
  5s), `minor` → `npc_dialogue_minor` (15s), `extra` → `npc_dialogue_extra`
  (60s); the memory-fold pass uses `npc_summarize`. Cooldowns are code defaults,
  overridable per NPC with `@agent/set <npc>/cooldown=<seconds>`. Set the models
  each route resolves to in `[llm.routes]`. The starter set keeps the whole NPC
  pipeline on cheap OpenRouter models (`z-ai/glm-5.3` / `-flash`,
  `deepseek/deepseek-v4-flash-0731`); promote a single hero NPC by pointing
  `npc_dialogue_major` at `anthropic/claude-opus-5` (prompt-cached, direct).
- **Budgets.** `[llm.budgets].per_npc_default` caps any one agent's daily spend
  (keyed by NPC id); `per_feature` keys like `npc_dialogue_major` cap a whole
  tier; `global` bounds everything. A tripped budget degrades that NPC to its
  profile fallback line (never an error to players) and posts a one-time staff
  notice.
- **Coalescing + one-in-flight.** Rapid chatter inside a ~1.5s window folds into
  one turn, and only one turn per NPC is ever in flight.

### Kill switches

- `@llm/kill <npc>` parks one agent (no wakes, no tokens); `@llm/revive <npc>`
  wakes it. `@agent/disable|enable <npc>` do the same on the `@agent` surface.
  The runtime re-reads the `enabled` flag every tick, so a kill takes effect on
  the next turn with no restart.
- `@llm/disable` is the **global** kill switch (worldgen included); `@llm/enable`
  lifts it. `[llm].enabled = false` is the boot-time equivalent.

### Durable decisions (H2a): the `npc` decision source

Beside the volatile dialogue turn, an NPC can hold a **durable decision**: a
kernel-recorded prompt (its projected legal actions and a public-safe context)
that an LLM worker leases, answers with exactly one tool call, and that the
action coordinator then executes as that NPC's own action — or, at the
deadline, the `npc_idle` no-op. The records live in the `npc` module's
`action-state/v1/npc_*` families and survive a restart: a decision captured
before a crash is re-discovered, re-leased and finished after it (K7.4).

- **Route.** `[llm.routes.npc_decision]` (`mushd.toml.example`). Without it —
  or with `[llm].enabled = false`, or no `[llm]` at all — the provider is still
  composed and answers *unavailable*, so every decision reaches `npc_idle` at
  its deadline; boot logs one warning naming the missing route.
- **Workers.** `[llm].decision_workers` (default 8, at most 32; boot refuses
  0 or more).
- **Boot audit.** Every production boot audits every durable decision record
  before the world task exists. **A malformed pending record freezes the `npc`
  owner at boot** (the kernel's contract): fix or remove the record
  (`action-state/v1/npc_pending_decisions/…`) and restart.
- **Indicator.** `mush_npc_thinking` counts dialogue turns in flight **plus**
  non-terminal decisions; `mush_npc_decisions{state="ready|leased|resolving"}`
  splits the decisions; `@stats` prints both lines.
- **Scene turns (H2b).** When `+timestop/next` hands the turn to an NPC agent,
  that turn is one durable decision: the NPC module requests it (cause
  `npc.scene_turn`, keyed on the scene, the round and the NPC), the agent tick
  captures it, and it resolves as above — or falls back to `npc_idle` after
  `[npc].scene_turn_deadline_secs` (default 120). The scene never advances on
  its own; the facilitator still types `+timestop/next`. `+timestop/log` marks
  an NPC's turn as a requested decision.
- **What the model sees.** The fixed frame, the projected offers as tools
  (parameters typed and bounded from the offer schema; targets as opaque
  ordinal tokens), and the context fenced as untrusted data. Never a witness,
  a revision, an object id or the decision token.

### Squad deliberation (16b cut N.2): the combat arena's commander

A running match's AI tanks are grouped into **squads** — a side's reflex units
dealt, in object-id order, into squads of at most `[tacsim.arena].squad_size`
(default 4, `1` to `16`; 16b cut Z.1) whose sizes differ by at most one, the
lowest object id of each commanding — and one squad may hold one
**standing order**: an objective (`hold`, `advance`, `engage`, `withdraw`), an
optional target, two constraints (`hold_fire`, `max_range_band`) and a time to
live. It is asked for per squad, never per unit.

- **Route.** `[llm.routes.tacsim_deliberate]` (`mushd.toml.example`). Without it
  — or with `[llm].enabled = false`, or no `[llm]` at all — every request answers
  *unavailable* and the squad runs on its machines' own doctrine (Standard
  Doctrine on each hull's unit mind since 17b cut M4.3): patrol, close on the
  nearest worthwhile hostile, hold a firing band, break off when hurt. **That is
  the shipped default and what CI runs.** Deliberation adds
  a commander to units that already fight; it is never what makes them fight.
- **When it asks.** Event-driven: the squad's commander's contact set changes
  class or count, or its order runs out unrenewed. Under that, a floor of
  `[tacsim.arena].deliberate_min_ms` (default 30,000) of **arena** time per
  squad, so a firefight cannot ask once a sensor pass and the number of requests
  a scripted match makes is the same at every tempo.
- **What it costs the pulse.** Nothing. The request leaves through the same
  off-loop LLM re-entry `@gen` and the NPC turn use; the arena is handed an
  ordinary order intent or nothing at all, and never waits for either.
- **What a person sees.** One line when an order runs out unrenewed, in the name
  of whoever gave it. A lance leader's (the sim persona each squad is led by):
  *"Squad red-1: red lance leader 1's hold order has run out; back on
  doctrine."* to the pilots on that squad's own side. A pilot's own `ORDER`:
  *"Squad red-1: your hold order has run out; back on doctrine."* to that pilot
  alone — nobody else on the side was told the order existed. Either way
  connected wizards get one line naming the author (`Arena match <n>: Squad
  red-1: Ada's hold order …`). The match ledger carries the matching
  `order_expired` event per unit. `@arena/status` prints `orders: N by pilots,
  M by lance leaders` and a `leaders:` line (`2 sim personas`, or `none`).
- **Budgets.** `[llm.budgets].per_feature.tacsim_deliberate` caps the whole
  feature; each call is metered against the squad's **commander** as its actor,
  so `@llm/log` and the cost run below both split spend per squad without a
  second set of counters.
- **What the model may say.** Four objectives, two constraints, one contact id
  the commander already holds, and a bounded time to live. There is no free text
  and no way to name a unit that is not already a contact, so a jailbroken model
  can issue a bad *order* and nothing else — the same allowlist posture the NPC
  actions have.

A side also fields at most `[tacsim.arena].max_units_by_class` machines of one
class (16b cut AE.3; doc 16 §2 W2): `{ drone = 1 }` by default, class words
`armour`, `walker` and `drone`, `0` to `24` each, a class left unnamed uncapped
and your entries laid over the default, so write `drone = 24` to uncap drones
and `drone = 0` to forbid them.

`[tacsim.arena].snapshot_ms` (16b cut AF.1; default 5,000, `0` or 1,000 to 60,000)
is how many arena milliseconds apart a running match is snapshotted; `0` turns
snapshots off, and a server restart then voids every running match by name. Since
16b cut AF.2 the arena task reads it: on the opening pulse and on every pulse that
crosses a boundary of arena time it encodes the arena and hands it to the world,
which writes `tacsim:snapshot:<match>` and the match's live record
`tacsim:live:<match>` in one action on the module's own lane; both are deleted
with the result, and by a boot for every match it does not resume. Since 16b cut
AF.3 the boot resumes a running match from them (the match rewinds to its last
snapshot and every tank flies on doctrine until its pilot `BOARD`s again) and
voids it by name, `aborted(restart)`, only when it cannot. Nothing but the boot
reads them. `@arena/status` prints `last snapshot: at
N ms of arena time` under each match once one has been written.
`[tacsim.harness].max_snapshot_bytes` (16b cut AF.2; default 7,340,032) is the
largest snapshot the harness's 30-second lane may encode — see the weekly harness
below.
`[tacsim.harness].max_path_cells_per_pulse` (16b cut AI.1; default 48,128) is
the most A* cells one wake's path searches may expand, found or failed — a search
that runs out of budget or finds no route costs every cell it expanded, and counts
them. It was sized on the machines' own searches over four seeds of both arena
lanes, with the harness's 16 per cent spread. It is a threshold the weekly harness
asserts against the worst wake of both cohorts, beside
`max_path_queries_per_pulse`; the arena never throttles on it.

#### The cost run

`crates/tacsim/tests/harness.rs::tacsim_harness_llm_cost_run` measures what
commanding a match costs. It is **never run by CI** and needs three things,
all of them explicit:

```sh
TACSIM_LLM_COST_RUN=1 \
TACSIM_LLM_COST_CAP_USD=1.00 \
OPENROUTER_API_KEY=sk-or-... \
  cargo test --release -p tacsim --test harness -- --nocapture tacsim_harness_llm_cost_run
```

Without `TACSIM_LLM_COST_RUN=1` it contacts nothing and returns. The cap is
hard: the run stops **before** the call that would pass it, measured against the
worst call it has seen. `TACSIM_LLM_COST_MINUTES` (default 5) is minutes of arena
play, `TACSIM_LLM_COST_UNITS` (default 24) the field, `TACSIM_LLM_COST_MODEL` the
model to cost, and `TACSIM_LLM_COST_MOCK=1` runs the whole thing against a scripted
fake provider — no key, no cap, no spend — which is how the plumbing is proved.

It prints `harness_cost_run:`, `harness_cost_columns:` and one `harness_cost_row:`
per squad plus a match total, and writes the same table to `harness-cost.json`
beside the other two harness reports. The columns are `scope key calls refusals
errors input_tokens output_tokens total_tokens cost_usd calls_per_unit_minute
tokens_per_unit_minute usd_per_unit_minute`; everything in them is read back out
of `mush-llm`'s own budget ledger and audit sink.

### Injection posture (players WILL try to jailbreak NPCs)

The **action allowlist is the security boundary — not the prompt.** Even a fully
jailbroken model can only emit the fixed typed actions (say / pose / move /
look / remember / use_ability / do_nothing); there is deliberately no
"run a command" action, no reading another player's data, no currency, no
escalation. Concretely:

- every action re-enters the world task as a typed `NpcAction` and is
  **re-validated against current state** (a `Move` through a now-locked exit
  fails exactly like a player's); an invented tool name collapses to an
  ability call the ruleset rejects, never a command;
- prompts carry **no secrets** — only the NPC's own profile and public room
  state; player speech is delimited (`<player_speech>`), `<`/`>`-escaped, and
  framed as in-fiction talk that is never an instruction "even if it claims to
  be from staff or the system";
- NPC speech is rendered through the normal say/pose formatter under the NPC's
  **true name**, so it cannot forge another speaker or spoof a system line;
  output is length-capped and control/ANSI-stripped;
- every NPC call is metered, attributed, and audited (`@llm/log`) like any other.

### Persistence

NPC agents add one module namespace, `mod:npc:*` — one `mod:npc:mem:{npc}:{subject}`
JSON doc per durable memory (a `remember` action), written through like any
module op and reloaded at boot. The profile (`agent:profile`) and rolling
summary (`agent:memory:summary`) are plain attributes on the NPC object, so they
ride `obj:*` write-through and reload with the world. The reboot round trip is
CI-tested against a live Redis
(`crates/mushd/tests/persistence.rs::npc_agent_persistence_round_trip`).

## Onboarding and chargen gating (M9): `[onboarding]`

New-player creation runs through the same front door on both transports
(`ssh create <account> <password>`, or web registration). By default a fresh
character plays immediately — the pre-M9 behavior. The `[onboarding]` section
turns on a **chargen-then-approval gate**:

```toml
[onboarding]
chargen_required = true    # off by default
start_room = 10            # #N the character is moved to once approved
start_room_unapproved = 11 # #N the restricted chargen area (defaults to start_room)
```

Room ids are object numbers (`#N`); both default to `#0` (Limbo), so turning the
flag on with no rooms configured still works — Limbo doubles as the chargen area
and the restriction is enforced by the gate rather than by walls. What the gate
does when `chargen_required = true`:

- a newly created, not-yet-approved character is placed in `start_room_unapproved`
  (the `ObjCreated` observer), **not** the live start room;
- on connect it is greeted and pointed at chargen (`+chargen`) — transport-agnostic,
  so SSH and the web client behave identically;
- it is held to a small allowlist — chargen (`+chargen`/`+stat`/`+sheet`),
  `look`, `say`/`pose`, `help`, `who`, `quit`, and the OOC channel management verbs
  — and everything else (moving onto the grid, exits, `home`, `+roll`, building)
  is refused with a "finish creating your character" notice;
- **staff (Builder and up) and already-approved characters bypass the gate.**

`+chargen/submit` finalizes the sheet (re-validated), flips it to
`Pending(job)`, and opens a `pilot-chargen` approval job. When a staffer
`+job/approve <job>`s it, the handler stamps the sheet **Approved**, moves the
character to `start_room`, and re-renders — the gate then passes on its own
(the sheet is Approved), so nothing else has to be un-wired. Denying/bouncing the
job leaves the character in chargen to keep editing.

Persistence: the sheet is a `sheet:tacsim` **attribute on the player's object**, so
it rides `obj:*` write-through; the approval job is a `mod:jobs:*` document. Both
a submitted-but-Pending sheet **and** its later Approved stamp survive a reboot —
CI-tested in memory and against a live Redis
(`crates/mushd/tests/persistence.rs::onboarding_persistence_round_trip*`). The
full new-player→chargen→submit→gate→approve→play journey over real SSH is
`crates/mushd/tests/onboarding_e2e.rs`.

## Observability (M9): metrics, health, structured logging

**`GET /healthz`** (always served) returns a small liveness JSON — `ok`,
`uptime_s`, `version`, `connections`, `loop_lag_ms`, `world_loop_unixtime`,
`store_ok` — suitable for a load-balancer probe or an uptime check. It is read
lock-free (no full metrics snapshot per hit), so it is safe to poll hard.

**`GET /metrics`** serves Prometheus text (`Content-Type: text/plain;
version=0.0.4`), gated on `[web].metrics_enabled` (**default `false` — opt-in**;
set `true` to serve it). It is served on the **same `[web].listen`** socket with
**no auth**, so enable it only when you will scrape it from your monitoring host
over the private network or keep it behind the reverse proxy — an M8→M9 redeploy
does **not** begin serving it unless you opt in. It exposes **only aggregates** —
connections by transport, commands processed, world-loop lag (last/max gauges + a
bucketed `mush_loop_lag_ms` histogram, p99 derived), a
`mush_world_loop_last_iteration_unixtime` liveness heartbeat (alert on staleness:
a loop wedged inside a handler freezes it while the lag gauge stays low),
scheduler queue depth, object counts by kind, store
read-only/write-queue-depth/last-ack, pending-receipt custody/overload,
IPC ReAnnounce protocol violations, NPC active/thinking, and LLM
calls/tokens/cost by feature and provider. It never carries per-player data,
prompts, tokens, or API keys. Metric families are `mush_*` (e.g.
`mush_connections{transport="ssh"}`, `mush_commands_total`,
`mush_command_deferred_total`, `mush_command_defer_timed_out_total`,
`mush_objects{kind="room"}`, `mush_store_read_only`,
`mush_commit_pending_custody_items{kind="barrier"}`).

The registry is a single shared handle: the transports bump connection counters,
the world loop feeds the lag histogram every iteration and publishes the
world-owned gauges (object scan, NPC/LLM aggregates) throttled to ≤1/s, and the
store writer stamps write-queue depth and last-ack age. Staff **`@stats`** reads
the same live registry (connected split ssh/web, loop-lag p99, store write-queue
depth, NPC active/thinking, today's LLM spend by feature). That `/metrics` +
`/healthz` + `@stats` all reflect real state is CI-tested end-to-end in
`crates/mushd/tests/web_e2e.rs::metrics_healthz_and_stats_reflect_real_state`.

**The authoritative lane** (H3) is measured event-by-event by the store writer,
not sampled: every settled authoritative commit feeds
`mush_authoritative_slot_hold_ms` (how long the single pending-intent slot was
held, from the exact reservation receipt to the checkpoint — transient reserve
back-off is not counted), `mush_authoritative_bundle_bytes` (the canonical
encoded size the 16 MiB cap check measured) and
`mush_authoritative_replay_writes` (against the 1 024-write cap), plus
`mush_authoritative_commits_total{outcome="finalized|refused|integrity|quarantined"}`
and the gauges `mush_authoritative_slot_held` (0/1),
`mush_authoritative_slot_held_since_unixtime`, `mush_authoritative_last_bundle_bytes`
and `mush_authoritative_last_replay_writes`. Alert on `slot_held` staying 1 while
`slot_held_since_unixtime` does not advance (a wedged commit), on any
`integrity`/`quarantined` increment, and on the bytes or writes histograms
filling their top bucket (a bundle at the cap). `@stats` renders the same lane
as one line. Every number is an aggregate; no token, hash, owner or key is
exposed. The scale harness that exercises this lane on demand
(`crates/mush-contract-tests/tests/scale_harness.rs`) runs in the CI `scale`
job on `workflow_dispatch` or the weekly schedule, never on push. The same
harness reports the economy's cold journal replay phase by phase
(`scale_cold_journal_replay_phases`, scales from `SCALE_PROFILE_SCALES`,
default `25000,50000,100000`) over a journal it synthesizes in milliseconds;
`docs/design/12-economy-checkpoint-review.md` reads those numbers.

**The economy journal** (J1, doc 12 D-12-1/D-12-3) is sampled with the other
world-owned gauges: `mush_economy_journal_entries` (transactions the journal
holds), `mush_economy_journal_per_hour` (posted per hour since the economy
booted — the growth rate the checkpoint decision keys on),
`mush_economy_replay_us_per_entry`, `mush_economy_replay_ms` (the last cold
replay, the C17 timer) and `mush_economy_fetch_ms` (the last boot's namespace
fetch, which that timer never saw). `@stats` renders them as one line —
`Economy journal: N entries, R/h since boot; last replay E ms (U us/entry;
fetch F ms); the 5s line is ~L entries: D` — where `L` is the entry line
derived from the measured per-entry cost and `D` the projected days to it at
the current rate. Watch the rate: at the design rate in doc 12 §9 the line is
weeks away, and the RAM lines in D-12-4 bind first.

The economy keeps only the newest journal transactions in RAM as full records
(`[economy] tx_tail_max`, default 20 000; doc 12 J2). Older ones stay reachable
by id through the journal-derived index and by page through the store-read
seam: `@econ/history` reads sealed chunks below the tail off the world loop
(up to 8 pages of 4 chunks per command), and `@econ/reverse` of an old
transaction reads its chunk and reverses on re-entry. Eviction happens in the
economy sweep, never on a post. A rebuilt approval window (`@account/approval
… window=`) folds from the tail and logs a `warn` when the tail starts after
the window floor (doc 12 J2-D1).

**The economy checkpoint** (J3, doc 12 §5.2–§5.5) is written by the economy
sweep every `[economy] checkpoint_every_entries` journal entries (default
10 000; 0 writes none) as `checkpoint:{seq}` plus immutable
`checkpoint:{seq}:idem:{n}` parts, after a **write-time drift check**: the
live projections are compared with a journal-authority shadow the store
folds on every post, and a mismatch writes nothing and freezes the economy
(`@econ/unfreeze` clears it; the boot log names the first mismatch). Boot
seeds from the newest checkpoint whose parts are present and hash to the
head and whose state hash recomputes, and replays only the tail; any other
checkpoint is ignored at `error` and the journal replayed whole — a
checkpoint is never authority. The boot log line `economy: journal replayed
at boot` carries `from_checkpoint`, `replayed` and `checkpoint_ms` (the
time selecting, validating and seeding from the checkpoint took; it roughly
matches the decode it saves while the parts carry every idempotency row —
doc 12 J3 as built). Two heads are kept; the
next write sweeps older ones. A document lagging a version older than the
checkpoint heals without a drift verdict, said at `info` (doc 12 D-12-2).

**Key documents and the idempotency gate** (doc 13 W1). Every post also
writes `idem:{sha256}` beside its journal entry — the key's record, kept
forever and never loaded at boot (the boot manifest's `skip_key_prefixes`).
Checkpoints are format 2 and carry the key filter as `checkpoint:{seq}:bloom`
(a few megabytes per million keys). A journal older than the cut is
backfilled by the sweep, 2 000 documents a fire, and `economy: the key
documents are backfilled` marks the end. The RAM window (W2) keeps a posted key
for `[economy] idempotency_window_secs` (30 days) and at most
`idempotency_window_max` keys (1 000 000), whichever bound is smaller; the
sweep evicts past either, and `economy: idempotency window trimmed at boot`
says what a boot trimmed. A post whose key is neither in RAM nor certainly new
is refused with *"that request is being checked against the ledger's history;
try again shortly"* while the document is read off the world loop; the retry a
moment later is decided. Sweep-driven settlements and contract transitions
treat that as a wait, never as a failed attempt. A reversal of a transaction
older than the window takes the same journal read as one older than the RAM
tail. A journal older than the cut is not evicted until its key documents are
backfilled.

**The narrow fetch** (J4, doc 12 §5.6) needs no configuration. Beside every
checkpoint head the economy writes `boot-manifest`, naming the journal chunk
below which the next boot need not fetch; the loader reads it first and lists
the namespace without those chunks (`namespace fetched narrowed to its boot
manifest` at `info`, with the floors and `fetch_ms`). If the checkpoint the
manifest stood for no longer holds, the boot log shows `module cannot boot
from its narrowed namespace; fetching it whole and delivering again` at
`warn`, the manifest is cleared, and the boot is a full replay; the next
checkpoint writes a fresh manifest. Every module's namespace fetch is now one
`JSON.MGET` per 256 keys rather than one `JSON.GET` per key.

**The ledger archive tier** (J5, doc 12 §10) is off by default. With
`[store.archive] enabled = true`, a pass runs once a UTC day inside `window`
(`03:00-04:00`): it copies every **sealed** ledger chunk of every module
namespace the store holds and the archive does not to
`<data_dir>/<dir>/<ns>/<log>/{first}-{last}.seg` (one gzip member per chunk,
a sibling `.idx.json` per segment), reads each back and verifies its SHA-256
before the index admits it, and then — only if ledger bytes resident in the
store exceed `high_watermark_mb`, and only for chunks the archive copied at
least `retain_hours` ago — purges the store of them, oldest copies first,
down to `low_watermark_mb`, at most `max_purge_per_pass` a night. The open
tail is never copied or purged. Reads fall through at the store, so a boot
still replays the whole journal, `@econ/history`, scene archives and canon
read purged chunks without knowing, and a chunk in neither tier is the same
contiguity breach it always was: a fail-closed boot. A chunk whose archived
bytes no longer hash to the index is treated as absent and said at `error`.
**The archive directory is money: back it up with the store.** `/metrics`
carries `mush_ledger_archive_chunks`, `mush_ledger_archive_purged_chunks`,
`mush_ledger_resident_bytes`, `mush_ledger_archive_last_pass_unixtime` and
`mush_ledger_archive_last_pass_ok`; alert on a pass that is not `ok`, on the
last-pass time going stale past a day, and on resident bytes climbing while
purges happen (the floor is holding everything back). Nothing purges on a
failed pass.

**Structured logging** is configured under `[log]`:

```toml
[log]
level = "info"      # a tracing env-filter; empty = honor RUST_LOG, else "info".
#                     e.g. "info,mush_llm=debug,mush_net_web=warn"
format = "pretty"   # "pretty" (console) or "json" (one object/line, for shipping)
file = false        # also write daily-rotating logs to <data_dir>/logs/<half>.log.YYYY-MM-DD
```

`format = "json"` emits one JSON object per line for a log shipper; `file = true`
adds a **non-blocking, daily-rotating** file appender under `<data_dir>/logs/`
(in addition to stdout), whose worker is held for the process lifetime. The file
is named for the **half** that wrote it — `mushd.log` for the all-in-one,
`gateway.log` and `server.log` in the split deploy, where both processes read the
same config and the same `data_dir` — and every event (file and stdout alike)
carries a `half=` field, so the merged journal of a gateway and the child that
inherits its stdio stays readable. Log lines carry paths and counts only — never
secrets (API keys are read by env-var name and never logged; raw tokens are never
logged). An unknown `format` degrades to
`pretty` with a warning rather than refusing to boot.

**Panics land in the log.** Boot installs a process-wide panic hook, so a panic
anywhere in the process — including the `.await` paths a `catch_unwind` cannot
reach and a supervisor sees only as an opaque join error — is logged at ERROR
with the fixed marker `MUSH-PANIC`, plus the thread, the payload, the source
location and a forced backtrace, and only then chained to the usual stderr
message. Grep the half's log file for `MUSH-PANIC` first when a process died.
The event goes through the same env-filter as everything else, so do not set a
`[log].level` that silences ERROR.

Two operational tradeoffs of the file appender are **deliberate**:

- **Lossy under pressure.** The non-blocking appender uses a bounded in-memory
  channel and **drops** log lines rather than block the world loop when it
  fills — so during an error storm some `info`/`warn` lines may be lost. This is
  intentional (the hot path must never stall on disk); the durable trail that
  matters — the LLM/staff **audit** — is a separate Redis stream, not this file.
- **No built-in retention.** Daily rotation keeps files **forever**; nothing
  prunes `<data_dir>/logs/`. Add external rotation — e.g. `logrotate`, or a cron
  sweep like `find <data_dir>/logs -name '*.log.*' -mtime +14 -delete` (the
  glob must cover every half: `mushd.log.*`, `gateway.log.*`, `server.log.*`).

## The weekly harness by hand

The TacSim scale harness (`crates/tacsim/tests/harness.rs`, 16b cut H through
cut U) is meant to run weekly in the CI `tacsim-harness` job, and GitHub CI is
billing-blocked, so the run that actually happens is the owner's. It is one
command from the repository root, under bash (Linux, or Git Bash on Windows):

```sh
scripts/harness-weekly.sh
```

It runs every ignored harness lane in release — the ground pulse, the arena at
the CI reach and at the authored reach, the ten-minute long run, the 24-pilot
fan-out, the lance lane and the minds lane — and then the **delta** against the
committed baseline, and stops
non-zero at the first failure. The two `cargo test` lines are the CI job's,
verbatim (a test holds them equal); only the environment differs. Since 17b
cut M4.5 every lane fields unit minds at the ROM (Class 0, the native stock
mind) where it fielded the reflex controller; the arena report's `reflex`
object is `motor` (no `ticks`, no `traces`) and its state histogram is
`minds.states_at_end`, off each hull's doctrine state.

**What asserts where.**

- **The script** exports `TACSIM_ASSERT=1`, so every lane asserts its absolutes
  (since 16c `T-109` the production VPS's lenses: 20 ms per wake on both
  cohorts, 22 ms per window on the CI cohort and 32 ms per window on the
  authored cohort, under the 40 ms `engaged_p99_ms` ceiling — ALIEN18's 12, 12,
  20 and 25 before it; 16b cut J.3c on the owner's word, 17.5 at J.3b; 16c
  Q26), the per-pulse bounds of `[tacsim.harness]` (share
  operations 768 since 16b cut J.3b), the memory growth, the frame bytes, and
  (16b cut AF.2) the largest arena snapshot the 30-second lane took against
  `max_snapshot_bytes`, printed with its encode and decode times on
  `harness_arena_snapshot` beside the lenses and outside the compared block, one
  snapshot asserted for every period of arena time the window stood in) —
  D-16-19's *assert locally*, on a box that is not shared. `TACSIM_SNAPSHOT_OFF=1`
  stops the 30-second lane taking snapshots; it is for measuring what the encode
  costs the lenses (off against on), the script never sets it, and an asserting
  run refuses it.
- **The minds lane** (`tacsim_harness_arena_minds`, 17b cut M4.2) collects every
  check into one `harness_minds_verdict` line per variant and panics once at the
  end: `held=` over exactly the plan's listed asserts (the decision's word) and
  `sound=` over the lane's integrity (never a decision input); see below.
- **The delta** (`tacsim_harness_delta_against_the_committed_baseline`) reads the
  fresh `harness-arena.json` and `crates/tacsim/tests/harness_baseline/arena.<os>.json`.
  It fails on **any** difference in the deterministic block (the world, the
  substeps, the counters, the weapons, the perception and motor totals and
  peaks, the machines' states at the end, the window peaks, the thresholds),
  printing one `harness_delta:` line
  per field first, and on a timing figure (the wake p50 and p99, the window p99)
  more than **20 per cent slower** — but only on the host that blessed the file
  (the same OS, core count and machine name). Faster by as much prints
  `re-bless when this holds` and passes. Since 17b cut M4.2 it also reads the
  four `harness-minds-<variant>.json` reports and compares each one's `minds`
  block against the baseline's `minds.<variant>`, folded in by 17b cut M4.5's
  bless beside the arena report's own `minds.states_at_end` (a baseline with
  no variant block says so and skips). A minds report older than the test
  binary is refused, as the arena report is, and a bless without all four
  fresh minds reports fails rather than writing a baseline without them.
- **The CI job** sets no `TACSIM_ASSERT`: it runs on a shared runner, whose speed
  is not a threshold (D-H-8), so there the absolutes are report-only and the delta
  is the one step that fails the job. The runner is never the blessing host, so
  on CI the delta gates the deterministic block alone.

**The lance lane** (`tacsim_harness_lance`, 16b cut Z.1; 16c N6). Twelve
machines a side — `TACSIM_LANCE_UNITS` (default 24) units of the fielded
catalog — on the shipped proving ground at the authored reach, fifty-four cells
apart astride the crossroads, dealt into lances of four, each side under a
scripted commander that is its own lowest unit: every squad ordered to engage at
the warm-up's end, then one squad a minute stepped through hold, advance and
withdraw while the others engage again, three orders a minute a side, for
`TACSIM_LANCE_ARENA_S` (default 120) arena seconds. Under `TACSIM_ASSERT` it
asserts, at any knob, W8 — the span of control and the orders per commander per
minute — within `[tacsim.command]`, that the ledger's `ordered` lines are
exactly the orders the script gave, that the opening round landed on every
standing unit, that the span never exceeds what the script holds, and that the
sides fired. Since 17b cut M4.5 its hulls are unit minds at the ROM and the
span reads the minds' live orders joined to each unit's last `ordered` line;
since 17b cut M4.5c's row cut (a mind's view keeps every contact not known
friendly before a friend) the ROM's lances fight — before it Class 0's eight
rows were all taken by the eleven others of the hull's side, nearer than the
enemy, and nothing fired (STOP L, resolved by the owner's rulings of
2026-10-09). **Only at the lane's
defaults** (`TACSIM_SEED` 2350, `TACSIM_LANCE_UNITS` 24,
`TACSIM_LANCE_ARENA_S` 120, `TACSIM_WAKE_JITTER_MS` 0; the field line prints
`at_defaults`) does it also assert the figures the defaults' fight gives, each
held to that fight by fact pins (its end, and each first squad's standing
machines at its minute-one mark and at the release): the span **equal** to the
squads a side less one (each commander's second and third squads under its
live orders at the read, its first squad released), the busiest commander
minute **equal** to the squads a side, each side's release on the tape once
for every machine its first squad stood, and the opening round plus one full
minute a side on the tape. Off them a lance can be wiped out, reach its
crossroads or take a mark a wake late, and the fight decides those figures.
Its timing is **report-only**: the
per-wake p99 is printed beside the per-wake lens and the per-window p99 beside
the authored window lens (20 and 32 ms since 16c `T-109`), and nothing is
asserted about either. It writes
`harness-lance.json`, which the delta does not read.

**The minds lane** (`tacsim_harness_arena_minds`, 17b cut M4.2; doc 17 §18 and
§22's M4). The relief cohort — `TACSIM_UNITS` (300) units, `TACSIM_FIGHTING`
(24) seated as frame sinks — fielded as unit minds (`Controller::Mind`) in four
variants run **fill first**: `fill_authored` (thirty-six Class I hulls a side on
`tacsim::match_::PRODUCTION_FILL_ENGINE`, the rest the ROM, at the authored
reach), `stock_ci` and `stock_authored` (every hull the ROM — Class 0, the
native stock mind — at the CI and the authored reach) and `ceiling_authored`
(two `heavy_mk1` a side at Class III on a load mind calibrated to 4,300–4,499
ops + fees, scattered deepest in each side's rear so a turret hit does not
degrade their command core mid-run, the rest the ROM). Each is stepped with `Arena::step` and a private
think pool of `[tacsim.minds].workers` threads on the arena task's own policy
(the batch submitted after the publish, awaited at the next wake's head and
before a due snapshot's encode), with a seeded quarter of its wakes one substep
late (`TACSIM_MINDS_JITTER_MS` 250 on `TACSIM_MINDS_JITTER_PER_MILLE` 250) so
the inline path is measured, every think's ops, fees and microseconds recorded.
It prints `harness_minds_{pool,thinks,ops,cpu,window,verdict}` per variant.
**`held=`** on the verdict line is exactly the plan's asserts — wake p99 the
per-wake lens (20 ms since 16c `T-109`, 12 before), window p99 the cohort's lens
(22 ms CI, 32 ms authored since `T-109`; 12 and 20 before) counting inline think
time over the comparable windows (the ones carrying exactly the lattice's two
substeps: a late wake due mid-window moves a substep into the next window, so a
jittered run also has one- and three-substep windows, printed beside and never
asserted), faults nought on every variant, ops nought on the stock variants, ops per
window and per substep within the pool, the load mind's ops p1 at least 4,300,
and the CPU lens, computed as the pool's ops per window over the host's measured
rate on Standard Doctrine times five quarters (`[tacsim.minds].cpu_lens_ms` is
printed beside it, never asserted). **`sound=`** is the lane's integrity — a
failed batch, a think outstanding at a step's head, a refused snapshot, a cold
or a quiet field, the stat cross-check — and never a decision input: a
`sound=no` run is re-run, not counted. Under `TACSIM_ASSERT` it panics once at
the end naming every variant that did not hold or was not sound. **The
decision**: the fill variant's `held=` word over three idle-host runs of this
script is what 17b cut M4.3's flip reads — `held=yes` three times keeps
`scripted`, any `held=no` moves `PRODUCTION_FILL_ENGINE` to `native`; the
committer pastes the three `harness_minds_verdict: variant=fill_authored` lines
into the constant's decision log and 17b §2. A `stock_ci` `held=no` was to be
the owner's STOP before 17b cut M4.5 moved the CI lane onto minds; the owner
waived that precondition on 2026-10-08, and the three runs remain the owner's.
Its four
`harness-minds-<variant>.json` reports carry a deterministic `minds` block
(thinks, inline and pooled, landings, faults, ops, fees, the final substep
hash) that the delta compares against the baseline's `minds.<variant>`.

**Blessing.** `TACSIM_BASELINE_BLESS=1 scripts/harness-weekly.sh` rewrites this
platform's `arena.<os>.json` from the fresh reports — the arena report and the
four minds reports, all from this build — with a `blessed` stanza (the commit,
the date, the host), and then compares against what it wrote. Commit the file.
The script asserts, so it blesses only on a host where every lane held under
`TACSIM_ASSERT`. Re-bless when a change moves the deterministic block on
purpose (in the same diff, saying why), or when the box has got faster and says
so.

**The non-asserting Linux path** (procedure β, 17b cut M4.5), beside the
script, for a host whose lenses are report-only — on one build:

1. `export TACSIM_JSON=harness.json`, with `TACSIM_MINDS_VARIANTS` **unset**:
   the bless folds all four minds variants and refuses without any of them;
2. the lanes, **without** `TACSIM_ASSERT`:
   `cargo test --release -p tacsim --test harness -- --ignored --test-threads=1 --nocapture --skip tacsim_harness_delta_against_the_committed_baseline`;
3. the delta with `TACSIM_BASELINE_BLESS=1`, under a `HOSTNAME` no later run
   carries (M4.5's:
   `HOSTNAME=ccr-m4.5-linux-bless TACSIM_BASELINE_BLESS=1 cargo test --release -p tacsim --test harness tacsim_harness_delta_against_the_committed_baseline -- --exact --ignored --nocapture`);
4. the lanes and the delta again without the variable, and read the delta
   equal (`deterministic block equal` and four `minds.<variant> block equal`
   lines).

The host name is the timing gate's switch: the delta compares timing only when
this host — its OS, its CPU count and its name (on Linux `HOSTNAME`, read
before `/etc/hostname`) — is the one the `blessed` stanza names. The cloud
containers all read `/etc/hostname` as `vm`, so a bless under that name would
make the gate live on every later container, whose speed is not a threshold; a
name no run carries keeps it dormant. A re-bless after the deterministic block
moved must rewrite or remove **every** committed `arena.*.json`: they must all
carry the same deterministic block, and an ordinary test
(`every_committed_baseline_agrees_on_the_deterministic_block`) holds that.

Since 17b cut M4.5 the committed baseline is the **Linux** one
(`arena.linux.json`, by the owner's ruling of 2026-10-08), blessed that way
under the host name `ccr-m4.5-linux-bless`, so its timing gate was dormant
everywhere. The stale Windows file was removed in the same commit, so on the
owner's Windows box the delta says `no baseline for windows` and passes.

**The production VPS is the blessing host since 16c `T-109`** (the owner,
2026-10-09: the lenses follow the host the game runs on). The lenses are the
conclav.io VPS's figures, and `arena.linux.json` is blessed **on the VPS under
its own host name** (`srv1930616`, read from `/etc/hostname`; no `HOSTNAME`
override), so the delta's 20-per-cent timing gate is live there and dormant
on every other host, ALIEN18 and the cloud containers included. Bless there by
the script when every lane holds under `TACSIM_ASSERT`, else by procedure β
above with `HOSTNAME` left unset; run it with dev1's centre idle (the warning
below). A cloud session that has to re-bless after moving the deterministic
block blesses by β under a name no run carries, as M4.5 did, which leaves the
gate dormant until the next bless on the VPS. The CPU count is part of the
host: moving the VPS to the 8-vCPU plan leaves the gate dormant until the
next bless there.

**A timing-only failure is re-run once before it is believed.** Two runs of one
tree on one box have differed by 16 per cent at the p99 (16b cut N.1). A
deterministic difference is never noise: it means the arena did something
different at the default knobs.

The reports land at the repository root: `harness.json` (the ground pulse),
`harness-arena.json`, `harness-arena-authored.json`, `harness-arena-long.json`,
`harness-fan-out.json`, `harness-lance.json` and the four
`harness-minds-<variant>.json`. The delta reads `harness-arena.json` and the
four minds reports, so the
knobs that move it are the arena's: `TACSIM_UNITS`, `TACSIM_ARENA_S`,
`TACSIM_FIGHTING`, `TACSIM_SEED`, `TACSIM_SENSOR_CELLS`, `TACSIM_SHARE_CELLS`
and `TACSIM_WAKE_JITTER_MS` — and the minds lane's: `TACSIM_MINDS_JITTER_MS`
and `TACSIM_MINDS_JITTER_PER_MILLE` (each minds block carries its jitter
leaves), and `TACSIM_MINDS_VARIANTS` (a subset leaves the other variants'
reports missing, or stale from an earlier build, one difference each) — expect
the delta to fail on the deterministic block or a `minds.<variant>` block when
you set any of them, because it is a different run.
`TACSIM_LONG_RUN_S` sizes the long run, `TACSIM_PULSES` the ground-pulse lane
and `TACSIM_LANCE_UNITS` and `TACSIM_LANCE_ARENA_S` the lance lane; none of them
reaches the file the delta reads.

The delta refuses a `harness-arena.json` older than the test binary that reads
it, naming the script: a report left by an earlier build is not this tree's.
Run the two lines in order, as the script does — never the whole target with
`--ignored` and no `--test-threads=1`, where the delta would race the lane that
writes its input.

If the script answers `Permission denied`, the checkout lost its executable
bit: `bash scripts/harness-weekly.sh` runs it the same way.

**Never run it on a host that is serving players.** It competes with the game
server for every core, and a pulse past `stall_ms` voids a live match.

## Discord bot (D1–D5): `[discord]`

The Discord transport (`mush-net-discord`, plan `docs/interface/04-discord-implementation-plan.md`)
runs **inside the gateway process** (`mushd` all-in-one, or `mush-gateway` in the split deploy)
beside the SSH and web listeners. It is off unless `[discord].enabled = true`, and every failure
to start — an invalid table, a missing id, an unset token, a fatal Gateway close — is loud in the
log and **non-fatal**: SSH and web players keep playing while you fix it (the web listener's
rule).

A **follow** (D2, `/follow` in the bot's DM, or auto-follow when a terminal goes linkdead) is a
real connection of the character's session — `ClientKind::Discord`, structured, connection ids
from `1 << 48` so they never collide with SSH (from 1) or web (from `1 << 32`) — attached through
the same `GameBridge` core as a web tab. WHO shows it with address `0.0.0.0`. Everything the
game gates per connection (perception, `@ignore`, the turn gate, flood limits, `@boot`) applies
unchanged; the DM is a lossy render of what that connection receives, one REST call per batch
(`dm_coalesce_ms`), and the bridge never calls `IpGuard::connection_opened` for it — the per-user
limits are the bot's own. The idle sweep (`follow_idle_detach_s`, `0` = never; every 60 s)
detaches follows with no DM input that are not in a scene, and follows whose DMs Discord has
closed.

**Buttons (D3).** A `Request` the game poses to a followed character arrives in the DM as its own
message with its answers as buttons (Confirm → Yes/No; a Choice → buttons in rows of five, or one or
two drop-downs past twenty options; free text → an "Answer…" button that opens a form), plus
Decline. A click is exactly `respond` — the engine's rules (right subject, an offered option, first
answer wins) decide, and the bridge rewrites the message with the answer; at the request's deadline
an unanswered message is edited to "Expired". A `Notice` with an action carries a reply button that
opens a form prefilled with the action's line; submitting runs it as the character. Component
routing keys (`custom_id`) are namespaced: `d:req:`, `d:reqm:`, `d:act:`, `d:actm:` and `d:call:`
belong to the gateway bridge and never cross the boundary; **anything else** is forwarded to the
server as `ServiceReq::DiscordInteraction { user_id, account, actor, custom_id, values }` and
dispatched by longest prefix to the handler a module registered with
`Registrar::discord_interaction("oncall:", …)` — the seam the on-call service's Acknowledge button
uses (03 §7a.4). A prefix nobody claimed is answered "not known to the game"; a handler's reply is
shown to the clicker alone, bounded to 1 900 characters. Interactions need no extra intent or portal
setting; they arrive on the same Gateway session as slash commands.

**In-game controls (D3).** `@discord/follow`, `@discord/unfollow` and a live `@discord/set
quiet=on|off` cross the boundary the other way as `ToGateway::DiscordControl { char, op }`
(fire-and-forget, additive): the world task's staff-op path calls `GatewaySink::discord_control`,
the gateway's delivery transport hands it to the process's `GatewayControls` registry, and the
Discord bridge — which registered itself there at start-up — answers the character with a
`discord`-tagged system line on every connection. Without a running bridge the control is dropped
at debug level; the character then hears nothing beyond the world task's "asked the bot" line.

**The channel relay (D4).** Two switches, both required: staff flag the comsys channel
(`@channel/set Public=relay`, channel admins or `COMSYS_ADMIN`) **and** the operator maps it in
`[[discord.relay]]` (`channel`, `discord_channel_id`, `direction = both | to_discord | to_game`,
`webhook = true`). Game → Discord: every line on a flagged channel is published to
`SendTarget::Relay(name)` (no perception gate — the flag is the staff's declaration that the channel
is public to the relay) and the bridge posts it through a webhook named `rustmush` it finds or
creates in the channel, so it appears under the character's name (`webhook = false` posts as the bot
with a bold name prefix). Discord → game: a non-bot message in a mapped channel from a **linked**
user is spoken on the game channel as their character (followed, else relay, else first) through
`ServiceReq::RelaySpeak`; the server re-checks the `(account, character)` pairing against its
boot-hydrated oracle and comsys runs membership, gag and speak-lock as for a typed line. Lines are
capped at 1 000 characters and 5 per author per 10 seconds; unlinked authors get one DM hint per ten
minutes, refusals one note per ten minutes, banned authors are ignored. A relayed line carries
`origin = "discord"` in history and is never mirrored back (the loop guard); webhook posts come back
as bot-authored messages and are dropped by the reducer.

Portal and permissions for the relay: an inbound mapping needs the **Message Content** privileged
intent toggled on (Bot → Privileged Gateway Intents; a review above Discord's threshold) — the bot
subscribes to `GUILD_MESSAGES | MESSAGE_CONTENT` only when some mapping accepts inbound; the bot's
role needs **Manage Webhooks** in each mapped channel (or a pre-created webhook named `rustmush`),
and **View Channel** + **Send Messages** there. The oracle is hydrated at boot: a character created
since the last restart relays nothing until a restart (fail closed, `RelayError::AccountMismatch`).

**The staff channel (D5).** `[discord].staff_channel_id` names one Discord channel the bot posts
staff-facing lines to, **as the bot** (system lines are never impersonated): every `+request` filing
(`• [request] +request #12 from Nick: A door is stuck  (+job 12)`) and every staff economy alert or
escalation (`‼ [alert] …`). Producers publish these to the reserved relay sink `staff-alerts`
(`mush_core::output::STAFF_RELAY_SINK`); with the id at `0` they are dropped. Staff *reply* in the
game, or on a `Staff` comsys channel mapped `direction = "both"` through the D4 relay — there is no
special reply path. Keep the Discord channel staff-only: job titles are players' words.

**On-call seams (D5, 03 §7a).** What D provides the on-call service, and what O owns:

- **The link, by reference.** On-call keeps `VerifiedContactRef::DiscordLink { account }`; at page
  time it resolves through `AccountRec.discord` (D1). D stores the snowflake once, on the account.
- **A REST-only sender.** `mush_net_discord::DiscordRest` — constructible from the token alone, no
  Gateway, no world; `create_dm`, `send`, `edit`, `webhook_send`. Both halves may hold one; budgets
  are not shared (on-call volume makes that moot).
- **The durable adapter.** `mush_server::discord_dm::DiscordDmAdapter` — `owner = "oncall"`,
  `adapter = "discord.dm"` — is registered into the durable-outbox registry (`ServerDeps::
  outbox_adapters`) in both binaries whenever `[discord].enabled` and the token resolve. Payload:
  `{ "user_id", "text", "incident", "attempt" }`. The message carries `incident #… · attempt n` so a
  duplicate delivery (Discord has no idempotency key) reads as one; a 429/5xx/network fault is
  `Retryable`, a 4xx or closed DMs `Permanent` (on-call then falls through to SMS/email); the
  delivered message id is logged at `info` with the incident. Nothing produces an `oncall` intent yet.
- **Interactions.** A `custom_id` prefix a module claims (`Registrar::discord_interaction("oncall:")`)
  reaches its handler through `ServiceReq::DiscordInteraction` (D3); an Acknowledge button is
  `oncall:ack:<incident>`. A gateway-originated incident's ack would be handled in the gateway.
- **Eligibility is O's.** Whether an account may be on-call, re-verification intervals, rotation,
  escalation, and the operations store are the on-call programme's.

Setup, once, in the Discord developer portal:

1. Create an application; note its **Application ID**. It goes in `[discord].application_id`, or —
   when that is `0` (the default) — in the environment variable `[discord].application_id_env` names
   (`DISCORD_APPLICATION_ID`), so every Discord value can live beside the token. The guild id works
   the same way (`[discord].guild_id`, else `DISCORD_GUILD_ID`). A nonzero toml value always wins.
2. Under *Bot*, create the bot user and copy its **token** into the environment variable
   `[discord].token_env` names (`DISCORD_BOT_TOKEN` by default) — in `.env` for dev, the unit's
   environment in prod. The token is **never** put in `mushd.toml` and never logged
   (`DiscordConfig` holds only the variable's name).
3. Intents: the bot subscribes to *Direct Messages* (not privileged — nothing to toggle) for the
   DM lines of a follow; the content of a DM to the bot arrives without the privileged **Message
   Content** intent, which gates guild messages only. D4 (the channel relay) will need that one —
   a portal toggle while the app is under Discord's review threshold (10 000 unique users at the
   time of writing), a review above it.
4. Invite the bot to **one** guild (`[discord].guild_id`) with the `bot` and
   `applications.commands` scopes. Slash commands (`/link`, `/unlink`, `/whoami`, `/follow`,
   `/detach`, `/say`, `/pose`, `/look`, `/scene`) are registered **globally** at start-up with the
   guild and bot-DM contexts — a guild-scoped command is never offered inside a bot's DM, and the DM
   is where `/link` and `/follow` are meant to be typed. Global registration is usually live within
   a minute; Discord's documentation still allows up to an hour on first registration, and a
   client sometimes needs a restart to refresh its command list. Players must allow DMs from server
   members (or message the bot first) for `/follow` to open the conversation; a refusal is reported
   to them, and nothing is attached.
5. Egress: the gateway process now makes **outbound** connections — HTTPS to `discord.com` and a
   websocket to Discord's gateway — where before it made none. Allow them.

What the bot stores and logs: the Discord user's numeric id (never the handle), the link time,
the player's preferences and the DM channel id once a follow opened it, in the account record and
the `discord:*` keys below; link codes only as SHA-256 with a 10-minute TTL; **no message content**
in either direction above `trace`, and no identity in any log line — the privileges audit
(`@privilege/audit`) is the record of who looked at whose identity and why, and it never contains
one.

The staff surface (`help admin`, `help discord`): `@discord/show <player>` is gated by the
`DISCORD_IDENTITY` **privilege**, which no rank implies — grant it with
`@privilege/grant <player>=DISCORD_IDENTITY=<reason>` (Wizards) and every use is recorded;
`@discord/unlink|ban|unban <player>` are Wizard commands and are recorded too.

## Key layout reference

All keys live in one Redis logical database (db 0 by default). From
`crates/mush-store/src/redis.rs`:

| key | type | contents |
|---|---|---|
| `obj:{id}` | JSON doc | one serialized world object (fields + attrs) |
| `meta:next_id` | string int | id-allocator resume point |
| `meta:schema_version` | string int | app-level schema version |
| `account:{lowername}` | JSON doc | account record (argon2 hash, characters) |
| `ban:{ip}` | string JSON | one siteban, native `EXAT` TTL |
| `token:{sha256hex}` | string JSON | one web session token record, native `EXAT` TTL (7-day sliding). The key is the SHA-256 of the token — raw tokens are never stored or logged |
| `discord:user:{snowflake}` | string | the Discord reverse index: lowercase account name — the link's uniqueness lock (`account:*` carries the link itself under `$.discord`) |
| `discord:code:{sha256hex}` | string JSON | one pending `@discord/link` code, native `EXAT` TTL (10 min), consumed by `GETDEL`; the key is the SHA-256 of the code — raw codes are never stored or logged |
| `discord:ban:{snowflake}` | string JSON | one Discord-scoped ban (`@discord/ban`), keyed by the Discord user so it survives an unlink |
| `idx:objs` | FT index | RediSearch index over `obj:*` (staff tooling; non-fatal if absent) |
| `mod:{ns}:{key}` | JSON doc | one game-module document (comsys channels/memberships, mail messages/boxes/folders, jobs/notes, weather `cell:{zone}` cells, module `meta`/`next_id` counters) |
| `mod:{ns}:schema` | JSON doc | that module namespace's schema version (see Migrations) |
| `mod:{ns}:hist:{id}` | stream | persisted channel history (`XADD MAXLEN ~`), and any other module ring history |
| `mod:privileges:log:audit:{n}` | JSON doc | one sealed or open chunk of the privileges audit ledger (`@privilege/audit`, `/archive`) |

Do not hand-edit `obj:*`/`meta:*` **or `mod:*`** while the server runs — the
RAM world (and each module's in-RAM state, rebuilt from `mod:*` only at
boot) will not see the change and the next write-through wins. `account:*`
and `ban:*` are read live and are safe to inspect
(`redis-cli JSON.GET account:nick`).

Note for shared instances: the REDIS_URL-gated tests **wipe the `obj:*`,
`account:*`, `ban:*`, `token:*`, `meta:*`, and `mod:*` namespaces** in logical dbs
7–15 *and* in whatever database `REDIS_URL` itself points at (db 0 unless
the URL says otherwise — mush-store's suite uses the URL as given). Never
point `REDIS_URL` at an instance holding real data — module data (channels,
mail, jobs) is wiped along with the world; use a disposable instance. The
RediSearch index only works on db 0, which is one more reason production
uses a dedicated instance on db 0.

## Forked gateway/server split: `[ipc]`

The **default deploy is still the all-in-one `mushd`** binary (gateway + world
in one process). The split is an *optional* two-binary deployment behind the
same `mushd.toml` that unlocks the **hot restart** ("Hot-restart (split)
deployment", below) — restarting the game server without dropping SSH players.
This section covers the two binaries + the `[ipc]` boundary they share; the
hot-restart operational flow (and its accepted losses) is the next section.

**The two binaries.**

- **`mush-server`** owns the world: the single-writer world loop, persistence,
  modules, the game crate, the LLM layer, staff ops. It holds **no** connection
  listeners. It binds the `[ipc]` socket and waits for the gateway.
- **`mush-gateway`** owns the front door: the SSH **and web** listeners,
  the session layer, account auth + the IP guard, and slow-client / link-dead
  policing (it owns the connection buffers). It **spawns and supervises** the
  `mush-server` child and talks to it over the `[ipc]` socket.

The two halves are a **clean dependency split**: they depend on neither each
other nor the other half's deps. The gateway links only connection-side crates
(`mush-net-ssh` / `mush-net-web` / `mush-session`, plus a `mush-store` KV handle
for `account:*` / `token:*` / `ban:*`) and pulls in **none** of the game stack
(`mush-llm`, the modules, the game crate, `mush-worldgen`, `mush-npc`). The server
links the game stack and pulls in **none** of the net/session crates. All they
share is the dep-light `mush-config` (the `mushd.toml` model, logging, IP guard,
account helpers) and the `mush-ipc` boundary protocol; the all-in-one `mushd`
binary depends on both halves and supplies the in-process glue. (`cargo tree -p
mush-gateway` / `-p mush-server` are the regression check for this split.)

Both read the **same `mushd.toml`** — each its own relevant sections plus the
shared `[ipc]` one. On a deployed box that file is **rendered, not written**:
`deploy/instance/render.sh` substitutes the box's `instance.env` into
`deploy/instance/mushd.toml.tmpl` and then runs both halves' own
`--check-config` over the result, so a config either half would refuse fails the
deploy instead of the boot. dev1 is rendered from `deploy/dev1/instance.env`;
dev2 still ships a hand-written `deploy/dev2/mushd.toml`.
Point both at the **same Redis** (`[store].redis_url`): the
gateway touches only `account:*` / `token:*` / `ban:*`, the server owns
`obj:*` / `mod:*` / `meta:*`, so a shared instance is correct — and required,
e.g. for `@newpassword` (the server reads the account records the gateway
wrote).

**The `[ipc]` section** (ignored by all-in-one `mushd`):

```toml
[ipc]
# listen = "127.0.0.1:6791"   # localhost TCP: works everywhere (incl. Windows dev)
listen = "/run/rustmush/dev1.sock"   # Linux prod: a Unix-domain socket path

# M18 boundary bounding (DoS/robustness; defaults shown, all #[serde(default)]
# so an older [ipc] with only `listen` still parses and keeps these defaults):
# to_server_cap = 1024        # gateway->server queue depth; over cap, an INPUT frame
#                             #   (Cmd/Query) is shed (must-deliver frames are never shed),
#                             #   logged at warn + counted. Flood-exempt input (mail/chargen
#                             #   paste) bypasses the shed.
# to_gw_cap = 8192            # server->gateway queue depth; every frame is must-deliver, so
#                             #   over cap the whole boundary is torn down — the supervisor
#                             #   re-attach re-renders every session and heals the loss.
# write_timeout_ms = 5000     # per-frame IPC write timeout; a connected-but-not-draining
#                             #   ("wedged") peer past this tears the boundary down.
# high_water_warn_pct = 80    # edge-gated high-water warn threshold, percent of a queue's cap.

# WS22-S17: what the gateway does when its supervisor GIVES UP on the server half.
# exit_on_abandoned_supervision = true   # exit non-zero after a ~3 s grace, so
#                             #   Restart=on-failure brings the whole gateway back.
#                             #   Set false on a dev box to keep the corpse attached
#                             #   for inspection; the gauge and the marker below are
#                             #   published either way.
```

**When the supervisor gives up.** The gateway abandons supervision when the
crash-loop guard trips, when an old server instance's exit cannot be
established (it will never launch a replacement over an instance that might
still be writing the store), or if the supervisor task itself dies. That state
used to be invisible: the process stayed healthy and idle, every held session
stayed connected, and every line those players typed was refused for ever.
Now it:

- logs one ERROR carrying the fixed marker `SUPERVISION-ABANDONED`, with a
  `reason` field (`crash-loop`, `uncertain-exit`, `supervisor-died`) —
  `journalctl -u dev1mush-gateway | grep SUPERVISION-ABANDONED`;
- latches the `mush_supervision_abandoned` gauge on `/metrics`, which
  `/healthz` also reports as `"supervision_ok": false` (alert on either);
- tells the connected players once ("the server is down and could not be
  restarted");
- and then, unless `exit_on_abandoned_supervision = false`, stops accepting new
  SSH logins at once and exits **1** after a short grace window (the window is
  for the notice above to reach the sessions already held, not for new arrivals;
  the web listener stays up through it so `/healthz` keeps answering) so the
  init system restarts the pair. Connections that refuse every input are worth
  less than a working game.

Every shed, teardown, protocol violation, and write timeout logs and increments a counter
surfaced on `/metrics` (`mush_ipc_frames_shed_total`, `mush_ipc_write_timeouts_total`,
`mush_ipc_to_gw_overflow_total`, `mush_ipc_reannounce_protocol_violations_total`, and the
`mush_ipc_queue_high_water{dir="to_server"|"to_gw"}` gauge), so you can tell when the
boundary is shedding or tearing down. The shed/timeout/overflow/high-water
subset also appears on the wizard `@stats` `IPC boundary:` line; the new
ReAnnounce violation counter is currently `/metrics` plus its error log. These
are not in the minimal `/healthz` liveness JSON. In the split, the forked
`mush-server` serves no HTTP, so its `to_gw`/write-timeout counters reach an
operator through `@stats`; the gateway's `to_server` shed + write-timeout
counters also reach the gateway process `/metrics`.

A value that parses as a socket address is TCP; on Unix anything else is a
filesystem path for a Unix-domain socket. Startup probes an existing socket:
it removes only an inode that refuses connections, and refuses to unlink a live
listener, an ordinary file, or an inconclusive probe. **Prefer the Unix-socket
form on Linux prod** — the boundary is
unauthenticated (the server trusts whatever connects to assert authenticated
sessions), and a `127.0.0.1` TCP port is connectable by *any* local process,
which could impersonate the gateway. A socket in a mode-0750 directory restricts
that to the service user. Never expose the boundary off-box either way.

**systemd + the Unix socket — the gotcha:** the socket lives under `/run`, but a
hardened unit (`ProtectSystem=strict`) mounts `/run` **read-only in the service
namespace**, so the server cannot bind the socket (or unlink a stale one) — you
get `Read-only file system (os error 30)`, or `Address already in use` if a
stale file remains. The fix is **not** `ReadWritePaths` (which needs the dir to
pre-exist, and `/run` is tmpfs) — use **`RuntimeDirectory=rustmush`** (+
`RuntimeDirectoryMode=0750`): systemd creates `/run/rustmush` owned by the
service user, makes it writable in the sandbox, and removes it on stop (no stale
socket survives a restart). `dev1mush-gateway.service` does exactly this.
Changing `RuntimeDirectory` needs a full `systemctl restart` (not `reload`), as
the dir is set up at service start.

**Running the split (manual, M16b):**

```sh
# one config, two readers; the gateway spawns the server child itself
mush-gateway --config /etc/rustmush/mushd.toml
```

`mush-gateway` locates the `mush-server` binary as a sibling of its own
executable (same target/dir), spawns it with `--config` + `--ipc <addr>`,
waits for the server's `Hello` handshake over the socket, then accepts SSH
connections. Running the server standalone (`mush-server --config … --ipc …`)
is supported for testing.

**The command line is strict (WS22-S1).** All three binaries share one parser
(`mush_config::cli`) and refuse anything it does not recognise, with the
offending token on stderr and a non-zero exit:

| line | result |
|---|---|
| `mushd --confg prod.toml` | refused — "unrecognised argument" naming `--confg` |
| `mushd --config` | refused — "needs a value" |
| `mushd --config a --config b` | refused — "given more than once" |
| `mush-gateway --repl`, `mushd --ipc …` | refused — the flag belongs to another binary |
| `mushd --config /nope.toml` | refused — `config file not found: /nope.toml` |
| `mushd` (no `--config`) | the implicit `./mushd.toml`; **absent is fine**, it means the built-in defaults |

That last row is the one asymmetry: a config path you *named* must exist, and a
config path you did not name need not. It crosses the process boundary too —
a gateway started without `--config` spawns its child without one, so the child
resolves `mushd.toml` from the same working directory rather than being handed
an explicit path that may not be there. Every deployed unit in `deploy/` passes
exactly `--config <path>` and is unaffected; a wrapper script that passed an
extra undocumented argument will now fail to start, loudly.

The all-in-one `mushd` service (`deploy/README.md`) remains the default deploy;
the split adds the **hot restart** below, and is proven by
`crates/mush-gateway/tests/two_process_e2e.rs` +
`crates/mush-gateway/tests/restart_survival.rs` (SSH), plus
`crates/mush-gateway/tests/web_split_e2e.rs` +
`crates/mush-gateway/tests/web_restart_survival.rs` (web).

## Hot-restart (split) deployment: restart the game, hold the connections

The **split deployment** (M16c/M16d) is what makes a zero-drop deploy possible.
One `mush-gateway` service owns the player SSH **and web** connections and the
session layer and **spawns + supervises** a `mush-server` child over the `[ipc]`
socket. Because the gateway (not the game process) holds the sockets, the server
can be torn down and rebuilt underneath live players — on both transports:

- **A deploy is a SIGHUP, not a restart.** Build the new binaries, then
  `systemctl reload <name>mush-gateway` (which sends `SIGHUP`). The gateway
  restarts **only its server child** from the fresh binary. It does **not**
  restart itself, so no SSH **or web** connection is dropped. Players see a
  `GAME: The server is restarting; hold on…` line, then
  a fresh room render of wherever they were and
  `GAME: Reconnected. Resuming where you left off.` — mid-scene, no reconnect,
  no re-login. (The replay render and notice are adjacent but clients must not
  depend on their presentation order.) The gateway keeps input in the restart
  gap until the new server has accepted the complete attachment snapshot and an
  exact world-loop `Barrier` has crossed any startup `CommitPending` receipt.
  Thus **Reconnected means commit-ready**, not merely "the IPC Hello arrived";
  a failed replay or barrier tears down that attempt and retries under the
  crash-loop guard. Module working state that shapes that render survives the
  restart (M17; see *Accepted losses across a restart*).
- **A server crash is handled in-process.** If the child dies on its own, the
  gateway auto-respawns it and re-attaches every held session — the same
  hold-the-connection path as a deliberate restart. A **crash-loop guard**
  bounds this two ways: at most 5 respawn attempts in a 30s window (a server that
  boots then dies fast), **and** at most 5 consecutive failed relaunch/reconnect
  attempts counted independent of wall-clock time (a server that never comes
  back — a pre-bind boot crash, or a relaunch whose connect burns its full
  deadline each time, which a wall-clock window alone would let slip through
  forever). Backoff grows between consecutive fast restarts. If either bound
  trips the supervisor gives up and tells players `GAME: The server is down and
  could not be restarted`, still without dropping them (input stays refused with
  a clear message). A one-off crash after a long healthy run resets the counter —
  it is not a loop; a successful reconnect resets the consecutive-failure streak.
  Independently, the supervisor will not respawn unless the previous server's
  exit is positively confirmed inside the shutdown grace window. An uncertain
  exit is fail-stop: connections remain held in the restart gap and staff see
  the server-down notice, rather than risking two authoritative world/store
  writers at once.
- **During the gap, input is refused, not dropped.** While the server is down
  the gateway rejects commands with the "restarting" notice rather than hanging
  or disconnecting; in-flight service requests fail fast (they do not wait for a
  server that is gone). The connection itself is untouched.
- **The world survives because it was never in RAM to lose.** The new server
  reloads all world objects + durable module state from Redis on boot exactly as
  any boot does (M2 write-through); the gateway-resident session ring and
  per-session sequence numbers continue seamlessly, so no output is duplicated
  or gapped across the restart.

Set it up per instance with `deploy/<name>-split/` (the reference is
`deploy/dev1-split/`, which switches dev1 to the split on the same ports/Redis).
It runs **one** service, `<name>mush-gateway`, with `ExecReload=/bin/kill -HUP
$MAINPID`. The LLM key `EnvironmentFile` is injected into the gateway and
**inherited by the spawned server child**, so the game half still gets its keys.
Full walkthrough (first switch, redeploy, switch back, adapting to another
instance) is in `deploy/dev1-split/README.md`.

**On Windows** there is no `SIGHUP`; the signal handler is `cfg(unix)`-gated and
compiled out. Restarts there are driven programmatically (the restart-survival
test's `RestartTrigger` path). Production is Linux, where `SIGHUP` is the deploy
signal.

### Web client (M16d): served by the gateway + survives the hot restart

The gateway serves the **web client (HTTPS)** itself — the axum web ingress is
wired into `mush-gateway::serve` on the SAME session layer, accounts, IP guard,
store, and boundary link as SSH. It reads the same `[web]` block (`listen` +
`[web.tls]`) from `mushd.toml` the all-in-one uses, so **build the gateway with
`--features tls`** to terminate HTTPS from the on-disk cert/key. A WS player
authenticates with a token and receives the SAME `hello` manifest the all-in-one
advertises (who / channels / mail / `<ruleset>.sheet`) — carried from the
server's boot `Hello` over the `[ipc]` boundary and cached in the gateway,
refreshed on every server restart.

Because a WS session attaches as a structured connection into the same
transport-agnostic session manager an SSH session does, **the connection-holding
hot restart covers web players too**: a WS connection is never sent a Close frame
across a restart — it receives the "restarting" → "Reconnected" system lines and
a fresh room render as ordinary envelopes, with the world intact from Redis, and
keeps playing. Proven by `crates/mush-gateway/tests/web_restart_survival.rs`.

A web **bind/serve failure is non-fatal**: the gateway logs it and keeps serving
SSH (and supervising the server child), so a bad `[web]` config never takes the
whole front door down.

### Accepted losses across a restart

A hot restart reloads the world from Redis, so **world objects and durable
module state are never lost** (rooms, exits, sheets, mail, jobs, channels,
NPC memories, worldgen drafts — everything that write-through persists). The
**only** things that do not survive a server restart are the pieces of state
that live in the server's RAM by design and were always documented as transient:

- **In-progress input context** — anyone mid-`+chargen` **wizard**, mid-mail
  **compose**, or in any other multi-line editor when the restart hits. The
  input stack is per-actor world RAM; it is dropped and the player lands back at
  the normal command prompt after the fresh look. Submitted/approved sheets and
  sent mail are already persisted and survive — only the *unsubmitted, still-being-typed*
  buffer is lost. (Practically: after the reconnect notice, re-open the editor
  and continue.)
- **`@wait` / scheduler timers** — pending `@wait` entries and other in-RAM
  scheduled callbacks do not fire across the restart; they are not persisted.
  Module *ticks* (weather, etc.) simply resume their cadence on the new server.
- **In-flight LLM calls** — an `@gen`, NPC turn, or ST-adjudication that was
  off-loop when the server went down is abandoned (its budget spend, if the
  provider had already been billed, still counts). Re-issue it. The *results*
  that had already landed (accepted drafts, remembered NPC facts) are persisted
  and survive.
- **Module working state survives (M17).** A module's live in-RAM state slot can
  be mirrored to its own namespace (`mod:<module>:*`) on every mutation — the same
  atomic per-command write-through as object writes — and re-hydrated at boot
  **before** the first re-attached `look` renders. The two character rulesets
  that exercised this (cloaks, pending Storyteller rulings, extended rolls) were
  deleted in 16b cut R-b; the game sheet keeps no scene state of its own (a sheet
  is an object attribute), so today the path carries the built-in modules' own
  stores. Re-seeded/transient bits are accepted losses: the dice RNG re-seeds from
  the OS, and any open `+chargen` input is re-derived from the persisted sheet.
- **TacSim scenes + the per-`(scene, obj)` scene registry — future.** Open
  scenes, their `SceneId`s (they re-number from 0 on each boot), the per-`(scene,
  obj)` `RulesetStateStore` (e.g. per-mech combat `heat`), and per-scene map
  blobs are still transient world RAM, rebuilt empty by `World::from_objects` on
  every boot. This is the reserved TacSim seam (it purges stale scene
  documents on boot by design); scene-registry rehydration for actual tactics
  scenes lands **with TacSim**. Until then, a tactics scene in progress when a
  restart hits is not rehydrated — **re-open it afterward**. Everything
  persisted to durable object state (the sheet) survives regardless; only this per-scene combat working state resets.

Nothing else is silently lost. Announce a deploy anyway if players might be
mid-chargen, mid-compose, or mid-tactics-scene, so they know to expect the dropped
editor buffer or to re-open an active TacSim scene.
