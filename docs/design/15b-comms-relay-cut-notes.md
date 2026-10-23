# 15b — Relay backbone implementation evidence

## R1 — Registry and first seed (2026-10-23)

**Candidate:** `916e8fee`, based on `origin/main` at `7c886408`.
**Status:** implemented and deployed; targeted and live integration checks passed.
**Full milestone acceptance: not yet claimed** under 15a §5, because the normal
workspace CI run is outstanding. No criterion is waived. Work is paused for the
owner's review of these results before R2.
R2 has not started. This record implements Gate 1 of
[15a](15a-comms-relay-implementation-plan.md).

### What this cut implements

`mush_sim::relay` owns one versioned catalog at `mod:sim:relay:registry`.
It records stable pair/half/node identities, installed object bindings, explicit
room coverage, operator identity, capacity and link policy. Link state is derived:
both halves enabled/powered/staffed means Live; any unavailable half means Degraded;
a destroyed half means Dead. R1 exposes no destruction or replacement operation.

The production content pipeline installs `tacsim.relay.luna_ceres@1`: Malapert
Exchange, two nodes, one pair, two halves. The Luna endpoint occupies Aitken's service
spine; the Ceres endpoint occupies its maintenance room. These are real authored
places and persistent objects. The dedicated Malapert compound and remaining star
nodes remain R10 content. Coverage is 12 Aitken rooms and 13 Ceres rooms.
The seeded capacity of 8 units/IC-second is metadata; byte accounting and scheduling
arrive with routing.

`@relay/list|check|show|enable|disable|power|staff` is restricted to SIM_ADMIN or
Royalty. Mutations are durable-ack gated and repeated settings do not advance the
revision. Reboot preserves identities and availability; planning the unchanged
pack reports no work and leaves the catalog untouched. Movement,
ordinary destruction and removal of a referenced binding are refused. Power and
staffing are explicit operator settings, not a new crew simulation.

Bounds are enforced on load and installation: 32 nodes, 64 pairs, 64 covered rooms
per node and 128 KiB per document. Diagnostics walk this bounded catalog, not the
world. A boot-only scan detects surviving or extra hardware without matching records.
Missing operators/objects, wrong locations or binding schemas, duplicate identities,
invalid bounds and malformed documents stop startup with a repair diagnostic.
The stored evidence is kept; boot never silently replaces it.

The catalog remains on the bounded legacy document lane. Its command, namespace
delegations and org-allocator access are recorded in the repository contract files.
R2 must reassess this lane before any authoritative mechanics depend on the catalog.
No mail/channel/order/report producer or consumer was rerouted in R1. The all-mail-is-IC
ruling remains binding; its transport implementation belongs to R6a.

### Local verification

**262 passed, 0 failed, 0 ignored** on the corrected candidate:

| Target | Passed |
| --- | ---: |
| mush-server / relay_registry | 5 |
| mush-contract-tests / legacy_handler_allowlist | 4 |
| mush-contract-tests / module_ns_owners | 4 |
| mush-contract-tests / state_slot_owners | 5 |
| mush-sim / transport | 24 |
| mush-mail / mail | 27 |
| mush-comsys / comsys | 48 |
| mush-propagation / e7c7 | 88 |
| tacsim / content | 36 |
| tacsim / ports | 21 |

Reproduction:

```sh
cargo test -p mush-server --test relay_registry \
  -p mush-contract-tests --test legacy_handler_allowlist --test module_ns_owners --test state_slot_owners \
  -p mush-sim --test transport -p mush-mail --test mail -p mush-comsys --test comsys \
  -p tacsim --test content --test ports -p mush-propagation --test e7c7 -j 4
cargo clippy -p mush-sim -p tacsim --lib -j 4 -- -D warnings
```

Clippy passed. Changed Rust files passed rustfmt; the workspace-wide cargo-fmt wrapper
exceeded Windows' command-length limit, so rustfmt was run directly on the affected
source files. `git diff --check` and `node --check scripts/relay-r1-live.mjs` passed.
Full workspace CI was not run in this session; the table is the tested scope.
After tightening the unchanged-pack assertion, the five `relay_registry` tests
were rerun and passed. Only the test and live-runner assertions changed after the
deployed candidate; shipping code remained at `916e8fee`.

Tests found and fixed two defects before deployment:
- Content boot initially lacked the declared content-to-sim persistence delegation.
  The existing-world restart test caught objects surviving without their catalog.
  Boot now asserts a durable catalog before any staff command can mask a missing write.
- Organization ID 0 is valid. Older-port composition tests caught a validator that
  rejected it when the Exchange was the world's first organization.

### Real-server acceptance

Target: the owner's disposable `conclav.io:/opt/rustmush/dev1`, split gateway/server,
Redis 8 on 6788, game SSH on 6789 and HTTPS/WebSocket on 6790.
The previous source, binaries, configuration, Redis snapshot and data directory are
retained under `/opt/rustmush/dev1/r1-evidence`. Dev2 was not changed; its gateway PID
was 539467 before and after the gate.

The build uses the repository's release profile and deployment script, a freshly
built embedded web client and matching source stamp. The VPS's system Node 18 was
too old for the checked-in web toolchain; Node 22 was installed under
`/opt/rustmush/dev1/r1-tools` without changing the system Node installation.

The fresh instance uses an IC ratio of 24 and configured epoch `0987-04-20 06:00`.
R1 has no route due times, throughput scheduler or delivery ledger to measure.
Seed: `tacsim.relay.luna_ceres@1`, content digest
`59f32f5d013fc15188f5e50763084e371ce489d26302c446acdd26790a65a6d9`.

