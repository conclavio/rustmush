# Cut L validation — 2026-09-22

Draft: final Linux gates and focused reruns are still running. Integration is
tracked in [PR #1](https://github.com/faenix7/rustmush/pull/1).

The frozen Linux run uses production source `5f8353b3`, including upstream
AA.1 and AA.2, and a production SPA stamped with that exact commit. `fbe91f75`
differs only in six test files. Upstream AA.3 then advanced main to `61597e06`;
the clean merge `ffa12f6b` adds its two production files and one test file.
The final continuation verifies all nine old/new Git blobs and uses an SPA
stamped `ffa12f6b`; the frozen run remains unchanged. The earlier Windows results
and their precise revisions
remain in [16b's Cut L record](../../design/16b-combat-implementation-plan.md).
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
| Split-process recovery | Two passed, one teardown failure | Pending at `ffa12f6b` |
| Workspace | 5,747 passed, seven failed, 82 ignored | Five corrected targets pending on Linux |
| Separate bot suite | Interrupted by the validation pause; three E2E timeouts | Clean full rerun pending |
| Shell/reset/bundle | ShellCheck, four syntax gates and both behavior fixtures passed | Unchanged |
| TLS listener | Passed | Unchanged |
| Lints and documentation | Workspace and TLS all-target Clippy and strict rustdoc passed | Affected-package all-target Clippy and strict rustdoc pending |
| Release | Three TLS binaries passed; both report-only harness cases pending | AA.3 binaries and harness pending |
| Mock cost and TypeScript | Pending | Unchanged |

Final 34-gate aggregate, the expanded AA.3 continuation, exact remaining counts,
harness measurements and cleanup verification are pending.

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
recorded as such. Linux confirmation is pending.

## Validation-host resource incident

Private debug artifacts filled the host filesystem. The owned runner and bot
processes were paused. Removing 334 completed ELF test executables from the
private target reclaimed 129.9 GiB of logical size and restored 127.5 GiB free;
source, logs, libraries and live application paths were preserved. Subsequent
builds retain a 40 GiB free-space reserve.

The five relevant services were active, with no restart-count changes and no
matching disk-write errors in the checked 20-minute journal window. This is a
bounded observation, not proof of absence of every historical effect. The pause
caught the bot's 49-test E2E target in flight; three timed out, so a clean full bot
rerun is required. Final private Redis shutdown and build-cache cleanup are
pending. No live instance was deployed, reset or removed.

GitHub Actions are separately blocked by billing. At `5f8353b3`, none of the 14
jobs in [run 35802824852](https://github.com/faenix7/rustmush/actions/runs/35802824852)
executed steps. Manual results do not imply a green GitHub CI run.
