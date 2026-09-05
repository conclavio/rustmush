# E ↔ K convergence review — Draft 0.1 — on the merged main (`4c80014`)

Grounded in the tree at `4c80014` (E1–E6c with verify folds; `origin/codex/k7-convergence` — K7 delayed
decisions, typed hook reads, K8 gates, K8.4 migrations, K8.5 compatibility rules, K8.6 CI — merged on
top). Cites are **path + symbol** and **doc §**; line numbers are deliberately absent (08i §8's rule).

## 1. Verdict

Both sides' claims very largely hold on the merged tree. Of 503 claims read across four passes, 438
verified as implemented or deliberately superseded on the first read; 65 escalated as partial,
contradicted or missing, and an adversarial refutation pass overturned **48 of those 65** — nearly all
were the behaviour implemented under another name, in another crate, or deliberately deferred by a
document the claim's own source supersedes. **17 gaps survive.** None is a money-loss or
authority-escape defect: the economy journal is still sole authority, every named failure mode fails
closed, and no legacy lane can mutate a K-owned typed family (`crates/mush-store/src/types.rs`
`validate_module_op` against `crates/mush-core/src/action_state.rs` `is_action_state_persistence_key`).
Four are the headline. **(a)** `10b` §3.10 retires cut 10's "no channel under a live lease" condition on
a kernel refusal that does not exist — `ControlLeaseState::checked_request_handoff`
(`crates/mush-core/src/control.rs`) refuses only on stale revision, an existing pending,
`NoControlChange` and boundary ordering, never asking who holds the lease — so as written cut 10 would
evict a live human operator and the gate's three-refusal test would fail on its middle leg.
**(b)** `crates/ruleset-wod/src/onboarding.rs` `on_approved` re-dispatches `"look"` through the compat
`mush_core::dispatch`, running the whole perception hook chain against an explicitly unavailable typed
reader — latent today, but the one real exception to 09 §C2's "production engine dispatch always uses
the live store". **(c)** K8.6 moved Redis out of the cross-platform job, but the Linux Redis-fault job
names only six proofs, leaving nine live-backend tests owned by no job and passing everywhere as a
silent skip. **(d)** `08h` §2 Q5's property/tenancy gate list is stale: since `ea285d1` ending a tenancy
ends its lease, so that transition now carries a cross-owner journal post and `Obj.home` writes — it
needs gate (3) and the ledger half of gate (1) after all. The remaining thirteen are accuracy, coverage
and scope items. Sequencing held on both lanes; the one process deviation is that K7.0–K8.6 crossed in a
single 24-commit merge rather than one branch per K7.x slice, with no recorded pre-merge `git merge-tree`.

## 2. Claim table per reader

| Reader | Claims | implemented | superseded | partial | contradicted | missing | escalated | **survived refutation** |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `k78` (K7/K8 contracts) | 194 | 177 | 0 | 17 | 0 | 0 | 17 | **5** |
| `e6k` (E6 ↔ K fold) | 115 | 103 | 1 | 11 | 0 | 0 | 11 | **2** |
| `gates` (08c/08h/08i/10b gates) | 126 | 73 | 21 | 26 | 4 | 1 | 31 | **8** |
| `compat` (K8.5 lanes, allowlist) | 68 | 62 | 0 | 6 | 0 | 0 | 6 | **2** |
| **Total** | **503** | **415** | **22** | **60** | **4** | **1** | **65** | **17** |

Refutation rate 48/65 (74%). Dominant causes, in order: the claim quoted pre-fold plan text a later
verify-fold section in the same document rewrote (E-C15, E-C110, E-C115); it demanded a registered
production consumer for a permission the contract deliberately leaves open (K-C23, K-C159, K-C186,
K-C187); it treated scheduled future work as a defect (G-08H-Q5-TICK-BUNDLE, G-08I-SP3,
G-10B-24-K6PREDICATE, S-10B-310-NO-LEASE-RECORD); or it was written against a pre-merge snapshot
(CODE-DECLARED-CADENCE-UNCOMMITTED — `declared.rs` is tracked and committed at `0afe161`).

## 3. Confirmed gaps, ranked

Severity weighs money → consent → disclosure → durability → scope. Owner applies the standing rule:
**E owns unless the fix is obviously K-side kernel internals.**

### S1 — cut 10 can evict a live operator; the retirement rests on a refusal that does not exist
`G-10B-310-RETIRED` (contradicted) + `G-10B-310-PREDICATE-POLICY` (missing). `10b` §3.10 retires
Draft 0.2's conditions (2) and (3) "in favour of the kernel's own answers", asserting
`ControlLeaseState::checked_request_handoff` refuses a request over a channel another holder leases.
It does not: `checked_request_handoff` refuses on `require_revision`, `DuplicatePending`/`PendingExists`,
`self.holder == draft.next_holder` → `NoControlChange`, and `require_boundary` — never comparing
`draft.requested_by` to `self.holder`. Requesting over a live lease is the ordinary path, and the tree
unwraps it (`mush_contract_tests::tactical_gate` Gate-3 revoke; the `Change::ControlLease` mutator in
`crates/mush-server/src/world_task.rs`). `HandoffProvider::prepare_request`
(`crates/mush-sim/src/handoff.rs`) adds no holder check and registers no `AuthorityGuard`. Only the
boundary leg is kernel-answered (`checked_apply_handoff` → `ControlLeaseError::BeforeBoundary`).
Condition (1) — station VACANT or held by the grant's own `FallbackSpec`, refusal naming the holder's
station and nothing else, plus a test pinning that E7 ships no eviction path — is therefore E-side policy
that does not exist (no `SeatOffer`/`SeatGrant`/`FallbackSpec` symbol under `crates/*/src`;
`@spatial/unassign` + `@spatial/assign`, `crates/mush-sim/src/commands.rs`, is itself an eviction pair).
**Smallest fix:** amend `10b` §3.10 to retire only condition (2), keeping (1) *and* (3) as cut-10 policy;
restate the gate as two policy refusals plus one kernel boundary refusal. **Owner: E.**

### S2 — a live re-dispatch runs the perception chain with an unavailable typed reader
`K-C7` (partial). `crates/ruleset-wod/src/onboarding.rs` `on_approved` — reached in production from
`crates/ruleset-wod/src/chargen_cmds.rs` on the `wod.chargen` approval — calls
`mush_core::dispatch(ctx.reg, ctx.w, target, "look", ctx.out)`. `crates/mush-core/src/cmd.rs` `dispatch`
is the `None` wrapper over `pub(crate) dispatch_with_action_state`, so
`crates/mush-core/src/commands/look.rs` and every `Ctx::can_perceive`/`is_concealed`/`hook_read_context`
in `crates/mush-core/src/module.rs` run with `typed_state.store() == None`. Fail-closed holds
(`ModuleActionState::get` → `ModuleActionStateReadError::Unavailable`, proven by the `TypedHook` veto
test in engine.rs) and no production hook reads typed truth yet, so this is latent, not live. It is
unfixable from ruleset-wod as written (`dispatch_with_action_state` and `ModuleActionState::store` are
`pub(crate)`); the remedy exists and is unused here — `Ctx::show_room` →
`commands::show_room_with_action_state(..., self.typed_state.store())`, "the one room-render path".
**Smallest fix:** replace the `"look"` re-dispatch with `ctx.show_room(target, dest)`. **Owner: E.**

### S3 — property/tenancy now needs gate (3): ending a tenancy posts to the journal
`G-08H-Q5-PROPTEN-GATES` (partial). `08h` §2 Q5 says property/tenancy needs gates (1) and (2) and *not*
(3) "since it appends no ledger entry". Since `ea285d1`, `crates/mush-housing/src/flow.rs` `commit`'s
`(_, Ended)` arm calls `end` → `end_lease` → `mush_economy::end_lease`
(`crates/mush-economy/src/contract_live.rs`) → `unwind`, which posts under
`contract/{id}/refund/{generation}` through `Ledger::prepare_append` (`crates/mush-economy/src/api.rs`);
`crates/mush-economy/tests/e4b1.rs`
`lb_rent_after_end_end_lease_posts_under_the_key_once_cancels_the_rent_and_refuses_a_foreign_key`
asserts `journal_len == posts + 1`. The same path writes `Obj.home` via `ctx.w.set_home`, which
`Engine::stage_prepared_action` refuses with `StagePreparedActionError::DirtyWorld`
(`crates/mush-core/src/engine.rs`), rechecked in `flush_prepared_action`. Gates (1) and (2) are now
closed, so the live blocker is (3) plus the world-write mix.
**Smallest fix:** amend `08h` §2 Q5 and the `08c` §12 status line to say the ending path needs all three
gates; keep the migration ordered behind them. **Owner: E.**

### S4 — nine live-Redis proofs are owned by no CI job
`K-C168` (partial). `81c438f` removed the "Start Redis 8 (Linux only)" step and its `GITHUB_ENV` export
from the cross-platform `test` job (correct; 09 §K8.6's first half holds). But `redis-faults` names only
`mush-store --test redis_suite`, `mush-server --lib` ×2, `authoritative_boot`,
`authoritative_process_kill` and `kill9_mid_play_world_survives_redis_gated`. Nine plain
`#[tokio::test]` proofs take a missing-`REDIS_URL` early return and are `#[ignore]`d nowhere, so they now
pass everywhere as a silent skip — the seven in `crates/mushd/tests/persistence.rs`
(`boot_round_trip_`, `m5_module_persistence_round_trip_`, `onboarding_persistence_round_trip_`,
`worldgen_persistence_round_trip_`, `wod_sheet_persistence_round_trip_`,
`wod_splat_sheet_persistence_round_trip_`, `npc_agent_persistence_round_trip_`, each `…_redis_gated`),
plus `crates/mushd/tests/m10c_e2e.rs` `m10c_persistence_twin_redis_gated` and
`crates/mushd/tests/shadowrun_e2e.rs` `sr5_sheet_survives_redis_reboot_gated`.
**Smallest fix:** one `cargo test -p mushd --test persistence -- --test-threads=1` step (plus the two
single-test lines) in the `redis-faults` job. A follow-on inventory test asserting every `*_gated` name
appears in `ci.yml` would make the invariant self-enforcing. **Owner: E lands the `ci.yml` lines; K
confirms §K8.6's wording.**

### S5 — nothing refuses an NPC adjudicator (half of 08i rule 2's named hole)
`G-08I-RULE2-LLM` (partial). E6b closed the sealer half:
`crates/mush-scenes/src/commands.rs` `is_sealable_principal` (`o.kind == ObjKind::Player`), checked at
conferral and again at seal, pinned by
`crates/mush-scenes/tests/e6b_seal.rs::e6b_b6_an_npc_sealer_is_refused_at_conferral_and_at_seal`. The
adjudicator half is unbuilt: `crates/mush-economy/src/contract_live.rs` `build_adjudicator`
(`@contract/adjudicator`) writes `ConsentSurface.adjudicator` from `parse_attestor` →
`crates/mush-economy/src/commands.rs` `parse_principal`, whose object arm is a bare
`match_object(...).map(PrincipalId::Object)` with no kind test, and
`crates/mush-economy/src/contract.rs` `attestor_satisfiable` returns `true` for every
`AttestorSet::Party(_)`. `ObjKind` appears nowhere in `crates/mush-economy/src`. Mitigation (severity,
not closure): no LLM completion can reach `@contract/attest` — `crates/mush-npc/src/action.rs`
`parse_actions` maps an invented tool to an inert `NpcAction::UseAbility` and
`crates/ruleset-shadowrun/src/agent.rs` `invoke` allows only `SR_ROLL_ABILITY` — so an NPC adjudicator
stalls a settlement rather than settling one.
**Smallest fix:** a kind test in `build_adjudicator` mirroring `is_sealable_principal`, plus one refusal
test. **Owner: E.**

### S6 — three decision-fault flavours freeze the world instead of the owner
`K-C76` (partial). 09 §K7.4 says a missing/mismatched pending-prompt pair, duplicate identity or
impossible transition freezes the *decision owner*. In `crates/mush-core/src/decision_source.rs`,
`DecisionSourceScanError::owner()` attributes `MissingPrompt`, `MissingDeadline`, `DeadlineLinkage`,
`DuplicateRepresentation`, `RecordIdentity`, `StaleCursor` and `Clock`, but its
`Decision(#[from] DecisionError)` variant carries no owner — and inside `DecisionSourceRegistry::scan`
three faults are raised by bare `?` while the `OwnerId` is in hand as the loop variable:
`validate_prompt_pair` (`PromptLinkage`), `observe_existing` (`DuplicateDecisionId`) and
`DecisionTerminalReceipt::from_terminal` (`InvalidTransition`). `DurableDecisionDriver::drive_one` then
leaves `failure_owner` unset and `crates/mush-server/src/world_task.rs` takes the `else` branch into
`apply_integrity_freeze` (whole-world `WorldMode::IntegrityFrozen`). Mitigations: the *wake* arm sets
`failure_owner` unconditionally and boot's `audit_durable_decision_sources` fails the spawn by design, so
only the periodic lost-wake scan — the path §K7.4 describes — is affected.
`ScheduleSourceScanError::Schedule` (`crates/mush-core/src/schedule_source.rs`) shows the wrapping
pattern decision_source omits.
**Smallest fix:** wrap those three `?`s with the loop's `owner.clone()`, as the sibling faults on the
same lines already do. **Owner: K** (kernel error attribution).

### S7 — no round-trip test proves the legacy/typed split does not clobber
`R-C16` (partial). 09b §3 permits unrelated fields beside a typed family "only when the ownership split
is explicit **and round-trip tests prove neither writer clobbers the other**". The split is explicit and
the implementation is genuinely non-clobbering both ways (`crates/ruleset-wod/src/sheet_cmds.rs`
`sheet_set` fences only `willpower_temp`; `crates/ruleset-shadowrun/src/sheet_cmds.rs` only
`PHYSICAL_OVERFLOW`; both read paths re-overlay via `WodCmd::read_ctx` /
`ConditionState::apply_to_sheet`). But no test issues a legacy write to an *unrelated* field after the
typed record exists and then re-asserts the typed value:
`crates/mush-server/tests/production_action_migrations.rs` ends on the same-field fence, and
`WodRollState`/`ConditionState` appear in no file outside the two ruleset `src/` trees; the Shadowrun
`physical_overflow` fence string is asserted by no test at all.
**Smallest fix:** two tests, one per migrated record: install typed state, `@sheet/set` an unrelated
field, re-read and assert the typed value survives. **Owner: K** (proves a K8.4 migration invariant).

### S8 — `plan_hash` has no domain tag
`S-08C-HASH-DOMAIN` (partial). Byte-compatibility is real and pinned (`crates/mush-economy/src/hash.rs`
`plan_hash`/`CanonicalPlan` skipping empty `asset_ops`/`custody_ops`/`hold_ops`, `PIN_DIGEST` and
`PIN_DIGEST_V3` hard-coded). The promised `hash_domain: "economy.plan/v3"` schema-doc note exists
nowhere: `plan_hash` feeds a bare `Sha256` with no domain prefix, and the five `plan_hash` sites in
`record.rs`/`plan.rs` carry no domain or encoding version. The merge sharpens it: K's
`CanonicalHash::for_canonical_value` (`crates/mush-core/src/authoritative.rs`) takes an explicit domain,
and `crates/mush-core/src/decision.rs` has `DECISION_PROMPT_HASH_DOMAIN` — the in-tree pattern the
economy has not matched.
**Smallest fix:** a `PLAN_HASH_DOMAIN` const plus a doc-comment note on `plan_hash`, never inside the
hashed bytes (08g §300/§343's own recorded fix). **Owner: E.**

### S9–S10 — K7 production NPC migration deferred (documented)
`K-C36`, `K-C37` (partial). The kernel half is closed: `DecisionSourceRegistry::scan` with
`validate_prompt_pair`/`validate_deadline_link`, boot/periodic repair via
`audit_durable_decision_sources` and `DurableDecisionDriver::drive_one`, and the authenticated completion
path `validate_response_fence` → `resolve_response` → `ActionCoordinator::decision_response_ingress` →
`preflight` → `prepare_and_stage_resolved`. Production NPC authority is still
`crates/mush-npc/src/runtime.rs` `NpcRuntime{in_flight, correlation}` driven from
`crates/mush-npc/src/module.rs`; the live indicators still read it (`world_task.rs` `npc_counts` →
`WorldSnapshotExtra::npc_thinking` → `mush_npc_thinking`), and `apply_action`'s `UseAbility` arm still
calls `grounding.invoke_ability` + `announce_power_use` outside `ActionCoordinator`. 09 §K7.1 keeps
production ruleset migration disabled; 08i §3 still books "K7.2 deletes `NpcAction::UseAbility`".
**Owner: K.** No E action; do not cite typed pending/prompt families as the live NPC indicator source.

### S11–S17 — accuracy, coverage and scope
| # | Gap | Fix | Owner |
|---|---|---|---|
| S11 | `S-10B-5-WHAT-MOVES` — 10b §5's "what moves at E7k" lists cut-2/cut-4 reservations as `mush-inventory`/`mush-industry` documents, contradicting the same doc's §4.1 ("authoritative from the first commit" on `ReservationBook`, `crates/mush-core/src/resource.rs`) and foreclosed by K8.5's lane rule (09b §2). Its "only when the kernel provides an atomic mixed bundle" caveat is satisfied by `ReplayWrite::LedgerChunkSet`. | Re-scope §5 to the Ledger append, work-order transitions and cross-owner materialization only | E |
| S12 | `G-08I-A14-BRANCHES` — K ran one `codex/k7-convergence` branch for K7.0–K8.6 (24 commits, single merge-back at `4c80014`), not one reviewable branch per K7.x slice, and no pre-merge `git merge-tree` is recorded anywhere. Branch origin, K6-memo-first start order and E's main-line discipline all held. | Amend 08i §7 Q14 / §9 answer 14 and 10b §11.3 to record actual practice, or re-affirm per-slice branching for K9 | joint (08i is joint; K owns its lane practice) |
| S13 | `E-C100` — no test changes the IC *rate* mid-gap and asserts `TimeJump` + nothing charged. `e4b1.rs::an_ic_epoch_jump_is_forgiven_as_a_time_jump` reseeds `GameClock { ic_ratio: 1, epoch_offset: 100_000 }` (phase jump at the default rate); every `classify_forgiveness` unit assertion passes `ratio_hint = 1`, so the `real_elapsed * ratio + ratio * SWEEP_GAP_DOWNTIME_SECS` branch is never exercised above 1. `Ctx::change_world_time_rate` has zero callers. | One test lowering `ic_ratio` between two sweeps | E |
| S14 | `S-10B-25-CONSENT` — one unforked hasher holds (`crates/mush-economy/src/contract.rs` `consent_hash`/`consent_intact`), but the named mechanism (`consent_digest(domain, canonical)` under `"campaign.seat.v1"`) is unbuilt and `consent_hash` is structurally unfactorable (fixed `CanonicalConsent`, no domain field) — as D-E7-32 itself records. | None now; the factoring ships with cut 9 | E (cut 9) |
| S15 | `E-C111` — 08f §2 item 8's framing is right but the claim's gloss overstates it: `with_title_issuers` was not empty before E6c (`housing_title_issuer()` from E5b/E5c), and E6b also added `with_canon_evidence` and `with_party_caps` in the same `modules.rs` chain. | Drop "only wiring changes" / "both empty tables" from the E6 summary | E |
| S16 | `R-C43` — `crates/mush-contract-tests/legacy-handlers.toml` holds **168** stanzas (158 baseline + 9 legacy-post-image + 1 read-only), not 169; every other element (`PINNED_BASELINE = 158`, the 159→158 `+scene` fold, the retire/re-declare/never-add rule, the nine names) is exact. Verified: 168/158/9. | One numeral in the census | E |
| S17 | `E-C115` — 08f's historical-cite rule is verbatim for §4 and §5.1–§5.4, but §2.1 and §5.6 carry no line cites at all and §4.6's were approximate from birth; §3's ~20 stale cites are governed by the headnote's "prefer §2.1 and §3.7 over §3's pre-build text" rather than by a §3 disclaimer. | Narrow the gloss to "the symbol is the citation everywhere" | E |

## 4. Gate ledger

| Source | Gate / dependency | Status on `4c80014` | Doc that must change |
|---|---|---|---|
| 08c §12 | (1) mixed write sets in one bundle | **CLOSED** — `ReplayWrite::LedgerChunkSet { write: AuthoritativeLedgerWrite }` beside `ObjectSet`/`ObjectDelete`/`ObjectAllocatorSet`/`ModuleJsonSet`/`ModuleDelete`, `ReplayTarget::Module` routing, both backends normalizing it (`crates/mush-store/src/{memory,redis}.rs`), conformance in `crates/mush-store/src/suite.rs` (`3d0788d`, K8.0) | 08c §12; 08g §10 row 6 |
| 08c §12 | (2) `Ctx` typed-family reads outside an action | **CLOSED** — `Ctx::{action_state, scan_action_state}` and `HookReadContext::{action_state, scan_action_state, typed_state_available}`, taken by `EventHook::{move_attempt, carry_attempt, perceives, is_concealed}` (`crates/mush-core/src/module.rs`, `0e3f2e8`) | 08g §10 row 6; 08h §2 Q5 |
| 08c §12 | (3) the seal inside the bundle | **CLOSED for the new lane** — bundle derives `ledger_seals` canonically (`AuthoritativeLedgerWrite::seal`, domain `authoritative_ledger_seal.v1`), wire v4, `LedgerSealSetMismatch`, `LegacyLedgerWrite` guard. **The legacy predecessor bar survives**: `flush_prepared_action` still rejects an unresolved legacy ledger predecessor (`RetainedSeals`) | 08c §12; 08h §2 Q5 (S3) |
| 08h Q5 | "no E5 record touches typed state" | **HOLDS** per record; rescope the evidence line from crate-level to per-record, since K8.4 legitimately put `WodRollState` in `ruleset-wod` | 08h §2 Q5 |
| 08h Q5 | "property/tenancy does NOT need gate (3)" | **STALE** — see S3 | 08h §2 Q5; 08c §12 |
| 08h Q5 | tick-bundle byte budget + measured bundle at the worked cap | **OPEN, not yet due** — `MAX_WORKED_PER_FIRE = 128` / `MAX_VISITED_PER_FIRE = 256` (`crates/mush-housing/src/module.rs`) against `MAX_AUTHORITATIVE_REPLAY_WRITES = 1_024` / `MAX_AUTHORITATIVE_BUNDLE_BYTES = 16 MiB`; the obligation binds the migration commit, which has not started | none |
| 08h Q6 | no world-IC ↔ cadence-`revision` bridge | **HOLDS** — `DeclaredCadenceProvider::now` compares `generation` only, never `revision`; the `ClockRef::Registered` parenthetical was overtaken by SP1 (D-LBK-7), which the same section decided | 08h §2 Q6 (drop the "no crate outside core" aside) |
| 08h §3 | `ChannelGrant` re-enumeration | **OPEN** — the gate is right (completeness check, not redesign; no union is ever assembled: `select_offer_channel_grant`, `crates/mush-core/src/action.rs`), but 08g §10 C12's consumer list predates K5 and misses `select_offer_channel_grant`, `ActionDraft::with_channel_grant`, `LegalActionProjectionError::ChannelGrant`, `control.rs`'s lease/grant guard | 08g §10 C12; 08i §5 SP1 |
| 08i §5 | SP1 — declared cadences (D-LBK-7), rule-1 tripwire, T6 CI line, K6 memo | **ALL CLOSED** — `0afe161` (`crates/mush-economy/src/declared.rs` `register`, `[economy.cadences]` → `EconomyCadenceCfg`; tripwire pinned `b0e3b52`), `redis_round_trip_preserves_exact_generations` in `redis-faults`, `7e1cf46` (`docs/design/09a-k6-look-back.md`) | — |
| 08i §5 | SP1 — E6c `Lot` decision | **DISCHARGED** — `crates/mush-market/src/record.rs` `Representation::Lot` is policy only, refused at listing creation; "It defines no `Lot`" | 08i §5 E-lane ordering string (E6c ran ahead of cuts 1–2; order-neutral once `Lot` discharged) |
| 08i §5 | SP1 — KD-11 lane statement + drift test | **CLOSED under another name** — 09's prepared-lane paragraph is the statement; `stage_prepared_action`/`stage_authoritative_action` refuse `DirtyWorld`/`LegacyModuleOps`, `flush_prepared_action` rechecks, and `prepared_action_refuses_legacy_work_added_after_staging` is the drift test | 08i §5 (rename to KD-11) |
| 08i §5 | SP2 ingress freeze | **HELD** — `schedule_source.rs` untouched since K4; `action_coordinator.rs` gained a sibling decision ingress (+575/−0); `ActionProvider` grew additively via defaults, governed by 09b §5 | — |
| 08i §5 | SP3 gate-status review | **NOT HELD as a meeting** — K8.4/K8.0/K8.1–K8.3 landed first; the pick itself is compliant (`+damage`/`+heal` disjoint from E6a's verbs, goldens in `production_action_migrations.rs`) | 08i §5 SP3 ("picked from" → "chosen away from") |
| 08i §4 | rule 1 (allowlist, not exempt) | **LIVE and CI-gated** | — |
| 08i §4 | rule 2 (no direct LLM mutation) | **HALF-CLOSED** — sealer done, adjudicator open (S5) | 08i §4 rule 2 |
| 08i §4 | rule 3 (no trigger side effect in a later commit) | **RULED and precedented** — `EconomicContract.pending`/`canon_published` with `a_restart_between_the_post_and_the_doc_write_converges`; the bounded pending-set form ships with cut 9 | optionally 09 §C6 |
| 08i §4 | rule 4 (shared store conformance) | **COMPLIANT** — no E `StreamAppend`/Lua/native index; ledger via `PreparedLedgerAppend`; `crates/mush-store/src/suite.rs` driven by both backends. E2-D30's per-cut E-crate Redis reboot test is an open XS item | 08g §11 |
| 08i §6 | ownership lines (config; kernel records) | **HELD** — one contact family (`crates/mush-tactics/src/module.rs`), one lease family (`crates/mush-sim/src/lib.rs`), one production unit authority (`UnitStore`), one placement authority (`SpatialState.placements`); no E crate registers a state family | — |
| 08i §9 a14 | branch discipline | **DEVIATED** (S12) | 08i §7/§9 answer 14; 10b §11.3 |
| 10b §5 | E7k trigger (after K8.0 + K8.1–K8.3) | **FIRED** — 09 §K8 marks K8.0–K8.6 closed; review-only corroborated both sides (09 §K3.1: "the journal remains sole authority until the E owner accepts and proves each migration") | 10b §5 (tense) |
| 10b §5 | gate-2 residue blocking housing | **DELIVERED by K** — `HookReadContext` is in the tree and `crates/mush-housing/src/hooks.rs` `AccessHook::{move_attempt, carry_attempt}` already take it (`may_reach` still reads legacy `HousingStore`) | 10b §5 |
| 10b §5 | "what would move at E7k" list | **STALE in two rows** (S11) | 10b §5 |
| 10b §5 | records shaped for mechanical migration | **ENFORCED** — `Registrar::classify_commands` fails composition closed; the tripwire requires `family` + `note`; `AuthoritativeBundleError::DuplicateWrite` | — |

## 5. K8.4 / K8.5 / K8.6 compatibility

**Required.** 09b §2 gives every registered command exactly one lane —
`CommandCompatibility::{ReadOnly, LegacyPostImage, Authoritative, EconomyCarveOut}`
(`crates/mush-core/src/cmd.rs`) — declared before registration or composition fails closed
(`crates/mush-core/src/module.rs` `Registrar::{classify_commands, command, alias_stage}`). A
`LegacyPostImage` emits only a bounded, strictly decoded whole-document post-image under a deterministic
owner key, must name its future typed family, key codec, migration slot and authoritative replacement,
and may never mutate a K-owned typed family, reservation, contact, lease, window, resolution or
production record — enforced structurally by `validate_module_op` and by `ModuleActionState` exposing
only owner-scoped `get`/`scan_family`. 09b §3 adds the per-record authority fence and the round-trip
evidence duty (S7); 09b §5 requires new mutating mechanics to start as provider + typed family *or
extend an existing owner*, and holds provider traits to backward-compatible defaults only.

**E6 satisfies it.** Every E module declares a lane — `mush-economy` `EconomyCarveOut`; `mush-market`,
`mush-party`, `mush-org`, `mush-housing`, `mush-scenes`, `mush-canon`, `mush-jobs`, `mush-mail`,
`mush-faction`, `mush-script` `LegacyPostImage` (40 call sites tree-wide, zero unclassified). The nine
non-baseline stanzas each name a future family (`sr:run`, `party:party`, `sr:profile` ×2, `sr:fence`,
`scenes:scene` ×2, `market:listing`, `market:catalog`); the census is 158 + 9 + 1 = 168 against
`PINNED_BASELINE = 158`. E6's market *extends an existing owner* rather than opening a second authority:
every money act routes through `mush_economy::{offer_contract, accept_contract, accept_contract_as,
create_system_obligation, issue_title}` (`crates/mush-market/src/{trade,api}.rs`), and `mush-party` has
no economy dependency. Boot validation, re-encode round trips and "failure emits no success" tests exist
per record (`crates/mush-market/src/boot.rs` `reconcile`; `crates/ruleset-shadowrun/tests/e6c.rs`
`vf_r1`/`vf_r2`, `e6c_b2_*`, `e6c_b6_*`, `e6c_b7_*`).

**E7 must.** (i) Add every new handler to `legacy-handlers.toml` in the same commit as its cut, with a
real category — never `baseline` — lowering `PINNED_BASELINE` when a baseline verb is retired or
re-declared (`+scene` 159→158). (ii) Call `classify_commands` per registration group: cut 2's and cut 4's
reservation verbs are `Authoritative` over `ReservationBook` (`crates/mush-core/src/resource.rs`) with
per-owner `AuthorityGuardKind::Reservation` validators (E7 writes the first ones); cuts 4/9/10/11's
workflow/catalog documents are `LegacyPostImage` with a named future family. (iii) Settle the ingress
shape before cut 2: `CommandHandler`/`Ctx` is synchronous and cannot reach
`ActionCoordinator::gateway_ingress`, so the first `Authoritative` registration needs an adapter seam
that does not exist. (iv) Keep clear of K-owned families — no second `ControlLease` validator, no
lease-like state family, no second `ContactState` lifecycle (08i §4, 09a §3), with the cut-5 witness
probes as tripwire. **CI:** `test` (both OSes) runs `fmt`/`clippy -D warnings`/`build`/`cargo test
--workspace` and is the *only* job running the allowlist tripwire; `kernel-contracts`,
`definition-wire-fixtures`, `direct-rand-policy`, `truth-serialization-policy` and `redis-faults` are the
named K8.6 gates. S4 is the one hole.

## 6. Cross-tree seams, both directions

1. **Declared cadences ↔ `DomainClockProvider`.** E registers one composition-frozen
`ClockProviderKind::Registered` provider per `[economy.cadences]` stanza
(`crates/mush-economy/src/declared.rs` `register`); K supplies the separate owner-keyed
`DomainClockProvider` (`crates/mush-core/src/clock_provider.rs`, `Registrar::domain_clock_provider`).
They never meet: `EconomicClock` is `{RealUtc, WorldIc, Manual, Cadence}` with no `DomainTick` arm and no
`From<ClockRef>`, so 10b's C2 prohibition is unwritable rather than merely observed. The only
`DomainTick` registrant is the `arena` fixture (`crates/mush-contract-tests/src/tactical_gate.rs`); the
production driver is cut 8. **Action:** rest 10b §2.5 C2 on `EconomicClock`'s shape, since its stated
release trigger has fired.
2. **Typed hook reads ↔ housing / E7 hooks.** K delivered `HookReadContext`;
`crates/mush-housing/src/hooks.rs` `AccessHook` already takes it while `may_reach` still reads legacy
`HousingStore`. The seam is live and unconsumed — the follow-through is S3, not more kernel work.
3. **`pin_deferred` ↔ authoritative bundles.** `Outbox::{pin_deferred, deferred_pin, truncate_deferred}`
(`crates/mush-core/src/output.rs`) is E's terminal-batch primitive, called from
`crates/mush-canon/src/lib.rs` `commit` and `crates/mush-economy/src/api.rs` `install_commit`;
`AuthoritativeBundle` is the other lane. They do not compose and need not — nothing fallible follows the
pin, which is what rule 3 asks. When E7k moves the settlement append onto
`PreparedLedgerAppend::authoritative_replay_write`, the pin becomes redundant for that write only.
4. **`recent_sealed` ↔ the ledger.** `crates/mush-canon/src/lib.rs` reads
`mush_core::recent_sealed(&data.kv, LOG, data::SEALED_TAIL_CHUNKS)` from `crates/mush-core/src/ledger.rs`.
Unchanged by the merge and unaffected by `LedgerChunkSet`, which carries the same
`AuthoritativeLedgerWrite` post-image seals feed from.
5. **`SealerGate`.** Composition-frozen in `crates/mush-scenes/src/data.rs` `SealerGateSlot`, consulted
at `crates/mush-scenes/src/commands.rs` conferral/seal, referenced from
`crates/mush-economy/src/contract_live.rs` as the gate the economy does *not* own — the pattern S5's
adjudicator fix should copy.
6. **Module order.** `crates/mush-server/src/modules.rs` `server_modules_with` fixes economy → market →
rulesets, with `with_canon_outcome_kinds([scene.outcome])`, `with_title_issuers([wod.housing?,
sr.market?])`, `with_canon_evidence` and `with_party_caps(vendor_caps(shadowrun))` on the economy. The K
merge's one behavioural fix here was classifying `mush-market` `LegacyPostImage`, without which every
server suite failed at setup — the composition root is the real integration surface, and E7 cuts should
extend it rather than fork it.

## 7. Fix list

**Fix before E7 cut 1** (ordered; effort in half-days)

| # | Fix | Owner | Effort |
|---|---|---|---|
| 1 | `10b` §3.10: retire only condition (2); keep (1) and (3) as cut-10 policy; restate the gate as two policy refusals + one kernel boundary refusal (S1) | E | 0.5 |
| 2 | `crates/ruleset-wod/src/onboarding.rs` `on_approved`: `ctx.show_room(target, dest)` in place of the `"look"` re-dispatch, with a test that the post-approval room render sees a live typed store (S2) | E | 0.5 |
| 3 | `08h` §2 Q5 + `08c` §12: property/tenancy needs all three gates on the ending path (`flow::commit` `Ended` → `end_lease` journal post + `Obj.home` writes) (S3) | E | 0.5 |
| 4 | `.github/workflows/ci.yml` `redis-faults`: add `-p mushd --test persistence`, `m10c_persistence_twin_redis_gated`, `sr5_sheet_survives_redis_reboot_gated` (S4) | E lands / K confirms §K8.6 | 0.5 |
| 5 | `build_adjudicator` kind test mirroring `is_sealable_principal`, plus one refusal test (S5) | E | 0.5 |
| 6 | `decision_source.rs`: attribute `PromptLinkage`, `DuplicateDecisionId`, `InvalidTransition` to the scan loop's owner (S6) | K | 0.5 |
| 7 | Two clobber round-trip tests, one per K8.4-migrated record (S7) | K | 0.5 |
| 8 | `08g` §10 C12: re-enumerate `ChannelGrant` consumers (five new ones since K5) — owed before cut 5, cheapest now | E | 0.5 |

**Fix during E7**

- `PLAN_HASH_DOMAIN` const + `plan_hash` doc note, never in the hashed bytes (S8) — E, with the E4b2 touch.
- Mid-gap IC *rate*-change forgiveness test (S13) — E, with cut 8's clock work.
- E2-D30: a Redis-gated reboot test in the same cut as each money-adjacent boot path (08g §11) — E.
- An inventory test asserting every `*_gated` test name appears in `ci.yml` (makes S4 self-enforcing) — E.
- `consent_digest(domain, canonical)` factoring under `"campaign.seat.v1"` (S14) — E, ships with cut 9.
- ~~First `Authoritative` command registration needs an ingress adapter seam (§5 (iii)) — E, before cut 2.~~ **DISCHARGED by E7 cut 2's B1 (2026-09-05).** The seam is `mush_core::cmd::Ctx::submit_authoritative` → `mush_core::Deferred::AuthoritativeSubmit` (a new arm beside `Persist`, drained by `Engine::drain_deferred` into a single-slot queue `Engine::take_authoritative_submit` empties) → `mush_server::world_task::CommandSubmitDriver::drive_one` → `ActionCoordinator::command_ingress` + `ActionCoordinator::preflight` → `mush_server::world_task::resolve_and_stage_offer` → `ActionCoordinator::prepare_and_stage_resolved`. It is a **selector, not an offer**: the handler names a registered `DefinitionRef` and the driver regenerates that actor's own projected offer (`ProjectedLegalActionSink::projected_for_action`), so no handler can construct an `ActionOfferId`; a second submission in one command voids the first and stages nothing; and `resolve_and_stage_offer` is the **one** spelling of the five steps, shared with the `WorldMsg::AuthoritativeAction` arm and pinned by `e7c2_b1_both_ingresses_call_one_five_step_function`. 08i §2 row 5 (*"There is no `Ctx`-level action submit"*) closes with it. Shape and proofs: 10b §3.2's "As built".

**Accept**

- K7 production NPC migration deferred (S9–S10) — 09 §K7.1 says so; do not re-file.
- `DEFAULT_DECISION_PROVIDER_WORKERS = 8` inert, and `CommandCompatibility::Authoritative` unregistered,
  until a production decision composition / the first migrated mechanic exists.
- Conservative over-declaration (`+nuyen` `read-only` in the toml but `LegacyPostImage` at the registrar;
  Shadowrun's whole-ruleset `LegacyPostImage` after `+damage`/`+heal` became refusals) — permitted by
  09b §2's ceiling semantics.
- `mush-market`'s wires lack `#[serde(deny_unknown_fields)]` where kernel records have it — a consistent
  E-side convention (10b §4 rule 1 defines strictness as the `TryFrom` wire), not a breach.

## 8. Doc amendments owed

| Doc § | Amendment |
|---|---|
| `08c` §12 | Gates (1) and (2) closed by K8.0/`0e3f2e8`; gate (3) closed for the new lane with the legacy-predecessor bar surviving; property/tenancy needs all three (S3). Closing paragraph: `mush-contract-tests` now hosts toy verticals **and** five workspace policy gates — the backlog verdict survives, its premise does not. |
| `08g` §10 | Row 6 (C7): all three K2 gates closed; drop "gate 2 open". C12: re-enumerate `ChannelGrant` consumers. §300/§343: keep `PLAN_HASH_DOMAIN` open (S8). §11: keep E2-D30 open. |
| `08h` §2 Q5 | Gate list stale (S3). Rescope "no E5 record touches typed state" from crate-level to per-record. Fix the `wod store.rs:190-192` anchor to `crates/ruleset-wod/src/economy/store.rs` `put_profile`. |
| `08h` §2 Q6 | Drop the "`ClockRef::Registered` appears in no crate outside core" aside — SP1 landed the registration the same section decided. |
| `08i` §4 rule 2 | Record the sealer half as discharged (E6b) and the adjudicator half as open (S5). |
| `08i` §5 | E-lane ordering string still lists E6c after cut 2 while the SP1 paragraph below records the `Lot` discharge that makes it order-neutral. Rename the KD-11 remainder to the code that implements it. SP3 wording: "picked from" → "chosen away from". |
| `08i` §7 Q14 / §9 answer 14 | Record the single-branch K7+K8 convergence and the unevidenced `git merge-tree` (S12). |
| `10b` §2.5 | C2: rest the `DomainTick` prohibition on `EconomicClock`'s shape, not on a trigger that has fired. "Absent repo-wide": note the `91208ab` baseline — `mush-market` and `mush-party` now exist, and E7 must not create either name. Canon-outcome row: the set holds `scene.outcome`, not empty. |
| `10b` §3.10 | S1 — the retirement of condition (3) is misattributed; keep (1) and (3). |
| `10b` §5 | S11 — re-scope "what would move at E7k"; the trigger has fired (review only). |
| `10b` §11.3 / headnote | Per-slice branching with pre-merge `merge-tree` is asserted as fact; it was not the practice (S12). |
| `08f` §2 item 8 / §4.6 | Drop "only wiring changes" and "both empty tables" (S15); the historical-cite rule applies to every section (S17). |

## 9. Questions for the K side

1. **Owner attribution (S6).** Is the whole-world freeze on `PromptLinkage`, `DuplicateDecisionId` and
   `InvalidTransition` in the *periodic* scan deliberate — do you read those three as inherently
   unattributable — or is the missing `owner: owner.clone()` wrapping in `DecisionSourceRegistry::scan`
   an oversight? A cross-page `merge_existing` duplicate genuinely spans owners; the other two do not.
2. **`Authoritative` ingress shape (§5 (iii)).** `CommandHandler`/`Ctx` is synchronous;
   `ActionCoordinator::gateway_ingress` is async over `IngressEnvelope`. What is the intended adapter for
   the first command registered `Authoritative`? E7 cut 2 hits this before anything else does.
3. **K7 NPC cutover (S9–S10).** Is `NpcAction::UseAbility`'s deletion still K7.2's, and does the
   pending/prompt-family-derived live indicator (replacing `npc_counts`' `rt.in_flight`) land with it?
   E's dashboards read `mush_npc_thinking` today.
4. **K8.6 job ownership (S4).** Should the nine unowned gated proofs join `redis-faults`, or were some
   intended to be retired with the Linux-leg export?
5. **Branch discipline for K9 (S12).** Re-affirm per-slice branches with a recorded pre-merge
   `git merge-tree`, or amend 08i answer 14 to the single-convergence-branch practice actually used?

## 10. Method

Four independent readers extracted 503 checkable claims from 09, 09a, 09b (K side) and 08c, 08f, 08g,
08h, 08i, 10b (E side), each classified against the merged tree as implemented, superseded, partial,
contradicted or missing. Every claim that did **not** read as implemented-or-superseded (65) went to an
adversarial refutation pass whose brief was to find the behaviour implemented under another name, in
another crate, in tests, or deliberately superseded by a later document — the burden was on refuting the
gap, not on proving it. 48 refutations succeeded; the 17 in §3 survived. Verification is by path +
symbol, never line number (08i §8), resolved in the tree at `4c80014`; the gap-supporting facts in §3
were re-checked directly (the nested `dispatch` in `crates/ruleset-wod/src/onboarding.rs`; ten `*_gated`
test names against `ci.yml`'s six; the 168/158/9 allowlist census; `hash_domain` absent from
`crates/mush-economy/src`; `checked_request_handoff`'s four refusal arms; `flow.rs` `end` → `end_lease`
and `set_home` against `StagePreparedActionError::{DirtyWorld, LegacyModuleOps}`; 40
`classify_commands` call sites; the 24 commits in `4c80014^1..4c80014^2` and the three `codex/` refs).
No file in the tree was modified, no test was run beyond reads, and `cargo fmt` was not run.

## 11. Fold outcome (2026-09-05, on `7419bd9`)

The eight **fix-before-E7-cut-1** items of §7, plus the "fix during E7" inventory test, executed as
one fold. Regression tests live in `// ---- convergence fold ----` sections and are named `cv_<n>_…`
after the §7 item they close. Nothing was committed; `docs/interface/` was not touched.

| # | Gap | What changed (path + symbol) | Test |
|---|---|---|---|
| 1 | S1 | `10b` §3.10: only condition (2) retired; (1) **and** (3) restated as **cut-10 policy**, with `checked_apply_handoff`'s `BeforeBoundary` as the single kernel leg; the two "refused by the kernel" test lines corrected in §3.10's test list | docs only |
| 2 | S2 | `crates/ruleset-wod/src/onboarding.rs` `on_approved` — `ctx.show_room(target, dest)` replaces the `mush_core::dispatch(…, "look", …)` re-dispatch; the empty-input-stack guard is kept for a narrower reason | `ruleset-wod/tests/onboarding.rs::cv_2_the_post_approval_room_render_sees_a_live_typed_store` (a `LookRendered` probe recording `HookReadContext::typed_state_available`; it reads `false` on the old code) |
| 3 | S3 | `08h` §2 Q5 (gate list rewritten; per-record rescope; the `put_profile` anchor fixed) and `08c` §12 (C2/K2 row + the K3-review paragraph): the ending path needs **all three** gates, and all three are closed — (1)/(3) by K8.0 `3d0788d`, (2) by K7 `0e3f2e8` — so the migration is **startable**, still an owner decision | docs only |
| 4 | S4 | `.github/workflows/ci.yml` `redis-faults`: new step "Reboot-from-Redis-alone round trips" — `-p mushd --test persistence`, plus the `m10c_persistence_twin_redis_gated` and `sr5_sheet_survives_redis_reboot_gated` single-test lines. New self-enforcing gate `crates/mush-contract-tests/tests/ci_gated_test_inventory.rs` | `every_backend_gated_proof_is_named_by_a_ci_job`, `the_gated_suffix_census_is_stable_and_every_live_backend_proof_names_its_backend` (removing the new CI step fails the first with the nine names) |
| 5 | S5 | `crates/mush-economy/src/contract_live.rs` `is_adjudicable_principal` + `ADJUDICATOR_IS_NOT_A_PRINCIPAL`, checked in `build_adjudicator` — mirrors `mush-scenes`' `is_sealable_principal`; only the `AttestorSet::Party(PrincipalId::Object)` arm narrows | `mush-economy/tests/e4b2.rs::cv_5_an_npc_principal_is_refused_as_adjudicator`, `::cv_5_the_adjudicator_kind_test_only_narrows_the_object_arm` |
| 6 | S6 | `crates/mush-core/src/decision_source.rs` — new `DecisionSourceScanError::OwnedDecision { owner, source }` (in `owner()`), and `DecisionSourceRegistry::owned_decision_fault` attributes exactly `PromptLinkage`, `DuplicateDecisionId` and `InvalidTransition` to the scan loop's owner at the three bare `?` sites | `mush-server` `world_task::tests::cv_6_a_malformed_decision_freezes_its_owner_not_every_other_owner` (a second composed owner, `CvBystanderModule`, stays ready and the scan continues) |
| 7 | S7 | `crates/mush-server/tests/production_action_migrations.rs` `typed_boot_records` + two round trips over `WodRollState` and `ConditionState` | `cv_7_wod_a_legacy_sheet_write_beside_the_typed_roll_state_survives_a_reboot`, `cv_7_shadowrun_a_legacy_sheet_write_beside_the_typed_condition_state_survives_a_reboot` |
| 8 | §4 `ChannelGrant` row (not S8 — S8 is `plan_hash`, still open) | `08g` §10: new sub-section **"C12 re-enumerated — `ChannelGrant` on the merged main"** — one production projection, **nine** consumers in three groups, the five new since K5 named; the C12 row itself annotated | docs only |
| 9 | §8 | Every remaining §8 amendment applied: `08c` §12 (gates + the `mush-contract-tests` premise), `08g` §10 (C7 row, C6/E2-D30 re-confirmed open), `08h` Q5/Q6, `08i` §2 headnote and rows 13/18/20/25/26/27/29/30/31, §4 rule 2, §5 (E6c order-neutral, SP1 closed, KD-11 renamed to its code, SP3 wording), §7 Q14 + §9 answers G-A/G-B/2/13, `10b` §2.5 (C2 on `EconomicClock`'s shape, the `mush-market`/`mush-party` note, the canon-outcome row), §5 (E7k precondition; "what moves" re-scoped), headnote + §11.3 (branch practice), `08f` headnote (the cite rule is document-wide) and §2 item 8 | docs only |

**Three notes on how the conservative reading was taken** (§9's questions are *not* answered here).

1. **Q1, owner attribution (item 6).** The conservative reading is "attribute, never drop, and never
widen": `owned_decision_fault` promotes **only** the three flavours 09 §K7.4 names. `DecisionId::derive`
hashes the controller — and so the owner — into the identity, so a duplicate inside one scan is
necessarily same-owner; a prompt pair and a terminal transition never span owners either. Aggregate
faults `observe_existing` can also raise (byte/count ceilings, canonical-encoding errors) are
properties of the *page*, not of one owner, and stay unattributed, so they still fail the world
closed rather than freezing a blameless owner. If K answers that all three were meant to be
whole-world, this is a two-line revert.
2. **Q4, job ownership (item 4).** Read conservatively as "none of the nine were meant to be
retired": all nine are now owned, the seven in `persistence.rs` by one whole-target step as §3 S4
prescribes. If K wanted some retired, deleting them also satisfies the new inventory test.
3. **Q5, branch discipline (item 9).** The deviation is *recorded* in `08i` §7 Q14 and `10b` §11.3
and the headnote; answer 14 is **not** amended to ratify single-branch practice, because that is the
owner's call. Until it is made, the recorded answer stands and the note reads as a deviation.

**One scoping decision inside item 4, stated because it narrows §7's wording.** The inventory keys on
*skippability*, not on the `_gated` suffix: a test is backend-gated when its own body reads
`REDIS_URL` or it carries `#[ignore]`. Four workspace tests end in `_gated` in the English sense
(`admin_commands_are_gated`, `chown_is_power_gated`, `harm_is_staff_gated`,
`resolved_revision_coordinates_are_bounded_unique_and_bridge_gated`); they run and fail under
`cargo test --workspace` and naming them in `ci.yml` would be noise. The suffix is still pinned as a
census — ten live-backend proofs, each of whose name must say `redis` — so the two halves cannot
drift apart. The scan also sees no macro-generated test, which is why `mush-store`'s `redis_gated!`
suite is covered by its `--test redis_suite` target line instead.

**Not done here, by design:** S8 (`PLAN_HASH_DOMAIN`), S13 (the mid-gap IC *rate* test), S11's
remaining E7-cut work, S14 (`consent_digest`), S16's allowlist numeral and the E2-D30 per-cut Redis
tests are §7 "fix during E7" items with their own cuts; S9–S10 are "accept".
