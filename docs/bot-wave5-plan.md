# Wave 5: adversarial playtests and defensible feedback

## Design

Use the external bot as a bounded player and the harness as an operator-owned
laboratory. Keep its runtime independent from game/NPC services. Build and test
the complete loop: explore a goal, report friction, challenge claims, reproduce a
failure, shrink the case, compare scenes blind, and export an actionable backlog.

1. **Bounded missions.** Versioned JSON declares testing permission, a pinned server
   or owned fixture scope, exact permitted commands, action limits and expected
   observations. All command origins pass the permission gate. Track attempted
   commands, settled receipts, places and coverage; reject loops outside the scope.
   Missing output, a contradicted player expectation and a documented-contract
   mismatch remain different candidate classifications. Preserve actor-local
   mission state and allowances across certified restore.
2. **Adversarial cast.** Supply JSON characters and runnable encounters for an
   unreliable guide, heckler, withholding teammate and conflicting patron. Each
   receives only its authored role and ordinary SSH observations. The first paired
   demonstration contrasts trusting and evidence-checking behavior after a guide
   urges a player to report arrival despite a blocked exit. Existing harness model,
   prompt, fault and repeat controls remain available for live experiments.
3. **Goal feedback and refutation.** `/feedback`, optional feedback after goal
   completion, and `/refute` use separately accounted structured question jobs.
   Feedback records ease/discoverability ratings and specific cited claims. A
   refutation docket contains explicit claims, never another actor's hidden memory.
   Refuters seek counterevidence or request a fresh test. Review answers never send
   commands or become in-character beliefs. Consolidation keeps refuted, contested,
   insufficiently tested and actionable claims distinct. Actionability requires a
   verified reproduction or an explicit operator disposition, not model agreement.
4. **Reproduction and reduction.** A versioned reproduction recipe binds an owned,
   fresh fixture campaign to an ordered oracle over actual observed episodes.
   Missing setup, infrastructure errors, denied actions and exhausted tapes are
   inconclusive, never reproduced failures. A bounded deterministic reducer removes
   optional steps/actors and response delays while retaining protected setup and
   required observations. Freeze inputs, reserve trial budgets, and rerun the final
   exported JSON regression. Record every attempt and whether minimality was proved
   under the supported reduction operations.
5. **Blind local review.** Export randomized scene packets with character identities
   and neutral context but without model/configuration/branch identities. Store the
   reveal key separately. A local TUI and headless rating validation record reviewer
   identity, packet/rubric revisions and ordinal ratings. Objective failure checks
   and human characterization/coherence/entertainment ratings remain separate.
6. **Inspection and continuity.** Mission coverage, feedback and refutation are
   visible in REPL/TUI, JSON reports and versioned checkpoint contributions. Retain
   old wave 2–4 save identities. Publish JSON demos, usage documentation, the review
   findings, and passing tests before committing and pushing `origin/main`.

## Adversarial plan review

Self-review before implementation; no independent reviewer is implied.

- A convincing complaint is not a bug report. Require exact local observations,
  preserve subjective ratings as opinions, distinguish assumptions from contracts,
  and require external reproduction or explicit operator triage for actionability.
- A refuter can hallucinate too. Validate its citations, retain disagreement, and
  never treat agreement between two models as proof. Cross-actor dockets are
  operator review inputs and must not enter character memory or hidden group state.
- A reducer can manufacture a smaller failure by deleting setup, exhausting a
  tape or timing out. Protect required setup, certify complete runs, bind an ordered
  observation oracle and rerun the final candidate. Budget trials and provider
  admission separately; initial automatic reduction accepts local tapes only.
- Fixed tapes reproduce recorded behavior, not causal susceptibility to a guide.
  Label that boundary. Live model repeats report counts, costs and uncertainty;
  one entertaining scene does not establish an improvement.
- Exact command permission must apply at the transport send boundary, including
  model actions, local pass-through, activities and scripts. Campaign fences remain
  coordinator-owned. Loading a mission cannot silently authorize a public server.
- Blind packets must not contain raw prompts, file paths, private goal/memory,
  response-model metadata or revealing branch labels. Keep the mapping outside the
  reviewer artifact, reject identity collisions, and sanitize terminal output.
- New default fields can break old save hashes. Version new contributions, validate
  journal provenance, and explicitly normalize only legacy defaults.
- Use bounded observations and artifacts. Preserve failed experiments for inspection.
  Cancelled review jobs remain cancelled and charged; restore must not replay them.

## Acceptance and validation

Run actual bot binaries over public SSH in the owned fixture. Verify limited
exploration finds a contradicted travel expectation; adversarial interaction
reproduces an agent-level failure; a careful branch avoids it; reduction removes
irrelevant cast/actions/delays and verifies the exported regression. Collect goal
feedback, refute a mistaken complaint, retain a supported usability issue with
fresh evidence, and export a consolidated backlog. Generate blind paired packets
and validate human ratings without inventing human judgments. Test malformed and
fabricated citations, missing observations, budget exhaustion, failed fixtures,
privacy boundaries, checkpoint compatibility and small TUI layouts. Finish with
the full bot suite, Clippy, formatting and runtime dependency audit.


