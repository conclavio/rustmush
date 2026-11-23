# Whole-instance backup and restore

One command writes one archive of everything a rustmush instance needs to
stand back up; a timer runs it every four hours; a second box fetches the
archives, restores one beside nothing that matters, and proves it is whole.

- **The runbook** (what to type, on a good day and a bad one):
  [`docs/ops.md`, "Whole-instance backup and restore"](../../docs/ops.md#whole-instance-backup-and-restore-doc-52).
- **The design** (what is in an archive, what a cut of a live instance can and
  cannot promise, what the check compares, the owner's open questions):
  [doc 52](../../docs/design/52-instance-backup-and-restore.md).

## Files

| File | Goes to | What it is |
|---|---|---|
| `rmbak.py` | `/usr/local/lib/rustmush-backup/` | The tool: `backup`, `run`, `status`, `show`, `verify`, `pull`, `restore`, `check`, `remove`, `tree`, `facts`. Python 3, standard library only. |
| `backup-serve.sh` | `/usr/local/lib/rustmush-backup/` | The forced command of the copy's SSH key on the box that takes backups: `list`, `status`, `fetch <name>` and nothing else. |
| `backup.conf.example` | `/etc/rustmush-backup.conf` | Which instances, where, how many are kept, and the pull's command. Every key with its default. |
| `rustmush-backup.service`, `.timer` | `/etc/systemd/system/` | The box that takes backups: every four hours at twenty past. **The interval is the timer's one `OnCalendar=` line.** |
| `rustmush-backup-pull.service`, `.timer` | `/etc/systemd/system/` | The box that fetches them: fifteen minutes later. |
| `install.sh` | run from the tree | `install.sh take [--serve-user]` or `install.sh pull`. Installs; enables nothing. |
| `pull-key.sh` | `/usr/local/lib/rustmush-backup/` | **The owner runs it**, on the box that fetches: makes the pull's SSH key, and with `--finish <fingerprint>` trusts the far side's host key, sets `PULL_SSH` and enables the pull. |
| `pull-access.sh` | `/usr/local/lib/rustmush-backup/` | **The owner runs it**, on the box that takes backups: password logins off (and proved off, and a key login confirmed by him) before it admits the pull's key with its forced command and adds one firewall rule. |
| `test_rmbak.py` | stays in the tree | The tests (`python3 -B deploy/backup/test_rmbak.py`), and the tamper tool the runbook's proof uses. |

## In one screen

```sh
# on the box that takes backups (root)
python3 /usr/local/lib/rustmush-backup/rmbak.py backup dev1        # one archive, now
systemctl start rustmush-backup.service                             # what the timer runs: every instance, prune, status
python3 /usr/local/lib/rustmush-backup/rmbak.py status              # the timer, the last run, the newest archives

# on the box that restores (root)
python3 rmbak.py pull                                               # fetch and verify what is new
python3 rmbak.py show   <archive>                                   # its manifest, in brief
python3 rmbak.py verify --deep <archive>                            # checksums, then the snapshot loaded and re-derived
python3 rmbak.py restore <archive> --as dev4                        # a copy: own Redis, units, loopback ports
python3 rmbak.py check dev4                                         # it boots, answers, and agrees with its manifest
python3 rmbak.py remove dev4 --yes                                  # take the copy away

# the bad day, on the instance's own box (docs/ops.md has the steps in order)
python3 rmbak.py tree <archive>                                     # only if the box lost its tree: lay it from the archive
python3 rmbak.py restore <archive> --in-place --yes                 # the store and data back under the instance's own name
```

`restore --as` refuses a name that is not an earlier restored copy, binds
loopback only, turns the LLM and Discord off in the copy's config, and gives
the copy an empty environment file: a copy never spends on a provider, speaks
on Discord or answers the public internet.
