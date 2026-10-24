# Policy/bandwidth acceptance evidence

This closes retrospective follow-up gate 3, RR-5 through RR-7, from base commit
`749e0f9808171314941f43fabec7792718b822e9` plus the changes committed with this report.
[Design 15e](../../15e-comms-policy-bandwidth.md) defines the contract and compatibility rules.

## Result

The gate passed: **2,086 passing automated tests, 3 pre-existing ignored tests, 93 checked live commands and 6 SIGKILL/restart boundaries**. No automated or final live failures remain.
The running/tested server SHA-256 is `6bdf7529856b6d00d5e27027da9373f62539b7b7c4e16f2a7aa6919dcdf9f8a4`.
All 1,495 build, help and harness inputs match the checkout after CRLF-to-LF normalization.
The saved dev1 baseline/config was restored: HTTPS 200, 2 nodes,
1 pair, no private installations. Dev2 remained at gateway PID 539467.
Normal GitHub CI for the publication is a separate result; this report does not claim it green.

## Real-server procedure

`scripts/relay-policy-live.mjs` uses the actual dev1 TLS/WebSocket server on port
6790 and read-only durable assertions on Redis 6788. The fixture first saved dev1's
existing database/config privately. It loaded the genuine six-payment R9b funded
checkpoint through a portless Redis reader and normal RESTORE commands, keeping
the live AOF enabled and consistent. Snapshots, credentials and config stay root-only
on the VPS; none are included here. The public star is upgraded by ordinary pack
plan/apply commands while the commissioned private hardware and payments are preserved.

The final live run verifies:

- Removing the real commissioned mobile pair, restarting while it is unmounted,
  and refitting it preserves its schedule and physical identities.
- Unauthorized/stale/invalid schedule edits leave the policy unchanged. Undiscovered
  private and unknown link IDs return the same result.
- Ceres-to-Mercury traffic retains its first accepted 60-second admission even after
  the declared policy becomes 900 seconds. Its second spoke advertises 1,800 seconds
  but adds no second operator admission. It actually arrives after a between-spokes crash.
- A paid multi-unit letter on a one-unit refill/burst schedule captures one real
  payment on its first unit. Incomplete content has neither an inbox entry nor a
  completed receipt. Increasing burst creates no free tokens; reducing maximum size
  leaves the accepted ticket intact. Partial progress and the consumed hold survive
  SIGKILL, the whole message arrives once, and a second restart does not charge again.
- New over-limit traffic excludes the newly restricted pair. The planner may use
  another route; physical-path selection is intentionally separate from ETA optimization.
- A real finite voyage payment, actual market fuel/provisions and ordinary sailing
  move the commissioned Fleet carrier into distinct coverage. Its genuine physical
  one-unit/second private pair carries a multi-unit message across a partial-progress
  crash, with no early visibility and one completed receipt/inbox delivery.

The paid example is envelope 404: 765 accounted bytes, six units, one 3,000-minor
charge, and completion six IC seconds after its first unit. The real private-link
example is envelope 408: seven units, frozen at 1/7 across SIGKILL, with completion
nine IC seconds after the first unit and one arrived receipt. Both checks assert
that the recipient has no inbox entry while serialization is incomplete.

The executed gate runner restored the saved baseline after each outcome. `fixture.json`,
`prepare.json`, `restore.json` and `health.json` record snapshot identities, restore
method, final binary/HTTPS checks and dev2's unchanged PID. `live.json` contains the
checked commands and postconditions; `metadata.json` summarizes their counts.

## Automated validation

| Log | Passed | Ignored |
|---|---:|---:|
| `relay-registry.log` | 63 | 0 |
| `producer-tests.log` | 206 | 0 |
| `server-integration.log` | 31 | 0 |
| `tacsim-lib.log` | 1480 | 1 |
| `help-commands.log` | 4 | 0 |
| `server-lib.log` | 302 | 2 |

The relay suite adds six production regressions for schedules, two-spoke admission,
paid serialization, Deferred windows, private narrow-link revocation and corrupt
state refusing boot. The producer suite includes token refill/burst/legacy/rollback
coverage. Boot parity and IC scenarios exercise the full composition; TacSim library
and help-contract suites cover the broader game and documented command registration.
The preceding availability/privacy CI run also exposed a stale capability-vocabulary
count in the server library (28 installed versus 25 expected). This gate names and
asserts the three new discover/use/speak capabilities explicitly, preserves the
unknown-capability refusals, and passes the full server library suite. The focused
CI failure is retained in `prior-ci-failure.txt`; no production permission check was weakened.

Strict all-targets Clippy for propagation/server/TacSim, full formatting, release
server build, Node syntax and Python syntax checks passed.

## Retained attempts and limits

`relay-registry-initial.log` preserves two old/new harness assumptions: a large
eligible message can now consume residual tokens instead of being skipped, and a
message excluded from one pair may still route through another pair. The revised
FIFO assertion initially selected the sender's local channel copy; that failed log
is `relay-registry-fixture-selection.log`. Selecting the remote recipient fixes the
fixture. `server-invalid-target.log` records an incorrect local test target name;
the final run uses the existing `ic_slice_loop` target. These failures are not counted
as passing tests. The final lifecycle review added dormant-policy retention through
actual mobile unmount/restart/refit and reran all suites after that correction.
The implementation review also closed the purchase boundary for
courtesy messages whose first serialized units had already been consumed.

The first live attempt passed upgrade and unmount/refit, then found that the old
checkpoint's observer already had legitimate operator authority. Its edit was
correctly allowed. The corrected harness registers a fresh ordinary character for
negative permissions and actual paid traffic. `live-authority-fixture-failure.*`
and `live-failed-*.json` retain that attempt; runtime code did not change for this repair.

Deferred-window closure/reopening, mid-progress revocation, clock rollback, repeated
same-time passes, legacy records and malformed records are deterministic production
regressions. The live run exercises selected real process/timing boundaries rather
than claiming every matrix row was independently replayed on the VPS.

Capacity remains per pair; aggregate hub contention and additional consumer APIs
are separate follow-ups. Policy revisions affect future tickets, while current
rate/burst govern all physical work. A registry change may conservatively discard
partial serialization during replan; already spent units and fees are not refunded.
Completed operator admission remains recorded. These are explicit contract choices,
not deferred acceptance defects.
