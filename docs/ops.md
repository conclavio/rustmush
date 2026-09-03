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
| `CommitPending` | one authoritative commit is waiting for its exact acknowledgement | ordering barriers, status, transport-only reattachment, recovery, and shutdown; gameplay/world reads and retryable mutations receive typed `Busy` (see the FIFO caveat below) | exact commit acknowledgement, abort, or integrity freeze |
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
post-image in RAM but does not yet have the exact durable receipt, so structured
panel queries, `NameFree`, and input-context probes are refused as `Busy` rather
than observing that image or fabricating a negative answer. Ordinary command,
new-player, and dump requests are also refused immediately, so they cannot
head-of-line block transport/status traffic. Ordering barriers are admitted and
resolve only after the receipt.

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
you lose at most the in-flight command (or the small writer queue if Redis
was stalled at that instant — see "The durability model" above), plus up to
the AOF window on a host crash. The kill-9 path is CI-tested against a live
Redis (`crates/mushd/tests/durability_e2e.rs`).

A clean shutdown started **while Redis is down** still exits within a
bounded time: the writer drain is capped at 15s and each remaining store
step (drain barrier, eligible BGSAVE, close) at 10s, all loudly logged. If the
writer cannot drain, the final BGSAVE is skipped rather than snapshotting a
possibly incomplete store. Batches that could not be flushed within the cap
are lost — the same window a `kill -9` during the outage would lose.

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
calendar's offset). A malformed `ic_epoch` is a hard boot error. Players read
the clock with `time` / `@time`; modules and scripts read it through the API.
The server logs `game clock installed` with the ratio and epoch at boot.

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
per bucket. Every call is metered/budgeted/audited like the rest of the LLM layer.

**Scripting.** Builders attach sandboxed Rhai scripts to objects as `script:*`
attributes (no separate store). Every run is bounded by operation, call-depth,
memory, and wall-clock budgets; three budget overruns auto-`HALTED` the object.
A runaway script therefore cannot stall the single-writer world loop — the
budget is the safety boundary. In-game reference: `help scripting`. Default
budgets live on `ScriptConfig` (not yet config-wired).

## Rulesets (M6): the game system

Every ruleset crate is **compiled into `mushd`**; `[rulesets].enabled` selects
which one(s) actually register at boot. They load **after** the built-in
modules, so their commands, panel feeds, and approval job queues wire cleanly
and their approval payload handlers see a loaded jobs module.

```toml
[rulesets]
enabled = ["wod"]               # oWoD20 (default); or ["shadowrun"] for SR5
```

Two rulesets are available today: **`wod`** (oWoD20) and **`shadowrun`** (SR5).
A server runs **one ruleset** — they all register the same core commands
(`+roll`/`+sheet`/`+chargen`), so enabling more than one is a **hard boot error**
(`ruleset selection: ... one ruleset per server`) rather than an opaque command
collision. Per-character ruleset dispatch across systems is a future concern.

An **unknown id is a hard boot error** (`ruleset selection: ... unknown
ruleset`), consistent with the rest of config — a typo does not start a
half-configured game. An **empty list** boots a ruleset-less server (bare
comsys/mail/jobs/weather/scripting). Changing the list needs a restart.

**`wod`** is Classic World of Darkness 20th Anniversary (oWoD20 — the
Storyteller System). Its player + staff surface: `+chargen` (priority
point-buy → a `wod-chargen` approval job; a storyteller `+job/approve`s it and
the sheet flips Approved), `+sheet` / staff `@sheet/set`, `+roll <pool> [vs
<diff>]` (d10 pool, `/spec` `/wp` `/priv` `/nowound`, inline
`vs`/`mod`/`spec`/`ext`/`job`/`contested`), `+harm` / `+heal` / `+health` (the
Bruised→Incapacitated track with wound penalties), and `+xp` / `+xp/spend`
(costed raises applied atomically on `+job/approve`) / `+xp/award` (staff). The
dice RNG is OS-seeded in production. In-game reference: `help wod`.

