# Availability/privacy acceptance evidence

This is retrospective follow-up gate 2, built from acceptance-repair commit
`1a0565cd2b274f416ca4c0de2754ae294863b2fc` plus the changes committed with this
report. The contract and migration behavior are in
[design 15d](../../15d-comms-availability-privacy.md).

## Final result

The final gate passed **150 checked commands and four SIGKILL/restart boundaries**
on binary `f4e40f4af9a869c745dd669398c8f49e0a1a0e589357a2e330ec5a6a703000aa`.
All 1,492 source inputs matched. The saved dev1 baseline was restored, HTTPS returned
200, and dev2 remained at PID 539467. Automated validation passed 2,490 tests with
no failures and eight pre-existing ignored tests.

`metadata.json` is the machine-readable test/source identity record; `live.json`
records checked commands and durable postconditions, rather than treating command
success alone as acceptance.

## Procedure and boundaries

The target is the real conclav.io dev1 TLS/WebSocket server on port 6790 and Redis
6788. The original dev1 database 0 and server config were saved privately before
the gate. The fixture helper reads the genuine R9b six-payment funded checkpoint
with a private, portless Redis process and copies its values through Redis RESTORE.
The live append-only log remains enabled and consistent. It does not substitute
an RDB beneath a live AOF, alter dev2, fabricate payments/industry receipts, or edit
clock anchors. `fixture.json`, `prepare.json`, `retest.json` and `restore.json`
record the source digests and checks. Private RDBs, config, credentials and actor
login records remain root-only on the VPS and are deliberately absent here.

The final harness uses a real 800,000-minor transfer from the existing finite
clearing balance, verifies the exact recipient balance and conserved total, buys
actual propellant/provisions, boards the existing commissioned hull and sails it
outside Luna coverage. At that location it checks:

- An undiscovered private pair and an unknown ID produce identical direct replies;
  the public board omits the private pair. Manage authority alone cannot publish it.
- A service-class grant does not grant use. Both endpoints need positive private
  authority; revocation before the hop forces conventional routing.
- Organization speaking is explicitly authorized and frozen on the source. Changing
  selection leaves accepted traffic intact; revoking live authority reroutes it.
  Organization 0 and scope 0 remain valid through source handoff and restart.
- Explicit public carriage delivers actual inbox content. Private receipt stamps
  require discovery independently of use, including after revocation and restart.
- One ordinary sender fills its 32 unresolved copies and is refused before and
  after restart. Another actor's order, acknowledgement and IC channel line actually
  arrive while those 32 copies remain unresolved.

Four server-child SIGKILL boundaries cover agency, zero-based identity, arrived
receipt privacy and saturated unresolved mail. Each crash verifies the dev1 child
and reconnects real clients. The saved baseline is restored afterward; `health.json`
checks the running binary digest, HTTPS 200, original two-node/one-pair registry,
absence of private installations and unchanged dev2 gateway PID 539467.

## Automated validation

The logs are from Linux on the VPS build tree with the production dependency roster:

| Log | Scope | Passed / ignored |
|---|---|---|
| `relay-registry.log` | Production relay integration, eight new availability/privacy regressions | 57 / 0 |
| `producer-tests-final.log` | mail, comsys and propagation | 205 / 0 |
| `server-integration-final.log` | boot parity and IC integration | 31 / 0 |
| `tacsim-lib-final.log` | TacSim library | 1,480 / 1 |
| `bot-tests.log` | Full isolated bot package | 717 / 7 |

That is 2,490 passing tests and eight pre-existing ignored tests. The bot suite ran after its fixture repair and before the final zero-ID
correction; the other four suites reran after that correction. The focused
`bot-arena-contract.log` adds 19 passing checks already included in the full bot
suite and is not counted twice. `clippy.log` is strict all-targets Clippy for server,
propagation, mail, comsys and TacSim; `fmt-final.log` is the full Linux formatting
check; `release.log` records the optimized server build. Syntax checks cover the
modified Node harnesses and Python fixture.

`source-sha256.json` covers 1,492 crate/Cargo/help/harness inputs. Hashing normalizes
CRLF to LF to compare the Windows checkout with Linux. `build-input-verification.json`
records all inputs matching. `health.json` compares the running process executable
with the final on-disk release binary. Report Markdown itself is outside that build
manifest.

## CI follow-up and retained attempts

The acceptance-repair push's normal CI run
[40125259513](https://github.com/conclavio/rustmush/actions/runs/40125259513)
finished with every required job passing except `test (rest)`. Its isolated bot
arena fixture had the same mismatched content-enabled/runtime-enabled composition
as the earlier TacSim fixture failure. `ci-bot-failure.txt` retains the relevant
excerpt. The fix uses the same `cfg.enabled` value for both halves of the fixture;
it does not weaken the production Fleet/Postmaster receipt guard. The focused and
full isolated bot suites above pass. CodeQL for that push passed. The new push's
normal CI is a separate result, not claimed green by this report.

The first complete gate passed 142 command checks and three crashes on binary
`e54fab5075d7fd476943c5557e969ba8cfdf04dba9d85503690ecad8a600c9df`.
`nonzero-first-pass-*` and `nonzero-first-pass.mjs` preserve that result. Final review
then found that organization/scope IDs are zero-based; the final replay adds that
case and its fourth crash. The final `live.json` and binary identity supersede the
first pass for acceptance.

Earlier harness/setup failures are retained as `live-*-failure.mjs/.log` and
`live-failed-*.json`: an incorrect loose-inventory precondition after installed
materials were already consumed, insufficient voyage funds, and a proposed clearing
transfer exceeding its finite balance. One reset overlapped Redis background save;
the helper now retries SAVE, and the duplicate failed transcript is retained.
The corrected gate requires a successful actual payment and checked balances.
These attempts are not counted as successful runs. Before the final replay, the
first health probe preceded TLS readiness; the live script did not start on that
attempt. Readiness was verified before launching it, and the final restoration
waited for the listener before collecting health.

## Remaining scope

This gate closes RR-1 admission isolation and RR-4 private discovery/use. Its limits
are initial constants, not latency guarantees under arbitrary multi-actor load.
Private pairs from older records default closed; operators must grant access or
explicitly open carriage. Exchange charter links remain public. Organization
selection is outgoing agency, not an organization inbox or receiver delegation API.
Policy/bandwidth (RR-5 through RR-7), additional consumers, generic mobile
installation extraction and live Discord acceptance remain follow-up work.
