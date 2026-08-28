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
   - shutdown: the clean-shutdown sequence triggers a final `BGSAVE`
     automatically.
   - shell: `redis-cli BGSAVE`.
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
persisted namespace (`mod:comsys:*`, `mod:mail:*`, `mod:jobs:*`) carries
its own schema version in a reserved `mod:{ns}:schema` document, and
`mushd` runs each module's registered migration set at boot under the same
rules (fresh namespace stamped without running; unstamped-with-data treated
as v0; newer-than-binary refused; one BGSAVE before the first step that
actually runs; docs + version bump committed per step, crash-resumable).
Module migrations transform kv documents only — streams (`mod:*` history)
are never migrated.

Operator procedure for upgrades that mention a schema change: take an extra
offsite copy of `dump.rdb`, deploy the new binary, start it, watch the log
for `store schema current` (and any `running module migration` lines).

## Read-only mode (store outage mid-game)

If the store starts failing writes (Redis down, disk full), the writer task
retries the failed batch forever with backoff **in order** — nothing is
dropped or reordered — and the world enters read-only mode. An outage is
*detected* by the store client's per-command timeout (~5s; without it the
Redis client would buffer commands silently until reconnect), so the mode
engages within seconds of Redis going away:

- Commands that change the world are refused with a player-facing notice;
  look/WHO/say/pose and other non-mutating commands keep working from RAM.
  A line that matches an **exit** is refused even when the exit is aliased
  to a read-only verb (`library;l`) — movement mutates. Welcome-screen
  `create`/`newchar` are refused with a "database unavailable" error.
- Connected Wizards get a `GAME:` alert on entry and again on recovery.
- The moment a retry succeeds, the stuck batch (and everything queued behind
  it) lands and the mode lifts automatically. No operator action is needed
  beyond fixing Redis.
- `@shutdown` still works while read-only (you can always stop the server).
- One exception to "nothing is dropped": a batch failing with a
  **deterministic** error (e.g. an `obj:*` key clobbered with the wrong
  type by an outside client) would fail identically forever, so it is
  dropped with a loud `BATCH DROPPED` log line instead of wedging every
  batch behind it; the store self-heals the next time those objects are
  written (batches carry full-object snapshots).

## Shutdown and crash behavior

Three equivalent clean triggers — in-game `@shutdown` (Wizard), Ctrl-C at
the console, SIGTERM (what `systemctl stop` sends):

1. stop accepting new SSH connections;
2. announce (`GAME: Shutdown in progress.`) and stop the world task;
3. drain the store writer (log: `store writer drained and exiting`);
4. final store barrier + `BGSAVE` (log: `shutdown BGSAVE triggered`);
5. exit 0.

`kill -9` / power loss skips all of that and is still safe (write-through);
you lose at most the in-flight command (or the small writer queue if Redis
was stalled at that instant — see "The durability model" above), plus up to
the AOF window on a host crash. The kill-9 path is CI-tested against a live
Redis (`crates/mushd/tests/durability_e2e.rs`).

A clean shutdown started **while Redis is down** still exits within a
bounded time: the writer drain is capped at 15s and each remaining store
step (drain barrier, BGSAVE, close) at 10s, all loudly logged. Batches that
could not be flushed within the cap are lost — the same window a `kill -9`
during the outage would lose.

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
| `idx:objs` | FT index | RediSearch index over `obj:*` (staff tooling; non-fatal if absent) |
| `mod:{ns}:{key}` | JSON doc | one game-module document (comsys channels/memberships, mail messages/boxes/folders, jobs/notes, module `meta`/`next_id` counters) |
| `mod:{ns}:schema` | JSON doc | that module namespace's schema version (see Migrations) |
| `mod:{ns}:hist:{id}` | stream | persisted channel history (`XADD MAXLEN ~`), and any other module ring history |

Do not hand-edit `obj:*`/`meta:*` **or `mod:*`** while the server runs — the
RAM world (and each module's in-RAM state, rebuilt from `mod:*` only at
boot) will not see the change and the next write-through wins. `account:*`
and `ban:*` are read live and are safe to inspect
(`redis-cli JSON.GET account:nick`).

Note for shared instances: the REDIS_URL-gated tests **wipe the `obj:*`,
`account:*`, `ban:*`, `meta:*`, and `mod:*` namespaces** in logical dbs
12–14 *and* in whatever database `REDIS_URL` itself points at (db 0 unless
the URL says otherwise — mush-store's suite uses the URL as given). Never
point `REDIS_URL` at an instance holding real data — module data (channels,
mail, jobs) is wiped along with the world; use a disposable instance. The
RediSearch index only works on db 0, which is one more reason production
uses a dedicated instance on db 0.
