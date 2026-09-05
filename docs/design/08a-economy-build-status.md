# Economy / org / ownership — build status & E6 handoff

**Companion to [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md)** (the
design; prefer it for *why*), [08b-e3-implementation-plan.md](08b-e3-implementation-plan.md) (the
E3 plan as reviewed and as built) and [08c-e4-implementation-plan.md](08c-e4-implementation-plan.md)
(the same for E4 — prefer its §3.5a/§4.7/§5.1/§6.1 "as built" blocks for *how* E4 fits together).
This doc is the **build ledger**: what of the E0–E8 plan is actually shipped, the decisions made
while building it, the live seams a later milestone consumes, and the concrete pickup point for
**E6**. Read this first when resuming economy work. **E5 is built** — for *how* the housing and WoD
layers fit together, prefer [08d](08d-e5-implementation-plan.md)'s §3.5 / §4.1 / §5.6 "as built"
blocks.

E0–E3 are on `origin/main` (the E3 commits, `99f5b9c` through the post-E3 module-order commit, were
pushed 2026-09-03). Each cut ran the standing rhythm — design-review-before-build, adversarial-verify — with
one variation for E3: by owner instruction the adversarial verify ran **once, over the whole E3
diff, after E3d**, and its confirmed findings were folded in a separate commit (see the last E3
Shipped row) rather than per cut. **E4 went back to per-cut verifies** — E4a, E4b1 and E4b2 each
have their own adversarial pass and fold commit, and 08c §3.7 / §4.8 / §5.2 record the outcomes.
**E5 kept the per-cut cadence**: E5a, E5b and E5c each have their own adversarial pass and fold, recorded
in 08d §3.6, §4.2 and §5.7; the **E1–E5 look-back** (08g) then folded its fix-before list — the K side's
D-LBK items (08h) merged in — as one more row below.

## Shipped

| Cut | Crate / area | Commit | One line |
|-----|--------------|--------|----------|
| **E0** | `mush-core::ledger` + server durability | `565ff1b`, `32ca322` | ack-gated Ledger seal/rollover: byte-scan retained-write table re-emits un-acked sealed chunks, poison-seal fail-closed, boot contiguity |
| **E1a** | `mush-org` (pure library) | `6e9a030` | id vocab + records + `OrgState` + the pure `authorize()` engine + closed `OrgMutationPlan` |
| **E1b** | `mush-org` `OrgModule` | `11090aa` | persistence, fail-closed boot, `authorize_in_world`/`submit` seam, audit ledger, destroy scrub |
| **E1c** | `mush-faction` `FactionStandingProvider` | `97afebf` | faction member/officer/intel resolved into org policy predicates |
| **E2a** | `mush-economy` (pure library) | `8bed472` | checked `Amount`, the journal model, `validate`→`apply`, SHA-256 `plan_hash`, idempotent reversal |
| **E2b** | `mush-economy` `EconomyModule` | `547aec6` | journal on the E0 Ledger, atomic post bundle, **strict durable money gate**, directional boot self-heal |
| **E3 plan** | docs | `99f5b9c` | 08b — three-pass-reviewed plan (adversarial, grounding, design-fit) |
| **E3a** | `mush-economy` (pure) | `e780fd7` | `EconomicAsset`/`Vault`/`ApprovalRule`/`ApprovalRequest`, `AssetOp`/`CustodyOp` in the plan, invariants 10–22, `plan_hash` v2 (E2-identical for money-only plans), `NeedsApproval`, the `CustodyEval` + extended `AuthEval` seams |
| **E3b1** | `mush-economy` module | `288f769` | asset docs as journal projections (+ the `frozen` latch), vault docs, the `CustodyProvider` seam + `ContainerProvider`, `@asset`/`@vault`, destroy gate + `ObjDestroyed` backstop + boot liveness/custody reconciliation |
| **E3b2 (org)** | `mush-org` | `cb7c8f3` | `OrgStatus`/`ScopeStatus` enforced in `authorize`, `RetireScope`, scope cap, capability vocabulary, `ScopeUseProvider`, `@org/scope`, `@org/grant scope=/limit=` |
| **E3b2** | `mush-economy` module | `ae3de96` | org-scoped wallets, approvals re-qualified live at post, system-debit gate in the bridge, non-staff reads under existence hiding, `@account/close|unfreeze|approval`, `@econ/approve|deny|approvals` |
| **E3c (sim)** | `mush-sim` | `d48b1eb` | `InteriorCustodyProvider` — vaults over interiors; economy boots after sim |
| **E3c (tactics)** | `mush-tactics` | `fe4faea` | `SetComponentAsset` + the M33-deferred `asset` scrub on destroy and at boot |
| **E3d** | `mush-faction` | `d8d17c3` | `Faction.org` + `migrated`, `@faction/org|migrate`, view-gated balance read-through, destroy refused while bound |
| **E3 verify fold** | economy + faction | `fa97479` | the end-of-build adversarial verify's confirmed findings, each with a regression test (§"E3 verify outcome") |
| **E4 plan** | docs | `1555b69` → `3a061f6` | 08c — three-pass reviewed, then owner answers, cadence clocks, and doc 09 conformance after the K merge |
| **E4a** | `mush-economy` (pure) | `29282fe`, fold `13f11d0` | `Hold`/`HoldOp`/`HoldEffect` + available-balance enforcement; clock/schedule/catch-up math; `Obligation` and `Contract` records with their pure state machines; the deterministic split; `ContractAuth`; `plan_hash` v3; `FactEval`, `ExternalRef` (the `CausalRef` rename), `reverify` |
| **E4b1** | `mush-economy` module | `0805f5e`, fold `389c521` | holds + obligations live: persistence and boot, journal-derived `held_amount` on its own seq anchor, the **one economy sweep** tick, the graded escalation ladder, `post_with`/`PostAuth`, infallible `install_commit`, `@hold/*`, `@obligation/*`, `@econ/alerts\|ack`, faucet-funded obligations under a mint cap |
| **E4b2** | `mush-economy` module | `e56c2a9`, fold `4972455` | contracts end to end: draft/offer/accept (escrow at accept)/attest/settle/dispute/cancel/expire, live re-verification, custody locks, `Lease`/`Collateral`, the sweep's contract step, `@contract/*`, the outcome table + roster lock + adjudicator, `EconomicVisibility::Audience`, and the plan-first terminal-transition rule |
| **E4c** | `mush-economy` + `mush-org` + `mush-jobs` + `mush-server` | `8873863`, fold `fa6a979` | the `FactProvider` seam (jobs implements it), the `CanonRecorded` attestation observer, `EconomyObserver` + compact events + the canon evidence bridge, `mush_org::audit`, `@econ/history` under the disclosure filter, `issue_title` for registered system principals; **two `mush-core` changes** — `WorldEvent::CanonRecorded` gains `source`, and (verify fold) `Outbox::pin_deferred` makes a terminal canon commit non-truncatable by a failing command |
| **E4c verify fold** | economy + jobs + server + core + canon | `fa6a979` | three lenses' confirmed findings, each with a regression test (08c §6.2): canon attestors must be **producer-declared outcome kinds** with a mandatory source and a party in `entities`; `jobs.resolved` answers from an applied-payload marker; a missed canon edge is recoverable (`@econ/unfreeze`, activation re-scan, `@contract/resolve <id>=settle [reason]`); deterministic conditions carry an observed-at stamp so `After` can anchor on one; `issue_title` needs a live object and mints a title **generation**; the reservation-visible / contract-hidden rule; publication keyed on a durable `canon_published` marker with a sweep retry |
| **E5 plan** | docs | `08d` | three-pass reviewed, plus the owner's twenty answers (08d §10) and the three accepted defaults (§11), all binding |
| **E5a** | `mush-housing` (pure) | `6b606e5`, fold `7155efc` | `Property`/`Tenancy`/`SupportPlan`, the tenancy state machine that **never ends a home on its own** (D-E5-1), the multi-resident best-of assessment (`assess_at` + `harden`), the fail-closed access predicate, the `Obj.home` invariant helpers, the story window's notice decay. 55 tests (08d §3.5/§3.6) |
| **E5b** | `mush-housing` module | `b1b4254`, fold `d9f9ceb` | persistence + boot reconciliation, the `MoveAttempt`/`CarryAttempt` hook, the destroy gate + `ObjDestroyed` backstop, the **pull-model** assessment tick, the lease-backed exact mode, the story job + escalation, `@property/*`, `@tenancy/*`, `+home`, `+guest`. 66 in `tests/e5b.rs` (crate 121) (08d §4.1/§4.2) |
| **E5c** | `ruleset-wod::economy` + `mush-config`/`mush-server` + four additive seams | `816edfd`, fold `ba75fc3` | the WoD layer: `[wod.economy]` parsed host-side, `SpendingProfile`/`CareerProfile`/four stances, the period evaluation on the cadence clock (one period per fire, rotation-capped), the attention budget with housing preemption, the bounded per-faction pool, non-response defaults, front attribution incl. orphaned fronts, `wod.usd` + three faucets + faucet-funded income in exact mode, the chargen Residence stage and the mode-dependent approval artifacts, `@wod/economy migrate`, `+career`/`+prompt`/`+resources`/`@career/set`. 60 tests — 40, plus 20 in the fold (08d §5.6/§5.7) |
| **E1–E5 look-back fold** | economy + housing + jobs + ruleset-wod + server + CI | *(this commit)* | 08g §11's fix-before list, code half (08g §13): CI's Format step advisory until the post-K6 reformat and `--no-fail-fast` on the test step (T1b/T4); housing **boot pass 0** — a room in two properties, two live tenancies on one property, a live tenancy on no property are **quarantined fail-closed** (staff-only at the door, the destroy gate refuses, the tick skips, "(under review)" renders, one page per conflict with the shipped repair, release only by re-validation, `@housing/quarantine show\|recheck`) with the **rent suspended through a new game-neutral economy seam** — an obligation carries a set of `SuspensionKey` holders and leaves/returns to the due pool with the set, the sweep forgives each suspended period as it falls due (`Suspension`), `@econ/ack`/`@obligation/resume`/`reinstate` release only their own key (D-LBK-1); room → `Unique\|Conflict` and property → `Live\|Conflict\|Vacant` projections maintained by the single write path with recomputation-equivalence tests (D-LBK-4a); visited/worked tick budgets 256/128, cursor past the last visited id, due check before the clone, a `population_version`-keyed idle short-circuit in both tickers (D-LBK-2); `TryFrom` wires for `SpendingProfile` and its nested types and a 240-byte cap on a loaded `PendingNotice` (D-LBK-3); `resolve_plain_job`/`deny_plain_job` with queue identity replace the public `set_job_status` (D-LBK-6); `CadenceClock.generation`/`schema` with serde defaults (D-LBK-7, the code half); `register_instrument` create-only + the engine-level `InactiveInstrument` test (E2-C18); the config → `with_reserved_rooms` chain pinned in `mush-server` (E5c-C231/C2); the lapse guard hoisted (E5c-C96); `Property.upkeep` dropped (E5b-D34); the boot replay log + review thresholds (C17); the comment batch. **24 tests** (housing 9, ruleset-wod 5, economy 5, jobs 4, server 1) |
| **E6a step 1** | `mush-economy` + `mush-config` + `mush-server` | *(this commit)* | the economy half of E6a, game-neutral (08f §2.1): **A′ consent by authorship** — an amend keeps the author's acceptance and escrow and refunds only the other parties' through `refund_plan_except` in one batch with the `Draft` doc, `contract_offer` re-stamps surviving consents and refuses an unbacked acceptance, `build_pay` refuses changing escrowed consideration, a `Draft` with escrow expires instead of being collected (M10/M16); the **rolling approval window** `ApprovalRule.window { total, period_secs }` (`@account/approval … window=<total>/<secs>\|off`) evaluated beside the threshold through `ValidateEnv::spent` from a RAM-only per-account `VecDeque` fed by one `note_window_debits` at post and at boot replay (D-E6-21); `create_obligation`/`cancel_obligation`/`reinstate_obligation` for `ObligationCause::Ruleset`; `offer_contract`/`accept_contract`/`amend_contract` over the command cores (`ContractSpec`, `ContractAmendment`, `AcceptOutcome`); `InstrumentDef.display` + one `render_amount` at every amount render (`@instrument/register … display=<symbol>:prefix\|suffix`, E2-C16); a `System` party refused structurally (`ContractError::SystemParty`, M4); `[economy].stale_contract_age_secs` + deadline-aware stale paging (DL-08); `pub contract_notice` + the hiding/parse helpers; the **DL-04 `OrgInbox`** observer with `econ:notice:{obj}` offline docs (≤ 32 per object, ≤ 240 bytes each) delivered at reconnect; the **DL-05 `EscalationOpened`** event fanned out by `StaffAlerts` to connected wizards on the ladder's own schedule, deduped per record step. No new primary verb (the rule-1 allowlist is unchanged). **64 tests** (economy 49 — `e4b1.rs` 9, `e4b2.rs` 35, `e4c.rs` 3, two unit tests in `render.rs`/`module.rs`; server 15 — `e6a_inbox.rs` 9, `e6a_alerts.rs` 6), plus three E4b2 pins re-pinned to A′. Includes the **verify fold** (08f §2.1, 29 `vf_<lens><n>_…` tests): the window is a discretionary-spend control an obligation period is exempt from (D-E6-21 amended), every amend refusal lands before the refund posts and the re-offer is non-fatal (`AmendReoffer`), `install_commit` pins its deferred batch, the org inbox is gated by the record's own view predicate and bounded at 32 holders, the rolling-window index tracks only windowed accounts and survives a clock jump, and the ladder's pages reach the first staffer to come on duty |