## Final design review

Self-review of the implementation and observed tests; no independent reviewer is
implied. The usable foundations are implemented without adding game-engine runtime
dependencies or hidden inter-bot knowledge queries.

- **Goal reviews and identity.** Reviews bind the exact observations supplied,
  actor identity, request revision and model. Future, fabricated, sent-only and
  foreign-journal citations fail. Review output has no command/action shape and
  stays outside recall. Cancellation retains request accounting without restoring
  a pending review. The live display uses a compact cached view; full exports
  retain evidence.
- **Refutation and triage.** Dockets contain explicit claims and opaque keys,
  never source goals/private history. Every claim must be challenged exactly once.
  Large sets are split into bounded JSON batches. Consolidation detects altered
  dockets and missing originals, retains disagreement, deduplicates exact sources,
  and groups exact actionable issue descriptions. Named operator disposition is
  required after refutation; model agreement alone is insufficient. This is an
  explicit conservative implementation choice, not a deferred permission request.
- **Permissions and durability.** The common send path gates model/manual/routine/
  script commands. Reservation precedes SSH dispatch and cannot be refunded on
  reload. Startup now checks scope before persisting a new immutable contract, so
  a rejected endpoint does not poison the namespace. Uncertain dispatches remain
  uncertain; stopping the mission is an explicit operator action.
- **Reduction validity.** Protected original indexes and oracle observers survive
  reduction. Exact observed lines, with optional command association, must appear
  in order per observer. Infrastructure failure and tape exhaustion are
  inconclusive. Baseline/final repeat allowances are reserved before trials; bot
  input files are frozen. The exported relative-path bot inputs are loaded and
  rerun. Fixture executable paths remain explicit toolchain dependencies.
- **Reduction scope refinement.** The implementation uses greedy step deletion,
  optional actor removal and all-response-delay removal. It deliberately reports
  no global or fixed-point minimality across the combined operations. That avoids
  overstating a local reduction. Fresh ungrouped tape campaigns are supported;
  generalized group-state reduction and probabilistic live-model reduction remain
  outside this foundation. Existing harness matrices handle live repeats/costs.
- **Blinding.** Packets contain observed scene text and neutral authored context,
  not raw captures/private memories/source paths. Model/profile/prompt/campaign/
  branch labels are redacted; case variants and name collisions fail explicitly.
  Random order and its mapping live in separate artifacts. Rubric, packet and
  reviewer bindings survive headless validation. Actual human ratings must be
  supplied by a human; tests do not invent them.
- **Continuity.** v4 checkpoints require mission, review and automatic-feedback
  policy contributions; replay validates their journals. Legacy defaults are
  removed only for v1–v3 hashing, and legacy saves reject unbound wave-5 state.

The SSH acceptance cycle demonstrated two useful outcomes: a claim that no route
exists is refuted by observed eastward travel, while an incomplete fixture help
listing survives as a discoverability candidate. A test-authored operator
triage then produces one actionable issue. This is a deliberately constrained
fixture case, not a reported production RustMUSH engine defect. The adversarial
case reduced seven steps/three actors to five steps/two actors while preserving
the guide intervention, blocked exit and false arrival claim. A careful paired
behavior uses the side door. Local tapes establish repeatable behavior, not the
causal susceptibility or superiority of any live model.


## Validation results

- Full `cargo test -p mush-bot --all-targets`: **258 passed** (150 unit and 108
  integration/test helpers), including all nine new playtesting integration tests.
- After the final scope-validation ordering correction, the complete SSH
  mission/feedback/refutation/triage/restore test passed again.
- Final Clippy with `--all-targets -- -D warnings`, `cargo fmt --all --check`
  and `git diff --check` passed.
- Runtime/build dependency audit with `--edges normal,build --target all` contains
  only `mush-bot` from RustMUSH. No new runtime dependencies were added.
- Actual saved wave 2, wave 3 and wave 4 campaign summaries passed offline inspection
  with their original checkpoint hashes.
- Built and ran the standalone CLI and fixture example. A bounded demonstration
  protecting the five core encounter steps completed 12 fresh trials, removed the
  bystander/clock action and final unnecessary goal-summary turn, removed response
  delays, and reproduced both final exported-recipe reruns. Seven steps/three
  actors became five steps/two actors; 60 requests reserved, provider spend $0.
  The full-recipe integration test also tried removing failure-producing turns and
  correctly retained those turns after inconclusive attempts.

Validation artifacts are retained in the main checkout's `target/bot-json-fix/`:
`wave5-tests-final.log`, `wave5-final-scope-test.log`, `wave5-clippy-final.log`,
`wave5-runtime-dependencies.txt`, `wave5-wave{2,3,4}-compatibility.json`, and
`wave5-demo/report.json` with the exported regression and per-trial records.
The pre-existing debug-build warning about an unbuilt web client remains unrelated
and does not affect the SSH fixture. No paid-provider or human-rating results are
claimed.
