# 43b. Outside review 2 of 2: ChatGPT Astra 6 on the testing strategy (received 2026-11-03)

*The second answer to the owner's question (see [doc 43a](43a-gemini-4-argon-review.md) for the question and
the first). Verbatim. This reviewer read GitHub `main` at `9899e0eb`, a recent green run and its JUnit
reports, not only the tree. Every claim is weighed against the tree in
[doc 43](43-testing-strategy.md) §4; nothing here is authoritative.*

---

My assessment: **rustmush has substantial backend correctness coverage. The next improvement should be better allocation of testing effort—not simply more tests.** The largest opportunities are real-browser integration, generated lifecycle scenarios, fewer repeated expensive executions, and clearer ownership of what CI actually proves.

I reviewed GitHub `main` at [9899e0eb5](https://github.com/conclavio/rustmush/commit/9899e0eb5da14c1c5e7a4474a177ee330f51f9ab), plus recent CI results and downloaded test reports.

A [recent green main run](https://github.com/conclavio/rustmush/actions/runs/41683232696) recorded **12,549 passing Rust tests** across 18 JUnit reports. It finished in **19m45s**, consuming approximately **341 runner-minutes**. There are also 80 frontend test files. Those numbers show substantial investment, though they do not establish coverage completeness.

The foundations worth preserving are the shared memory/Redis behavior suites, real process-death and recovery tests, Store-ACK-aware fixtures, production-composition stories, independent bots exercising SSH, and nextest's resource isolation and reports.

I would add or strengthen coverage in these areas:

| Area | Assessment | Recommended change |
|---|---|---|
| **The assembled browser client** | Stores, controllers, mounted jsdom components, and actual WASM have tests, but I found no real-browser runner exercising the built SPA against a server. Those separate layers cannot establish that browser loading, sockets, focus, rendering, and assets work together. | Add a small browser suite: login/character selection, command submission, reconnect/resume, one typed action, and successful WASM/canvas rendering. Keep detailed permutations in the existing fast tests. [Current test configuration](https://github.com/conclavio/rustmush/blob/9899e0eb5/web/vite.config.ts#L34) |
| **Combinations of lifecycle operations and failures** | There are good randomized accounting checks and extensive fault matrices. However, examples such as the accounting generator use one fixed seed, while the fault matrix exercises individual failure points on fresh stores. | Add stateful generated sequences: submit → retry → lose acknowledgement → restart → cancel → advance time. Check conservation, sole custody, terminal-outcome uniqueness, and recovered/live equivalence after every operation. Save minimized failures as regressions. [Accounting test](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/mush-economy/tests/e2a.rs#L503), [fault matrix](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/mush-contract-tests/tests/gate5_fault_matrix.rs#L536) |
| **Actual release-runtime behavior** | The nightly release job builds the embedded SPA and release binaries, but stops at compilation. Existing provisioning checks exercise debug binaries. | After the existing release build, run a short startup, embedded-client fetch, TLS connection, and split-process handshake smoke. A second full release-mode suite would be excessive. [Release job](https://github.com/conclavio/rustmush/blob/9899e0eb5/.github/workflows/ci.yml#L801) |
| **Important configuration variants** | A test name can be covered while an alternate configuration remains unexercised. For example, the captain/escort story supports `FIELD_RAID=director`, but no workflow selects it; the closeout documentation records missing run evidence. | Give important variants separate test identities or explicit matrix entries. Track coverage of **test plus configuration**, including manual evidence. [Alternate story branch](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/mush-bot/tests/mining_cohort.rs#L912) |

The clearest overtesting is **repeated execution and excessive fixture cost**, rather than too many distinct behavioral assertions.

1. **Several CI jobs repeat proofs already included in the main sweeps.**
   The `rest-libs` sweep includes the contract tests, while separate kernel, randomness-policy, serialization-policy, and wire-fixture jobs execute overlapping tests again. The TypeScript export/drift commands appear twice. Those four named jobs consumed about **36 runner-minutes combined** in the sampled run; that is an optimization opportunity, not a guaranteed saving.

   Give each proof one execution owner and preserve visibility through reports. Keep separate runs when they genuinely change the environment—Redis, TLS, feature resolution, and the independent bot build are meaningful distinctions. [Workspace sweep](https://github.com/conclavio/rustmush/blob/9899e0eb5/.github/workflows/ci.yml#L674), [additional policy jobs](https://github.com/conclavio/rustmush/blob/9899e0eb5/.github/workflows/ci.yml#L1288)

2. **Some narrow regressions repeatedly pay for long real-time journeys.**
   Two casualty/write-off regressions each settle two fields using a 90-second disengagement setting. They took **211.6 and 211.0 seconds** in CI. Their assertions are valuable; repeatedly waiting for the same timer is the expensive part.

   Keep a representative real-task journey, and run the variant matrix through a controllable-clock driver using the same demotion and persistence implementation. Across the sampled suite, just **328 tests taking over 30 seconds accounted for 56% of accumulated test time**. Improving those fixtures offers more than repeatedly adding shards. [Regression example](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/tacsim/tests/shard_1/field_gc4.rs#L445), [real-time wait](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/tacsim/tests/support/field/field_host_rig.rs#L275)

3. **Some tests constrain source spelling more than behavior.**
   Frontend tests regex-match Svelte loop variables, markup, branching, and keyboard handlers. One still explains that this is necessary because no component harness exists, although mounted-component tests now exist. These tests can obstruct harmless refactoring while missing runtime failures.

   Replace behavioral source probes with mounted assertions. Architectural scans deserve separate treatment: retain useful guards, but prefer compiler-enforced capabilities or structured parsing over substring recognition as the codebase grows. [Concrete frontend probes](https://github.com/conclavio/rustmush/blob/9899e0eb5/web/src/lib/stores/unit.test.ts#L935)

There are several structural changes I would prioritize:

- **Make the test inventory describe execution, not mentions.** The gated-test inventory accepts `workflow.contains(test.name)`, including comments documenting manual-only tests. That proves acknowledgement, not automated execution. Introduce a small manifest identifying suite, configuration, backend, trigger, owner, and expected selection. Validate it against discovered test IDs and reject empty selections. Manual exemptions should remain explicit. [Current inventory check](https://github.com/conclavio/rustmush/blob/9899e0eb5/crates/mush-contract-tests/tests/ci_gated_test_inventory.rs#L597)

- **Separate domain organization from scheduling.** Today, rebalancing TacSim can require moving test modules, sibling files, and ownership metadata between numbered directories. Keep grouped binaries where they reduce linking costs, but move scheduling decisions toward generated assignments or runner filters. The recent extraction of shared field rigs is a good step. [Shard-maintenance requirements](https://github.com/conclavio/rustmush/blob/9899e0eb5/.github/tacsim-integration-shards.txt#L46)

- **Consolidate fixture infrastructure, while keeping assertions independent.** Share boot, acknowledged writes, restart, clock control, transport simulation, and observation helpers. Avoid a universal fixture that silently enables everything. For compatibility tests, prefer immutable historical fixtures over repeatedly reconstructing old content by stripping fields from today's manifests.

- **Make CI enforceable and avoid obsolete work.** GitHub currently reports `main` as unprotected and lists no rulesets. As more contributors or agents land changes, I would require an aggregate CI result. Also cancel superseded PR runs; the main CI workflow currently lacks the cancellation policy already used by CodeQL. Account for documentation-only changes when designing the required check.

- **Treat nightly health and manual evidence freshness as maintained outputs.** The seven preceding scheduled runs were red; recent failures included deterministic-baseline drift after legitimate changes. Current main includes a carefully investigated [baseline repair](https://github.com/conclavio/rustmush/commit/ade71092afee917c391ef7f7aab1c47e2af2631f), so this is a process finding, not a claim that the repair remains broken. Relevant behavior changes should obtain targeted baseline evidence before merge, and manual performance/evaluation lanes should record their last successful revision.

I would implement this in order: **deduplicate CI and cancel superseded PR runs; add a small browser smoke suite; remove repeated real-time waits from the slowest regression families; then introduce generated lifecycle tests and the execution manifest.** Preserve the expensive production-paced acceptance stories, but give them a deliberate periodic role rather than expanding them for every edge case.