| **E6a** | `mush-party` (new) + `ruleset-shadowrun::economy` + `mush-economy` (`cancel_contract`) + `mush-config`/`mush-server` | *(this commit)* | the Shadowrun run-and-pay layer (08f §3): **`mush-party`** — `Party`/`Invitation`/`PartyStatus` with a `TryFrom` wire, `Forming → Locked{contract} → Disbanded`, caps 8/16, a bounded idle-expiry tick, `+party/*`, and the three authority-free seams (`lock`/`release`/`unlock`) the ruleset drives it through; its dependency list is `{core, serde, smol_str, tracing}` — **not a principal, never learns about money** (D-E6-5). **`ruleset-shadowrun::economy`** — `[shadowrun.economy]` parsed host-side (`enabled = false` off-config), `sr.nuyen` at scale 0 with `¥` prefix, the faucets `system:sr.chargen`/`system:sr.migration` and the sink `system:sr.lifestyle`, `sr.month` as a **declared** cadence the host contributes to `[economy.cadences]` (`@sr/clock` is show-only), lifestyle upkeep through `create_obligation` with the miss/downgrade rule driven by `missed_total` **deltas** (one notice on the first miss, a one-tier replacement on the second, `maximum_catch_up` 1, `+lifestyle/set` refused in arrears), the bounded run board over `offer_contract`/`accept_contract`/`amend_contract`/`cancel_contract` (escrow at publication, per-principal and board caps, `Public`/`Faction`/`Invitation` visibility through `mush_faction`, an outcome table whose every row sums exactly with named payout shares, the A′ crewing amendment that rewrites every row to the roster's **own** accounts, `on_expiry = Dispute`), the chargen allotment and the idempotent `@sr/migrate dry\|commit` from `system:sr.migration`, `@sheet/set …/nuyen` frozen after approval, and `+run`/`+lifestyle`/`+nuyen`/`@sr`. One economy addition: **`cancel_contract`**, the fourth programmatic seam, admitting A′'s other face (a live contract with no deliverables nobody but its author accepted). Wiring: `mush-party` in `server_modules_with`, the SR board observer in `with_observers` when Shadowrun is enabled, `ruleset_modules(.., sr)`, `[shadowrun]` in `mush-config`. Five new verbs pinned in `legacy-handlers.toml`. **118 tests** — `mush-party` 51, `ruleset-shadowrun/tests/e6a.rs` 41, `mush-economy/tests/e4b2.rs` +4 (the cancel seam), plus the in-crate wire/config/store units. Includes the **verify fold** (08f §3.7, 27 findings over three lenses, 32 `vf_<lens><n>_…` tests): a crewless listing's outcome table pays only the payer, so an adjudicator cannot settle an uncrewed run into the fixer's account (M1); `+run/crew accept` re-checks the live-run rule and takes the party lock **before** the amendment, and unlocks on a refusal (M2); a per-record `missed_seen` high-water mark keeps the lifestyle ladder walking after a downgrade (M3); `+run/withdraw` refuses once a runner has bound themselves (M5); the upkeep replacement opens before the old record is cancelled (M6); boot releases a party whose listing closed **or** whose listing was skipped, resolving the opaque `ContractRef` from the ruleset side (R1/R2); `@sr/migrate` asks the idempotency index rather than the profile (R3); a closed account is re-resolved (R4); closed listings are deleted and `+run/list` is bounded on records **visited** with an `after=` cursor (D1); and the account/faction/org/fixer/`+nuyen` refusals are one message each (D3–D5, D11, D12). Two game-neutral economy additions: `idempotency_resolved` and `EconomySnapshot::accounts_of`, plus `MAX_OUTCOME_NAME_BYTES` and `resolve_org_visible` |
| **E6b** | `mush-scenes` + `mush-canon` + `mush-economy` + `ruleset-shadowrun::economy` + `mush-config`/`mush-server` | *(this commit)* | outcomes, fences, custody (08f §4): **the seal** — `SceneRecord` gains `sealer`/`seal_run`/`outcome`/`ic_floor` (all `#[serde(default)]`, invariants in `validate_clock`, `SceneOutcome` behind a `TryFrom` wire), `@scene/facilitate <scene>=<who> [run=<id>]` confers it (staff, and only onto a **player** — 08i rule 2, asked again at seal), `@scene/icfloor` raises a **monotonic, bounded** IC floor that never writes `clock_generation`, and `+scene/outcome` writes a *second* `CanonDraft` beside `emit_scene_concluded` with `kind: scene.outcome`, `source: run:{contract}#{row}`, `entities = subjects ∪ {sealed_by}` named at seal time, and `ic_time` = the floor raised to the world IC clock (OPEN 3); `mush-server` registers exactly that one kind and **never** `scene.concluded`, and the economy still checks **kind + source + party intersection and nothing else**, because `CanonRecorded` carries no `detail`. **Four settlement routes** — `+run/post route=<row>:attest\|canon:<kind>:<source>\|before:<unix>\|after:<row>:<secs>\|fact:<p>:<k>:<id>` and `gate=…`, with `run:self` naming the id the offer is about to mint, `ContractSpec.deterministic` and the permitted-evidence list derived from the routes, `notice.kinds` no longer the constant `["attest"]`, and a new `AdjudicatorShape::Impersonal` so a run nobody's word settles is still crewable. **The fence** — `sr:fence:{id}` / `sr:quote:{id}` on `store.rs`'s pattern with total loads and key-derived allocator floors, `+fence/list\|show\|quote\|accept\|deliver` and staff `@sr/fence`, a price computed from **three declared inputs only** (the source-controlled `[shadowrun.economy.fence_baseline]`, the permille settlement rate, the fence's declared demand) so a hidden fact contributes nothing, escrow strictly before custody (`+fence/deliver` refused until the deal is in force — D-E6-9) and **`TitleTransfer` never in a fence template** (D-E6-10). **`@asset/stolen`** — `AssetOp::ReportStolen` at ordinal 8 recording `Disputed` **plus** a bounded `StolenReport` with public/restricted/covert visibility, filed by the title owner, an `asset.insure` insurer or staff, with the claimant derived and the report released with its claim. **DL-03** — an org or system depositor's deposit comes home to the backing of a vault it owns, under an authority-shaped exception (`authorize_vault_home`) and never a cause- or key-shaped one, with anything that cannot come back **paged**. **DL-11** — `with_party_caps` (the seventh wiring-frozen table) plus `accept_contract_as`, ids only, no money authority. **DL-09** — a bounded `mush_canon::records_for_source` read seam bridged by `mush-server` (the economy still has no canon edge), read at unfreeze, at activation and once per boot, with the canon tail now carrying `entities` so recovery is no weaker than the live path. **C16** — bounded per-actor refusal telemetry at the existence-hiding renderer, surfaced by `@econ/alerts`. Plus `render_account`/`render_org_listed` as the one masking pair, `deposit_into_vault` and `@asset/title … catalog=` (the first producer `CatalogRef` ever had), and the augmentation record that **payment writes and the sheet never sees** (D-E6-16). Two new verbs pinned in `legacy-handlers.toml` (`+fence`, `@scene`). **67 tests** — `mush-scenes/tests/e6b_seal.rs` 22, `mush-server/tests/e6b_seal.rs` 7, `ruleset-shadowrun/tests/e6b.rs` 27 + 3 in-crate, `mush-economy/tests/e4b2.rs` +5, `mush-canon` +3. Includes the **verify fold** (08f §4.6, 27 findings over three lenses, 19 `vf_<lens><n>_…` tests): a `DepositInto` condition may only name a vault its owner is a party to, or `vault.view` was a permanent wedge on somebody else's vault (M1); the seal names **the run staff bound to this scene**, or any conferred sealer could settle any run by typing its id (M2/R12); a live deal's asset cannot be reported stolen out from under it (M3); the org-actor arm moved above the scope branch so an unscoped org account is never debitable (M4); a canon source names its **row**, so a seal selects the outcome it names instead of every canon row at once (M5); a covert theft report no longer lapses the holder's quote, and no longer shows up in an encumbrance count (D1, D2); releasing a `Disputed` claim clears the report with it (R1); a retconned record is never handed back as evidence (R2); a v3 identity migration stamps the namespace so a rollback refuses rather than erasing a terminal seal (R3); and the IC floor is bounded ahead of the world and refused after the seal (R4). Plus the **independent verify fold** (08f §4.6, 28 further findings over the same three lenses, 24 `vf2_<lens><n>_…` tests, 2 doc-only): **one object is one deposit deliverable at a time**, refused at `+fence/quote` and again in `check_offerable`/`activate`, or eight quotes on one item were eight escrows off one delivery (M1); a partly routed run must route every row or name an adjudicator, or the payer settled the unrouted row on their own word (M2); a `canon:` **gate** is refused, because a gate is not a row and could never match (M3); `@scene/facilitate` gains a composition-frozen **`SealerGate`** so a run's own payee cannot hold the seal that pays them, asked at conferral and again at seal (M4); `+fence/accept` re-asserts custody before the fence escrows (M5); `stranded_deposits` is the exact complement of `returnable` (M6); **`@asset/catalog`** (`AssetOp::SetCatalog`, ordinal 9) so a pre-E6b title can be catalogued at all and a re-catalogue lapses its quote (M7); re-verification **replaces** the condition's set with the adjudicator's like the two writing paths do (M8); a fence's vault cannot move under a live deal (M9); the canon route allowlist is inverted from a denylist, so a listing can cite only **its own** run and rows (D1); `@asset/release`'s index stops being an oracle for a covert theft report (D2); the chronicle line no longer names the contract to a non-party subject (D3); lapsed quotes are swept and stop wedging their seller (D4); the whole-store quote ceiling is enforced on write, and the loader keeps contract-bound quotes (D5); a canon `kind`/`source` is length-capped in the economy (D6); hidden holds collapse to one line like hidden accounts (D7); `EconomicAssetWire` carries the write path's caps and boot skips a bad asset doc loudly rather than failing (D8); `+scene` is re-declared `legacy-post-image` and `PINNED_BASELINE` drops to 158 (D9); `@asset/list` is bounded on records visited with an `after=` cursor (D10); the canon re-scan and `owner_home` **filter, then take**, or DL-03 and DL-09 both failed silently on a large world (R1, R2); the read seam also reads the last sealed ledger chunks and pages when a cited source has no record anywhere (R3); a **generation stamp** stops a pre-amendment seal re-attesting the amended surface (R4); the re-entrancy latch covers the tail walk (R5); a corrupt persisted outcome no longer bricks the boot (R6); and the open tail is decoded once per lookup (R7) |
| **E6c** | `mush-market` (new) + `mush-economy` (`ContractSpec.deliverables`, `deliver_settled`, `issue_title(catalog)`, E3-C36) + `mush-org` (DL-06) + `ruleset-shadowrun::economy` + `mush-server` | *(this commit)* | the neutral market (08f §5): **the gate first** — `MarketStore::projection(catalog, market, snapshot, hidden_orgs, now)` takes **no viewer argument at all**, which is the proof: `band`/`depth` fold only public listings of non-hidden selling organizations, `last_trade`/`volume` fold only **settled journal transactions** (a trade row is re-checked against the economy's own `ContractState::Settled { transaction }` before it counts), depth is bucketed into `DEPTH_BUCKETS` 8 price levels **and** floored to `DEPTH_QUANTITY_STEP` 10 so one seller's quantity is never republished, and the whole per-viewer surface is `+market/list`/`show` through the existence-hiding renderer — `No such listing.` identical for nonexistent and unviewable, an unusable filter rendering an **empty list**, a hidden vendor's listings **absent** rather than refused, and the walk bounded on records **visited** (`MARKET_PAGE_VISIT` 256, `MARKET_PAGE_MAX` 256 rows). **`mush-market`** — `→ {core, org, economy}` and nothing else (answer 20; the economy gains no edge back), five families (`market:catalog:{pack}:{code}` · `listing` · `quote` · `vendor` · `source`) on `ruleset-shadowrun::economy::store`'s pattern with `TryFrom` wires, re-derived indexes, key-derived allocator floors and a **total** load; `CatalogEntry` + `Representation::{Instrument, Lot, Asset, Entitlement, Installed}` with `MAX_CATALOG_ENTRIES_PER_PACK` 512 and a pack that validates whole before it writes (a bad entry fails the **pack**, names the file, and the server boots); `Listing`/`Supply::{Stock, Market, Procurement}`; **buyer-as-issuer** quotes (`offer_contract` with the buyer as actor → the buyer's own `accept_contract` places the escrow → the vendor accepts through E6b's `with_party_caps`, which grants no money authority → activation, **the single point where `quantity` decrements**), `Deliverable::TitleTransfer` moving title inside E4's own settlement; **a quote reserves nothing** (OPEN 4 — no reservation family, no new `Hold` field) and every stale reason (lapse, re-price, re-version, sold out, halted maker, retired vendor) renders **one** deterministic string charging nothing (D-E6-13); NPC makers with `spread`/`volume`/`loss`/`window` budgets asked **before** each quote is issued, funded only by capitalization, sales or a `create_system_obligation` drip under the `[economy]` mint cap, and never a `System` party (D-E6-4, doc 10 7A). **Three game-neutral economy additions** — `ContractSpec.deliverables` (a programmatic offer could move money and never goods), `deliver_settled` bounded exactly as `accept_contract_as` is (a contract holds one instrument, so a commodity or entitlement leg is a second post the seller's own authority makes), and `issue_title(… catalog)` so a stocked asset is catalogued **at birth** and E6b's fence can price what the market sold — plus **E3-C36**: a `Title` re-issue over a **frozen** written-off record refuses `AssetFrozen` (08g §3 #2). **DL-06's minimal slice** — `OrgMutationOp::{SetOrgStatus, PutRole}`, the revision bump, `@org/status … [hidden=yes|no]`, `Closed` terminal — which is `Organization.hidden`'s **first write path** and what finally gives E6a's D11 an end-to-end test. **Shadowrun** registers into the market's three wiring-frozen tables (`with_catalog_sources`/`with_availability`/`with_delivery`): the source-controlled `sr.core` pack (27 entries, every shipped representation, both availability paths, **no `Lot` entry**), an availability test rolled through the **non-rendering** `mush_dice::sr5::{Sr5Pool, Sr5Opts}` seam and **recorded on the quote**, and delivery of a commodity line, an entitlement line and the `InstalledAugmentation` — the first production caller `SrProfile::install` has ever had (E6b's M11). `[shadowrun.economy].catalog_paths` stops being inert, and `CatalogEntry.baseline` supersedes the `fence_baseline` table, which survives only as an override. `sr.market` joins `with_title_issuers`. Two new verbs pinned in `legacy-handlers.toml` (`+market`, `@market`); `PINNED_BASELINE` untouched at 158. **57 tests** — `ruleset-shadowrun/tests/e6c.rs` 31, `mush-org/tests/e6c_lifecycle.rs` 6, `mush-economy/tests/e4c.rs` +3, 15 in-crate in `mush-market`, 2 in `ruleset-shadowrun`'s `catalog.rs`. **Independent verify fold (2026-09-05, 08f §5.6)** — a second, independent adversarial verify over D/R/M folded **28 confirmed findings** and corrected **three claims §5.6 makes that are false**, in place. **The gate itself leaked**: `hidden_seller_orgs` composed `hidden_orgs` from the **vendor table** while `load` is total, so one skipped `market:vendor:{id}` put a hidden organization's price and quantity into the public `band`/`depth` that `+market/list` still hid — derived from the records the projection folds now (D1); `+market/quote`'s named budget refusals were a free solvency oracle on an org wallet `@account/show` hides, collapsed to the one `STALE_QUOTE` string (D2); an empty page **discarded its cursor**, so a wall of 260 unviewable listings made every older public one unreachable (D3); a maker's `spread` moved the quote and not the band, so the safe projection published a band that need not contain its own last trade — `price_for` is the single price every surface reads (D4); `+market/depth` deep-copied the **whole economy snapshot** on every player call (D5); `volume`/`last_trade` published one trade's exact quantity that `DEPTH_QUANTITY_STEP` exists to suppress (D6); the venue default still let a **lapsed** listing choose it (D7); `default_market` walked the whole listing and quote maps rather than the pair index (D8); `@market/catalog load` took an unvalidated **path** as the pack name (D9). **Recovery failed silently**: `mush-market` had **no boot reconcile at all**, and the economy heals a lost terminal transition with a raw doc write — so a settlement healed at boot never reached `MarketObserver` (goods undelivered though the money moved, the trade row lost, the buyer's quote slot wedged for ever, R1) and a healed cancellation never returned the stock or the maker's window (R2); nothing ever deleted a **lapsed listing** or an aged **tape row**, so 64 lapses wedged a vendor for ever and 64 traded pairs filled `MAX_QUOTES` and refused every new quote for ever (R3, R4 — new `mush-market/src/boot.rs`, a `Coarse(60 s)` sweep, `MAX_TRADE_ROWS` 1 024); a dropped pack entry left listings publishing depth in a good nothing can define (R6); `retire_vendor` truncated at 64 and no per-vendor or per-pair cap was re-run on load (R8, R10 — the trade ring now evicts by `Settlement.at`); the quote→contract link was written after the last acceptance and the `Settled` arm was not idempotent (R9). **Money and consent**: a purchase whose goods leg cannot be delivered took the buyer's money by **default**, not as a residual, and paged nobody — `DeliveryProvider::can_deliver` at accept plus `open_escalation` at settlement (M1); a maker's budgets bounded **quoting, not selling**, so N quotes behind one maker all passed and all charged (M2); a **lapsed listing still sold** (M3); **DL-06 was inert** — no file in `mush-market/src` mentioned `OrgStatus`, so a suspended, dissolving or closed organization kept trading and could re-register (M4, `api::org_may_trade`); the loss budget was spent and returned at the catalogue's **live** baseline, so a pack reload between the legs minted or destroyed budget (M5, `Quote.booked_loss`); a baseline-less entry made the loss budget **inert** (M6); a re-spread maker honoured its old quotes (M7, `ObservedRevisions.source`); a listing could name **another organization's** maker and spend its budget (M8); `+market/sell` pinned a vendor's cash in N escrows against one object (M9); `@market/vendor stock` **minted title over any live object**, a player character included (M10); and the public tape had no **self-dealing** test (M12, `Quote.self_dealt`). **92 tests** — `e6c.rs` 31 → **55**, `e6c_lifecycle.rs` 6 → **7**, `mush-market` 15 → **22** (new `tests/projection_bounds.rs` and `tests/projection_cost.rs`) |
| **E7 cut 1** | `ruleset-tacsim` (new) + `mush-sim` (`RouteDef`) + `mush-config`/`mush-server`/`mushd` + `mush-contract-tests` | *(this commit)* | the TacSim pack loader, the seed latch and the corridor fixture (10b §3.1): **the digest first, and it is not a new one** — `pack_digest` is a thin wrapper over `mush_core::DefinitionDigest::for_canonical_value_with_limit` under the domain tag `tacsim.pack.v1`, so cut 1 defines no second hash and no second canonical encoder (§9 q3 closed by the tree). One function, `RulesPack::canonical_value`, builds what is hashed — the manifest **without its own `digest` field** plus every entry — so a self-referential hash is unrepresentable, formatting is not content (a reordered table is one digest; whitespace **inside a value** is another), and a declaration computed over any other bytes is refused **at load**, naming both digests. **`ruleset-tacsim`** — `→ {mush-core, mush-sim}` and nothing else (no economy, org, party, faction, market or tactics edge); `PackKind::{RulesDefinition, CampaignSeed}` with Q6A's distinction enforced structurally (a rules pack instantiates nothing, a seed pack defines no rules); strict `deny_unknown_fields` wires at every level with `MAX_PACK_ENTRIES` 512, `MAX_SEED_OBJECTS` 2 048, `MAX_PACK_BYTES` 512 KiB, `MAX_REQUIRES` 16 all **re-run on load**; four document families (`tacsim:pack:{id}@{version}` · `tacsim:seed:{pack}@{version}` · `tacsim:pin:{subject}` · `tacsim:meta`) on `ruleset-shadowrun::economy::store`'s pattern with a **total** loader whose floors are derived from every **parsed key**, loaded or not (E6b's R8) — which is what makes a corrupt seed latch still bar a re-seed, the only fail-closed direction available. **The seed** — `@tacsim/seed <pack> [dry|commit]`: `dry` writes nothing and lists every planned object, `commit` applies once, a second `commit` is a **HIT** writing nothing, a *different* version is refused **naming the applied one** through a re-derived `pack → applied version` index, and the objects and the latch ride **one `WriteBatch`** (`World::create` marks dirty, `Ctx::persist` queues a `ModuleOp`, `flush_dirty` folds both), so a reboot sees both or neither. **`RouteDef` closes G1 — in `mush-sim`** (OPEN 2): §3.1 put the type in the ruleset and §3.6 made `mush-sim` its consumer, which is a cycle, so the **type** lands beside `Leg` with `MAX_LEGS` 32 and a pure `route_duration(def, drive, payload)`, adding **no crate edge** and changing no existing type, while the **data** is pack-authored. **The corridor fixture** — `tacsim.core@1` (the ten commodity family ids of §9 q10, `metals` and `structural_alloys` in use, one recipe `alloy_smelt`) and `tacsim.corridor@1` (a Ceres extraction site, a transfer node, a Phobos yard, one route joining Ceres to Phobos, `requires tacsim.core@1`), 15 entries against a 64 cap, and **no quantity of anything** (§9 q4: there is no interim). **Wiring, as the owner overrode OPEN 3** — TacSim is a **layer**, not a `[rulesets].enabled` id (doc 10 §20): a `GameModule` with no `SheetSchema`, switched by `[tacsim].enabled`, composed **after** the one character ruleset in `mushd`, with `ruleset_modules` and its single-ruleset guard untouched and the unknown-id fixture moved off `"tacsim"`. `[tacsim]`/`[tacsim.time]` are parsed host-side (`enabled = false` off-config); `enabled`/`pack_paths`/`seed_paths`/`corridor` are live and the pulse/domain keys and all of `[tacsim.time]` are **parsed and inert**, with a pinned test proving `[tacsim.time]` reaches `load_or_seed_world_timeline` **never**. The **host** reads the pack bytes so a malformed file refuses the boot **naming the file**, while a corrupt stored document is skipped loudly and the world boots — the two opposite answers each have their own test. One new verb pinned in `legacy-handlers.toml` (`@tacsim`, `legacy-post-image`, family `tacsim.pack`); `PINNED_BASELINE` untouched at 158; K8.5's `classify_commands(LegacyPostImage)` declared before the first `reg.command` and asserted by name in the K8.6 gate. **32 tests** — `ruleset-tacsim/tests/e7c1.rs` 23, `mush-sim` 4 in-crate, `mush-server` 5 (2 `config_ext`, 1 `timeline`, 2 composition pins). Cut 1 registers **no** `FactProvider`, canon outcome kind, authority-guard validator, clock provider, schedule source or typed state family, touches `mush-core`'s resources not at all, and has no LLM path. **Independent verify fold (2026-09-05, 10b §3.1's fold block): 19 confirmed items across P/S/B, four false claims in the tree corrected in place, B7 refuted.** The three HIGHs: the published-version ledger's digest was written and read by nothing, so an in-place re-signed pack was adopted silently and `@tacsim/pack verify` was a tautology — `boot_reconcile` now **refuses the boot naming the file and both digests** (owner decision) and `verify` compares against the ledger row (**P1**, covering S1); `TacsimStore::resolve`'s retired-seed check was scoped to the *pack* rather than the *subject*, so retiring one applied seed pack refused **every** subject of **every** loaded pack, misattributed (**P2**, covering S2, and deviation (e)'s scope sentence corrected); and `@tacsim/pack verify` was unpaged and open to any player, re-hashing up to 32 MiB on the world's writer per invocation — now staff-only, paged on records visited, with a named pack a direct lookup (**B1**). Then: magnitude bounds on the pack wire (`MAX_RECIPE_UNITS`, `MAX_ROUTE_PROPELLANT`, `MAX_LEG_SECS`; zero propellant explicitly legal — **P3**); boot pages and skips a route whose placed object the world no longer has, instead of materializing it over a tombstone (**S3**); routes keyed `(pack, version, id)` so a later-sorting pack cannot shadow the corridor's `ceres_phobos` (**S4**); `@tacsim/pack list`'s page marker moved behind the visibility filter, so it is no longer a pack-existence oracle (**B2**, and the Disclosure block's "at every switch" corrected); `@tacsim/pack show` paged with a cursor argument instead of 515 messages (**B3**); a pin must name a **declared subject**, `MAX_PINS` 128, and `@tacsim/pin list` plus `@tacsim/pin <subject>=` give the family its first listing and its first `ModuleWrite::Del` (**B4**, covering P4); `MAX_SEED_OBJECTS` **2 048 → 256** so it sits below `MAX_PACK_ENTRIES` and can actually fire (**P5**); a duplicate entry id refuses instead of collapsing last-write-wins (**P6**); `nominal_drive`'s capacity raised to 4 000 so the corridor is flyable carrying its own declared propellant, with propellant ruled **inside** payload for cut 6 — no digest moved (**P7**); `PackRequirement` gains an optional content `digest` and `verify_sources` refuses a `requires` cycle (**P8**, and the optional field is hashed only when present, so no published digest moved); the seed idempotency guard moved **into** `apply` (**P9**); `put_seed`'s drop-the-latch arm made loud with a minimal fallback and its unreachability pinned (**S5**); `commit` gates on the durable ack (**S7**); the configured-pack cap applied **before** any file is read (**B5**, covering S6); `deny_unknown_fields` on `[tacsim]`/`[tacsim.time]`, the one section whose keys carry a boot precondition (**B6**); and `RulesPack::new`'s `# Errors` route/kind rule corrected in place with `(RulesDefinition, Route)` added to `misplaced` (**B8**). **B7** (a per-entry visibility axis) is refuted: adding one moves no published digest, so it lands in cut 3 at no cost. **+20 tests** (17 in `e7c1.rs`, 1 each in `seed.rs`/`store.rs`, 1 in `config_ext`, 1 in `mush-config`), one rewritten; `PINNED_BASELINE` still **158** — the new pin paths are arguments to an existing switch, so the `@tacsim` stanza's `note` is refreshed, not joined. |

Test tally after E3 (post-fold): **mush-org 46**, **mush-economy 157**, **mush-sim 72**,
**mush-tactics 52**, **mush-faction 56**, **mush-server 62**; `cargo test --workspace --no-fail-fast` runs 2012 passing. The one
failing workspace test, `mushd::web_e2e::spa_and_healthz_are_served`, predates E3 (it needs a
`web/dist` build). `cargo fmt` is **not** applied anywhere in E3: the tree had 1334 rustfmt hunks at
`d7f6707`, so CI's fmt step was already red before E3.

Test tally after E4 (each cut post-fold): **mush-economy 403** — `e4a.rs` 83, `e4b1.rs` 61,
`e4b2.rs` 59, `e4c.rs` 35, plus the E2/E3 files and the in-crate unit tests — **mush-jobs 27**
(`e4c_facts.rs` 3), **mush-server 118** (`e4c_bridge.rs` 11), **mush-org 46**.
The E4c verify fold added **25**: 17 in `e4c.rs`, 7 in `e4c_bridge.rs`, 1 in `e4c_facts.rs`, all
named `vf_<lens><n>_…`.
`cargo test --workspace` runs **2497 passing, 0 failing** — the E3-era
`mushd::web_e2e` failure is gone (it is `spa_if_built_and_healthz_are_served` now and tolerates an
unbuilt `web/dist`). Both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) are green in every
cut. `cargo fmt` stays off, unchanged.

Test tally after E5 (each cut post-fold): **mush-housing 121** — `e5a.rs` 54, `e5b.rs` 66, 1
in-crate — and **ruleset-wod +60** in `e5c.rs` (40, plus 20 in the E5c fold).
`cargo test -p ruleset-wod -p mush-housing -p mush-economy -p mush-server` runs **923 passing**;
`cargo test --workspace` runs **2735 passing across 184 suites, 0 failing**; `cargo clippy
--workspace --all-targets -- -D warnings` is clean. `cargo fmt` stays off, unchanged.

Test tally after the E1–E5 look-back fold: **mush-housing 130** — `e5a.rs` 54, `e5b.rs` 75, 1
in-crate — **ruleset-wod 270** (`e5c.rs` 65), **mush-economy 408** (`e4b1.rs` 66), **mush-jobs 31**
(`jobs.rs` 24), **mush-server 155 (+2 ignored)**. `cargo test -p mush-housing -p ruleset-wod -p mush-economy
-p mush-jobs -p mush-server` runs **994 passing, 0 failing**; `cargo test --workspace --no-fail-fast`
runs **2779 passing across 184 suites, 0 failing**; `cargo clippy --workspace --all-targets -- -D
warnings` is clean. rustfmt: 2,467 hunks in 88 files at `5409c73` (reformat after K6, 08h §3); CI's **Done:** the whole-tree reformat landed as `91208ab` on the converged main (rustfmt.toml, CI Format blocking; gates identical 2924/0/48).
Format step is advisory until then.


Test tally after E6a step 1 **and its verify fold**: **mush-economy 464** — `e4b1.rs` 82,
`e4b2.rs` 94, `e4c.rs` 38, 18 in-crate — **mush-housing 136**, **mush-server 181 (+2 ignored)** —
`e6a_inbox.rs` 9, `e6a_alerts.rs` 6 — **mush-contract-tests 36** (the rule-1 allowlist unchanged: no
primary verb was added). `cargo test -p mush-economy -p mush-housing -p mush-server
-p mush-contract-tests` runs **817 passing, 0 failing, 2 ignored**; `cargo test --workspace
--no-fail-fast` runs **2999 passing across 191 suites, 0 failing, 48 ignored**; `cargo clippy --workspace --all-targets
-- -D warnings` and `cargo fmt --all --check` are clean.

Test tally after **E6c**: **mush-market 15** (in-crate: `ids.rs` 2, `record.rs` 4, `store.rs` 5,
`projection.rs` 4), **mush-org 52** (`e1a.rs` 23, `e1b.rs` 20, `e6c_lifecycle.rs` 6, 3 in-crate),
**mush-economy 485** (`e4c.rs` 41, the three `e6c_b4_…` tests included), **mush-server 195
(+2 ignored)** (the module-order pins now name `market` between `party` and `script`),
**ruleset-shadowrun 200** (`e6c.rs` 31, `e6b.rs` 36, `e6a.rs` 68, 65 in-crate),
**mush-contract-tests 36** (`+market` and `@market` pinned `legacy-post-image`; `PINNED_BASELINE`
**untouched at 158**), **mush-config 44**. `cargo test -p mush-market -p mush-economy -p mush-org
-p mush-server -p ruleset-shadowrun -p mush-contract-tests -p mush-config` runs **1 053 passing,
0 failing, 3 ignored**; `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` are clean, and both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`)
are green — E6c adds **no `AssetOp` variant at all**, so no ordinal moved.
`cargo test --workspace --no-fail-fast` runs **3 286 passing across 202 suites, 0 failing,
50 ignored**.

Test tally after **E6c's independent verify fold**: **mush-market 22** (19 in-crate — `ids.rs` 2,
`record.rs` 4, `store.rs` 8, `projection.rs` 5 — plus `tests/projection_bounds.rs` 2 and
`tests/projection_cost.rs` 1), **mush-org 53** (`e6c_lifecycle.rs` 7), **mush-economy 485**,
**mush-server 195 (+2 ignored)**, **ruleset-shadowrun 250** (`e6c.rs` 55, `e6b.rs` 36, `e6a.rs` 68,
`shadowrun.rs` 25, `persistence.rs` 1, 65 in-crate — the row above's 200 omitted the two older
suites), **mush-contract-tests 36** (the fold adds **no primary verb**, so `PINNED_BASELINE` stays
**untouched at 158**), **mush-config 44**. The same seven-crate gate runs **1 085 passing, 0 failing,
3 ignored**; `cargo test --workspace --no-fail-fast` runs **3 318 passing across 204 suites,
0 failing, 50 ignored**; `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` are clean, and both digest pins stay green — the fold adds no `AssetOp`
variant either.

Test tally after **E6b, its verify fold and the INDEPENDENT verify fold**: **mush-scenes 60**
(`tests/e6b_seal.rs` 24, `tests/timestop.rs` 30, 6 in-crate), **mush-canon 12**,
**mush-economy 482**, **mush-server 195 (+2 ignored)** (`tests/e6b_seal.rs` 14),
**ruleset-shadowrun 193** (`e6b.rs` 36, `e6a.rs` 68, 89 in-crate), **mush-contract-tests 36**
(`+scene` re-declared `legacy-post-image`, `PINNED_BASELINE` 159 → **158**), **mush-config 44**.
`cargo test -p mush-scenes -p mush-canon -p mush-economy -p mush-server -p ruleset-shadowrun
-p mush-contract-tests -p mush-config` runs **1 022 passing, 0 failing, 2 ignored**;
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` are clean, and
both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) are green — `AssetOp::SetCatalog` takes the
next free ordinal, 9, and `AssetOp` serializes by variant name, so nothing the pins cover moved.

