# Performance and client debt delivery

Owner scope (2026-10-24): complete the performance/client group, integration-test
on disposable `conclav.io` dev1, commit and push each accepted gate, then pause.
Sight caching is investigation only. No production cache or field-diff sheet
protocol is authorized by this delivery.

| Gate | Scope | Status |
|---|---|---|
| 1 | B-85 split-process health | Accepted on dev1 |
| 2 | T-99 SSH echo; history correlation/dedup; keyframe coordination | Accepted on dev1 |
| 3 | T-102 restart-safe reply framing and completion ordering | In progress |
| 4 | T-106 parallel pilot frames; T-107 sight-cache investigation | Pending |

## Gate 1: authoritative health

The server publishes its world iteration timestamp, loop lag and persistence
health over IPC once per second. The gateway retains the source timestamp:
buffered samples cannot make a stalled world look fresh. Startup, a sample older
than five seconds, a future timestamp, unavailable persistence, broken integrity,
or abandoned supervision yields `ok: false` and HTTP 503. Healthy responses use
HTTP 200. The response adds `world_ok` and `world_max_age_s`; the existing fields
remain available. A disconnected boundary clears the stamp, and a replacement
server must establish its own health.

IPC version 8 requires deploying gateway and server together. This health feed
is ephemeral; it creates no Redis records and does not count mirrored observations
as gateway world-loop histogram measurements.

Validation: 317 configuration, IPC, gateway and web unit/integration tests passed,
including real split-process restart and persistence tests. Both TLS release
executables built successfully. The live dev1 gate checked a nonzero authoritative
stamp, SIGSTOP causing HTTP 503 with a live gateway, SIGCONT recovery, SIGKILL
causing an unhealthy interval, and replacement-server recovery under the same
gateway PID. Dev2's gateway PID did not change. The first harness invocation raced
listener startup; the harness now tolerates connection refusal while polling,
and the rerun passed all seven assertions.

Evidence: [live samples](evidence/performance-client-2026-10-24/health-live.json),
[test log](evidence/performance-client-2026-10-24/health-tests.log),
[release build](evidence/performance-client-2026-10-24/health-release.log).
Reproduce with `scripts/client-health-live.py` on dev1 after deploying both peers.
The script changes no configuration or database contents and resumes a stopped
child even on failure.

Accepted release SHA-256:

- server: `61a7cc0ff3622d4c1ba8490e36bcf5a74bb259962539d0c80f20533dbcf74306`
- gateway: `15497cc1aafb327481e1bc67bffe3153bf2fc56de7b69107d0ab80eebd05caf9`

## Gate 2: SSH and client synchronization

PTY ECHO=0 now suppresses server-side draft echo, including asynchronous redraws;
prompt masking overrides it temporarily, and normal prompts restore the negotiated
mode. Production SSH receives the gateway's envelope sequence without taking on
web receipt-ACK retention. History requests have distinct connection-local ids,
mode generations reject obsolete queued requests, and the TUI deduplicates live
narratives against its priming/history pages. A wholly duplicate priming page still
advances the cursor. The clear/backlog invariant is explicitly asserted.

Sheet queries invalidate the next pushed baseline through the shared
`SheetView::query_keyframe` helper, used by both generic and toy query handlers.
The next push is a keyframe; subsequent unchanged-schema pushes remain full-data
deltas. Both clients replace the full data map, so omitted staff fields disappear.
No field-diff protocol was introduced. Dev1's game composition has no legacy
`VitalsSource` sheet provider: the server query/mutation integration is exercised
by the toy ruleset tests on the VPS, not claimed as a playable live sheet flow.

Live acceptance also exposed a transport shutdown defect: disconnecting the whole
SSH connection immediately could discard queued farewell bytes. The pump now sends
exit status, EOF and channel close in order, and disconnect cleanup runs once.
Real OpenSSH now receives Goodbye and exits 0.

Validation on dev1:

- The broad client/server run exercised 899 tests: 896 passed initially and three
  failed under concurrent build load. Focused reruns passed. The persistence
  probes now retry only pre-admission Busy within a deadline, and the observer
  board test waits for the arena's acknowledged exit. Both complete repaired
  suites passed (64 tests). The split arena story also accepts an already
  classified first observation while still requiring the classified contact.
- SSH library and real loopback regressions: 345 passed, including ECHO and farewell.
- Web sheet/game stores: 102 passed. Strict affected-crate Clippy passed; the
  final SSH shutdown change also passed its own all-target Clippy.
- The real front door passed ECHO=0/1, one-copy history, TUI entry/exit followed by
  history, and graceful QUIT on both sessions. The TLS release built successfully.
- All 20 changed Rust/web source files in this commit were matched by normalized
  SHA-256 against the tested VPS tree. Earlier gate source remains the deployed base.

Evidence: [live SSH](evidence/performance-client-2026-10-24/ssh-live.json),
[broad run](evidence/performance-client-2026-10-24/client-final-tests.log),
[repaired scenarios](evidence/performance-client-2026-10-24/client-scenario-repaired.log),
[SSH regressions](evidence/performance-client-2026-10-24/ssh-close-tests.log),
[web tests](evidence/performance-client-2026-10-24/client-web-tests.log),
[Clippy](evidence/performance-client-2026-10-24/client-clippy.log),
[SSH Clippy](evidence/performance-client-2026-10-24/ssh-close-clippy.log),
[release](evidence/performance-client-2026-10-24/client-final-release.log),
[source manifest](evidence/performance-client-2026-10-24/client-source-manifest.json).
Reproduce front-door acceptance with `scripts/client-ssh-live.py` on dev1.

Accepted release SHA-256:

- server: `24c27a40dc0da32de6aab533224b7b7f45c05963f06b175f4dd07adc7814c029`
- gateway: `677b8c9a211d72fd3af9d3af6d5fca8d9721840ee73fd98a2299a1554f99f1d0`