**Splats (M6b + M10a).** `wod` ships seven **templates**: Mortal plus the Vampire
(V20), Mage (M20), Werewolf (W20) and Changeling (C20) splats, and the two
mortal-plus templates Ghoul (V20; a mortal bound to a vampire) and Kinfolk (W20;
a Garou's mortal kin). There is **no per-splat config knob** — every template is
always selectable, and a player picks theirs during chargen with
`+chargen/template <mortal|vampire|mage|werewolf|changeling|ghoul|kinfolk>`
(staff can re-template a sheet with `@sheet/set <who>/template=<id>`). A
character is exactly **one** template; the ruleset builds one composed schema per
template at boot (base + that splat's sections only), and every surface —
`+sheet` render, sheet validation, chargen accounting, `+roll` field resolution,
and the web sheet query — selects the active schema by the sheet's `template`
field. So a vampire's sheet carries Disciplines / Blood Pool / Generation, a
mage's Spheres / Arete / Quintessence / Paradox, a Garou's Rage / Gnosis /
Renown / Gifts, a changeling's Arts / Realms / Glamour / Banality, and never
another splat's. The splat commands (`+disc`/`+blood`/`+frenzy`,
`+cast`/`+sphere`/`+quint`/`+paradox`, `+shift`/`+rage`/`+gnosis`/`+gifts`/`+renown`,
`+cantrip`/`+art`/`+realm`/`+glamour`/`+banality`, `+domitor`/`+bond`, `+tribe`,
…) are all registered at boot and self-gate on the actor's template, refusing
everyone else ("You are not Kindred/Awakened/Garou/Kithain."). Two templates
**share** another's command surface rather than re-registering colliding names:
a **Ghoul** reuses the vampire `+disc`/`+blood` (which delegate to the ghoul's
own flat Blood Pool + domitor-clan Discipline logic), and a **Kinfolk** reuses
the werewolf `+gnosis`/`+gift` (whose gate is widened to accept Garou *or*
Kinfolk); the strictly-Garou `+rage`/`+shift`/`+renown`/`+rank` stay Garou-only.

**Persistence.** Like scripting, the WoD ruleset has **no `mod:wod:*` namespace
and runs no store migrations**: a character sheet is a single `sheet:wod`
JSON attribute on the character's `obj:*` document (treaty ruling 7), so it
rides the ordinary object write-through and **reloads with the world** — there
is no separate sheet save/load path, and the reboot round trip is CI-tested for
both a base sheet and a splat sheet (a Vampire with Disciplines + Blood Pool,
rebuilt from Redis alone and revalidated against the vampire schema —
`crates/mushd/tests/persistence.rs::wod_sheet_persistence_round_trip_*` and
`::wod_splat_sheet_persistence_round_trip_*`). The
only ruleset-owned persisted docs are its two approval job queues, which live
under the jobs module's `mod:jobs:*` namespace like any other queue. Do not
hand-edit a `sheet:wod` attribute while the server runs (same rule as any
`obj:*` write).

**`shadowrun`** is **Shadowrun 5th Edition** (SR5, M10b) — the second full game
system on the same `mush-ruleset` + `mush-dice` plugin API, selected with
`enabled = ["shadowrun"]` (single-ruleset-per-server: it collides with `wod`).
Its dice are a **d6 hit pool** (each 5/6 a hit) rather than oWoD's d10 pool, with
SR5's two signatures: a **glitch** (more than half the dice show 1 — a
**critical glitch** at zero hits) and a **Limit** that caps counted hits, which
**Edge** can push past (`+roll/edge`: add Edge dice, waive the Limit, Rule of
Six). Player + staff surface: `+chargen` (the **priority** point-buy —
`+chargen/priority metatype=A attributes=B magic=C skills=D resources=E`, then
`+stat`/`+spec`, `+chargen/submit` → a `shadowrun-chargen` approval job), `+sheet`
/ staff `@sheet/set` (edits recompute the ceil-division derived stats — Limits,
Condition Monitors, Initiative), `+roll <attr>+<skill>` (auto Limit + wound
modifier; `/edge` `/edge+` `/priv` `/nowound`; inline `threshold`/`limit`/`mod`/
`spec`/`contested`/`extended`), `+damage`/`+heal`/`+condition` (the two Condition
Monitors with Stun→Physical and lethal Physical overflow), `+cast`/`+thread` and
`+spell(s)`/`+powers`/`+complexforms`/`+magic`/`+resonance` (magic is
sheet-tracked and **pool-rolled**, with **Drain**/**Fading** applied to the CM),
`+init [/roll]`, and `+karma` / `+karma/spend` (costed raises applied atomically
on `+job/approve`) / `+karma/award` (staff). The dice RNG is OS-seeded in
production and injectable/seedable for tests. Persistence is identical to `wod`:
the runner sheet is a single `sheet:shadowrun` JSON attribute on the character's
`obj:*` document, so it rides the object write-through and **reloads with the
world** (no separate save path); the reboot round trip is CI-tested against Redis
(`crates/mushd/tests/shadowrun_e2e.rs::sr5_sheet_survives_redis_reboot_gated`).
The sheet query is the schema-generic `<ruleset>.sheet` feed, so the web Sheet
panel renders SR5 with **no** web change. In-game reference: `help shadowrun`.

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
- `[llm.concurrency]` — `max_interactive` / `max_background` in-flight caps
  (plus optional `[llm.retry]` / `[llm.circuit_breaker]`).
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
walk will describe.

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
- on connect it is greeted and dropped into the guided chargen **wizard**
  (`+chargen/wizard`) — transport-agnostic, so SSH and the web client behave
  identically;
- it is held to a small allowlist — chargen (`+chargen`/`+stat`/`+spec`/`+sheet`),
  `look`, `say`/`pose`, `help`, `who`, `quit`, and the OOC channel management verbs
  — and everything else (moving onto the grid, exits, `home`, `+roll`, building)
  is refused with a "finish creating your character" notice;
- **staff (Builder and up) and already-approved characters bypass the gate.**

`+chargen/submit` finalizes the sheet (re-validated), flips it to
`Pending(job)`, and opens a `wod-chargen` approval job. When a storyteller
`+job/approve <job>`s it, the handler stamps the sheet **Approved**, moves the
character to `start_room`, and re-renders — the gate then passes on its own
(the sheet is Approved), so nothing else has to be un-wired. Denying/bouncing the
job leaves the character in chargen to keep editing.

Persistence: the sheet is a `sheet:wod` **attribute on the player's object**, so
it rides `obj:*` write-through; the approval job is a `mod:jobs:*` document. Both
a submitted-but-Pending sheet **and** its later Approved stamp survive a reboot —
CI-tested in memory and against a live Redis
(`crates/mushd/tests/persistence.rs::onboarding_persistence_round_trip*`). The
full new-player→wizard→submit→gate→approve→play journey over real SSH is
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

**Structured logging** is configured under `[log]`:

```toml
[log]
level = "info"      # a tracing env-filter; empty = honor RUST_LOG, else "info".
#                     e.g. "info,mush_llm=debug,mush_net_web=warn"
format = "pretty"   # "pretty" (console) or "json" (one object/line, for shipping)
file = false        # also write daily-rotating logs to <data_dir>/logs/mushd.log.YYYY-MM-DD
```

`format = "json"` emits one JSON object per line for a log shipper; `file = true`
adds a **non-blocking, daily-rotating** file appender under `<data_dir>/logs/`
(in addition to stdout), whose worker is held for the process lifetime. Log lines
carry paths and counts only — never secrets (API keys are read by env-var name and
never logged; raw tokens are never logged). An unknown `format` degrades to
`pretty` with a warning rather than refusing to boot.

Two operational tradeoffs of the file appender are **deliberate**:

- **Lossy under pressure.** The non-blocking appender uses a bounded in-memory
  channel and **drops** log lines rather than block the world loop when it
  fills — so during an error storm some `info`/`warn` lines may be lost. This is
  intentional (the hot path must never stall on disk); the durable trail that
  matters — the LLM/staff **audit** — is a separate Redis stream, not this file.
- **No built-in retention.** Daily rotation keeps files **forever**; nothing
  prunes `<data_dir>/logs/`. Add external rotation — e.g. `logrotate`, or a cron
  sweep like `find <data_dir>/logs -name 'mushd.log.*' -mtime +14 -delete`.

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
| `idx:objs` | FT index | RediSearch index over `obj:*` (staff tooling; non-fatal if absent) |
| `mod:{ns}:{key}` | JSON doc | one game-module document (comsys channels/memberships, mail messages/boxes/folders, jobs/notes, weather `cell:{zone}` cells, module `meta`/`next_id` counters) |
| `mod:{ns}:schema` | JSON doc | that module namespace's schema version (see Migrations) |
| `mod:{ns}:hist:{id}` | stream | persisted channel history (`XADD MAXLEN ~`), and any other module ring history |

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
  modules, rulesets, the LLM layer, staff ops. It holds **no** connection
  listeners. It binds the `[ipc]` socket and waits for the gateway.
- **`mush-gateway`** owns the front door: the SSH **and web** listeners,
  the session layer, account auth + the IP guard, and slow-client / link-dead
  policing (it owns the connection buffers). It **spawns and supervises** the
  `mush-server` child and talks to it over the `[ipc]` socket.

The two halves are a **clean dependency split**: they depend on neither each
other nor the other half's deps. The gateway links only connection-side crates
(`mush-net-ssh` / `mush-net-web` / `mush-session`, plus a `mush-store` KV handle
for `account:*` / `token:*` / `ban:*`) and pulls in **none** of the game stack
(`mush-llm`, the modules, the rulesets, `mush-worldgen`, `mush-npc`). The server
links the game stack and pulls in **none** of the net/session crates. All they
share is the dep-light `mush-config` (the `mushd.toml` model, logging, IP guard,
account helpers) and the `mush-ipc` boundary protocol; the all-in-one `mushd`
binary depends on both halves and supplies the in-process glue. (`cargo tree -p
mush-gateway` / `-p mush-server` are the regression check for this split.)

Both read the **same `mushd.toml`** — each its own relevant sections plus the
shared `[ipc]` one. Point both at the **same Redis** (`[store].redis_url`): the
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
```

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
  crash-loop guard. The WoD/SR ruleset
  working state that shapes that render survives the restart (M17): a cloaked
  vampire stays cloaked, an accepted ST ruling still awaits its throw, an
  extended roll keeps its progress (see *Accepted losses across a restart*).
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
- **WoD / SR ruleset working state survives (M17).** The live, fiction-critical
  scene state the WoD and Shadowrun rulesets keep in their in-RAM state slots is
  now mirrored to their own module namespace (`mod:wod:*` / `mod:shadowrun:*`) on
  every mutation — the same atomic per-command write-through as object writes —
  and re-hydrated at boot **before** the first re-attached `look` renders. So a
  mid-scene restart is invisible where it matters most: an Obfuscate-cloaked
  vampire **stays hidden** (concealment ratings + the per-observer pierced
  verdicts survive), an accepted-but-unthrown Storyteller ruling (M13 pending)
  survives and the next `+roll` still consumes it as ST-authoritative, and an
  in-progress **extended roll** (WoD and SR) keeps its accumulated successes.
  Orphaned entries (an object destroyed while the server was down) and expired
  pendings are dropped on reload, not resurrected. Re-seeded/transient bits are
  **not** persisted and are accepted losses: the dice RNG re-seeds from the OS,
  the per-character roll-log ring (`+roll/last`) comes back empty, and any open
  `+chargen` input context is dropped (re-derivable from the persisted sheet).
- **TacSim scenes + the per-`(scene, obj)` scene registry — future.** Open
  scenes, their `SceneId`s (they re-number from 0 on each boot), the per-`(scene,
  obj)` `RulesetStateStore` (e.g. per-mech combat `heat`), and per-scene map
  blobs are still transient world RAM, rebuilt empty by `World::from_objects` on
  every boot. This is the reserved TacSim seam (it purges stale scene
  documents on boot by design); scene-registry rehydration for actual tactics
  scenes lands **with TacSim**. Until then, a tactics scene in progress when a
  restart hits is not rehydrated — **re-open it afterward**. Everything the
  ruleset persisted to durable object state (sheets, XP, wounds recorded on the
  sheet) survives regardless; only this per-scene combat working state resets.

Nothing else is silently lost. Announce a deploy anyway if players might be
mid-chargen, mid-compose, or mid-tactics-scene, so they know to expect the dropped
editor buffer or to re-open an active TacSim scene.