| Live check | Result and retained evidence |
| --- | --- |
| Upgrade the existing store | Passed: all 351 pre-existing object identities remain; two nodes, two halves and their real room/object bindings validate. Full service restart leaves the catalog identical. [existing-store.json](evidence/relay-r1-2026-10-23/existing-store.json) |
| Fresh bootstrap | Passed: the repository reset procedure cleared only dev1's store/data, preserved SSH host identity, created a staff account and booted the configured packs. VPS log: `r1-evidence/fresh-reset.log`. Subsequent checks use this fresh world. |
| TLS WebSocket commands | Passed: 33 commands cover both halves, no-op content plan, enable/disable, power, staffing, idempotency, staff authorization, hidden-name refusal parity and binding lifecycle guards. [exercise.json](evidence/relay-r1-2026-10-23/exercise.json) |
| Existing communication | Passed between two actual connected characters while the pair is disabled: room speech, page, channel, mail notice and readable mail body. This proves R1 does not reroute traffic; it does not claim the future mail-transit cut is implemented. Same exercise transcript. |
| Confirmed-write crash boundary | Passed: SIGKILL the server child after a confirmed disable at revision 16; the gateway starts a new child. Catalog identity, coverage and disabled state are exactly preserved, then five WebSocket commands verify and restore Live at revision 17. [crash-restart.json](evidence/relay-r1-2026-10-23/crash-restart.json), [verify.json](evidence/relay-r1-2026-10-23/verify.json), [before-restart.json](evidence/relay-r1-2026-10-23/before-restart.json) |
| Malformed or missing registry | Passed: startup refuses the malformed document and a missing document with surviving hardware; neither is silently reseeded. [negative-cases.json](evidence/relay-r1-2026-10-23/negative-cases.json) |
| Duplicate identity or contradictory binding | Passed: duplicate half ID is refused; swapping the two physical half objects is refused as a location/binding contradiction. The exact injected document remains stored in each case. Same negative-case evidence. |
| Real SSH client | Passed through `ssh -p 6789 play@conclav.io`, in-band login and `@relay/check`; output agrees with WebSocket and persisted state. [ssh.json](evidence/relay-r1-2026-10-23/ssh.json) |
| Final health | Gateway and Redis active, Live pair at revision 17, dev2 PID unchanged. [environment.json](evidence/relay-r1-2026-10-23/environment.json), [final-registry.json](evidence/relay-r1-2026-10-23/final-registry.json) |

Final fresh-world bindings: operator #22; Ceres node #664 in room #112, 13 covered
rooms; Malapert node #665 in room #213, 12 covered rooms; Luna half #666 and Ceres
half #667. These numeric IDs are this instance's allocations; canonical identities
are the stable contract.

The release server SHA-256 is
`1993391ad4ca9af6f570d662c908e55b5779a21d0bb2bd07733859a2c0ec838b`;
gateway SHA-256 is
`7f2f19b830cc6050ee2193570898389572214c6cd80aaf4d884da10f0bbad566`.
Both are built from `916e8feec5d87e27a4be869c6fcdde7ebc98f1ee`.

### Live reproduction and retained artifacts

These commands are for the owner's **disposable dev1 instance only**. The upgrade
and fresh-reset procedures are destructive; their pre-R1 backups remain under
`/opt/rustmush/dev1/r1-evidence`. Credentials remain there in a root-only file and
are not committed. The JSON evidence in this repository contains no password or
authentication token.

The local production-composition test is the Cargo command above. On the VPS,
the final client run used the committed `scripts/relay-r1-live.mjs`, copied to
`/opt/rustmush/dev1/relay-r1-live.mjs`:

```sh
export PATH=/opt/rustmush/dev1/r1-tools/node_modules/node/bin:$PATH
export NODE_EXTRA_CA_CERTS=/opt/rustmush/dev1/tls/cert.pem
node /opt/rustmush/dev1/relay-r1-live.mjs exercise https://conclav.io:6790 \
  /opt/rustmush/dev1/r1-evidence/credentials.json /opt/rustmush/dev1/r1-evidence 6788
# Kill the mush-server child belonging to dev1mush-gateway; await its replacement.
node /opt/rustmush/dev1/relay-r1-live.mjs verify https://conclav.io:6790 \
  /opt/rustmush/dev1/r1-evidence/credentials.json /opt/rustmush/dev1/r1-evidence 6788
python3 /opt/rustmush/dev1/r1-negative.py
python3 /opt/rustmush/dev1/r1-ssh.py
```

The exact child-PID selection, executable-path assertion, SIGKILL and restart wait
are retained in `/opt/rustmush/dev1/r1-finish.sh`. It runs the sequence above and
ends with `R1_LIVE_GATE_PASSED` in `r1-finish.log`. Other retained procedures are
`r1-build.sh`, `r1-upgrade.sh`, `r1-existing.py`, `r1-fresh.py` and `r1-metadata.py`.
Build and upgrade logs are `r1-build.log` and `r1-evidence/upgrade.log`.

Before recording the final results, three test-harness assertions were corrected:
the unchanged-pack check now verifies a valid no-op **plan**; bad-record diagnostics
use a journal cursor per attempt; and SSH comparison normalizes terminal line
wrapping. The final evidence above comes from rerunning those corrected checks.

### Stop and outstanding work

R1's implementation and real-server checks are complete. Normal workspace CI is
**not run**, so the full §5 acceptance remains pending; targeted checks are not
presented as a substitute. The retained live evidence is ready for owner review.
R2 has not started. Before R2, complete that CI criterion and re-ground against
the then-current main; reassess the bounded legacy persistence lane before making
the catalog authoritative to routing. Continue only on the owner's instruction.
