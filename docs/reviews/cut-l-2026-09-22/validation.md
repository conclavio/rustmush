# Cut L validation — 2026-09-22

All twelve final Linux gates passed. Integration is tracked in
[PR #1](https://github.com/faenix7/rustmush/pull/1).
The [portable evidence summary](validation-summary.json) records commands, gate
outcomes, counts, source/binary hashes, tool versions and cleanup verification.

The frozen Linux run uses production source `5f8353b3`, including upstream
AA.1 and AA.2, and a production SPA stamped with that exact commit. `fbe91f75`
differs only in six test files. Upstream AA.3 then advanced main to `61597e06`;
the clean merge `ffa12f6b` adds its two production files and one test file.
The final continuation verifies all nine old/new Git blobs and uses an SPA
stamped `ffa12f6b`; the frozen run remains unchanged. The earlier Windows results
and their precise revisions remain in
[16b's Cut L record](../../design/16b-combat-implementation-plan.md).
AA's documentation closeout at `84154d1a` was merged in `0ccf2347`; the diff from
`ffa12f6b` contains only four documentation files, with no production/test changes.
The local full TacSim suite at `ffa12f6b` passed 884 tests, with zero failures and
two ignored across 25 targets. Svelte checks reported zero errors/warnings; the
production SPA build and commit guard passed.

## Linux results

The frozen workspace reported **5,747 passed, seven failed and 82 ignored**.
There were 355 Cargo targets: 312 binaries and 43 doctest crates. The 356 libtest
summary rows include separate ordinary and compile-fail partitions for
`mush_core`; no nested worker result was counted twice. Environment-gated tests
that return early are included as libtest reports them; the dedicated Redis and
mock-provider gates supply evidence for those paths.

| Gate group | Frozen source result | Final continuation |
|---|---|---|
| Redis store | 66 passed | Production unchanged |
| Staff bootstrap | Config and CLI targets passed; real first-account, lock and second-account refusal passed | Production unchanged |
| Provisioning | Smoke boot, three debug binaries, actual render and refusal controls passed | Production unchanged |
| Redis recovery | Authoritative boot, quarantine repair, generations, kill9 durability, persistence and m10c twin passed | Production unchanged |
| Split-process recovery | Two passed, one teardown failure | All three passed at `ffa12f6b`, including the live-Redis process-cut matrix |
| Workspace | 5,747 passed, seven failed, 82 ignored | All five corrected targets passed: campaign census 58, simulation census 18, IPC action eight, teardown one, load/soak four |
| Separate bot suite | Interrupted by the validation pause; three E2E timeouts | Clean full rerun: 262 passed, zero failed/ignored |
| Shell/reset/bundle | ShellCheck, four syntax gates and both behavior fixtures passed | Unchanged |
| TLS listener | Passed | Unchanged |
| Lints and documentation | Workspace and TLS all-target Clippy and strict rustdoc passed | All-target Clippy for the six affected packages and full-workspace strict rustdoc passed |
| TacSim integration | Included in the frozen workspace | Full suite: 884 passed, zero failed, two ignored across 25 targets |
| Release | Three TLS binaries and both report-only harness cases passed | All three AA.3 TLS binaries and both final report-only harness cases passed |
| Mock cost and TypeScript | Mock provider: one test, two calls, 936 tokens; TypeScript export and drift checks passed | Unchanged |

The frozen aggregate is **31 passed of 34 gates**, with the three failures shown
above: split-process recovery, workspace fixtures and the interrupted bot gate.
The expanded AA.3 continuation passed **all twelve gates**. Evidence and cleanup
verification completed. Mock accounting recorded $0.000160 of simulated cost;
no paid call ran.

The frozen Linux arena harness measured p50/p95/p99/max
**1.633/11.914/12.027/12.081 ms**. Its p99 is just above the 12 ms target; timing
assertions were disabled, so this is a completed report-only run, not a pass of
that threshold. Static-ground p99/max were 7.843/8.374 ms. Combat counters were
783 fired, 710 hits, 33 sections and 11 units destroyed, 325 blocked lines,
1,006 path queries visiting 87,008 cells with no failures, 1,435,041 ledger bytes
and zero lagged pulses. An independent comparison matched all 105 arena and 42
static deterministic fields against the earlier Windows reports; timings from
different hosts/load are not a controlled performance comparison.

The final AA.3 arena run measured p50/p95/p99/max
**2.070/14.854/14.998/15.167 ms**; static-ground p99/max were **6.430/7.148 ms**.
Its arena p99 also exceeds 12 ms. Both report-only harness cases completed, with
timing assertions disabled; neither Linux run establishes a pass of that target.
All 105 arena and 42 static deterministic fields match the frozen Linux run and
the earlier Windows reports exactly.

## Failures preserved and corrected

The initial run at `e7761f14` lacked JSON/search modules in its private Redis:
10 store tests passed and 56 failed on missing commands. Loading the installed
modules let the identical test binary reach 65 passed and one genuine failure.
`JSON.MGET` returned null for a wrong-type module key, and the batched reader
silently omitted it. Commit `5f8353b3` classifies null entries individually,
skipping only confirmed streams or vanished keys. The existing corruption
assertion was retained; all 66 Redis tests now pass. Local store unit and memory
checks passed 96 tests. ShellCheck's staff-argument finding was separately fixed
by the explicit conditional in `50347e03`.

The split-process failure came after successful recovery, at deliberate fixture
shutdown: SSH closed before its final banner arrived, and the fixture's detached
thread unwrapped the expected supervision error. Commit `6463f71d` instead
observes gateway completion after the runtime drops and requires the exact
supervision-abandoned result. Crash points, ownership, replay, transaction status
and the single-mutation assertion remain intact.

The seven workspace failures occupied five targets:

| Target | Cause and correction |
|---|---|
| `mush-campaign/e7c10` (one failure), `mush-sim/e7c10` (two) | An absolute-path `/src/` substring matched the checkout's own directory, admitting integration tests to production censuses. `55b5561f` classifies `<crate>/src` relative to `crates/`; authority allowlists and expected counts are unchanged. |
| `mush-server/ipc_authoritative_action` (two) | Actor-spoofing and malicious-release assertions encountered pre-ownership `Busy`. `fbe91f75` retries byte-identical requests within existing deadlines; the release test resends both frames. Rejection, duplicate and reader-liveness assertions remain. |
| `mush-server/ipc_teardown` (one) | A finite flood and fixed delay did not establish a blocked write on Linux. `fbe91f75` caps the receive buffer, paces output and observes the exact write-timeout event before requiring output and closure, within one 20-second deadline. Arbitrary errors cannot pass. |
| `mushd/load_soak` (one) | Linux TCP buffers could absorb the entire finite burst. `fbe91f75` bounds fixture send/receive buffers and observes session detachment before reading again. Socket closure and healthy-client isolation remain required; keepalive expiry cannot substitute for backpressure. |

Local focused checks passed 76 census, eight IPC action, one IPC teardown and four
load/soak tests. Their Clippy checks, formatting and diff checks passed. These are
fixture corrections, not seven production fixes; the failed frozen sweep remains
recorded as such. The same five complete targets now pass all 89 tests on Linux.
Including the separate three-test recovery target, all six corrected targets
passed 92 tests at `ffa12f6b`.

## Validation-host resource incident

Private debug artifacts drove the host filesystem to **100% reported use**, with
115 MiB free at the lowest observed point (133 MiB at discovery). No `ENOSPC`
error was observed. The owned runner and bot processes were paused. Removing
334 completed ELF test executables from the
private target reclaimed 129.9 GiB of logical size and restored 127.5 GiB free;
source, logs, libraries and live application paths were preserved. Subsequent
builds retain a 40 GiB free-space reserve.

The five relevant services were active, with no restart-count changes and no
matching disk-write errors in the checked 20-minute journal window. This is a
bounded observation, not proof of absence of every historical effect. The pause
caught the bot's 49-test E2E target in flight; three timed out. The clean full bot
rerun at `ffa12f6b` passed. After verifying the downloaded evidence archive, the
final cleanup removed only the private build target, reclaimed 102,506,549,248
bytes and restored 183.0 GiB free (`df` displayed `184G`). No private build process
remained; Redis port 63987 was closed. The final read-only service check found
the same five existing services active/running with zero restarts. Source trees,
archives, manifests, scripts, logs and reports were retained. No live instance
was deployed, reset or removed.

The immutable evidence archive contains 104 files, with all 103 manifest entries
verified; its SHA256 is
`c9403080ce34bcd8cb554c1a4716cf9ec65bbb8363ac29411d1fc90d2dc1e94f`.
The cleanup receipt was fetched separately afterward and is not covered by that
archive; its SHA256 is
`99b700ff5dea917ba79d34b1babe0277eb01fcc5bea7bc3dace42616fb920943`.
The portable summary reports no warnings or integrity mismatches. Validation used
Linux 6.8, Rust/Cargo 1.98.0 and Redis 8.10.1 with JSON/search modules. Normal builds
used two jobs; the final release used three after checking CPU/memory headroom,
with the configured Thin LTO unchanged.

GitHub Actions are separately blocked by billing. At `0ccf2347`, none of the 14
jobs in [run 35810933029](https://github.com/faenix7/rustmush/actions/runs/35810933029)
executed steps. Manual results do not imply a green GitHub CI run.
