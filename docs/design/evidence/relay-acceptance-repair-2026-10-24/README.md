# Relay acceptance repair evidence

Parent: R10 commit `57c5747dc02370e6854188e009dda61b9108df94`.

The repair corrects the centre test fixture's mismatched game-content flags, removes
the nonexistent `+relay/rates` command from current help and the startup hint, and
makes the R9b live harness verify actual recipient movement and forwarding.

## Source and validation

`source.json` and `source-sha256.json` identify the local and VPS candidate files;
the five hashes agree. `build-input-verification.json` additionally records 1,448
matching crate/Cargo/help inputs, accounting for two unchanged files whose Windows
working copies use CRLF while their Git blobs and Linux copies use LF.
`runtime-harness.patch` is the exact code/help/harness delta with zero context
from the parent, excluding the acceptance prose and evidence added afterward.
`runtime-sha256.txt` identifies the release executables used for the live rerun.

The final-candidate logs contain 1,480 passing TacSim library tests (one existing
ignored test), four passing help contracts, 21 passing boot-parity tests, ten passing
IC integration tests and 49 passing relay integration tests. No test failed in these
runs. The original library failures are retained in the retrospective evidence;
`r10-rest-failure.txt` records the later help-contract failure from the same CI run.
Full formatting, TacSim library/test Clippy with warnings denied, and the release
server/TLS gateway builds passed.

Build, lint and test commands used `--locked` and `-j4` on the Linux VPS; test/dev
debug information was disabled for tests and lint. Exact commands and final live
results are recorded in `metadata.json`. The single ignored library test was already ignored in
the original CI run; this repair does not change test selection or ignore attributes.

## Live procedure and retained attempts

The complete R9b campaign runs through the real dev1 HTTPS/WebSocket front door with
Node 22.23.4, using the existing staff/test accounts and verified lane-v12 baseline.
Start the gateway and verify HTTPS readiness before launching the harness. The
supported command is the existing `scripts/relay-r9b-live.mjs`, with the existing
four arguments and output under `r9b-evidence/acceptance-repair`. No provider, financial
receipt, work order, clock anchor, or transit state is patched to create success.

After real docking, the changed recipient is the nearby shore observer, moved from
Deimos to Ceres using
supported staff placement. The harness checks the stored location, resolved transport
destination, forwarding count and physical flight. It kills/restarts the real server
during that flight, checks frozen timing and no early inbox entry, and restarts after
delivery to verify one inbox entry. The pilot remains aboard and leaves normally
after docking.

`live-full-campaign-transfer-refusal.json` retains the completed funding, manufacture,
commissioning and voyage stages and the honest failure: staff unplacement also refuses
an underway carrier exit. `full-campaign-source.json` and its matching SHA file retain
that harness identity. Moving coverage and restart in passage passed before docking.
The continuation uses the same running world after docking, with no reset or
fabricated receipt. `recipient-transfer-continuation.mjs` contains the final harness's
exact helpers and transfer/lifecycle block plus a bootstrap that verifies the genuine
programme, prior evidence hash and same server binary. An intermediate continuation's
stale pilot-location assertion is retained in `live-continuation-assertion-failure.json`
and `continuation-assertion-failure.mjs`. A later attempt delivered correctly but its
inbox command used a stale pre-restart WebSocket; it is retained in
`live-continuation-socket-failure.json` and `continuation-socket-failure.mjs`. The final
block uses the reconnected observer client after every restart.
`live.json` is the final continuation result, not a claim that one uninterrupted full
campaign ran with the final script. `live.log` retains the chronological run output.

`live-startup-failure.json` and `live-startup-readiness-failure.json` are pre-command
connection failures. The first launch also selected the VPS's unsupported system
Node; the accepted run uses its existing supported runtime. An interrupted preflight
attempt is preserved separately: code inspection corrected the return-to-ship cleanup
before committing to the funding wait, and the whole campaign restarted from baseline.
These attempts are not counted as successful acceptance.

The complete pre-repair database snapshot/configuration is retained only in the
root-owned VPS evidence directory. A read-only capture retained a genuine funded
checkpoint after the sixth actual payment; its metadata is included here, while
the database snapshot stays private. Credentials, server
configuration, Redis dumps and authentication tokens are excluded from this repository.
The original R9b evidence is retained unchanged; the cut ledger explicitly withdraws
its false-positive recipient-transfer claim.

## Final result

The final continuation passed: recipient #679 moved from #261 to #101, the resolved
destination changed, and forwarding increased from 0 to 1. The 1,813-IC-second flight
survived restart with its due time unchanged. There were 339 early checks; the inbox
appeared after the due time and contained one matching letter before and after the
post-delivery restart. The acknowledged envelope was retired by the latter restart,
so `nearby-coverage-and-recipient-transfer.forwarded` is null; use the retained
`recipient-transfer-in-flight` milestone for its route and forwarding count.

The final lifecycle and conservation checks passed. Eleven crash boundaries lie on
the accepted campaign/continuation path; two additional crashes belong to the retained
interrupted continuations. `restore.json` and `health.json` confirm baseline restoration,
healthy dev1 HTTPS/gateway/Redis and unchanged dev2 PID. The funded checkpoint was
retained privately but never restored or used to fabricate this result.

Retained text logs normalize line endings and trailing whitespace only; test results,
timestamps and command/reply contents are unchanged. The patch uses zero context
(`git apply --unidiff-zero`) so blank context lines do not introduce whitespace noise.