Test tally after **E6a and its verify fold**: **mush-party 53 (+1 ignored)** —
`tests/e6a_party.rs` 24, 29 in-crate — **ruleset-shadowrun 154** (`e6a.rs` 68, 60 in-crate),
**mush-economy 469** (`e4b2.rs` 99, the four `e6a_cancel_contract_…` seam tests and the fold's
outcome-name cap included), **mush-server 181 (+2 ignored)**, **mush-config 44**,
**mush-contract-tests 36** (the rule-1 allowlist is unchanged by the fold: it added no verb and no
mutating switch — `+run/list` gained an `after=` option and nothing else). `cargo test -p mush-party
-p ruleset-shadowrun -p mush-economy -p mush-server -p mush-config -p mush-contract-tests` runs
**937 passing, 0 failing, 3 ignored**; `cargo test --workspace --no-fail-fast` runs **3136 passing
across 195 suites, 0 failing, 49 ignored**; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt
--all --check` are clean.

### The E4 seams a later milestone consumes

Wiring-frozen tables on `EconomyModule`, in the composition root's construction order
(`mush-server/src/modules.rs`; **module order itself is unchanged since E3c**):

- `.with_custody(vec![..])` (E3b1) — `CustodyProvider`s.
- `.with_config(EconomyConfig)` (E4b1) — the `[economy]` mint cap.
- `.with_facts(vec![Arc::new(JobsFactProvider)])` (E4c) — `FactProvider`s answering a contract's
  `TypedCondition::Fact`. `kind` is a namespaced string (`jobs.resolved`; doc 10 §15's
  `sim.*`/`tactics.*` next), `id` is the provider's own stable id, `revision` is passed through, and
  **`None` (unknown) is unsatisfied** everywhere.
- `.with_observers(vec![Arc::new(CanonEvidenceBridge)])` (E4c) — post-commit `EconomicEvent`s with
  **ids and kinds only**. An observer must not re-enter the economy; a settlement's
  `ContractChanged` carries the landed transaction id, which is the publication key.
- `.with_title_issuers(vec![..])` (E4c) — the system principals `mush_economy::issue_title` will
  mint title for. **Empty today**; E5's `sys:wod.housing` is the first entry, and `@asset/title`
  stays the staff path meanwhile. The object must be live and non-`GOING`, and the key carries a
  title **generation**, so a re-issue after a write-off mints rather than reporting `AlreadyPosted`.
- `.with_canon_outcome_kinds(vec![..])` (E4c verify fold) — the canon kinds that count as a
  **producer-declared outcome**. **Empty today, deliberately**: `scene.concluded` is a lifecycle
  record any participant can produce, so a contract gated on one is a contract its payee can settle
  alone. E6b registers `scene.outcome` here once `mush-scenes` can seal one under facilitator
  authority; until then `@contract/offer` refuses every `canon:` attestor set.

Plus: `mush_org::audit(ctx, OrgAction)` records an org-authority act that mutates no org state
(`@econ/approve|deny` is the first caller), and `mush_economy::may_view_account` +
`commands::mask_account` are the one masking rule `@econ/history`, `@contract/show` and
`@hold/show` all render through.

## The live seams a later milestone consumes

- **Authorization (read):** `mush_org::authorize_in_world(&World, &AuthRequest) -> Decision`. Pure
  engine underneath is `mush_org::authorize(&OrgState, &AuthRequest, &dyn PredicateEval)`. Since
  E3b2 it denies `Inactive` for a Retired scope or a non-Active org before any grant/policy work.
- **Org mutation (write):** `mush_org::submit(&mut Ctx, &OrgMutationPlan, OrgAction)`; ops now
  include `RetireScope`. Id minting is `OrgStore::alloc_*`.
- **Org wiring hooks (frozen at server assembly):** `OrgModule::with_predicates(..)` (E1c),
  `.with_capabilities(BTreeSet<CapabilityId>)` (E3b2 — the vocabulary `@org/grant` checks;
  `mush_economy::CAPABILITIES` is the economy's list), `.with_scope_use(..)` (E3b2 —
  `ScopeUseProvider`, implemented by `mush_economy::EconomyScopeUse`, so a bound scope cannot be
  retired).
- **Money + title + custody (write):** `mush_economy::post(&mut Ctx, &EconomyPlan)` — the plan
  carries `requested_lines`, `asset_ops`, `custody_ops`, `expected_revisions`,
  `expected_asset_revisions`; one durable bundle. `reverse(..)` reverses money lines only and
  refuses an effect-carrying original. **`post` is a handler's last fallible call** (a later `Err`
  discards the persists but not the world/store mutation).
- **Pure engine:** `validate(&plan, &snapshot, &ValidateEnv { auth, custody, now })`; the seams are
  `AuthEval` (money + `authorize_asset` / `authorize_title_receipt` / `authorize_vault` /
  `authorize_maintenance` / `authorize_approver`, all default-`false`) and `CustodyEval`.
- **Custody providers:** implement `mush_economy::CustodyProvider` and wire via
  `EconomyModule::with_custody(vec![..])`; the built-in `ContainerProvider` is always last;
  `mush_sim::InteriorCustodyProvider` is the precedent. Exactly one provider resolves an op (the one
  that `handles` the backing), refused if any other `claims` the object; moves ride the
  `CarryAttempt` hook chain. `post` admits **one custody effect per plan** at the module layer.
- **Destruction:** `mush_core::behavior::DestroyGate` registered with `reg.destroy_gate(..)`
  (economy's `EconomicDestroyGate` is the second user after sim's); the `ObjDestroyed` observer is
  the backstop and boot liveness the third leg — only `@destroy` emits the event.
- **Approvals:** `Account.approval: Option<ApprovalRule>`; `post` → `PostOutcome::NeedsApproval(id)`;
  `@econ/approve|deny`; approvers are re-qualified live at consumption; one live request per
  `plan_hash`; the consumed id rides the journal entry (`consumed_approval`).
- **Reads:** `mush_economy::may_view_account(w, actor, AccountId)` is the single view gate (staff,
  owner, `economy.account.view` on the scope); every refusal renders through the existence-hiding
  helpers in `commands.rs`.
- **Faction linkage:** `mush_faction::backing_org(w, &Faction) -> Result<OrgId, LinkError>` is the
  one resolver (`Unbound` / `Missing` / `Conflict`).
- **Durable gates (core):** `gate_on_durable_ack_strict()` for money; the non-strict
  `gate_on_durable_ack()` for the E3 control-doc writes (approval rules, vault/account/asset status).

## Decisions made while building (not all in the original doc 08 draft)

1. **Org creation is staff-only** (E1). Player-created orgs wait for a milestone with an economy
   consumer (neutralizes global-id / name-collision existence oracles). Delegated per-org admin
   (`org.role.manage`) still ships. *(E3: a shared player haven or team stash is therefore a
   staff-minted org; a non-org vault is owner-only + staff.)*
2. **`PrincipalId::Object` is clean** — no embedded generation. ObjIds are never reused, so the
   "reused id" threat is false; a destroyed object is scrubbed, not shadowed by a generation guard.
3. **`ScopeSelector` is a closed enum** — `Exact` / `SubtreeFrozen{snapshot_revision}` / `SubtreeLive`.
   Only `SubtreeLive` widens to later children (explicit); depth is capped at creation (≤8), so a
   containment walk terminates and an over-deep check fails closed. *(E3b2: scopes per org are
   capped at 64; `@org/scope create` stamps `created_rev = revision + 1` because
   `Organization.revision` is otherwise inert — a proper revision-bump op is a follow-up.)*
4. **Delegation fully re-resolves its live source each call** (never a captured `source_revision`),
   and the grantor must actually hold the source grant (both enforced at construction and re-checked
   live). Delegation is narrow-only.
5. **Faction is NOT fully migrated onto org (E1c scope cut).** E1c shipped only the *predicate
   provider*; the faction level/officer/intel authorization model stays faction-side (R-D1).
   Membership-mirroring into `OrgMembership` was **dropped**. *(E3d closed the deferral: the
   Faction→backing-`OrgId` mapping is `Faction.org`, and the "non-destructive membership adapter"
   is **struck** — see 20.)*
6. ~~The org capability vocabulary is a wiring hook, not yet enforced.~~ **Enforced since E3b2**:
   `OrgModule::with_capabilities` freezes the vocabulary at assembly (empty = accept all);
   `@org/grant` refuses an unknown capability and `limit=` on a non-amount capability.
7. **The journal is the sole economic authority.** Balances, the idempotency index, the reversed set,
   the txid index, and the allocators are **re-derived from the journal at boot** (R-B2) — none is a
   separately-persisted boot authority, so a companion-doc drop can never double-post. *(E3: asset
   title/assignment/encumbrance/lifecycle are journal-authority too — every change is an
   `AssetEffect` carrying the full post-state inside a journal transaction, and `asset:{obj}` docs are
   projections with the Account heal/freeze rules.)*
8. **Boot reconciliation is directional** (R-B1): a projection merely *lagging* the journal silently
   re-derives to the head; only real *drift*, an *ahead* projection, an *unreplayable* journal, or a
   per-instrument *conservation break* freezes / fails. Whole-economy freeze is the E2 granularity.
   *(E3: asset drift/ahead also latches the whole-economy freeze; a missing asset doc rebuilds
   **frozen**; an unjournaled asset doc is not loaded. Effect-only entries contribute nothing to
   conservation. **The whole-economy latch is now durable** — it rides `meta` and survives the boot
   that heals the drifted docs; `@econ/unfreeze` clears it. Before the E3 fold it was lost at the
   next boot, an E2-era gap.)*
9. **`Amount` forbids operator arithmetic** — no `Add`/`Sub`/`Neg`, only `checked_*`; serde-transparent
   `i64`; the client wire form is a decimal string (never the raw i64). `MAX_SCALE` caps render.
10. **Reversal is engine-only.** A plan carries no caller-settable `reverses` field; a reversal is a
    dedicated path fed a real transaction. Deterministic `econ.reverse/{id}` key + `is_reversed` guard
    make it idempotent + at-most-once. *(E3: a reversal reverses money lines only, never evaluates an
    `ApprovalRule` (an approval rule must not be a clawback shield), and **refuses** an original that
    carried asset/custody effects — title is restored by an attributed `@asset/transfer`.)*
11. **Strict durable gate for money** (invariant 9 / R-R1): the *posted* confirmation, AND an
    idempotency-HIT (`AlreadyPosted`) confirmation (R-R2), are emitted only after the durable ack; a
    dropped write suppresses them. *(E3: a `NeedsApproval` outcome and every control-doc write ride
    the non-strict gate.)*
12. **Durability checkpoint deferred** (R-D1): full-journal-replay-at-boot is the baseline; a
    persisted checkpoint lands when journal size makes boot cost matter. Cold-chunk *play-time*
    paging is deferred.
13. ~~No non-staff economic read in E2.~~ **A bounded non-staff read since E3b2** (D-E3-6): owners and
    `*.view` holders may `show`/`list` accounts, assets, vaults with a fixed field set; **every**
    refusal renders through an existence-hiding helper that collapses against *every* resource the
    error names ("No such account." for nonexistent, unviewable, and unviewable-but-refused alike);
    unauthorized list filters return empty; lists cap at 20. The journal stays staff-only; the
    disclosure *filter* is still E4.
14. **Holds / contracts / `SpendingProfile` are not built.** `Account.held_amount` is a projection
    fixed at zero (holds are E4); abstract WoD Resources are the E5 slice. Assets and vaults ARE
    built (E3).
15. **The `frozen` latch is doc-authority, outside every journal effect** (D-E3-1). A backstop
    freeze, `@asset/unfreeze`, vault/account status, and an approval rule are doc writes that touch
    **no journal-authority field, `revision` included** — bumping `revision` there reads as drift at
    the next boot (caught in test). Corrupt asset/vault docs fail the boot; a corrupt approval doc
    is skipped loudly but its id still floors the allocator.
16. **Approvals bind to `plan_hash`, live on the Account, and are re-qualified live** (D-E3-2).
    `ApprovalRule { threshold, policy }` per account; one live `ApprovalRequest` per hash (a second
    identical submission serves the incumbent); recorded approvers must still qualify at consumption
    (`economy.account.approve` on the scope + the role for `Threshold.from_role`; set membership,
    necessary and sufficient, for `NamedSet`; the approver must be live); the requester never
    counts, so `Threshold{1}` means one other person; `OneAuthorizedActor` is refused at set time
    and fails closed if stored; requests expire (24 h), are capped per requester (4) and per account
    (16), and GC at 7×TTL. An approval **reserves nothing** (holds are E4) and authorizes an
    intent, not a state (expected revisions are outside the hash). There is no asset-value
    escalation — two-party consent (E4) is that control.
17. **Title needs authority on both sides** (D-E3-12): source (owner / `asset.title.transfer` on the
    asset's scope / staff) **and** destination (an org → the capability on `to_scope`; another
    character → staff only). `Title` (create) and `Encumber` are staff-only in E3; `Release(Lien)` is
    open to its creditor. `TransferTitle` clears assignment and maintenance. `SetMaintenance` needs
    debit authority on the named account, which must be the owner's or in the asset's scope.
    Scope/owner coherence (invariant 18) ties every scoped resource to the org that owns it.
    **Authorization is asked about each account's gross debit, never its net** (a net-zero pair on
    one account was a free journal write — the fold's H1).
18. **Custody is a provider seam; one provider resolves a whole op** (D-E3-3). `CustodyProvider`
    frozen at assembly; the container provider runs `can_move_to` + the depth cap + the
    `CarryAttempt` hook chain + one `move_to`; the interior provider requires `sim.participant`,
    pre-validates the destination, and withdraws as `Unplace{keep:true}` + one `move_to` (the
    two-move form could strand the object with a false recovery-log line), running every carry hook
    except sim's own. Custody standing = carried (exact `Obj.location == actor`) or co-located as
    owner/assignee/controller/staff; **exits and actors are never custody** (an exit is filed in
    `exits`, invisible to every custody query); a vault's own backing cannot be deposited; a
    withdrawal lands only on the actor or the actor's location and never inside any vault's
    backing chain. Custody effects are audit-only; boot reconciles the last `Deposit` per object
    against the provider and freezes a titled mismatch per asset (a provider `None` is
    unverifiable, not a mismatch).
19. **Module order matters: economy boots after sim, and faction after org.** `SpatialState` is
    installed at registration, so an economy booting first sees an empty state and a definite "not
    held" for every interior deposit. Org still precedes economy; a server without
    `SimulationModule` takes the provider's `None` path safely. *(Owner decision 2026-09-03:
    `FactionModule` loads after `OrgModule` so faction boot can validate every `Faction.org` link
    against loaded org state — a missing org or a two-factions-one-org conflict is reported loudly
    at boot and never unbound; `backing_org` stays the at-use enforcement. The 24 h approval TTL
    and staff-only character-to-character title transfer were confirmed at the same time.)*
20. **The E1c membership adapter is struck** (D-E3-11). Role templates need real `OrgMembership`;
    a faction-backed org authorizes through direct grants and `FactionStandingProvider` predicates;
    `@account/approval` refuses `from_role` for a role the org does not have. A faction officer with
    no membership and no grant sees ratings and no money and cannot spend from the org wallet.
21. **Faction pools migrate by explicit, static, per-key marking** (D-E3-9). `Faction.migrated`
    decides what is money; registering an instrument never reclassifies a rating.
    `@faction/migrate <f>=<key> <sysacct> /confirm` takes the pool verbatim as minor units, posts
    first (key `faction.migrate/{fid}/{code}`), then marks; if the idempotency record already
    exists it marks and logs the residual instead of re-posting, so an edited pool still converges.
    `@faction/destroy` is refused while any link exists; two factions bound to one org both fail
    closed. Balances render in `+faction`/`@faction/show` only through `may_view_account`.
22. **`PolicyRef` is not built** (D-E3-5); per-resource policy is a leaf scope + `AccessPolicy`
    rules against `ScopeSelector::Exact`. `CatalogRef { ruleset, code }` is the opaque precursor of
    §9.4's `CatalogId` (E6 owns the reconciliation).
23. **The four-axis split holds** (D5): `@asset/assign|transfer` leave `StationAssignment` seats
    and `control_gen` untouched, and a station assignment leaves the title record untouched (tested
    in `mush-sim/tests/custody.rs`). A new title owner has **no** verb to evict an operator in E3.
24. **`@chown` and title are independent both ways** (§9.1); `@chown` is not gated. The destroy
    gate covers the target and everything it transitively contains (a live title, an active or
    non-empty vault backing, a principal holding a balance or a title); `@destroy/override` bypasses
    only `SAFE`, never the gate; the non-staff refusal is neutral text.

25. **Programmatic resolution first; page a human only for a genuine decision; stop only the subject**
    (owner decision 2026-09-03, standing for every economy cut from E4 on; 08c §4.6 / D-E4-14). A game
    has perhaps one staffer who understands an economy alert and may be hours away, and they will
    take it to an LLM anyway — so the economy forgives downtime/time-jump backlogs with an
    attributed record (doc 08 §2 forbids charging them), lets contract parties resolve disputes
    unanimously, pages only for refusals a human must decide (closed/missing account, inactive org,
    retired instrument) with the recommended command already filled in, suspends only that
    obligation or contract, and never stops the world for a local problem. The whole-economy freeze
    remains the one world-stop and now also opens an escalation record carrying its diagnosis.

## E3 verify outcome (end-of-build, three lenses, probe-confirmed)

Confirmed and folded, each with a regression test: **(H)** net-zero self-transfer bypassing debit
authorization; **(H)** exits accepted as vault contents but invisible to every custody query;
**(H)** the whole-economy freeze latch lost at the next boot (E2-era); **(M)** cross-resource
existence leaks through the error renderer (pay destination, `@asset/maintenance` account);
**(M)** a withdrawal landing inside another vault's backing chain; **(M)** a vault's own backing
deposited into another vault; **(M)** no durable gate on the new control-doc writes; **(M)**
`@faction/migrate` unable to converge after a pool edit; **(L–M)** the migrate opening the org
account before its last fallible call; allocator floors persisting only one boot; `Account.revision`
moving backwards on a lag heal; actors depositable as vault contents; a destroyed `NamedSet`
approver still counting; approval caps checked after a stale-request retire; non-recovery control
verbs usable while frozen. Read-verified sound: `post` ordering and the terminal-fallible rule in
every command, the both-sides title rules, custody standing, approval consumption and hash
binding, reversal unforgeability, E2-era journals under the E3 binary (hash pin + live retry), latch
polarity across reboot, entry boundedness, key routing, module order.

## What E4 closed (the E3 pickup list, resolved)

Every hook the previous edition of this section named is now built, and the entries are kept here
so a reader can see which cut answered what:

- **Holds + `held_amount`** — E4a's pure `Hold`/`HoldOp` and available-balance enforcement, live in
  E4b1 with a journal-derived projection on its own seq anchor. Two approved plans over one account
  no longer both post.
- **Two-party consent** — E4b2's contracts: the acceptance *is* the settlement authority, bound to
  the consent hash and re-checked live (`ContractAuth`).
- **`Encumbrance::Lease` / `Collateral`** — E4b2 places them at activation and releases them at
  settlement or refund; a lease's obligation is created and cancelled with the contract.
- **Clocks + bounded catch-up** — E4a's schedule math and E4b1's one economy sweep; cadence clocks
  exist through persistence, boot and the sweep (no `@econ/cadence` verb yet — E5's).
- **The disclosure filter** — E4c. `EconomicVisibility` is now **enforced for reads**:
  `@econ/history` renders the viewer's own side in full, collapses every counterparty they may not
  view into one `other accounts` token (no ids, no amounts, no count), omits `Staff` entries except
  the effect-only ones, and shows a description only to a party; `@contract/show`, `@contract/list`,
  `@hold/show|list` and `@obligation/show|list` render through the same `mask_account` rule. The
  verify fold added the rule that ties them together: **a reservation on an account is a fact its
  viewers may see; the contract behind it is not**, so a `Staff`-visibility escrow shows as a
  reservation with no purpose and no beneficiary to anybody outside that contract's class.
- **Approval → org audit** — E4c's `mush_org::audit`; `@econ/approve|deny` append an entry naming
  the request id and the account scope.
- Still open from that list, unchanged by E4: an `Organization.revision` bump op +
  `SetOrgStatus`/`PutRole` (decision 3); per-asset isolation of the whole-economy freeze; the
  durability checkpoint (12); `CustodyRef::Registered` has no provider; the inert non-quarantined
  `Unplaced` placement doc after an interior withdrawal; an operator-eviction verb for a new title
  owner (E7). (`TacticsModule` has been wired into `server_modules` since E3c — the line that said
  otherwise is struck, DL-15.)

## What E5 closed (the E4 pickup list, resolved)

- **Cadence clocks have a verb — and a programmatic creator.** `@econ/cadence create|show|list|
  advance` shipped in E5b; E5c lifted the creation core into
  **`mush_economy::create_cadence(ctx, name, source, period, catch_up)`** so a ruleset's boot can
  make the campaign clock its configuration names (there is nobody to type a verb at boot). One
  creator, so the caps, the duplicate-name rule and the "anchored at now" rule cannot drift. E4b1's
  B6 (a schedule on a deleted clock) and S2 (cadence starvation) are now exercised against **real**
  clocks by `mush-housing/tests/e5b.rs` and `ruleset-wod/tests/e5c.rs`.
- **The income table is `create_system_obligation`'s first caller**, as planned — and E5c added its
  counterpart, **`mush_economy::set_system_obligation(ctx, faucet, id, Option<Amount>)`**, so a
  faucet can re-rate or cancel a standing mint it created. Same four locks (minted by this faucet,
  still live, both accounts `Active`, inside `mint_cap`); a missing record and somebody else's
  answer the same `Unauthorized`.
- **`Delinquent`'s consequence hook is housing's.** An exact tenancy mirrors its obligation onto the
  ladder's first rung through `HousingEconomyObserver` and walks the rest on its own cadence;
  `@obligation/reinstate` is still the manual path, and nothing ends a home automatically (D-E5-1).
- **The title issuer is registered, and now conditionally.** `EconomyModule::with_title_issuers`
  carries `sys:wod.housing` **only when `[rulesets].enabled` selects `wod` and
  `[wod.economy].housing` is on**; `@property/create` mints the anchor's title through
  `issue_title` under it, and refuses when the list is empty.
- **Sub-threshold drip: still documented, not fixed.** E5's income lines are bounded by
  `EconomyConfig::mint_cap` rather than by an `ApprovalRule`, so the drip is out of their way; a
  budget primitive is still unbuilt and is E6/E8's.
- **Notifications still reach only `PrincipalId::Object`.** Unchanged by E5: `contract_live::notify`,
  `sweep::notify_principal` and the escalation fan-out all stop there, and an org or system
  landlord still has no inbox. The `EconomyObserver` seam is where that belongs.

## Pick up here: E6 — markets, quotes, catalogs

Design: doc 08 §18 "E6"; 08e is the plan document. What E4 and E5 leave as the hooks, concretely:

- **`with_canon_outcome_kinds` is still empty, deliberately.** E6b owes the attestation seam a
  **producer**: `scene.outcome`, outcome-bearing, sealed under facilitator/adjudicator authority,
  named at offer, registered through that table. `scene.concluded` never attests, and neither does
  E5c's `wod.career.exposure` — an evidence record its own subject produces is one they could settle
  a contract with alone. Until a kind is registered, `@contract/offer` refuses every `canon:`
  attestor set, which is fail-closed and visible to whoever drafts it.
- **The per-faction off-screen pool is a shared seam.** `ruleset_wod::WodEconomyStore::open_ops()`
  is a derived `BTreeMap<FactionId, u32>` of currently-open front-attributed prompts, capped at
  `MAX_OFFSCREEN_OPS_PER_FACTION` (8). A faction simulation that starts running its own operations
  should read and contribute to that map rather than growing a second budget — one ceiling per
  faction is the whole point.
- **`FrontRef` is the pre-C6 attribution shape** (08d §7 item 3, §5.6). The canon `source` string
  `front:{faction}:{front}` is the stable half and is what E4c matches an attestor on; the record
  becomes a projection of the kernel causal chain once doc 09's C6 lands.
- **A fact provider is how a domain answers a contract.** `mush-jobs` is the worked example
  (`JobsFactProvider`, `jobs.resolved`, payload-bearing jobs only, answered from an **applied
  marker** and never from a status). Doc 10 §15's `sim.*`/`tactics.*` kinds register the same way.
- **Housing's seams are library-level now.** `mush_housing::create_tenancy` (abstract, live
  immediately) and `mush_housing::submit_support_plan` are the two entries a layered module uses;
  a plan older than `PLAN_MAX_AGE_PERIODS` (2) is *absent*, so any evaluator must restate every
  period.
- **Mode flips have a pattern.** `@wod/economy migrate` is the shape a later reconciliation should
  copy: staff, one-shot, idempotent, it lists what it did, and it mints nothing in either direction.
- **Still open from the E3 list, unchanged by E4 and E5:** an `Organization.revision` bump op +
  `SetOrgStatus`/`PutRole`; per-asset isolation of the whole-economy freeze; the durability
  checkpoint; `CustodyRef::Registered` has no provider; the inert non-quarantined `Unplaced`
  placement doc after an interior withdrawal; an operator-eviction verb for a new title owner (E7).

## Build discipline (keep it)

Each cut: (1) a short design review / decision pass before building; (2) build the smallest
coherent library→module rhythm; (3) an adversarial-verify pass hunting fail-open / money-safety /
durability holes, **probe-confirmed** (the E3 verifiers wrote throwaway tests, which turned
plausible into confirmed and kept the fold honest); (4) fold every confirmed finding with a
regression test; (5) commit with the verify outcome in the message. E3's single end-of-build verify
found three high and several medium holes across four cuts — the per-cut cadence E0–E2 used would
have caught the E3b1 custody holes before E3c built on them; prefer per-cut verifies for E4.
