# 52. Whole-instance backup and restore — one archive, a cut of a live instance, a timer, and a restore that proves it

Draft 0.3, 2026-11-23, a design, one tool and its unit files. Written against `origin/main` at
`da2db6ef7`; Draft 0.1, the same day, had every question open, Draft 0.2 recorded the owner's
rulings, and this draft records the first real archives (§7.8). It closes `T-112` of [the simulator backlog](16c-simulator-backlog.md): until now no
instance took a backup, and a restore needed six directories that no recipe copied together with
the store. The tool is `deploy/backup/rmbak.py`; the runbook is
[`docs/ops.md`, "Whole-instance backup and restore"](../ops.md#whole-instance-backup-and-restore-doc-52).

**What is built and proved, and where.** Everything in this draft was run on conclav.dev, against
`dev3`, a throwaway instance deployed there by the repo's own `deploy.sh` and brought up by the
named-launch recipe, and `dev4`, the name its restored copies take (the owner's word of 2026-11-23:
dev3 and dev4 may stand on conclav.dev for this). **Since the merge of PR #778 the tool is installed
on conclav.io**, at the owner's go: the timer runs there every four hours, dev1 and dev2 each have
a first archive, and dev1's was restored on conclav.dev and checked (§7.8). The pull's three
commands are still the owner's to run (§5). The numbers below are measured ones, and §7 says which
run each came from.

**Draft 0.2 records the owner's rulings of 2026-11-23.** Asked Q-52-1, Q-52-2 and for the go on
conclav.io, the owner answered: "instance keys in the archive yes i agree ... how many archives to
keep, 18 on conclav.io is fine, for conclav.dev i would keep the same 18 plus 1 per day from the
last month (so ~45-48 backups) .. let's install node on both boxes so they can do their own full
builds, and any other tools required so my windows box isnt in the loop .. i've added a hostinger
firewall rule to allow 50.6.20.140 incoming to port 2222, i'll let you handle the ufw side after
disabling root ssh password login (i have console access as a backup)".

- **Q-52-1 is ruled as proposed**: the key families and the SSH host key ride in the archive.
- **Q-52-2 is ruled, and not as proposed** for conclav.dev: the newest 18 there too, and beyond
  those one archive a day for the last month. Built as two tiers (§10, Q-52-2).
- **Q-52-4 is ruled by what he did**: the panel now admits conclav.dev (`50.6.20.140`, the address
  on that box's own interface) to port 2222, so the copy is the pull.
- **New scope: each box builds its own deploys**, with the Windows box out of the loop. Built as
  `deploy/vps-runner/build-tools-install.sh` (Node, pinned and verified) and
  `deploy/redeploy/stage-source.sh` (the tree and the web client, made on the box), and proved by a
  full build from a clean checkout on conclav.dev (§7.7). It also changes Q-52-3's reason, not its
  default.
- **The sshd and firewall change stays his to run.** He offered it to the session; a session does
  not change a box's login or firewall settings, so it is one script he runs
  (`deploy/backup/pull-access.sh`, §5), and the key is one more (`pull-key.sh`).
- Q-52-3 and Q-52-5 to Q-52-10 are still open at their defaults.

**The questions** (each in full in §10, with its alternative and reason):

| | Question | Default, or ruling |
|---|---|---|
| Q-52-1 | Do secrets ride in the archive? | **Ruled 2026-11-23, as proposed:** the four key families and the SSH host key **in** the archive; provider keys **never** |
| Q-52-2 | How many archives are kept on each box? | **Ruled 2026-11-23:** 18 an instance on conclav.io; on conclav.dev the newest 18 and one a day for the last 30 days (about 45) |
| Q-52-3 | Do the binaries ride in every archive? | yes: 109 MB an archive instead of 2.7 MB, and a restore needs no build |
| Q-52-4 | Which box starts the copy? | **Ruled 2026-11-23, by the panel rule he added:** conclav.dev pulls, with a key that can list and fetch archives and nothing else |
| Q-52-5 | A staff login for the check of a real archive | none: the check reads the clock from the copy's store; the staff password stays on conclav.io |
| Q-52-6 | Where a failed backup is heard | the failed unit, `rmbak.py status`, and the pull failing on conclav.dev; no pager yet |
| Q-52-7 | Are archives encrypted at rest? | not yet: both boxes are the owner's; required before a copy leaves them |
| Q-52-8 | A standing restore drill | every pulled archive is loaded and re-derived at once; a weekly boot of one, as a later unit |
| Q-52-9 | Help from the server | none needed for the cut; two backlog rows (an honest `@dump`, an offline `--verify-store`) |
| Q-52-10 | dev3 and dev4 after this lands | both removed from conclav.dev once a real archive has been restored there |

A count or a time here is one a run printed. Where a claim rests on reading the code and not on a
run, it says so and names the file.

## 0. In brief

- **One command, one file.** `rmbak.py backup dev1` writes
  `dev1-<UTC>-<commit8>.rmbak.tar.zst`: the store at one instant, the data directory beside it,
  the configuration and unit files as installed, both binaries, and a manifest that lists every
  member with its SHA-256 and carries the **facts** of the store. It is written beside its final
  name, synced, read back whole, and only then moved into place, mode 0640 to one group or 0600.
- **The cut needs no help from the server and does not disturb it.** The store is one Redis fork,
  taken the way a replica takes one; Redis forks between two commands, so every `MULTI`/`EXEC` is
  whole in it. It is the state a `kill -9` at that instant would have left, which the server
  already promises to boot from. The files the store refers to are copied after the fork and the
  copy is kept only if they held still across it (§2.3). On a busy dev3 a cut took 5 to 9 seconds,
  5 of them Redis's own wait before a replica's fork.
- **The facts are derived from the snapshot, never from the live store.** The backup loads its own
  `dump.rdb` into a scratch Redis and reads it there: key counts and a digest by family, every
  ledger's position and digest across the store and the archive tier, each instrument's sum, the
  IC clock's last checkpoint. A restore loads the same file and derives the same facts **before
  the server starts**; any difference fails the restore and nothing boots.
- **A timer, not GitHub.** `rustmush-backup.timer` runs every four hours at twenty past; the
  interval is one line. One run does every instance in turn, prunes, and writes `status.json`.
- **The copy is a pull.** conclav.dev fetches with a key whose only use on conclav.io is a forced
  command: `list`, `status`, `fetch <name>`. The production box gains no way into the build box.
- **The restore stands a copy up beside nothing.** `rmbak.py restore <archive> --as dev4` makes
  the copy its own Redis, units, directories and loopback ports, with the LLM and Discord off and
  no provider key, in 3 to 4 seconds; `rmbak.py check dev4` then holds the booted world to its
  manifest. `--in-place` puts a store and its data back under the instance's own name: the bad
  day's path (§6.4).
- **The IC clock does not jump.** A world restored hours later freezes at the archive's last clock
  checkpoint, counts down five minutes, and resumes from that same IC second. The hours the
  archive spent on disk are discarded, as after any outage (§6.3).
- **The check has teeth** (§7): fifteen kinds of damage, from a flipped byte to a balance edited
  inside the snapshot with every checksum made to match, each refused by name.

## 1. What an instance needs to come back

Paths are dev1's; every instance has the same shape (`deploy/README.md`). "Pinned" means the store
holds something that refers to the file, so the two must be of the same instant.

| What | Where | Pinned by the store | In the archive? |
|---|---|---|---|
| The store: every `obj:`, `account:`, `mod:`, `auth:`, `meta:` key | the instance's own Redis (`/run/redis-dev1mush/redis.sock`) | — | **yes**, `redis/dump.rdb`, one fork |
| The ledger archive tier (sealed chunks the store has been purged of: money history) | `data/archive/<ns>/<log>/*.seg`, `*.idx.json` | yes: a chunk in neither tier is a hole, and the boot refuses it | **yes** |
| The quarantine spool and its resolved records | `data/quarantine/`, `data/quarantine-resolved/` | yes: each record names an exact write | **yes** |
| The four key families: offers, resolution, decisions, geology | `data/offer-keys/`, `resolution-keys/`, `decision-keys/`, `geology-keys/` | yes: offers, prompts and accepted windows carry a key version; the world pins the geology key's fingerprint | **a secret**: in the archive by default, or listed with digests and kept apart (Q-52-1) |
| The SSH host key of the public port | `data/ssh_host_ed25519` | no | **a secret**, as above (Q-52-1) |
| The named-launch document | `data/launch.json` | yes: it is ids from the store, and a missing one refuses the boot | **yes** |
| A bring-up's resume state | `data/bringup/state.json` | yes, during a bring-up | **yes**; the rest of `data/bringup/` is not |
| The rendered config, its inputs | `/opt/rustmush/dev1/mushd.toml`, `instance.env` | `launch_path` follows `launch.json` | **yes**, `config/` |
| The unit files and the Redis config, as installed | `/etc/systemd/system/dev1mush-server.service`, `dev1mush-gateway@.service`, `redis-dev1mush.service`, `/etc/redis/redis-dev1mush.conf` | no | **yes**, `config/units/`, `config/` |
| The commit it runs | `build/.source-commit`, `build/web/dist/.commit` | no | **yes**; the manifest's `commit` and the archive's name |
| Both binaries | `build/target/release/mush-server`, `mush-gateway` | no | **yes** by default (Q-52-3); the manifest records each one's SHA-256, and whether the running process is that file |
| The web client, the unit pack, the sounds | compiled into `mush-gateway` | no | with the gateway binary; there is nothing on disk |
| The help files, the deploy directory, the deploy's scripts, the bundle's stamp and index page | `build/docs/help/`, `build/deploy/`, `build/scripts/*.sh`, `build/web/dist/` | no | **yes** (small): with the binaries it is everything `deploy.sh` reads. The rest of the tree (sources, tests, docs) is **rebuilt from the repo** at the commit when it is wanted |
| Provider keys (LLM, Discord) | `/opt/rustmush/dev1/dev1.env`, root 0600 | no | **never**: a secret the owner holds elsewhere; the manifest names the file |
| The staff password file | `/root/dev1-keeper.pw` | no | **never**: the account's hash is in the store |
| Logs | `data/logs/`, the journal | no | no: the journal keeps them, and they are not needed to stand up |
| The native TLS pair | `/opt/rustmush/dev1/tls/` | no | no: unused behind Caddy, regenerated by the deploy |
| Locks, sockets, drain markers, the world's hold | `/run/…`, `/var/lib/dev1mush-gateway/`, `server.hold` | no | no: restoring one would be a fault |
| Box-wide things: Caddy's config and certificates, the doorway's host key, the client downloads | `/etc/caddy/`, `/var/lib/sshdoorway/`, `/opt/rustmush/clients/` | no | no: they are the box's, not an instance's (§8) |

Three facts in that table were not known before this work and changed the design:

- **The release gateway embeds the web client** (`crates/mush-net-web/build.rs` refuses a release
  build without `web/dist`). A rebuild at the manifest's commit therefore needs Node and the wasm
  toolchain as well as cargo. On 2026-11-23 neither box had Node: the bundle was built on the owner's Windows
  box at each deploy. That is why the binaries ride in the archive by default. The owner's ruling
  of the same day puts Node on both boxes (§7.7), so a rebuild is now possible there; the archive
  still carries the binaries, because a restore that needs no build is the one that works on a
  box that has nothing yet.
- **The store holds no record of a purge.** When the archive pass moves a sealed chunk to disk and
  deletes it from the store, the only record is `purged: true` in the segment's index file
  (`crates/mush-store/src/archive.rs`). So nothing in the store says which files it depends on,
  and the two tiers can only be matched by holding the files still across the fork (§2.3).
- **A key family that is missing is made again, silently.** An empty `offer-keys/` gets a fresh
  key at boot and the boot succeeds; outstanding offers are then void. Only the geology key is
  checked against a pin. A backup that lost a key directory would look healthy until someone used
  an old offer.

## 2. The cut

### 2.1 What the server already guarantees

Read from the code at `da2db6ef7`; the reader was a subagent, and the two claims this design leans
on hardest (one transaction a flush; ledger appends in the same transaction as the documents they
describe) were then tested by the runs of §7.2.

- **One flush is one transaction.** `RedisStore::write` (`crates/mush-store/src/redis.rs`) builds
  one `MULTI`/`EXEC`: every changed object, every module document queued in that engine step, the
  clock checkpoint, `meta:next_id`. A ledger append rides the batch of the command that made it:
  an economy post puts the journal chunk, the idempotency record, the account documents and the
  economy's `meta` into the same transaction (`crates/mush-economy/src/api.rs`, `install_commit`).
- **A single ordered writer.** At most 64 ordinary batches, 16 authoritative ones and 8 control
  jobs wait at any instant (`crates/mush-server/src/persist.rs`). A command whose result must not
  be lost (a money move, an account's creation) does not answer until its batch is acknowledged.
- **Some units span more than one transaction**, and the boot already heals a store cut between
  their halves, because a crash can land there too: a prepared authoritative action is three
  scripts (reserve, finalize, checkpoint) and `reconcile_authoritative_for_writable_boot` replays
  or checkpoints whichever half it finds; a sealed chunk may be written again, byte for byte,
  until its batch is acknowledged; the outbox re-sends at least once. **One unit is not healed:**
  a sign-up at the gateway is three writes (the account, the character, the link), and a cut
  between them leaves an account without its character, which staff repair by hand
  (`crates/mush-gateway/src/accounts.rs`). A backup can catch a sign-up in that window exactly as
  a crash can.
- **`kill -9` is safe** (`docs/ops.md`, "The durability model"): the world reloads from the store.

### 2.2 The store: one fork

`redis-cli --rdb <file>` asks the instance's Redis for a snapshot as a replica would. Redis forks
(after its `repl-diskless-sync-delay`, 5 seconds by default) and streams the child's view into the
file. A fork happens between two commands, and a transaction or a script is one command, so the
file holds every `MULTI`/`EXEC` whole or not at all.

- It does not touch the store's own `dump.rdb`, its save points or its append-only log, and it
  does not block the server: the cost is one fork of a store that is megabytes today.
- It needs no `BGSAVE`, so it cannot collide with one: Redis runs the two forks one after the
  other. Each instance has its own Redis process, and the timer does the instances in turn, so
  two forks never run at once on the box.
- The instant is known to the second: the file records its creation time, and the manifest carries
  it beside the wall-clock window the transfer ran in.

### 2.3 The files: copied after the fork, kept only if they held still

The data directory is copied after the fork. That alone would be wrong in both directions: a file
copied too late could hold a ledger chunk sealed after the fork, which a boot would merge into a
journal that is ahead of every account document; a file copied too early could miss a chunk that
was purged from the store just before the fork, which is a hole.

So the cut is taken like this, and taken again if the last step fails:

1. Read the name, size, modification time and inode of every file the store can refer to
   (everything under the data directory except `logs/`, the bring-up's scratch and lock files).
2. Take the fork (§2.2).
3. Copy the files.
4. Read step 1's list again. If it is the same list, nothing was written, replaced or removed
   from before the fork until after the copy, so the copy is those files **as they were at the
   fork**. If it differs, discard the copy and the snapshot and start again; after four tries the
   backup fails and says a ledger archive pass or a bring-up is running.

Every writer of those files writes a temp file and renames it over, so a change is always a new
inode: the archive pass (`write_durably`), the key families, the spool. A temp file caught in the
directory is left out of the copy and listed as skipped. The timer runs at twenty past the hour,
clear of the archive pass's window (03:00 to 04:00 UTC), so a second try is rare; the test
`test_a_cut_is_taken_again_when_the_files_moved` lands a file between the fork and the copy and
holds the backup to a second attempt.

### 2.4 The guarantee, stated once

> An archive holds the store as one Redis fork left it at one instant T, and every file the store
> refers to exactly as it was at T. It is the state a `kill -9` of the server at T would have left
> behind, with one difference in the backup's favor: nothing acknowledged before T is missing,
> because the snapshot is Redis's own memory and not what had reached its disk.

What follows from that, and what does not:

- **Money is whole.** Every posting is one transaction, so in any snapshot each instrument's
  balances sum to zero and every ledger is contiguous across the two tiers. The backup checks both
  on the snapshot it just took and fails loudly if either breaks (the archive is still written:
  it is the best copy there is, and the live instance needs a look).
- **What was in flight at T is not in it**: the engine step being built and up to 88 queued jobs,
  which the server had not yet told Redis. A player may have read the result of an ordinary
  command that is among them; a command that waits for its acknowledgment (money, an account) is
  either wholly in the archive or was never answered.
- **A unit of more than one transaction can be cut between its halves** (§2.1). The boot heals all
  but the sign-up, and the restore runs that boot.
- **Nothing in RAM by design is in it**: an open editor, a `@wait`, a call to a model in flight
  (`docs/ops.md`, "Accepted losses across a restart"). A running sim-center match resumes from its
  last snapshot in the store, at most five seconds old.
- **It is not a moment the players agreed on.** No one is paused, nothing is announced, and the
  cut can fall in the middle of anything.

### 2.5 What help from the server would buy, and why it is not asked for

Nothing in the cut needs a quiesce. Three things a server-side command could add, none built:

- **A barrier.** A `@dump`-like command that drains the writer and then forks would put every
  command answered before it into the snapshot, closing the in-flight window of §2.4. Today's
  `@dump` does not do this honestly: it reports when Redis accepted the `BGSAVE`, not when the
  save finished, and a "save already in progress" error is reported as success
  (`crates/mush-store/src/redis.rs`, `bgsave`), so the file it points at can predate the writes it
  was ordered behind.
- **A boundary for the sign-up.** One transaction for the account, the character and the link
  would remove the only unit a cut can leave for staff.
- **An offline check in the server's own code.** `mush-server --verify-store <config>` would run
  the boot's own reads (the contiguity scan, the economy's replay and conservation tripwire, the
  key rings) against a store without starting a world. The check in §6 re-derives the ledger and
  conservation rules in the tool, about 150 lines that must follow the server's formats; the boot
  of the copy is the authority, and the tool's own reading is the second opinion.

Q-52-9 proposes two backlog rows and nothing built now.

## 3. The archive

```
dev1-20261123T162000Z-ebe5840f.rmbak.tar.zst      zstd (gzip where a box has no zstd), mode 0640 root:rmbackup
  manifest.json                                    first, so `show` reads it without the rest
  redis/dump.rdb
  data/…                                           the data directory without logs/, bringup scratch, locks
  config/instance.env, mushd.toml, redis-dev1mush.conf, units/*.service
  build/.source-commit, web/dist/.commit, web/dist/index.html, docs/help/…, deploy/…, scripts/*.sh,
        target/release/mush-server, mush-gateway   everything deploy.sh reads: `rmbak.py tree` lays it out
  manifest.sha256                                  last: an archive cut short has none
```

**The manifest** records: the format, the instance, the host, the commit; the wall time, the
fork's window and the snapshot file's own creation second; the IC clock's last checkpoint as a
second and as a date on the game calendar; the Redis version and modules; each binary's SHA-256
and whether the running process is that file; every member's path, size, mode and SHA-256; every
directory (an empty key directory must come back empty, not missing); what was skipped; what is
never archived; the secrets kept apart, with digests; and the facts.

**The facts** are what a restore is held to:

| Fact | What it is | What it catches |
|---|---|---|
| keys, by family | a count and a SHA-256 over every persistent key's name, type and canonical value, for `obj`, `account`, `auth`, `meta` and each `mod:<namespace>` | a store that is not the one backed up: the wrong file, a load that fell back to an empty append-only log, an edit, a module that read a value differently |
| keys with a time to live | counted, not digested (web tokens, tempbans, link codes leave by the wall clock) | more of them than were archived |
| objects, accounts, `meta:next_id`, the schema version | counts and values | a missing or added login; an allocator that went back |
| each ledger | first and last chunk and seq, the entry count, how many chunks are in the store, in the archive tier, in the archive tier only; a SHA-256 over every entry in order | a hole, a reordering, a lost tail, a chunk that differs between the tiers |
| the archive tier | segments, chunks, chunks marked purged; every chunk inflated and hashed against its index | a missing or damaged segment |
| the economy | per instrument, the number of accounts and the sum of their posted balances | money made or lost: the sum is zero or the snapshot is refused |
| the IC clock | the last checkpoint (real second, IC second, generation), the freeze if any, the epoch | a clock that went back |
| the authoritative lane, the quarantine spool | intents, receipts, outbox entries; spool records | a spool that did not come back with its store |

**Safe to leave on disk.** The directory is root's, mode 0750; an archive is 0640 to the group
`rmbackup` when the copy's account exists and 0600 otherwise. It holds password hashes, session
token hashes and, by default, the key families: it is as sensitive as the store. It is written
as `.partial-<name>.<pid>`, synced, read back whole against its own manifest, renamed, and the
directory synced; a run that dies leaves a `.partial-` or `.stage-` entry that the next run
removes after a day, never a file with an archive's name.

**Size and time, measured on dev3** (4,905 keys at the first cut, 11,924 at the last; the store's
snapshot was 3.7 MB): with the binaries 108.8 to 109.9 MB and 8.6 to 9.1 s; without them 2.7 MB
and 5.3 to 6.5 s. The binaries are 388 MB on disk (the server 211 MB, the gateway 177 MB with the
web client inside it) and compress to about 105 MB.

## 4. The timer

`deploy/backup/rustmush-backup.timer` and `.service`, for conclav.io, installed by
`deploy/backup/install.sh take`.

- **The interval is the timer's one `OnCalendar=` line**: `*-*-* 00/4:20:00`, every four hours at
  twenty past. It is changed on the box with `systemctl edit rustmush-backup.timer` (an empty
  `OnCalendar=` line, then the new one); the unit file's comment says so. `Persistent=true` takes
  a run the box slept through.
- **It never overlaps itself and never forks two instances at once.** The service is one `oneshot`
  that does every instance in `INSTANCES` in turn; systemd does not start a second while one runs;
  and the tool holds a lock on the backup directory against a run by hand.
- **It prunes** to `KEEP` archives an instance (Q-52-2) at the end of each instance's turn, and
  sweeps a dead run's leftovers.
- **It fails loudly.** Any instance that fails makes the run exit non-zero after the other
  instances have had their turn: the unit is `failed`, the lines are in the journal at `err`,
  `status.json` in the backup directory says which instance and why, `rmbak.py status` exits
  non-zero, and the pull on conclav.dev fails too, both when the far side reports a failure and
  when the newest archive it holds is older than `MAX_AGE_HOURS` (9): a backup that has silently
  stopped is noticed on the other box within a day. There is no pager behind any of these yet
  (Q-52-6).
- **It yields to play**: `Nice=15`, idle I/O class, a low CPU weight.
- **It cannot send an archive anywhere.** `ProtectSystem=strict` with one writable directory, and
  `PrivateNetwork=true`: it reaches each store over its Unix socket and has no network at all.

The same unit ran on conclav.dev against dev3 (§7.4): 9.1 s, 5.8 s of CPU.

## 5. The copy

**conclav.dev pulls** (Q-52-4). `rmbak.py pull`, run by `rustmush-backup-pull.timer` fifteen
minutes after each backup, asks the far side for its list, fetches what it does not hold, reads
each archive back whole against its manifest, loads its snapshot into a scratch Redis and derives
the facts again (`PULL_DEEP=yes`), and only then keeps it. It prunes to `PULL_KEEP`.

**The key can fetch archives and nothing else.** It is the key of one line in the
`authorized_keys` of a dedicated account, `rmbackup`, on conclav.io:

```
restrict,command="/usr/local/lib/rustmush-backup/backup-serve.sh" ssh-ed25519 AAAA… backup-pull@conclav.dev
```

`restrict` turns off the terminal, every forwarding and the agent. The forced command replaces
whatever the client asked to run; `backup-serve.sh` reads the request as data and answers three:
`list`, `status`, and `fetch <name>` where the name must match an archive's whole name. It cannot
write, delete, name a path or start a shell, and the account can read the backup directory and
nothing else of the instances' (the test `test_serve_and_pull` sends it nine things it must
refuse). This is `ssh … fetch <name> > file` and not `scp`: `scp` needs the SFTP subsystem, which
would let the key read any world-readable file on the box unless sshd's own config were changed to
jail it, and a forced command needs no change to sshd at all.

**Why a pull.** A push would put on conclav.io a key that logs in to conclav.dev. With a pull the
production box holds no credential for anything; the build box holds one that reads backups it is
about to hold anyway.

**What the pull needed, and its cost.** Until 2026-11-23 conclav.dev could not reach conclav.io's
admin sshd: port 2222 was admitted in the Hostinger panel from the owner's address only. The
owner has added the rule for `50.6.20.140`, which is the address on conclav.dev's own interface
(`ip -4 addr show ens3`; no outside service was asked), and the port answers from there now. The
cost of that rule: on 2222 sshd still accepts **root's password** (`conclav-vps` notes of
2026-10-31: `PermitRootLogin yes` and `PasswordAuthentication yes` win there), the panel rule was
what shielded it, and it is now reachable from a box where every session works as root. **Until
step 2 below is run, that exposure is open.**

**Three commands, all the owner's.** They make a key, change sshd's settings and the firewall, and
admit a key: no session runs them. Each script says at its head what it does, stops at the first
thing that fails, and prints the next command with the value it needs already in it.

1. **On conclav.dev**, as root: `bash /usr/local/lib/rustmush-backup/pull-key.sh`. It makes
   `/root/.ssh/rustmush-backup-pull` (ed25519, no passphrase, because a timer uses it; its whole
   power is the forced command of step 2) unless it exists, and prints step 2's command with the
   public key in it.
2. **On conclav.io**, as root: `bash /usr/local/lib/rustmush-backup/pull-access.sh '<public key
   line>'`. In this order:
   - it refuses unless root has a key of its own in `authorized_keys`;
   - it makes the account `rmbackup` (no password, a root-owned `authorized_keys`);
   - it turns **password logins off**: a drop-in, `/etc/ssh/sshd_config.d/00-rustmush-no-passwords.conf`
     (`PasswordAuthentication no`, `KbdInteractiveAuthentication no`, `PermitRootLogin
     prohibit-password`; it sorts first, and sshd keeps the first value it reads), then `sshd -t`,
     then a reload, which ends no session;
   - it proves it twice: `sshd -T` reads passwords off for root and for `rmbackup`, and the
     running sshd, asked on the port, offers neither `password` nor `keyboard-interactive`. If
     either proof fails the drop-in is removed, sshd reloaded, and the script stops;
   - **it waits** for him to prove a key login from his own box in a second terminal and type
     `yes`; anything else puts sshd back as it was and stops;
   - only then it writes the one key line, with the forced command, and adds **one** ufw rule,
     TCP 2222 from `50.6.20.140` and nothing wider, and only if ufw does not already admit that
     port from anywhere (the notes say it does: the panel is what narrows it);
   - it prints its host key's fingerprint inside step 3's command.
3. **On conclav.dev**, as root: `bash /usr/local/lib/rustmush-backup/pull-key.sh --finish
   <fingerprint>`. It reads conclav.io's host key off the wire and keeps it only if it has that
   fingerprint, writes `PULL_SSH` into `/etc/rustmush-backup.conf`, asks for the list, checks that
   the far side refuses a command that is not one of the three, runs one pull and enables the
   pull timer.

Both scripts are tested against stub `sshd`, `systemctl`, `ssh` and `ufw` in a sandbox root
(`OwnerScripts` in `test_rmbak.py`): the order, the refusals before anything is touched, and that
sshd is put back at every stop (not confirmed; `sshd -t` refuses; `sshd -T` still reads passwords
on; the running daemon still offers one). They have not been run on the boxes.

Until the pull is in place an archive crosses by hand through a session's own root access:
`scp -P 2222 root@conclav.io:/var/backups/rustmush/<name> .` and onward to conclav.dev. That is
how §9's first real archive travels.

## 6. The restore and the check

### 6.1 A copy beside nothing: `restore <archive> --as dev4`

1. **Read the archive whole** before anything on the box is touched: every member against the
   manifest. A damaged archive never costs an earlier copy its place.
2. **Refuse a name that is not a copy.** `/opt/rustmush/<name>` must be absent, or carry the
   marker a restore leaves and be replaced on purpose (`--replace`). dev1 can never be the target
   of `--as`.
3. **Lay it out as a deploy does**: `/opt/rustmush/dev4/{build,data,instance.env,mushd.toml}`,
   the data directory owned by the service user with the archived modes, its own Redis
   (`redis-dev4mush`, a Unix socket, the archived module list), its own units.
4. **Make it harmless.** The copy's config is the archived `mushd.toml` with its paths moved and
   eleven keys rewritten: SSH and web on `127.0.0.1` at the given ports, no trusted socket, no
   proxy list, no downloads directory, its own IPC socket and Redis, `[llm].enabled = false`,
   `[discord].enabled = false`. Its environment file is empty: no provider key reaches a copy. Its
   units have `Restart=no`: a copy that will not boot stays down and says so.
5. **Load the store, with the append-only log off.** Redis 8.10 started with `appendonly yes` and
   no append-only directory comes up **empty** beside a `dump.rdb` (tried on conclav.dev: 4 keys
   in the file, `DBSIZE` 0). So the file is loaded by a server run with the log off, the log is
   turned on there, which rewrites it from the loaded data, and only then does the unit start on
   its own config. A restore that skipped this would boot a fresh, empty world and look healthy;
   the next step is what would catch it.
6. **Derive the facts before anything boots and compare them to the manifest.** Any difference,
   and any broken invariant, is printed and the restore stops with the server never started.
7. Start the server, then the gateway.

Measured on conclav.dev, five runs: **2.8 to 4.1 s** from the command to a served copy (reading
and unpacking the archive 0.7 to 2.0 s, the store's load 0.7 s, the facts 0.1 to 0.2 s, the
server's boot 1.0 to 1.1 s).

### 6.2 The check: `check dev4`

| | What it asks | Passes when |
|---|---|---|
| The units | Redis, the server, a gateway generation | all active |
| The boot's log | this run's lines, by invocation id | `world loaded from store` with the manifest's object count; `world timeline installed`; **no line at ERROR that the live instance had not logged itself** (the manifest carries the live server's distinct ERROR lines since its boot, numbers and times taken out; `--known-errors <file>` supplies them for an archive that lists none); none of the boot's refusals (conservation, contiguity, quarantine, a corrupt object); the geology key matches its pin |
| `healthz` | the copy's own web port | `ok`, `store_ok`, `integrity_ok`, `server_ok`. While the world counts down its outage freeze it does not tick, so the server answers **503 with `world_ok=false`**: that passes only with the other three true, and after the thaw the answer must be 200 with all of them |
| The store under the running server | the facts again | every ledger still whole and every instrument still zero; the same accounts; **each ledger's archived entries still there, entry for entry** (a digest over the prefix the archive held); the clock's checkpoint not earlier than the archive's |
| The clock | the freeze in the store | frozen at exactly the archive's last checkpoint, counting down (§6.3); with `--wait-unfreeze`, the freeze gone from the store and the checkpoint moving on from that second, with no login needed |
| A staff session, when a password is given | `connect`, `@tacsim/time`, `time`, `@stats`, `@tacsim/port status` | the archived login works; `@tacsim/time` reads the frozen second; the readiness report's `geology:` line reads `pinned … [ready]` and any line not ready is named (197 of 197 ready on the restored dev3); with `--wait-unfreeze`, the world thaws and the clock runs on from there |

Measured: 1.1 to 1.3 s without the wait, and five minutes more with it (304 s; the clock then read
120 IC seconds past the archive's checkpoint, five real seconds at 24 to 1).

### 6.3 The IC clock on a restore

The clock is not a function of the wall clock alone: the world checkpoints its IC second in every
batch it commits, and a boot measures its outage from the last checkpoint in the store
(`docs/ops.md`, "Outages"). A restore is an outage as long as the archive's age:

- **The world freezes at the archive's last checkpoint**, at most a second before the fork on a
  busy world and ten on an idle one, for `[game].outage_countdown_s` (300 s). It refuses world
  actions meanwhile; reads, chat and `@tacsim/time` answer. Then it thaws and the clock runs on
  **from that same IC second**. The boot logs `boot_outage=OutageFreeze`.
- **The time the archive spent on disk is discarded.** Nothing catches up: a journey that had
  three IC days to run still has three. Everything keyed to IC time agrees with the store,
  because both are of the same instant.
- **What is keyed to the wall clock ran on**: a web session token, a tempban and a link code
  expire by real time, so a restore a week later comes up with every web login expired. The
  archive tier's `retain_hours` and the rate limits are real time too.
- **A restore within thirty seconds of the fork** is under the threshold and re-anchors silently,
  with no freeze (seen once, §7.1).
- **A box whose clock is behind the archive's** would not measure an outage at all and would run
  the clock from its epoch. The check fails a copy whose checkpoint reads earlier than the
  manifest's.
- A deliberate freeze (`@tacsim/time freeze`) in the archive stays a freeze.

This is the outage policy the owner already ruled (doc 27, Q-R4), met by a restore; this doc adds
nothing to it. A restored copy on conclav.dev runs its clock on from the cut, apart from the live
instance's, from the moment it thaws.

### 6.4 The bad day: `restore <archive> --in-place`

Putting dev1's store and data back under dev1's own name, on conclav.io or on a box rebuilt in its
shape. The copy path above proves an archive; this path uses one. The runbook has the commands;
the order is:

1. The tree at the manifest's commit, installed as a deploy installs it, with the world held
   (`SERVER_HOLD=1 deploy.sh`): units, Redis, the rendered config. On a box that has lost its
   tree, `rmbak.py tree <archive>` lays it from the archive: the archive carries everything
   `deploy.sh` reads, so no repo and no build are needed.
2. Both halves stopped. `restore --in-place --yes` refuses while either runs.
3. The data directory that is there is moved aside (`data.pre-restore-<UTC>`), never deleted; the
   archive's takes its place.
4. The Redis files that are there are moved aside the same way
   (`/var/lib/redis-<inst>mush.pre-restore-<UTC>`); the store is loaded into the instance's own
   Redis by the same log-off sequence, and the facts are compared before anything else happens.
5. `deploy.sh` again (it renders `launch_path` from the restored `launch.json`), the hold lifted
   by hand, the units started.
6. `check <instance>`.

Rehearsed on conclav.dev as dev3 wiped and restored onto itself: §7.5.

## 7. What was run, and what it showed

All on conclav.dev, 2026-11-23, tool at this PR's head, dev3 at `da2db6ef7`.

### 7.1 The cycle

| Step | Result |
|---|---|
| `backup dev3`, the world up and the Registry rating a line | 108.8 MB, 130 files, 4,905 keys, 7 accounts, 4 ledgers, 52 economy accounts summing to 0, IC 2353-09-12 15:36:48; 8.6 s, one attempt |
| `restore … --as dev4` | the restored store matches its manifest; up in 2.8 s |
| `check dev4 --staff keeper` | every check passed: 1,069 objects loaded, `boot_outage=OutageFreeze`, no ERROR, `healthz` ok, all 4 ledgers entry for entry, `@tacsim/time` "frozen at IC 74226209808", the archive's checkpoint to the second; `time` reads September 12, 2353, 15:36 |
| the same, restored 27 s after its backup | `boot_outage=SilentReanchor`, no freeze, the clock 24 IC seconds on: the under-threshold branch |

### 7.2 A busy world

Ten backups back to back (without binaries, 5.3 to 6.5 s each) while a staff session posted money
about six times a second and the Registry rated: the journal's last seq went 167, 214, 260, 301,
341, 381, 421, 462, 502, 542 across the ten. **Every one took its cut in one attempt, every
instrument summed to zero in every snapshot, every ledger was contiguous, and each archive loaded
back to its own facts.** This is the run that tests §2.1's two claims: a posting cut in half would
have shown as a sum that is not zero or a journal ahead of its accounts.

### 7.3 The check has teeth

`deploy/backup/test_rmbak.py tamper <archive> <case> <dir>` writes a damaged copy of a real
archive. Each case below was run against dev3's 108.8 MB archive; the same cases run against a
small synthetic instance in the test suite on every PR.

| Damage | Caught by | In the words |
|---|---|---|
| one byte of the compressed file flipped | `verify` | `the archive cannot be read: unexpected end of data` |
| the last third cut off | `verify` | the same |
| one byte of a key file changed, the manifest untouched | `verify` | `data/resolution-keys/resolution-key-0000000001.hex does not match its manifest entry` |
| the store's `dump.rdb` left out | `verify` | `1 listed member(s) are missing, the first 'redis/dump.rdb'` |
| an extra file slipped in | `verify` | `holds 'unlisted-extra', which the manifest does not list` |
| **every checksum made to match**, and one account's balance raised by 1000 inside the snapshot | `verify` passes; `verify --deep` and `restore` refuse | `key family mod:economy has the manifest's 467 keys and a different digest`; `instrument tacsim.credit does not conserve: its posted balances sum to 1000, not 0` |
| checksums matched, a middle chunk of the money journal deleted | `--deep`, `restore` | `ledger economy/journal: chunks 2 to 2 are in neither the store nor the archive tier`; `seq 39 is followed by 65` |
| checksums matched, the journal's last entry deleted | `--deep`, `restore` | `ledger economy/journal is {… "last_seq": 108 …}, the manifest says …` |
| checksums matched, a login added | `--deep`, `restore` | `accounts is 8, the manifest says 7` |
| checksums matched, the clock's checkpoint moved back a day | `--deep`, `restore` | `clock is {"checkpoint": {… "ic_unix": 74226123408 …`; `key family mod:rustmush_kernel_time … a different digest` |
| checksums matched **and the manifest's facts replaced by the damaged store's own**, the balance raised | `--deep`, `restore` | `instrument tacsim.credit does not conserve` (an invariant needs no manifest) |
| the same, the chunk deleted | `--deep`, `restore` | `chunks 2 to 2 are in neither the store nor the archive tier` |
| checksums matched, a segment of the ledger archive tier removed | `--deep`, `restore` | `archive tier: org/audit chunk 0 (…): cannot be read back: … No such file or directory`; `archive_tier is {"chunks": 6, …}, the manifest says {"chunks": 7, …}` (§7.6) |
| checksums matched, one byte of a ledger archive segment flipped | `--deep`, `restore` | `archive tier: org/audit chunk 0 (…): cannot be read back: Error -3 while decompressing data: incorrect data check` (§7.6) |

Run as real restores over a standing dev4: the balance case stopped with
`the restored store does not match its manifest in 4 way(s); nothing was started on it`, the
server and gateway inactive; the flipped byte was refused before the standing copy was touched,
and that copy was still serving.

**One damage nothing catches**, and it is stated here so it is not discovered later: with every
checksum made to match, one byte of a **decision** key was changed, and the copy restored, booted
and passed every check. A key file is 32 random bytes; nothing in the store can tell one from
another except the geology key, whose fingerprint the world pins (the check reads the boot's
`geology key matches its pin`). The archive's checksums protect a key from the moment of the
backup onward; a key file damaged on the live box before a backup is backed up damaged.

### 7.4 The timer's unit, and the pull

`systemctl start rustmush-backup.service` on conclav.dev, `INSTANCES=dev3`: 109.9 MB in 9.1 s of
wall clock and 5.8 s of CPU inside the unit's sandbox (no network, one writable directory, the
scratch Redis as the `redis` user in the unit's private temp directory). `rmbak.py status` printed
the newest archive and exited non-zero until the timer was enabled. Enabled, the timer then fired
by itself at 16:20:06 UTC: 110.7 MB in 9.2 s, the oldest archive pruned (`KEEP=4` there), the
unit `Deactivated successfully`, `status` clean, the next run listed for 20:20. `rmbak.py pull` through
`backup-serve.sh` (a local stand-in for `ssh`: no key was made) fetched both archives, verified
and deep-verified each, in 2.2 s.

### 7.5 The bad day, rehearsed

dev3 wiped and restored onto itself, while its Registry was in the middle of rating a line.

| Step | Result |
|---|---|
| `backup dev3`, the world running | 16,099 keys, 7 ledgers, the money journal at seq 935 in 35 chunks, IC 2353-09-13 01:31:36 |
| the disaster | both halves stopped; `scripts/reset-instance.sh --yes --no-start`: `DBSIZE` 0, the data directory empty, `launch_path` commented out, the world held; and the SSH host key deleted too, which the reset spares |
| `restore --in-place --yes` | **2.3 s**. The Redis files that were there kept at `/var/lib/redis-dev3mush.pre-restore-<UTC>`; `the restored store matches its manifest: 16099 keys, 7 accounts, 7 ledgers whole across both tiers, every instrument sums to zero`; nothing started |
| `rm server.hold`, then `deploy.sh` | **5.1 s** to a served world; `launch_path` live again, rendered from the restored `launch.json` |
| `check dev3 --staff keeper` | every check passed in **1.3 s**: 1,069 objects, no ERROR, all 7 ledgers entry for entry, the geology key matches its pin, the archived login works. The host key's fingerprint is the one from before the wipe |

**8.7 s from a wiped instance to a checked one.** The whole outage was 16 s, under the
thirty-second threshold, so the boot re-anchored silently.

**And on a box that has lost the instance altogether**: the same run again after dev3's units,
Redis, config, tree and data were all removed (`/opt/rustmush/dev3` gone), with the archive and
the tool and nothing else.

| Step | Result |
|---|---|
| `rmbak.py tree <archive>` | the build tree laid from the archive: both binaries, the stamps, the help files, the deploy directory and the deploy's scripts, which is everything `deploy.sh` reads (`web bundle: source commit … matches`) |
| an empty environment file, then `SERVER_HOLD=1 deploy.sh` | the units, Redis and the rendered config installed, the world held: 15.1 s for the two steps |
| `restore --in-place --yes` | 2.4 s; the held deploy's empty data directory and Redis files kept aside; the store matches its manifest |
| `rm server.hold`, `deploy.sh` | 5.0 s to a served world |
| `check dev3 --staff keeper` | every check passed; the host key's fingerprint unchanged |

**22.5 s from nothing to a served world**, with no repo, no build and no toolchain on the box.
This outage was 32 s, so the boot froze (`boot_outage=OutageFreeze`) and the check, waiting for
the thaw to ask the readiness report, took 301 s. `@tacsim/port status` is not answered during
the countdown; asked right after the thaw it read 190 of 197 lines ready, the other seven being
surface sites still mapping their ground after the boot (after the first rehearsal's boot all
197 read ready ninety seconds on). The check reports lines that are not ready as a note and
fails only on the geology line.

### 7.6 The archive tier

On dev1 and dev2 today the tier holds copies and has purged nothing (dev1's first archive carried
22 chunks in 5 segments, none purged): a chunk is purged only above 512 MB of ledger resident in
the store, and the stores are megabytes. So the purge was made to happen on dev4: its
`[store.archive]` set to an all-day window, no retention and zero watermarks, and the server
restarted.

| Step | Result |
|---|---|
| the pass, a minute after the boot | `ledger archive: pass complete copied=7 purged=7`: four segments on disk; the money journal has one chunk left in the store and five only in the tier |
| `backup dev4` | `economy/journal: seq 0..112 in chunks 0..5 (1 in the store, 5 only in the archive tier)`; `org/audit` and `sim/recovery` each one and one; every ledger whole across both tiers |
| `verify --deep` | the snapshot loads to the manifest's facts |
| a segment removed, and a byte of one flipped (checksums matched) | refused, as in §7.3's last two rows |
| `restore … --as dev4 --replace`, `check` | the store matches; the boot logs `ledger archive tier attached … chunks=7 purged=7` and `economy: journal replayed at boot entries=113 … replayed=113`: the server read the whole journal back through the tier; every check passed |

A cut that lands in the middle of a pass is the case §2.3's second attempt is for. It is held by
a test (a file landed between the fork and the copy, a second attempt taken), and was not met
live: the pass on dev4 took under a second.

### 7.7 A box builds its own deploy

The owner's ruling of 2026-11-23: both boxes do their own full builds, the Windows box out of the
loop. Where that box was in the loop, and what replaces it:

| The Windows box did | Now |
|---|---|
| built the web client at the commit (`npm ci && npm run build`: Node 22, the wasm32 target) and shipped `web/dist` as a tar | `deploy/redeploy/stage-source.sh` builds it on the box, as an unprivileged user |
| made the tree tar (`git archive` of its own checkout) and piped it over ssh | the same script makes it from the box's own clone of the public repository |
| carried the redeploy scripts inside that tar | unchanged: they are still read out of the tree tar, which the box now makes |
| carried archives between the boxes, by `scp` through a session | the pull (§5) |
| built the bundle for any release build on conclav.dev | the box does |

What **stays on the owner's Windows box, by design**, and why a box never needs it. The owner, the
same day: "my windows box will stay in the loop for 3d asset management where the raw assets are
required". The goal is that the two boxes build and deploy the server, the web client and its
WebAssembly on their own; anything that needs the raw 3D assets is not theirs to do.

- **Everything that reads the raw assets**: the purchased packs themselves, the Unity project,
  and the bake steps (`scripts/unit-bake`, `ship-bake`, `planet-bake`, `ground-extract`: Unity or
  Blender over the packs). **No asset tooling is installed on a box and no raw asset is moved to
  one**; the pack models never go in the public repository (the owner's ruling of 2026-10-29).
- **What a box build consumes instead, and where it gets it**: the *baked* output of those
  steps, which is committed, so a box gets it from its clone of the repository at the commit it
  builds and from nowhere else. That is 60 model files under `crates/mush-pixel/models/` (56 MB,
  each naming the pack, version and prefab it was baked from), the looks under
  `crates/mush-pixel/looks/`, and the content packs under `crates/tacsim/content/`. The web
  build copies the models and the designer's catalogue out of those directories
  (`web/scripts/build-wasm.mjs`); the server compiles the rest in. A change to a model reaches a
  box as a commit: baked on the Windows box, committed, built on the box.
- A build from a clean clone (below) is the proof that nothing else is read.
- **The Unity client** (`scripts/release-unity-client.ps1`): built with Unity and uploaded to
  `/opt/rustmush/clients`. It is not part of a server deploy.
- **The operator's seat**: the root SSH key, the `gh` login, the provider keys placed in
  `<inst>.env`. These are who runs a deploy, not what it is built from.

**What a box needs**, beyond what `deploy/vps-runner/provision.sh` already installs (the C
toolchain, git, Redis with its modules, a pinned rustup and toolchain for `ghrunner`):
`deploy/vps-runner/build-tools-install.sh` installs **Node 22.23.5**, the official linux-x64
tarball from nodejs.org, verified against the SHA-256 nodejs.org publishes for it and pinned in
the script, unpacked root-owned under `/usr/local/lib/nodejs` with `node`, `npm` and `npx` linked
into `/usr/local/bin`; and `xz`, `zstd`, `python3`, `git` from apt where one is missing. No
third-party apt source is added (Ubuntu 24.04's own Node is 18, which the web build refuses). The
wasm32 target is added by `stage-source.sh` in the tree it builds, so it always belongs to the
toolchain that commit pins.

**Proved on conclav.dev**, from nothing but the box and github.com, the npm registry and
static.rust-lang.org: `build-tools-install.sh` (the checksum `OK`, Node v22.23.5, npm 10.9.9);
`COMMIT=main bash stage-source.sh dev3` cloned the repository, resolved `4b8d7c56b`, built the web
client as `ghrunner` and left the two tars in **25 s** (the client itself 19 s); then both halves
from those two tars into an empty target directory, `--locked`, as `ghrunner`: the bundle guard
passed, both binaries built in **324 s** at 20 jobs with no warning, and both report IPC version
15. **349 s from a clean checkout to both release binaries, with no other machine involved.**
conclav.io gets the same script at §9's step 3; its build is about 25 minutes at four jobs.

### 7.8 The first real archives (conclav.io, after the merge)

2026-11-23, the tool as merged (`ee2fce6d`), at the owner's go. dev1 and dev2 were never
restarted or reconfigured.

| Step | Result |
|---|---|
| `build-tools-install.sh` on conclav.io | the checksum `OK`; Node v22.23.5 under `/usr/local`, beside Ubuntu's own 18.19.1 in `/usr/bin`, which the web build refuses and which is left as it was |
| `install.sh take`, `INSTANCES=dev1 dev2` | the tool, the conf and the units; nothing enabled |
| `systemctl start rustmush-backup.service`, by hand | **dev1**: 113.0 MB, 139 files, 29,868 keys, 8 accounts, 8 ledgers, IC 2353-09-16 10:42:48, 53.2 s. **dev2**: 112.6 MB, 135 files, 27,742 keys, 7 accounts, 8 ledgers, 26.5 s. One attempt each; 1 min 32 s of CPU for the two at the lowest priority |
| the live servers meanwhile | no line at WARN or ERROR in either server's log across the run (a lagging world loop logs one); `healthz` `loop_lag_ms` 0; no restart; Redis's fork 7.1 ms on dev1 and 4.1 ms on dev2 |
| `systemctl enable --now rustmush-backup.timer` | `status` clean; the next run listed for 20:20 UTC |
| dev1's archive to conclav.dev, by `scp` through the session | 4.1 s out and 3.7 s in; `verify --deep` there 1.6 s |
| `restore … --as dev4 --replace --cpus 24-31` | `the restored store matches its manifest: 29867 keys, 8 accounts, 8 ledgers whole across both tiers, every instrument sums to zero`; **a served copy in 5.4 s** (5.5 s the first time): 2.3 s to read and unpack, 1.1 s the store's load, 0.5 s the facts, 1.2 s the boot |
| `check dev4 --wait-unfreeze --known-errors <live dev1's ERROR lines>` | every check passed: 1,070 objects, `boot_outage=OutageFreeze`, all 8 ledgers entry for entry, frozen at IC 74226537768, the archive's checkpoint to the second; `healthz` 503 with `world_ok=false` during the countdown and 200 after it; the clock 600 IC seconds on after the thaw; the geology key matches its pin |

dev1's world in the archive: 54 accounts of `tacsim.credit` and one of `tacsim.licence.freeport`,
each summing to zero; the money journal at seq 657 in 29 chunks; 22 chunks copied to the archive
tier and none purged.

**Two things the real restore found, neither the restore's fault, both now understood by the
check:**

- **dev1 and dev2 each log one ERROR a boot, and so does a copy of either.** `tacsim::port::stock:
  a port storefront's stock and its listing disagree vault=18 good=propellant listing=65`: once in
  dev1's log and once in dev2's since their boots of 2026-11-22, and again in the copy at its
  first tick after the thaw. The copy is repeating the live world, which has a standing fault at
  that storefront. A check that failed on every ERROR would fail every restore of these worlds
  for a reason that has nothing to do with the restore; one that ignored ERRORs would be blind.
  So a backup now records the live server's distinct ERROR lines since its boot (`live_errors`
  in the manifest), and the check passes an ERROR only if the live instance had logged the same
  one. Run without that list, the same check fails on this line, as it should. The storefront
  itself is the game's to fix, and is reported to the lead.
- **`healthz` answers 503 for the whole five-minute countdown.** A frozen world does not tick,
  the loop's timestamp goes stale after five seconds, and `world_ok` reads false. The first check
  ran three seconds after the boot and saw 200; one a minute later saw 503. This is the server's
  behavior on dev1 and dev2 too, after any restart more than thirty seconds after the last
  checkpoint: anything that watches `healthz` will call the instance down for five minutes.
  Reported to the lead; the check now expects it.

The archives these runs took were written before `live_errors` existed, so the check was given
live dev1's ERROR lines by `--known-errors`. The tool on conclav.io is brought up to this PR
once it has merged, and archives carry the list from then on.

## 8. What this does not do

- **The box.** Caddy's config and certificates, the doorway's host key and config, the firewall,
  the panel rules, the runner and the client downloads are the box's, not an instance's. A box
  that is lost is rebuilt from `deploy/README.md` and the notes; an archive then puts an instance
  back on it. A recipe for the box is its own piece of work.
- **Provider keys.** A restored production instance needs its environment file put back by the
  owner. A copy never has one.
- **Point-in-time recovery.** An archive is one instant every four hours. Replaying the store's
  append-only log up to a chosen second is possible in Redis and is not built.
- **A third place.** Two boxes of one owner, in two providers. A copy outside both is Q-52-7's
  subject.
- **A migration.** A restore runs the archive's own binaries on the archive's own store. Booting
  an old store on a newer build is the migration machinery's job and not this tool's.
- **Proof that the copy plays.** The check proves the world boots, serves, holds its ledgers and
  its clock. It does not fly a ship. A bot's sortie on a restored copy would be a stronger drill
  (Q-52-8).

## 9. Order of work

| Step | What | State |
|---|---|---|
| 1 | The tool, the units, the tests, this doc and the runbook; the cycle proved on conclav.dev | **this PR** |
| 2 | The owner rules Q-52-1 and Q-52-2 | **ruled 2026-11-23**; the two-tier retention, the build tools and the owner's two scripts added to this PR |
| 3 | On conclav.io: `build-tools-install.sh`, `install.sh take`, `INSTANCES`, one run by hand, the timer | **done 2026-11-23** (§7.8) |
| 4 | The first real archive of dev1, carried to conclav.dev by hand, restored as dev4 and checked; the restore time and what the check found in the runbook | **done 2026-11-23** (§7.8): 5.4 s to a served copy |
| 4a | The tool on conclav.io brought up to the follow-up PR (`live_errors` in each manifest) | after that PR merges |
| 5 | The pull: §5's three commands | the owner's to run |
| 6 | dev3 and dev4 removed from conclav.dev (Q-52-10) | after step 4 |

Not in these steps: a weekly boot drill (Q-52-8), a pager (Q-52-6), encryption (Q-52-7), the two
server rows (Q-52-9).

## 10. Owner survey

Every question has a proposed default, an alternative and a reason. **(expensive to reverse)**
marks an answer that would be costly to undo.

**Q-52-1 — Do secrets ride in the archive?** **Ruled 2026-11-23: accepted as written.**
- **Proposed default:** the four key families and the SSH host key are **in** the archive
  (`SECRETS=in-archive`). Provider keys and the staff password file are **never** archived; the
  manifest names them.
- **Alternative:** `SECRETS=apart`: the key files are left out, the manifest lists each with its
  SHA-256, and a restore takes them from a directory the owner keeps (`--secrets <dir>`, checked
  against the digests) or stands the copy up with fresh keys (`--without-secrets`). Built and
  tested.
- **Reason:** the two kinds of secret are different. A key family is made by the server, seen by
  no one, and exists only on that disk; the store is bound to it, and a missing one is replaced
  silently at boot (§1). If it is not in the archive there is no other copy, and an archive without
  it restores a world whose offers and prompts are void and whose mining is off. A provider key is
  issued by someone else, held by the owner elsewhere, worth money outside the game and replaceable
  in a minute. The archive already holds every password hash and session token hash, so it is
  already as sensitive as the store; the key families add nothing an attacker with the archive
  could use without also running the world. Kept apart, the keys need their own backup, and that
  second backup is the thing most likely to be forgotten.

**Q-52-2 — How many archives are kept on each box?** **Ruled 2026-11-23: 18 on conclav.io as
proposed; on conclav.dev the newest 18 and one a day for the last month, not the proposed 84.**
- **As ruled and built:** `KEEP=18` on conclav.io (three days at four hours). On conclav.dev
  `PULL_KEEP=18` and `PULL_KEEP_DAILY_DAYS=30`: the newest 18, and beyond those **one archive a
  UTC day, the day's last one**, for the 30 days up to the newest archive's own day. That is 45
  archives when backups run every four hours (18, and 27 dailies the 18 do not already cover).
  Three choices inside the ruling, each the session's and each one line to change:
  - *The day's last archive is the one kept.* By the time a day has left the newest 18 it is
    over, so its last archive is settled, and it is the one that holds the whole day.
  - *Days are UTC days*, as the archive names are.
  - *The month is counted back from the newest archive held, not from today.* If backups stop,
    nothing ages away: the history a dead backup left is exactly what one would want kept.
  The pull fetches only what this rule would keep, so nothing is fetched to be pruned in the same
  run. With the binaries in, 110 MB each: 2.0 GB an instance on conclav.io (4 GB for dev1 and
  dev2) and 5 GB an instance on conclav.dev (10 GB for both).
- **Proposed default, superseded:** `PULL_KEEP=84` on conclav.dev (two weeks of every archive).
- **Alternative:** fewer on the production box (6: one day, 1.3 GB for both) and the history only
  on conclav.dev; or a ladder (every four hours for two days, then one a day for a month), which
  is more code than a count.
- **Reason:** the production box needs enough to cover a weekend of the copy being down; the
  build box is where history is cheap. Nothing is live yet, so two weeks is already more history
  than a pre-launch world has use for; it is a number in a file.

**Q-52-3 — Do the binaries ride in every archive?**
- **Proposed default:** yes (`BINARIES=yes`).
- **Alternative:** `BINARIES=no`: an archive is 2.7 MB instead of 109 MB, and a restore needs
  both binaries built at the manifest's commit (`--binaries <dir>`). Built and tested.
- **Reason:** the gateway embeds the web client, so a rebuild needs Node and the wasm toolchain as
  well as cargo. When this was written neither server had Node (§1); since the owner's ruling of
  2026-11-23 both can build (§7.7), so the alternative is now workable and costs a build: 5 minutes
  on conclav.dev, about 25 on conclav.io. On a bad day "build it first" is still that wait, on a
  box that must already have its toolchain. With the binaries in, a copy is up in four
  seconds on any box with Redis, an instance is back from nothing in 22.5 s (§7.5), and what
  runs is byte for byte what ran. The cost is disk and
  about 1.3 GB a day between the two boxes, which neither will notice. If it ever matters, the
  middle path is to carry the binaries only in the first archive after each deploy.

**Q-52-4 — Which box starts the copy?** **Ruled 2026-11-23 by the panel rule the owner added for
conclav.dev's address: the pull.**
- **Proposed default:** conclav.dev pulls, with a forced-command key (§5).
- **Alternative:** conclav.io pushes to an account on conclav.dev whose forced command only
  receives archives. It needs no panel rule and no change to conclav.io's sshd.
- **Reason:** with a pull the production box holds no key for anything. The pull's cost is the
  panel rule for port 2222, and with it the need to turn password logins off there first (§5),
  which is worth doing on its own account. If the owner would rather not open 2222 to a second
  address at all, the push is the answer, and the receive script is a day's work.

**Q-52-5 — A staff login for the check of a real archive.**
- **Proposed default:** none. The check of a dev1 or dev2 archive on conclav.dev runs without a
  staff session: the boot, `healthz`, the facts, and the clock read from the copy's own store.
  The owner can log in to the copy himself, through an SSH tunnel to its loopback port, with his
  own account.
- **Alternative:** a copy of `/root/dev1-keeper.pw` placed on conclav.dev, so the check also
  proves an archived login works and asks `@tacsim/time` at the front door.
- **Reason:** keeper is God on the live dev1. Its password on the build box, where every session
  works as root, is a larger exposure than the two extra lines of proof are worth; the rehearsal
  on dev3 already proved the login path with a throwaway world's password.

**Q-52-6 — Where a failed backup is heard.**
- **Proposed default:** nowhere new. The unit fails, `rmbak.py status` and `status.json` say why,
  and the pull on conclav.dev fails within a day (§4). Someone has to look.
- **Alternative:** an `OnFailure=` unit that posts to a place the owner reads: a Discord webhook,
  an email, a phone push.
- **Reason:** there is no alerting on either box today for anything, and one built for backups
  alone would be the wrong shape. The hook is one line in the unit when there is a place to post.

**Q-52-7 — Are archives encrypted at rest?**
- **Proposed default:** not yet. They are root-only files on two boxes of the owner's.
- **Alternative:** encrypt each archive to a public key whose private half is not on the box that
  takes backups (`age`), so a stolen backup directory is useless. conclav.dev would then need the
  private half to run its checks, which moves the question to that box.
- **Reason:** it protects against a copy leaving the two boxes, and today none does. **It becomes
  required the day an archive goes to a third place** (object storage, a laptop), and that is
  when to build it.

**Q-52-8 — A standing restore drill.**
- **Proposed default:** every pulled archive is loaded and its facts derived again at once
  (`PULL_DEEP=yes`, built: two archives in 2.2 s). A weekly boot of the newest archive as dev4,
  checked and removed, is a later unit, outside the soak's hours and pinned to cores 24 to 31
  (`--cpus`).
- **Alternative:** boot every pulled archive (twelve boots a day), or none until asked.
- **Reason:** the load-and-derive catches everything in §7.3's table except what only a boot
  reads. A weekly boot catches a build that can no longer read its own store. Twelve a day buys
  nothing more on a world with no players.

**Q-52-9 — Help from the server.**
- **Proposed default:** none now. Two rows for the backlog: `@dump` reports when its save has
  finished and no longer calls a refused save a success; `mush-server --verify-store` runs the
  boot's reads against a store without starting a world (§2.5).
- **Alternative:** build a draining snapshot command first and have the timer use it.
- **Reason:** the cut is already whole without it; the barrier only moves which in-flight
  commands are in the file. The offline check is the more useful of the two, because it would let
  the pull use the server's own rules in place of the tool's reading of them.

**Q-52-10 — dev3 and dev4 after this lands.**
- **Proposed default:** both removed from conclav.dev once a real archive has been restored there
  (the runbook has the commands). dev4 is made again by each `restore --as`.
- **Alternative:** keep dev3 standing as the rehearsal's source, with its own four-hourly timer.
- **Reason:** a standing instance on the build box is one more thing the soak and the gate share
  cores with, and a real archive is a better subject than a throwaway's.

## 11. What this work did not check

- **conclav.io under play.** §7.8's run was on an idle evening with nobody connected: the fork
  took 7 ms and neither server logged a lagging loop. The same under a full sim center is not
  measured.
- **The timer's own first run on conclav.io** (20:20 UTC, 2026-11-23) had not come round when this
  was written. The same unit ran by hand there, and the same timer fired by itself on conclav.dev.
- **A large store.** The facts read every key; at 12,000 keys that is a fifth of a second. At a
  million it will be minutes and want a look.
- **The key families other than geology** (§7.3).
- **Redis across versions.** Both boxes run 8.10.3. An archive loaded by an older Redis, or with
  a module missing, fails at the load and says so; a newer one is expected to read it and is not
  tried.
- **The pull over a real SSH key** and the two scripts of §5 on the boxes themselves: tested
  against stubs, not run. The owner runs them.
- **A build on conclav.io.** §7.7's proof is conclav.dev's; conclav.io has the same Node now and
  has not built with it.
