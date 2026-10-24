# Performance and client debt delivery

Owner scope (2026-10-24): complete the performance/client group, integration-test
on disposable `conclav.io` dev1, commit and push each accepted gate, then pause.
Sight caching is investigation only. No production cache or field-diff sheet
protocol is authorized by this delivery.

| Gate | Scope | Status |
|---|---|---|
| 1 | B-85 split-process health | Accepted on dev1 |
| 2 | T-99 SSH echo; history correlation/dedup; keyframe coordination | In progress |
| 3 | T-102 restart-safe reply framing and completion ordering | Pending |
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
