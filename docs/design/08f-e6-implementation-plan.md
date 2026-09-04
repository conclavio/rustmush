# E6 implementation plan — the Shadowrun run-and-pay economy

**Status:** Draft 0.3 — 2026-09-04. Re-grounded at `ce43714` after the E1–E5 look-back (08g §11
item 8, DL-02); DL-01 Option A′, DL-10's rolling budget and E2-C16's `InstrumentDisplay` are E6a
economy items. Companion to
[08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (§9.4, §11.2, §14.2,
§18-E6), [08a-economy-build-status.md](08a-economy-build-status.md) (the ledger and the E4 seams as
shipped), [08c-e4-implementation-plan.md](08c-e4-implementation-plan.md) (**E4 as built** — prefer
its §3.5a/§4.7/§5.1/§6.1 blocks over §3–§6's pre-build text), and
[08d-e5-implementation-plan.md](08d-e5-implementation-plan.md) (the structure copied here). **The
binding constraints are the owner's twenty-four answers in
[08e-e6-owner-questions.md](08e-e6-owner-questions.md) §3** — restated in §10, cited by number
throughout, overriding doc 08 §11.2 where the two differ — plus doc 10 §12–§15 where it names E6.
Grounded in the tree at `34a53c0` (main, K3 merged) and re-checked at `ce43714` for 0.3; every
`file:line` was checked. Departures are numbered decisions (D-E6-n, §8).

> **Numbering.** The briefing's shorthand drifts one place from 08e §3 between answers 11 and 23.
> **08e §3's numbering is authoritative** and is what this document cites: 11 vendors/quotes,
> 12 clinics, 13 fence pricing, 14 `report_stolen`, 15 delayed settlement, 16 config, 17 chargen,
> 18 migration, 19 cuts + no team vaults, 20 `mush-market`, 21 party, 22 roles, 23 LLMs, 24 board.

> **Gates.** E6a and E6b are **unblocked** — E4b2 (`e56c2a9`, fold `4972455`) and E4c (`8873863` +
> fold) shipped. E6c gates on doc 10 §14's **safe market projection**, not the belief engine.
> Re-ground §3–§5's E4 references against the built economy before each cut starts.

---

## 1. Scope — what E6 is, in the owner's terms

Shadowrun is "much more materially economic than WoD" (08e §3 preamble). E6 makes a runner's money
exact and their work contractual, on top of E4 as built. §10 has the answers; this is what E6 builds:

- **Exact personal money** — `sr.nuyen` at scale 0, the account authoritative after chargen approval
  (answer 1), **karma untouched** (answer 2). Today `nuyen` is a plain sheet number
  (`crates/ruleset-shadowrun/src/schema.rs:298`, `0..=50_000_000`, default 0) read in one place, the
  chargen Resources ledger (`.../src/chargen.rs:351`).
- **Lifestyle as an E4 obligation on an explicit `sr.month` cadence**, autopaid, **at most one
  downgrade per unpaid period** and none for a downtime or clock-jump backlog (answer 3). Today
  `lifestyle` is a cosmetic enum (`.../src/data.rs:225-232`, field `schema.rs:148-152`) with no cost
  anywhere in the crate.
- **Runs as contracts with real principals** — players, authorized org members, staff, player STs and
  campaign systems, **never an uncontrolled faucet**, with author / payer / representative /
  facilitator / adjudicator as distinct *fields* — distinct people only where a conflict of interest
  exists (answers 4, 22; the roles rule in §3.3).
- **An ephemeral party** that is not an organization, not a principal, and owns no account or vault
  (answers 5, 19, 21); acceptance freezes the roster and the exact shares (answers 7, 8).
- **Typed settlement, never prose** — sealed scene outcome, authorized attestation, deterministic
  condition, or composite, all through E4's one `TypedCondition` alphabet, with the adjudicator and
  permitted evidence named in the offer before anyone accepts (answer 6); E6a itself settles by
  attestation alone (D-E6-20), the other routes arrive with E6b. An LLM proposes; it cannot
  invent funds, alter frozen terms, or release escrow (answer 23).
- **A neutral `mush-market`** (answer 20): catalog policy over a small source-controlled starter
  catalog with versioned packs (answers 9–10), vendors and quotes with a declared expiry clock and
  engine-resolved availability tests (answer 11), and a safe public projection.
- **Fences that never launder title** (answer 13): title ≠ possession, a typed `report_stolen`
  (answer 14), and a delayed consignment that **escrows before it takes custody** (answer 15).
- **`[shadowrun.economy]`** (answer 16), chargen shopping as one live cash authority (answer 17), and
  an idempotent migration from `system:sr.migration` (answer 18).

**Exit criterion (doc 08 §17.5/§18-E6):** scenarios 25 and 26 pass — offer, escrow, complete, split
and settle a run under success *and* failure; quote and accept a fenced-goods sale requiring real
delivery and exposing only observer-authorized provenance — plus scenario 29 (an LLM proposal delayed
past quote or policy revision is refused deterministically).

### 1.1 Explicitly out of E6

- **The belief/knowledge engine.** `Claim`/`Belief`/`Evidence` are design-only (08e §1); doc 10 §14
  gives E6c the **safe market projection** instead. Belief-sensitive fence pricing, **heat, notoriety,
  investigations and title laundering** — answer 19's E6c list — therefore move **past E6**, because
  the safe projection supplies no beliefs (§11 q3).
- **Consent by authorship (DL-01 Option A′).** An amendment authored by a party never voids that
  party's own acceptance; every other acceptance is voided and refunded as E4b2 does today. With only
  the payer accepted, `+run/crew accept` amends the roster and payout rows without moving the escrow.
  Full per-party binding stays out of E6 (08g §12.1 item 1; fed back in §7).
- **TacSim industry** (`mush-industry`, deposits, recipes, capacity, the C8 coordinator) and
  heterogeneous reservations — doc 10 §14's boundary puts them in E7; E6's holds stay money-only (T4).
- **Clinics beyond an atomic installation record** (answer 12). E6b ships the augmentation record and
  the rule that payment never edits the sheet; the ruleset-owned installation action that revalidates
  Essence, prerequisites, custody, clinic capability and sheet revision atomically is later. Buying a
  formula or a teacher never bypasses the Karma cost.
- **Owner abandonment** (`title_owner: Option<PrincipalId>`) — 08c §5.x keeps it in E7 with the
  mechanical recipe written out.
- **Team vaults and party-owned money** (answers 5, 19, 21): a persistent crew is a staff-minted
  organization with an account and a vault (08a decision 1).
- **Receipt-based reimbursement** (answer 7's "once attestations are mature"):
  `ExpenseTreatment::Reimbursed` stays declarative; E6 ships a fixed advance.
- Nothing here touches `ruleset-wod` or `mush-housing` (E5).

---

## 2. Cut structure

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E6a** | `mush-party` (new), `ruleset-shadowrun`, `mush-economy` (A′, the rolling window, the obligation and contract seams, `InstrumentDisplay`, `contract_notice`, the M4 fold, DL-05, DL-08), `mush-server` (DL-04, wiring) | `sr.nuyen` + the idempotent migration; lifestyle obligations on an `sr.month` cadence; the ephemeral party subsystem; the bounded run board; run templates over E4b2's roles / outcome table / roster lock; `[shadowrun.economy]`; chargen as one live cash authority | uncontrolled faucets, board spam, an org-treasury drip, a fixer skimming, split arithmetic, party lifecycle |
| **E6b** | `mush-scenes`, `mush-economy` (`report_stolen`, the DL-03 deposit seam, DL-11's vendor accept), `ruleset-shadowrun` | the facilitator-sealed **`scene.outcome`** producer and its registration; the four settlement routes through one condition alphabet; deterministic fence pricing with title ≠ possession; typed `report_stolen`; delayed consignment with escrow-before-custody; the augmentation record | forged seals, prose settlement, fence laundering, consignment custody races |
| **E6c** | `mush-market` (new), `ruleset-shadowrun` | catalog + content packs and the representation policy; listings, order books, quotes with declared expiry clocks; the **safe public projection**; vendors from stock / market source / procurement with engine-resolved availability tests; NPC market makers with explicit budgets | market existence oracles, stale quotes, market-maker insolvency |

**Dependencies** (no cycle; neither `mush-economy` nor `mush-org` gains an edge): `mush-party →
{core}` only — it is not a principal and must not learn about money (answers 5, 21). `mush-market →
{core, org, economy}` — listings name principals and settle through contracts; the economy never
depends on the market (doc 08 D11, doc 10 §14). `mush-scenes → {core, canon, proto}` **unchanged** —
the seal's authority is `perm::has_role` (`crates/mush-scenes/src/commands.rs:46-48`), so no
`mush-org` edge is needed. `ruleset-shadowrun` gains `{economy, org, party, faction}` in E6a
(`faction` for board membership — M5); `canon` arrives with E6b and `market` with E6c. Today it
depends only on `{mush-ruleset, mush-dice, mush-core, mush-jobs, mush-proto}`
(`crates/ruleset-shadowrun/Cargo.toml:9-19`) — every edge is new, as E5c's were.

**Module order** (`server_modules_with`, `crates/mush-server/src/modules.rs:102-233`: comsys, canon,
org, faction, scenes, sim, tactics, economy, mail, jobs, housing, weather, script): `mush-party` is a
server module and sits anywhere before the rulesets; it reads no economy state and the economy reads
none of it. The board mirror is an `EconomyObserver` frozen at wiring (`with_observers`, added when
`shadowrun` is enabled — the `issuers` pattern at `modules.rs:114-117`). `mush-market` boots
**after** `mush-economy` (it validates listings against real accounts, assets and vaults at boot —
the argument that put economy after sim); `ruleset-shadowrun` stays in `ruleset_modules` after
`server_modules` (`modules.rs:315`, built with `::new()` and no config path today).

**Economy seams each cut consumes as built.** *E6a:* `create_system_obligation` + the `[economy]`
mint cap (`crates/mush-economy/src/api.rs:1083-1129`, `config.rs:32` — 50,000 major units/period);
cadence clocks (`clock.rs:210-249`); the `@contract/*` surface with outcome-indexed terms
(`contract.rs:236-276`), `ContractRoles` (`contract.rs:528-537`), roster lock + amendment
generations, named adjudicator + `EvidenceKind` (`contract.rs:487-500`),
`EconomicVisibility::Audience` (`record.rs:340-365`), `ContractAuth` purposes, the plan-first
terminal-transition rule, and the per-class `contract_show` render (08c §5.1). *E6b:*
`with_canon_outcome_kinds` (`module.rs:635-639`, type `contract.rs:322`), the `CanonRecorded`
observer with its kind + **mandatory source** + party-intersection gates (`attest.rs:217-255`,
`canon_set_accepts` at `contract.rs:1746-1753`), `TypedCondition` (`contract.rs:399-441`), the
`DepositInto` custody lock, `After` + `stamp_observed`, and
`Encumbrance::{Disputed, Lease, Collateral}` (`record.rs:510-543`). *E6c:* `CatalogRef`
(`record.rs:556-562`, set nowhere today), `InstrumentKind::{Commodity, Entitlement}`
(`record.rs:52-61`), `issue_title` + `with_title_issuers` (`api.rs:1552`),
`may_view_account`/`mask_account`, and the `FactProvider` seam (`facts.rs:68-77`; worked example
`crates/mush-jobs/src/facts.rs:35-60`).

**Everything E6 adds to `mush-economy` — the whole list** (re-grounded at `ce43714`: 0.2's list was
stale — G1 — and `@econ/cadence` shipped in E5b, §11 q2):

1. **DL-01 A′** in `contract_amend`/`contract_offer`/`build_pay` (+ a `Draft`-with-escrow expiry in the sweep, M10) — E6a, S. The smallest change: `contract_amend` takes the author as the acting party (or the issuer); if every acceptance is the author's it skips the unwind, otherwise a partial `refund_plan_except` releases only the holds backing other parties' considerations (key `contract/{id}/refund/{gen}`, `ContractAuth::unwinding`) and `acceptances.retain(party == author)` keeps their hold ids in `escrow`; `contract_offer` re-stamps the surviving acceptances' `consent_hash` **iff** every consideration with `from == author` is claimed by a `Held` hold in `qualifying_escrow`, else refuses; `build_pay` refuses changing a consideration a `Held` hold backs; `amendments` bumps only when a refund posted. Tests: payer-amend keeps acceptance + hold; fixer-issuer amend voids the payer's; a changed own consideration is refused at offer; cancel after A′ refunds; reboot loads (boot's `Draft => {}` arm adopts nothing it should not).
2. **DL-10** `ApprovalRule.window { total, period_secs }`, journal-derived per-account rolling sum, evaluated with the threshold — E6a, S (D-E6-21; `@account/approval … window=<total>/<secs>`, doc authority on the account, E2/E3 docs unchanged). `EconomySnapshot` carries no journal, so `post_with`/`prepare_commit` hands `evaluate_approvals` a `spent: BTreeMap<AccountId, Amount>` for the plan's debit accounts through `ValidateEnv`; the sum is the account's negative lines plus `Placed` hold effects over `posted_at > now − period` (wall clock), a refund never re-opening it; a RAM-only per-account `VecDeque<(posted_at, Amount)>` maintained in `index_transaction` and rebuilt at boot from replay keeps it O(entries in window). Over the window ⇒ `NeedsApproval` on the threshold's own path. The sweep never places, so no obligation is affected.
3. **`create_obligation` / `cancel_obligation` / `reinstate_obligation`** for `ObligationCause::Ruleset` — debtor never `System`, actor debit authority checked — E6a, S. No player-debit creator exists at HEAD (`@obligation/create` is staff-only and stamps `cause: Staff`; `offer_lease` is contract-shaped; `create_system_obligation` needs the faucet to own the debit account). The seam: debtor account owner never `System`, both accounts `Active` on one instrument, the actor holds debit authority on the debit account (`WorldAuth::authorize` — the player for `+lifestyle/set`, the approver at chargen), `minted_by: None`; cancel/reinstate gated on the same namespace (the `set_system_obligation` lock shape). `ObligationAuth` already permits a non-`System` debit on the record's own account.
4. **`offer_contract` / `accept_contract` / `amend_contract`** seams (the `offer_lease` shape) so a ruleset never types `@contract` — E6a, S–M. Every `@contract` builder is private and `put_contract` is `pub(crate)`; a nested `cmd::dispatch` is reachable from a handler but has no precedent in an E crate and re-runs per-invocation gates.
5. **`InstrumentDef.display: Option<InstrumentDisplay>`** + one `render_amount` — E6a, XS (E2-C16, 08g §12.1 item 4). `display` goes after `status`, `#[serde(default)]`, `{ symbol: SmolStr, position: Prefix | Suffix }`; consumers are the `to_decimal_string(scale)` sites and the five raw-minor renders of 08g §3; fold item 7 (E2-C18, `register_instrument` refuses an existing id — today an upsert) lands first.
6. `pub contract_notice` and the hiding helpers (E5b-D36/37) — E6a, XS. `notice_lines` and `show_class` are private; `may_view_contract` is `pub`; export `contract_notice(ctx, &EconomicContract) -> Vec<String>`.
7. (E6b) `AssetOp::ReportStolen` + `@asset/stolen` (08e §4 item 6, D-E6-18); (E6b) the DL-03 deposit seam; DL-11's vendor accept (`with_party_caps`) — first for E6b's NPC fence (M15), reused by E6c's vendors and makers (G8).
8. Wiring: `mush-party` in `server_modules_with`; the SR board observer in `with_observers` when `shadowrun` is enabled; `ruleset_modules(.., sr)`; `[shadowrun.economy]` in `mush-config`. Later cuts register `with_canon_outcome_kinds(vec!["scene.outcome"])` (E6b; `modules.rs:206`, empty today) and `with_title_issuers([… "sr.market"])` (E6c; `modules.rs:199`, empty today). Registration, not code.
9. **M4 fold:** `check_structure_basic` refuses a `System` party or `from` — E6a, XS; it makes D-E6-4's refusal structural.
10. **DL-08:** stale-contract paging is deadline-aware (skip while an unlapsed deadline exists) and the 7 d `STALE_CONTRACT_AGE_SECS` const becomes `[economy].stale_contract_age_secs` — E6a, XS; today every 14-day run would be paged at day 7.
11. **DL-05:** `EconomicEvent::EscalationOpened`, fanned out by the server's `broadcast_wizards` on the ladder's schedule (§3.3) — E6a, S.

---

## 3. E6a — run and pay

### 3.1 Records

```rust
// ---- mush-party (new; game-neutral, NOT an economic principal) -------------
pub struct Party {
    pub id: PartyId,  pub name: SmolStr,  pub leader: ObjId,
    pub members: Vec<ObjId>,               // accepted, ≤ MAX_PARTY_SIZE (8); order is display only
    pub invited: Vec<Invitation>,          // { who, by, at, expires_at }, ≤ MAX_PARTY_INVITES (16)
    pub roles: BTreeMap<ObjId, SmolStr>,   // "face", "decker" — labels, never authority
    pub status: PartyStatus,               // Forming | Locked { by: ContractId } | Disbanded { how }
    pub expires_at: i64,                   // idle expiry, refreshed by any membership change
    pub revision: u64,
}
// ---- ruleset-shadowrun -----------------------------------------------------
pub struct SrProfile {                     // `sr:profile:{obj}`
    pub account: Option<AccountId>,        // opened at chargen approval or migration
    pub lifestyle: LifestyleTier,          // street|squatter|low|middle|high|luxury (data.rs:225-232)
    pub upkeep: Option<ObligationId>,      // the sr.month obligation; None when the tier costs 0.
                                           // A CACHE: the record self-identifies by cause
                                           // Ruleset{"sr.lifestyle"} + credit = system:sr.lifestyle (DL-07)
    pub unpaid_periods: u8,                // keyed on the sr.month ordinal of the last miss, never
                                           // wall time; carried across a replacement (M11)
    pub arrears: Amount,                   // what +lifestyle/pay posts; keyed on the same ordinal
    pub minted: Option<Mint>,              // { key, transaction, at } — the chargen allotment (M12, M13)
    pub migrated: Option<Migration>,       // { from_sheet_value, approved_at, transaction, at } —
                                           // SheetDoc has no revision (mush-ruleset/src/doc.rs:60-70)
    pub revision: u64,
}
pub struct RunTemplate {                   // `sr:run:{id}` — the BOARD entry; terms live in the contract
    pub id: RunId,
    pub contract: ContractId,              // money, authority and consent are E4b2's, never duplicated
    pub principal: PrincipalId,            // accountable payer: Object | Organization — never System (D-E6-4)
    pub notice: RunNotice,                 // { name, window, kinds, johnson: Option<PrincipalId> } —
                                           // copied at post, rendered by +run/show (M5); `johnson` is
                                           // the visible representative when announced (answer 4, G7)
    pub board: BoardVisibility,            // Public | Faction(FactionId) | Invitation { party: PartyId }
                                           // — enforced ruleset-side, never the contract's Audience (M5)
    pub faction: Option<FactionId>,        // the Faction(_) payload, denormalized for +run/list faction
    pub slots: u8,  pub posted_at: i64,
    pub deadline: EconomicDeadline,        // the contract's own (RealUtc): nothing else expires an
                                           // Offered contract holding escrow (M8)
    pub expires_at: i64,                   // = that deadline's instant, ≤ BOARD_TTL
    pub party: Option<PartyId>,  pub revision: u64,
}
```

`RunTerms` is built at `+run/post` and written straight into E4b2's `ConsentSurface` through the
`offer_contract` seam (§2 item 4): one `Consideration` from the payer's account; `ContractRoles` plus
`ConsentSurface.adjudicator` and `evidence`; and an **outcome table** whose `payouts` name each
frozen recipient account exactly (`contract.rs:236-256`). Answer 7's five slots — runner payout,
fixer fee, expense advance, reimbursable reserve, bonuses/penalties — are **named payout shares
inside each outcome row**, not new economy fields; the reserve is a share back to the payer's own
account, named explicitly in the failure row, because `ExpenseTreatment::Returned` is declarative
and read by nothing (`contract.rs:206-225`). `PayoutShare.to` must exist and be `Active` at offer
(`check_offerable`, `contract.rs:1604-1608`), so at post the crew's share sits provisionally on the
payer's own account and `+run/crew accept` rewrites it to the members' `SrProfile.account`s (M6).

### 3.2 State machines

```text
Party:   Forming ──payer accepts the party's application──▶ Locked{contract}
         Forming ──leader disbands / last member leaves / idle past expires_at──▶ Disbanded{how}
         Locked  ──contract Settled | Cancelled | Expired──▶ Disbanded{Settled|Released}
         Locked  ──substitution──▶ Forming   (the contract must be AMENDED first; §1.1)
Board:   Draft ──+run/post: the payer drafts as issuer, offers, and accepts (escrow) in one seam call──▶ Listed
         Listed ──+run/crew accept: an A′ amendment adds the crew as parties and rewrites every
                  outcome row to their accounts, then re-offers; the payer's acceptance and escrow stand──▶ Crewing
         Crewing ──every runner accepts (the first acceptance locks the roster)──▶ Crewed (Active)
         Listed | Crewing ──the deadline, or the payer withdraws──▶ Closed  (unbound: escrow refunded; entry dropped)
         Crewed ──the contract settles, or its deadline lapses and DISPUTES (M7)──▶ Closed
Life:    Paid ──miss──▶ Arrears{1} = Grace                (one private notice, NO downgrade)
         Arrears{1} ──miss──▶ Delinquent: one tier down; the record is REPLACED at the new tier
                     (create_obligation), unpaid_periods carried                     (answer 3, M11)
         Arrears{_} ──the next period pays (Grace) / +lifestyle/pay: arrears, then reinstate──▶ Paid  (silent)
         any ── ForgiveCause::{Downtime,TimeJump,Suspension} ──▶ unchanged, no downgrade
         Grace | Delinquent ── +lifestyle/set ──▶ REFUSED (pay the arrears first)
```

**No listing exists before its escrow does** (answer 24). `+run/post` drafts with the payer as issuer
(D-E6-19), offers and accepts through the `offer_contract` + `accept_contract` seams — no `@contract`
command is dispatched — and E4b2 places a party's consideration hold at *their* accept, exactly the
Johnson-escrow shape the shipped rig proves (`crates/mush-economy/tests/e4b2.rs:249-251`). A refused
accept (no funds, an `ApprovalRule` threshold or rolling `window`, a suspended scope) lists nothing
**and cancels the draft**, so no unfunded `Offered` contract counts against
`MAX_OPEN_CONTRACTS_PER_ISSUER` (M17). Between the A′ amendment and the re-offer a `Draft` can hold
live escrow (`/offer` refuses a closed payout account and `collect_draft` skips a draft with holds):
`+run/crew accept` retries or cancels, and the sweep expires a `Draft` whose deadline lapsed (M10).
**One downgrade per unpaid period, ever:** the downgrade is driven by `ObligationChanged` /
`missed_total` deltas, not by the arrears counter's absolute value, and a forgiven period is never a
miss — a ten-period downtime backlog forgives with a classified cause and downgrades nothing
(answer 3; doc 08 §2 forbids charging a backlog). Two E4 facts shape this (M11, G12): `Delinquent`
leaves the due pool and autopay stops (`obligation.rs:171-173`, `sweep.rs:459-461`), so the second
miss *replaces* the record one tier down; and `plan_sweep` forgives a backlog's excess but still
attempts `maximum_catch_up` in one fire, so the lifestyle schedule sets it to **1** — one fire, one
attempt, never a second miss from downtime — while the `sr.month` clock's own `catch_up` (3) forgives
on the clock first. `unpaid_periods` and `arrears` are keyed on the `sr.month` ordinal at the miss,
never wall time (E5c T3).

### 3.3 Money and faucets

`ruleset-shadowrun` registers at boot: `sr.nuyen` (`InstrumentKind::Currency`, **scale 0** —
answer 1; already the crate's documented example at `crates/mush-economy/src/ids.rs:8,85`; with
`display: Some(InstrumentDisplay { symbol: "¥", position: Prefix })` once fold item 7 (E2-C18) makes
`register_instrument` refuse an existing id — 08g §12.1 item 4), the faucets `system:sr.chargen`
(answer 17) and `system:sr.migration` (answer 18), and the sink `system:sr.lifestyle`. `sr.month` is
created by `@sr/clock` over `mush_economy::create_cadence` (`api.rs:1365-1425`), default
`CadenceSource::WorldIc` (answer 3: an explicit campaign clock, never unrestricted real time and
never a literal calendar); it is a legacy `EconomicClock::Cadence(id)` until D-LBK-7's convergence
(§9). Who may fund (answer 4): a player from their own account; an org officer holding
`contract.draft`, `contract.accept` and `economy.account.post` on the named scope, subject to that
account's `ApprovalRule` — threshold and rolling `window` (D-E6-21); staff or a player ST for a
principal they act for; a campaign through a **funded NPC organization** — never a `System`
principal directly (D-E6-4: the tree makes one unfundable, E6a makes it unofferable).

The run's issuer is its payer (M1). A `System` account named as the payer's is refused at `+run/post`
with the account's own hiding text (M4). Outcome conditions are attestations; the adjudicator
defaults to the payer (M2). A crewed run's deadline disputes (M7). The allotment and the migration
are one-shot attributed posts bounded by the priority table, not by `[economy]`'s standing-mint cap
(M12).

**Roles (answer 22, amended by the owner at 0.3).** Distinctness is enforced where a conflict of
interest exists — the adjudicator is never a party (payer, runner, representative, facilitator) and
never the poster; the representative and the facilitator are never the payer; the author and the
payer **may** coincide (a Johnson posting their own run); an organization's author is the posting
officer. Operationally (M1): `+run/post` is run by the payer — a character from their own account, or
an officer holding `contract.draft`, `contract.accept` and `economy.account.post` on the org's scope
(`scope=`), which makes the org the issuer; a fixer is `roles.author`/`representative` with a fee row
and never the issuer. Why: `@contract/amend` is issuer-only and A′ keeps only the *amending party's*
acceptance, so a fixer-issuer would refund the payer's escrow at every `+run/crew accept` and leave a
`Listed` entry standing on nothing (D-E6-19).

**Who is told (G5 — E6a prerequisites, not fix-during extras).** Contract notices reach `Object`
parties only (`contract_live.rs:2643-2663`, `sweep.rs:956-960`), and org payers are E6a's design
case: without these an org is told nothing when its run is crewed, settled, disputed or refunded.
**DL-04:** an `OrgInbox` economy observer delivers a kinds-only line to every connected holder of
`acts_for` on the org, with an offline notice per the housing pattern (server, S). **DL-05:**
`EconomicEvent::EscalationOpened` plus a `broadcast_wizards` fan-out following the ladder's schedule
(economy + server, S).

### 3.4 Commands

```text
+party  ·  /create <name>  /invite <who>  /uninvite <who>  /accept <id>  /decline <id>
        ·  /leave  /kick <who>  /role <who>=<label>  /disband
+run/list [public|faction|mine]        the board, newest first, ≤ BOARD_PAGE
+run/show <id>                         the board's own notice under the board's visibility rule (M5)
+run/post <name>=<pay> outcomes=<a,b,c> [scope=<org>] [johnson=] [adjudicator=<attestor>]
                                       [slots=<n>] [board=public|faction:<f>|invite:<party>] [deadline=]
                                       (the payer is the issuer — M1; evidence is `attest`, forced — M2)
+run/split <id>=<slot|who> <exact|N‰>  edit a slot share while still Listed — an A′ amendment (M16)
+run/apply <id>=<party>                the leader offers their party (every member needs an sr.nuyen account — M6)
+run/crew <id>=accept|decline          the payer's answer; accept = the A′ amendment (crew added, payouts rewritten, re-offered)
+run/withdraw <id>                     the payer cancels a Listed run (refund via @contract/cancel)
+lifestyle  ·  /set <tier> (next period, never the current one; refused in arrears)  ·  /pay
+nuyen                                 the derived balance post-approval; the sheet field before
@sr/economy [show]   @sr/clock <name>=<source> <period>   @sr/migrate [dry|commit] [<who>]
@sr/profile <who>                                                              (all staff)
```

Every refusal renders through the economy's existence-hiding discipline: a run, party or profile the
actor may not view is "No such run." / "No such party." — identical for nonexistent and unviewable
(08a decision 13). `+run/show <id>` renders the board's own notice (name, state, window, condition
kinds, the Johnson when announced) under the board's visibility rule — faction membership through
`mush_faction`, never the contract's `Audience` (M5). A faction is not a principal: the audience
clause of `may_view_contract` is `acts_for`, which for an org means spenders, not members, and
`MAX_AUDIENCE` is 32. So `Public` = any profiled character; `Faction(f)` = `mush_faction`
`is_member`/`membership_visible`; `Invitation { party }` = the party's members and invitees; the
contract's own visibility is `Parties` until crewed and stays so — the crew are parties, so
`Audience { members }` is unnecessary and `announce_issuer` maps to the notice's `johnson` (G7).

**Refusals at post and crew** (M2, M6, M9, M16–M18, §11 q7). `+run/post` refuses an `adjudicator=`
naming the payer or the poster; any `before`/`after`/`fact` outcome row — E6a settles by attestation
(D-E6-20), `adjudicator=` defaulting to `party:<payer>`; a payer account the poster may not view; and
a `System` account, with that account's hiding text and no draft left behind (M4). On a refused
payer accept it cancels the draft (M17). `remainder_to` is inert under a table but must still
resolve to a live account in the instrument: it is the payer's (M18). `+run/apply` refuses a member
with no `sr.nuyen` account; `+run/crew accept` derives every `PayoutShare.to` from the roster
member's `SrProfile.account`, never from input (M6), refuses a party containing the adjudicator (M9),
and stores the account-distinctness verdict from the core/org oracle at the lock (doc 10 §15 R3,
G10). `+run/split` while Listed is an A′ amendment (`editable` refuses once anyone accepted); the
generation bumps only when a refund posted, so splits never exhaust `MAX_AMENDMENTS` (M16).

`+lifestyle/set` is refused while the current record is in `Grace`/`Delinquent`; `+lifestyle/pay` is
an attributed post of the arrears to `system:sr.lifestyle` under the player's own authority, then a
reinstate. The obligation itself is created through `create_obligation` (§2 item 3) — no
player-debit creator exists at HEAD — with the player's own debit authority at `+lifestyle/set` and
the approver's at chargen.

### 3.5 Chargen and migration

**Chargen (answer 17).** Until E6c's catalog exists there is nothing to shop for, so E6a ships the
**cash authority** and keeps today's behaviour otherwise: the approval handler
(`crates/ruleset-shadowrun/src/chargen_cmds.rs:450-477`, registered `lib.rs:262-270`, which today
writes only `approved`) gains a step that, atomically after its validation, opens the `sr.nuyen`
account and mints the priority allotment (`data.rs:1177`:
`[450_000, 275_000, 140_000, 50_000, 6_000]`) from `system:sr.chargen` — a **one-shot attributed
post** like WoD's (`wod/chargen.rs:149-222`, 08d dev. 10) with the approver as actor (nobody acts
for `System`) and the deterministic key `sr.chargen/{obj}`, recorded in `SrProfile.minted`. The
`[economy]` mint cap binds `create_system_obligation` only: 450,000¥ exceeds the 50,000 major cap,
so `MintCapExceeded` is unreachable at chargen and the bound is the priority table
(`resource_nuyen[A]`) plus E5c-M5's boot warning (M12). A refused mint never fails the approval
(08d deviation 11): the character is approved with an empty account and `@sr/migrate commit`
adopts them (M13). **The 5,000¥ carry-over cap is not applied yet** — answer 17: "until shopping
exists, keep current behavior and do not apply the cap." After approval `@sheet/set …/nuyen` is
refused (the field is the chargen budget and the migration source and nothing else) — the check
lives in `sheet_set`'s map arm (`sheet_cmds.rs:118-200`), keyed on `doc.approved`; `chargen_settable`
(`chargen_cmds.rs:542-560`, `+stat`'s in-chargen denylist) is untouched — `nuyen` must stay settable
in chargen, it is the Resources budget. `+sheet` and `+nuyen` render the account balance instead of
the stored number, viewer-aware (self + staff, `may_view_account`; omitted, never placeholdered —
the `wod/commands.rs:456` pattern, G2).

**Migration (answer 18).** `@sr/migrate dry` lists every approved character with a non-zero sheet
`nuyen` and the exact mint it would make. `commit` posts once per character under the deterministic
key `sr.migration/{obj}`, binding `from_sheet_value` and the approval stamp (`Approved { by, at }`,
`chargen_cmds.rs:449-477`); `SheetDoc` carries no revision (`mush-ruleset/src/doc.rs:60-70`). It
mints from `system:sr.migration` (**not** the chargen faucet) as the same one-shot attributed post,
writes `SrProfile.migrated`, and stops treating the sheet number as authoritative. `@sr/migrate
commit` also adopts an approved character whose profile records no chargen mint (M13). The verb is
E5c-D5's migrate shape (`wod/migrate.rs:109-131`: staff, one-shot, idempotent, reports what it did)
with `dry|commit` added because this one mints. A re-run is an idempotency HIT: it re-reports the
first mint and mints nothing. A sheet edited afterwards keeps the bound value and stamp and is
reported as a mismatch, never re-minted. Boot follows E5c-D6: a lifestyle obligation self-identifies
by `cause: Ruleset{"sr.lifestyle"}` plus credit = `system:sr.lifestyle`'s account, `SrProfile.upkeep`
is a cache, `sr::boot::run` adopts or cancels, and the identity rule (`obj == key`, loud skip) is
DL-07.

### 3.6 E6a tests (~40)

**Party:** create/invite/accept/decline/leave/kick/disband; size and invite caps; idle expiry; `Locked` refuses membership change; a substitution needs the amendment and returns `Forming`; reboot at every status.
**Board:** a post with no funds lists nothing and cancels the draft (M17); a listed run's escrow is real (assert `held_amount`); `MAX_OPEN_RUNS_PER_PRINCIPAL` and `BOARD_CAP` refuse the next post; an expired unbound listing closes and refunds; a faction-visible run is invisible to a non-member and renders a notice — never terms — to a faction member through `mush_faction` (M5); a non-viewer gets "No such run."; window budget: the fifth ≤ T run from an org account needs approval (DL-10); the org payer is notified through DL-04 when its run is crewed and settled.
**Money:** the split sums exactly and **no remainder reaches the fixer** (the outcome table makes `remainder_to` inert, `contract.rs:268-270`; it resolves to the payer's account — M18); a fixer fee is a named share that cannot widen after the lock; each outcome row pays exactly what it names; an advance comes back in the failure row through a share named to the payer's account; payout accounts are the members' own and a member without an account cannot crew (M6); payer-amend keeps escrow (A′); a fixer-issuer is refused at post (M1); an adjudicator naming the payer, or a party containing the adjudicator, is refused (M9).
**Faucets:** a `System` payer is refused at post with the account's hiding text and no contract was drafted (M4, D-E6-4); a campaign org funded by `create_system_obligation` posts; an over-cap `create_system_obligation` for that org is `MintCapExceeded` (the chargen allotment is not under the cap — M12).
**Settlement:** a `before`/`after`/`fact` outcome row is refused at post (M2); the payer's attestation settles the row; a deadline on a crewed run disputes, never refunds (M7); a `Draft` holding escrow past its deadline is expired by the sweep (M10).
**Lifestyle:** a paid period is silent; one miss notices (`Grace`) and does not downgrade; a second downgrades exactly one tier by replacing the record with `unpaid_periods` carried; `+lifestyle/set` in arrears is refused; a 10-period backlog with catch-up 1 downgrades nothing; ten forgiven downtime periods downgrade nothing; a tier change applies next period; a zero-cost tier creates no obligation; boot adopts a self-identifying record and cancels an orphan (DL-07).
**Chargen/migration:** approval opens the account and mints once, a second approval is a HIT, `@sheet/set …/nuyen` is refused afterwards while `+stat nuyen=` in chargen still works; `dry` writes nothing, `commit` mints once, a re-run returns the original result, an edited sheet is reported and not re-minted; a character approved with no mint is adopted by `commit` (M13).
Extend the shipped **Shadowrun run rig** (`crates/mush-economy/tests/e4b2.rs:167-213`, whose instrument is already `@instrument/register nuyen=0` at `:186`) with an `sr.nuyen`-registered ruleset world.

**Hazards on this axis.** *Board spam:* per-principal caps, escrow-at-publication, `BOARD_TTL` (= the contract deadline, M8), a page cap, and the rolling `window` on an org account against a ≤-threshold drip (M3, D-E6-21). *A fixer skimming:* every share is named and each row must sum — E4b2 already refuses an unmatched credit (`ContractPurpose::Settle` carries netted `(account, amount)` pairs, 08c §5.1); the fixer is never the issuer (M1) and payout accounts are derived, never input (M6). *An uncontrolled faucet:* D-E6-4 (a `System` payer refused at post, no draft left) plus the live mint cap on the campaign org's standing drip, re-read at every settlement; the chargen allotment is bounded by the priority table (M12).

---

## 4. E6b — outcomes, fences, custody

### 4.1 `scene.outcome` — the producer E4c's fold owes the economy

`mush-scenes` today produces exactly one canon record: `scene.concluded`
(`crates/mush-scenes/src/lib.rs:301-330`), `Restricted`, `entities = participants`,
`source: scene:{uid}`. E4c's fold disqualified it as evidence and left `with_canon_outcome_kinds`
empty on purpose (`crates/mush-server/src/modules.rs:151-157`), so every `canon:` attestor set is
refused at `@contract/offer` today. The tree proves the fold right and sharpens it: **any connected
player may `+timestop` in an unscened room (`crates/mush-scenes/src/commands.rs:102-104`) and is made
`facilitator` by that act (`commands.rs:160`)**, and `+timestop/end` needs only `driver_scene` —
facilitator or staff (`commands.rs:216-236`, `:389`). A seal signed by "the facilitator" alone would
be exactly as forgeable as `scene.concluded`. So the seal is a **conferred** authority:

```rust
// added to SceneRecord (crates/mush-scenes/src/data.rs:111-154), both #[serde(default)]
pub sealer: Option<ObjId>,          // conferred by staff; None means this scene CANNOT be sealed
pub outcome: Option<SceneOutcome>,  // written once, terminal, never edited
pub struct SceneOutcome {
    pub name: SmolStr,              // matches an outcome row ("success"/"partial"/"failure")
    pub subjects: Vec<ObjId>,       // named AT SEAL TIME — not copied from `participants`
    pub note: String,               // ≤ MAX_OUTCOME_NOTE_BYTES; chronicle text, never authority
    pub sealed_by: ObjId,  pub sealed_at: i64,
}
```

- **`@scene/facilitate <scene>=<who>`** (staff, the `Role::Royalty` check already at
  `commands.rs:46-48`) confers `sealer`. **`+scene/outcome <name>=<subject…> run=<id> [note]`** seals, only
  from `sealer` or staff, only once, only on an `Active`/`Paused`/`Closed` scene. A scene with no
  conferred sealer produces no outcome at all — fail-closed exactly like the empty table it replaces.
  (`+scene` is `ArgShape::None` with no switches today, `commands.rs:672-682`; the switch set is new.)
- The commit is a second `CanonDraft` beside `emit_scene_concluded`: `kind: "scene.outcome"`,
  `source: run:{contract id}` named by the sealer — a `scene:{uid}` is unknowable when the run is
  offered, and the economy compares `source` as an opaque string (`canon_set_accepts`,
  `contract.rs:1746-1753`; the `scene:{uid}` at `attest.rs:32-34` is a doc comment — M14),
  `entities: subjects ∪ {sealed_by}`, `scope: Restricted`,
  `detail: { scene, outcome, sealed_by, round }`. `ic_time` is the seal's own `ctx.game_now_unix()`,
  **not** `scene_ic_time()` (`data.rs:175-178`), because a long timestop's scene clock advances only
  `round_seconds × (rounds − 1)` and nothing floors it — a stale stamp would chronicle a settlement
  before events it followed (doc 10 §15/6A puts the general scene time floor in E7).
- **`entities` is named, not inherited.** `scene.concluded` copies `participants` (`lib.rs:319`),
  frozen at scene creation with no add path (`commands.rs:119-141`; a later arrival is only logged
  `Joined`, `lib.rs:552-561`). The economy requires the record's entities to name or act for a
  contract party (`attest.rs:217-226`, `names_a_party` at `attest.rs:249-255`), so a runner who joined
  mid-scene would silently fail to match. Naming subjects at seal time makes the match reliable *and*
  is what answer 6 asks for.
- `mush-server` then registers `with_canon_outcome_kinds(vec![SmolStr::new("scene.outcome")])`
  (`modules.rs:157`). `scene.concluded` is never registered.

**Review amendment (0.2, re-grounded at 0.3 — M14) — the seal is not a per-run staff gate.** A
staff-conferred `sealer` would put one staffer in front of every run (08a decision 25, the
staff-scarce rule), and the run's **named adjudicator** (consented at offer — answer 6 — and never a
party or the poster of that run: answer 22) already ends the run by `@contract/attest`, replacing
every condition's attestor set (D-E6-8; the E6a route, M2). The seal is the *canon* route beside it,
signed by a staff-conferred `sealer` or staff. The seal names its run: `source: run:{contract id}`
(a `scene:{uid}` is unknowable at offer), `entities = subjects ∪ {sealed_by}`. Who may seal is the
producer's gate (adjudicator identity is not visible to `mush-scenes`, which has no economy edge —
§2); the economy checks kind, source and party intersection as today. A `detail`-bearing
`CanonRecorded` (today it carries `{canon_id, kind, entities, source}` only,
`mush-core/src/module.rs:523-533`) or DL-09's read seam is what an economy-side `sealed_by` check
waits on. Staff conferral remains for scenes no contract names.

### 4.2 The four settlement routes, one alphabet (answers 6, 19)

| Route | The condition E6b writes | Supplied by |
|---|---|---|
| Sealed scene outcome | `Attestation { by: Canon { kind: "scene.outcome", source: "run:{contract id}" } }` — source **mandatory** (`contract.rs:1553-1557`) | a conferred sealer or staff (§4.1, M14); the adjudicator's own route is the next row |
| Authorized attestation | `Attestation { by: Party(p) \| Capability{scope,capability} \| Staff }` via `@contract/attest` | the adjudicator named at offer |
| Deterministic | `Fact` / `TitleHeld` / `DepositInto` / `Before` / `After` — E6b onward: E6a has no `sr.*` provider, and a bare `Before` row is `Ready` at the last acceptance (M2, D-E6-20) | the journal, or an `sr.*` `FactProvider` |
| Composite | several of the above; outcome rows index the ones deciding *which* result | — |

The **named adjudicator replaces** each condition's own attestor set, for canon and people alike
(08c §5.1 dev. 4, §6.1 dev. 3), so `+run/post adjudicator=` is what actually binds who may end the
run, and the runners consent to it at acceptance. **Prose settles nothing:** a canon record with no
matching typed condition attests nothing whatever it says, and an LLM ST's proposed outcome reaches
the world only as a `+scene/outcome` a conferred human seals (answer 23).

### 4.3 Fences, custody, and title (answers 13–15)

```text
+fence/quote <fence>=<obj> [immediate|consign]   a deterministic FenceQuote on a declared clock
+fence/accept <quote>                            drafts + offers; the FENCE accepts FIRST (escrow)
+fence/deliver <id>                              refused unless the contract is already Active (D-E6-9)
+fence/list   +fence/show <id>
@asset/stolen <obj>=<public|restricted|covert> [reason]        the typed report (economy; §2 item 2)
```

- **Escrow before custody (answer 15).** `+fence/deliver` refuses while the contract is `Offered`,
  because E4b2 custody-locks a `DepositInto` object only for an `Active`/`ReadyToSettle` contract
  (08c §5's boot re-derivation) — a seller delivering early would hand over an unlocked, unpaid
  object. The fence's `Consideration` is escrowed at their accept; only then does the runner deliver.
- **Immediate vs consignment.** The immediate quote settles on `DepositInto` alone for less; the
  consignment adds `After { condition: <deposit idx>, delay, clock: RealUtc }` for more — exactly the
  shape the shipped fence test drives (`crates/mush-economy/tests/e4b2.rs:778-800`, with
  `@contract/condition {id}=after 0 600`). `fence_delay_secs` bounds the delay to 24–72 h and the
  offer shows it. Notices fire only on custody, settlement, cancellation and dispute (answer 15).
- **Title ≠ possession (answer 13).** The deliverable is `DepositInto`, **never** `TitleTransfer`:
  the fence buys custody. `Encumbrance::Disputed` and the original title survive; the settlement's
  `AcquisitionRecord` is evidence, not guilt (answer 14).
- **Deterministic pricing, strict visibility (answer 13).** `FenceQuote { id, fence, seller, object,
  catalog, offer: Amount, basis: Vec<PriceFactor>, settlement: Immediate | Consign { delay },
  expires: EconomicDeadline, observed: RevisionSet }`. `PriceFactor` is computed **only** from what
  the fence may observe (the asset record under `may_view_*` rules, the seller's presentation, the
  catalog baseline) plus a declared legality/demand table. A provenance fact the fence may not see
  contributes **nothing — not a discount** — so a low quote is never an oracle for hidden state.

**Re-grounded at 0.3 (M15, M19, G3, G4).** A staff-built NPC fence has no officer, and an org party
accepts only through `acts_for` holders or staff (`contract_live.rs:337-345`), which would put staff
in every deal: the fence accepts **programmatically** through DL-11's vendor accept
(`with_party_caps`), an E6b item (§2 item 7). A vendor org's consignment deposit is stranded at
unwind today — `refund_plan` returns deposits to `Object` depositors only and `authorize_vault`
withdraws only to `Object` (E4b2-D28) — so the DL-03 seam (contract-scoped `Deposit` authority for
the counterparty vault plus a `Withdraw` destination of an org-owned vault's backing;
`destination_ok` admits it) lands before consignment ships. `Organization.hidden` is honoured
nowhere the economy renders (E1-D18/D19/D29, doc 08 §17.2 #12, no test): an org party, vendor or
fence renders through one `render_org_listed(ctx, org)` — its name iff `!hidden || staff ||
member/officer`, else "an organization" — and a hidden fence's quotes are absent for non-members,
never "No such"; the first scenario-12 test lands with `+fence/list`. The fence's money is sound as
written: its escrow refuses on `InsufficientAvailable`, so "stops quoting" is automatic (M19).

### 4.4 Augmentations (answer 12, minimal)

`schema.rs:275-279` stores `augmentations` as a free-text `Map<Number 0..600>` valued in Essence ×100,
and `essence_from_augmentations` (`derive.rs:211-217`) runs **only** from chargen finalize
(`chargen.rs:645-646`) — `apply_derived` deliberately does not touch Essence (`derive.rs:184-187`),
so a post-approval `@sheet/set augmentations:` already desyncs Essence today. E6b therefore ships
only: an `InstalledAugmentation { object, catalog, grade, essence_cost, clinic, acquisition:
AcquisitionRecord, installed_at }` on `SrProfile`; the rule that **a service contract's settlement
never writes the sheet**; and the refusal that keeps Karma costs on `+karma/spend`'s job flow
(`crates/ruleset-shadowrun/src/karma.rs:146-218`, queue `shadowrun-karma`, handler registered
`lib.rs:272-281`; no karma↔nuyen path exists, `karma.rs:360-442`). The atomic installation action —
and moving the Essence recomputation onto every write — is the later clinics slice.

### 4.5 E6b tests (~26)

**`scene.outcome`:** a scene with no conferred sealer cannot be sealed and its contract never settles; a non-sealer non-staff player is refused; a second seal is refused; `kind`/`source`/`entities` are exactly what the observer matches and a seal naming another run attests nothing (M14); a runner who joined mid-scene is named in `subjects` and matches, one omitted does not; `scene.concluded` from the same scene attests nothing; a `canon:scene.outcome` set with no source is refused at offer (`contract.rs:1553-1557`); the seal's `ic_time` is the world IC clock, not the frozen scene clock; a sealed outcome selects the matching row and pays it; two rows satisfied at once reads `Undecidable` and disputes rather than freezing.
**Routes:** each of the four settles; a composite needs every gating condition plus one row; the adjudicator replaces a runner's own attestor set; an unregistered canon kind is refused at offer.
**Fence:** `+fence/deliver` refused while `Offered`; the consignment matures only after the delay; the immediate sale settles at once; the seller's title and `Disputed` encumbrance survive settlement; two worlds differing only in hidden state produce identical quotes; an expired quote is refused (scenario 29's shape); a hidden fence org renders as "an organization" to a non-member and its quotes are absent (G3, scenario 12); the fence accepts with no officer connected (M15); an org fence's consignment deposit comes back at unwind (DL-03).
**`@asset/stolen`:** the owner, an insurer and an authority may file, a bystander may not; covert visibility is invisible to the thief; a later purchase generates evidence and does not clear the claim.
**Augmentation:** settlement writes the record and not the sheet (assert `sheet:shadowrun` unchanged across every test in the file).

**Hazards on this axis.** *Prose settlement:* a canon record with no matching typed condition is inert, and the seal is conferred. *Fence laundering:* the deliverable is custody and `TitleTransfer` is never in a fence template. *Consignment custody races:* the deliver-refusal before `Active`, the `DepositInto` lock while live, and E4b2's unwind returning each recorded deposit from the journal's `last_custody` (08c §5.1).

---

## 5. E6c — the market

### 5.1 Catalog and representation policy (answers 9–10, doc 10 T5)

```rust
pub struct CatalogEntry {                 // `market:catalog:{pack}:{code}`
    pub catalog: CatalogRef,              // { ruleset, code } — the economy's opaque precursor (record.rs:556-562)
    pub pack: SmolStr,
    pub definition_version: u32,          // bumped by the pack; a quote records the version it priced
    pub name: SmolStr,
    pub baseline: Option<Amount>,         // a REFERENCE price, never an intrinsic one (doc 08 §9.4 / D9)
    pub availability: AvailabilityRule,   // None | Test { skill, threshold, interval } | Attested
    pub legality: Legality,               // Legal | Restricted | Forbidden
    pub representation: Representation,   // THE POLICY DECISION
    pub stackable: bool,  pub tags: BTreeSet<SmolStr>,
    pub rules_ref: Option<SmolStr>,       // a book/page citation, never mechanics
}
pub enum Representation {
    Instrument,   // fungible commodity: an InstrumentKind::Commodity account (record.rs:52-61)
    Lot,          // provenance-bearing: a Lot with immutable lineage (doc 10 T5)
    Asset,        // an EconomicAsset with title, custody, encumbrances (E3)
    Entitlement,  // InstrumentKind::Entitlement — licences, programs; non-transferable
    Installed,    // becomes an InstalledAugmentation/component on installation (§4.4)
}
pub struct Lot {                          // `market:lot:{id}`
    pub id: LotId, pub catalog: CatalogRef, pub owner: PrincipalId,
    pub quantity: u64, pub grade: u8, pub legal: Legality,
    pub parents: Vec<LotId>,              // split → one parent; merge → all. NEVER discarded (T5)
    pub created_at: i64, pub revision: u64,
}
```

The policy is decided **per entry by category and gameplay significance, with price only as a hint**
(answer 9): important, serialized, restricted, upgradeable or stealable gear is `Asset`; ammunition,
drugs, reagents and commodities are `Instrument`, or `Lot` where grade, source or legal status
matters (T5); licences and programs are `Entitlement`; cyberware is `Installed`. The starter pack is
small and representative — weapons, armour, drugs, programs, a deck, magical goods, a service,
cyberware — never an exhaustive transcription (answer 10), and vendors produce offers *from* the
catalog without mutating an entry. Split and merge write child lots naming their parents; **lineage
is append-only and never discarded**, though a rendered view may coarsen it (T5). Coarsening rules
and legal-status propagation beyond the starter set are E7's (doc 10 §16 item 2).

### 5.2 Listings, quotes, vendors

```rust
pub struct Listing { pub id, pub seller: PrincipalId, pub catalog: CatalogRef, pub source: Supply,
                     pub price: Amount, pub quantity: u64, pub visibility: MarketVisibility,
                     pub expires: EconomicDeadline, pub revision: u64 }
pub enum Supply { Stock { vault: VaultId },                 // held in the storefront vault
                  Market { source: MarketSourceId },        // a registered NPC market source
                  Procurement { lead: EconomicDeadline } }  // a delayed delivery contract
pub struct Quote { pub id, pub listing, pub buyer: PrincipalId, pub price: Amount,
                   pub definition_version: u32, pub observed: RevisionSet,
                   pub expires: EconomicDeadline,           // ITS CLOCK IS DECLARED (answer 11)
                   pub availability: Option<AvailabilityOutcome> }
```

**Quote expiry declares its clock** (answer 11): `QUOTE_TTL_LIVE` ≈ 1 h on `RealUtc` for an in-person
session, `QUOTE_TTL_REMOTE` ≈ 24 h for asynchronous offers, both configurable. A quote whose
`definition_version` or `observed` revisions have moved is refused **deterministically**, not
re-priced (scenario 29). Stocked legal goods need no availability test; scarce or restricted goods
run an **engine-resolved, recorded** Shadowrun test through the ruleset's own dice
(`crates/ruleset-shadowrun/src/roll.rs`), stored on the quote; ST attestation is only for exceptional
or authored availability. Accepting a quote drafts an ordinary E4b2 contract — consideration, a
`TitleTransfer` or `DepositInto` deliverable, a `Before` deadline — so E6c adds no settlement
machinery at all. **Only the issuer offers** (E4b2's rule), so a buyer
accepting a quote is the issuer and the vendor accepts programmatically through DL-11's
`with_party_caps`, landed with E6b's fence and reused here (G8); a hidden vendor renders through
`render_org_listed` and its listings are absent for non-members, never "No such" (G3). **NPC market makers (doc 10 7A):** `MarketSource { principal: OrgId, cash:
AccountId, inventory: VaultId, spread, volume_budget, loss_budget, window }`. A maker quotes only
inside its budgets and **stops quoting** when cash or inventory runs out. Funding is explicit —
world-start capitalization, sales, or a `create_system_obligation` drip from a registered faucet
under the `[economy]` mint cap — never an unlimited account (7A; answer 4's rule applied to NPCs).

### 5.3 The safe public projection (doc 10 §14)

The projection is E6c's gate and is deliberately **aggregate**: per `(catalog, market)` it publishes
last trade, a bounded price band, depth by price level, and volume over a window. It publishes **no
account id, no listing id the viewer may not see, no counterparty, and no quantity attributable to
one seller**, and it coexists with the existence-hiding renderer rather than bypassing it — a listing
the actor may not view is "No such listing.", identical for nonexistent and unviewable, and an
unauthorized filter returns an empty list (08a decision 13). Depth is bucketed so a single
distinctive quantity cannot identify its owner. That is the whole of doc 10 §14(b)'s "explicit safe
projection, not the full belief engine".

```text
+market/list [tag=|legal=]   +market/show <catalog>   +market/depth <catalog>
+market/quote <listing>[=<qty>]   +market/accept <quote>   +market/sell <obj>=<vendor>   +market/orders
@market/catalog load|list|show <pack>   @market/vendor create|fund|stock|retire <vendor>
@market/source create|budget|halt <source>                                        (all staff)
```

### 5.4 E6c tests (~28)

**Catalog:** a pack loads, versions and reloads idempotently; a bad entry fails the pack, not the server; each `Representation` produces the right artefact on purchase (a `Commodity` account line, a `Lot`, an `EconomicAsset` with title, an `Entitlement`, an `InstalledAugmentation`); a split then merge keeps every parent in the lineage and a coarsened render never loses it.
**Quotes:** expiry on each declared clock; a stale `definition_version` refuses; a stale `observed` revision refuses; an availability test is recorded and a failure refuses without charging.
**Vendors:** stock, market-source and procurement each settle through a real contract; a procurement's lead time is a `Before` deadline.
**Projection:** two worlds differing only in hidden listings publish identical projections; a single large listing is not identifiable from depth; an unviewable listing renders "No such listing."; filters return empty rather than refusing.
**Makers:** a maker stops quoting at its cash, inventory and loss budgets; funding above the mint cap is refused; a halted maker's outstanding quotes expire and settle nothing.

**Hazards on this axis.** *Market existence oracles:* aggregate-only projection, bucketed depth, the shared existence-hiding renderer. *Stale quotes:* version + revision binding, refused deterministically. *Insolvency:* explicit budgets checked before every quote, no faucet behind them.

---

## 6. Build discipline

As 08c §7 and 08a's standing rhythm, per cut: (1) confirm the cut's slice of this plan against the
tree; (2) build, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`;
(3) **a probe-confirming adversarial verify of that cut** — three lenses, throwaway probe tests —
before the next cut builds on it; (4) fold every confirmed finding with a regression test named
`vf_<lens><n>_…`; (5) commit with the verify outcome in the message; **push only on the owner's
word**. Both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) stay green in every cut, and
**`cargo fmt` stays off** (08a).

## 7. What E6 feeds back

1. **Per-party amendment binding (E4).** 08c §5.1 deviation 3's open question, answered by use: a roster substitution is the common case and voiding every acceptance to serve it is wrong. E6a ships **A′** — consent by authorship, the amending party's own acceptance survives (§1.1, §2 item 1) — which covers the payer's crewing; a substitution after the runners accepted still voids theirs. Full per-party binding (a per-party sub-hash over the surface slices that party is bound to) stays an E4 consent-model change.
2. **A real consumer for `ExpenseTreatment` (E4).** E6a uses `Retained`/`Returned` through explicit payouts; `Reimbursed` still has none until receipts exist (answer 7).
3. **Participation fact kinds (E7; doc 10 §15 R6/T13).** E6a's frozen roster is the Shadowrun analogue of `sim.station.served`: register `sr.run.participated { party, actor, from, to }` beside the T2 boundary facts, answered from an **applied marker**, never a status (08c §6.2). Doc 10 §15 also gets (G10): account-distinctness at roster lock (R3, C23/C24) — `+run/crew accept` calls the core/org boolean oracle and stores the verdict; `consent_hash` goes `pub` for T10's consent grants; `sr.run.participated` stays E7 and is answered from `Settled{transaction}`, never from board state.
4. **The `scene.outcome` record shape (E7).** Doc 10 T6's declared operations and §16 item 5's campaign layer need the same thing — an outcome-bearing, conferred-authority seal naming its subjects — and should reuse it. The **scene IC time floor** (doc 10 §15/6A) is the proper fix for §4.1's `ic_time` workaround. T10's proxy-seat invitation is the same audience-visibility contract template as an invitation-only run board entry.
5. **`report_stolen` is generic (E5/E7).** Insurance (doc 10 §14) and salvage claims consume the same `Disputed` encumbrance. **`mush-market` serves WoD and TacSim** — answer 20's whole point.
6. **`@econ/cadence` shipped in E5b** (§11 q2); E6a tests E4b1's B6/S2 against real clocks, and `sr.month` becomes a declared cadence at D-LBK-7's convergence (§9).

## 8. Decisions

- **D-E6-1 `sr.nuyen` is scale 0 and the account is authoritative after approval** (answer 1); the sheet field survives only as a chargen budget and migration source; the instrument renders through `InstrumentDisplay { symbol: "¥", position: Prefix }` (E2-C16, 08g §12.1 item 4).
- **D-E6-2 Karma never touches the economy** (answer 2): no instrument, no conversion, no deliverable.
- **D-E6-3 Lifestyle is one fixed-amount obligation on an `sr.month` cadence**, created through `create_obligation` (`cause: Ruleset{"sr.lifestyle"}`), and a downgrade is driven by a *missed* period, never a forgiven one, at most one per period (answer 3). `Obligation.amount` is a fixed `Amount`, so a tier change replaces the obligation rather than scaling it — **and so does the downgrade**: `Delinquent` leaves the due pool and autopay stops (`obligation.rs:171-173`, `sweep.rs:459-461`), so the second miss replaces the record one tier down with `unpaid_periods` carried; `+lifestyle/set` is refused in `Grace`/`Delinquent`, so a tier change can never wipe a miss; the schedule's `maximum_catch_up` is 1, so a backlog is one attempt and never a second miss (M11, D-E6-23).
- **D-E6-4 A campaign or NPC principal that funds a run is an `Organization`, not a `System` principal** — funded by `create_system_obligation` under the `[economy]` mint cap (answer 4, doc 10 7A). The tree makes a `System` payer *unfundable* (nobody acts for one, staff included — 08c §4.7's verify fold) but *offerable*: `check_structure_basic` refuses no `System` party or `from`, `parse_principal` accepts `sys:`, and the offer would stand as a zombie nobody can accept and nothing expires (M4). E6a refuses it at post — decided on the account owner before any draft exists, rendered as that account's existence-hiding text, never a faucet-shaped message (E4c D1), tested by "no contract was drafted" — and the fold refuses it in `check_structure` (§2 item 9). Faucets mint *into* org accounts; they are never contract parties.
- **D-E6-5 The party is a record, not a principal, and the contract never points at it** (answers 5, 19, 21; 08c §5.1's E6a handoff). The roster freezes *into* the consent surface as parties and named payout accounts; `Party.status = Locked { contract }` is the party-side back-reference only, set by the ruleset's own seam at `+run/crew accept` (§3.2) and released by its `EconomyObserver` on the contract's terminal `ContractChanged` — the same observer mirrors `Active` to the board's `Crewed` (the housing precedent, `mush-housing/src/econ.rs:96-108`; the wiring of §2 item 8).
- **D-E6-6 A run always carries an outcome table, even with one row** (answers 7–8) — the only way to make "no unspecified remainder to the fixer" structural, since `remainder_to` is inert whenever `outcomes` is non-empty (`contract.rs:268-270`) and each row's payouts must sum exactly.
- **D-E6-7 `scene.outcome` is sealed by a staff-conferred sealer or staff — never a self-granted facilitator — and names its run** (answer 6; E4c fold E1; §4.1 re-grounded at 0.3): `source: run:{contract id}`, because a `scene:{uid}` is unknowable at offer, and `entities = subjects ∪ {sealed_by}` named at seal time, not copied from `participants`. The named adjudicator ends the run by `@contract/attest` (D-E6-8) and is never a party or the poster (answer 22); an economy-side `sealed_by` check is unwritable today (`CanonRecorded` carries no `detail`) and waits on a `detail`-bearing event or DL-09's read seam (M14). Any player can become a facilitator by starting a scene (`commands.rs:102-104,160`); a seal by anyone else attests nothing.
- **D-E6-8 The named adjudicator is the run's settlement authority** (answer 6), replacing each condition's own attestor set for canon and people alike (08c §5.1 dev. 4, §6.1 dev. 3).
- **D-E6-9 A fence escrows before it takes custody** (answer 15): `+fence/deliver` is refused while the contract is `Offered`, because the `DepositInto` lock exists only for a live contract.
- **D-E6-10 A fence buys custody, never title** (answer 13): the deliverable is `DepositInto`; the title record and any `Disputed` encumbrance survive settlement; laundering is later and explicit.
- **D-E6-11 A fence quote is computed only from observable inputs** (answer 13) — hidden provenance contributes nothing, not even a discount, so a quote is never an oracle.
- **D-E6-12 Representation is catalog policy** (answer 9, doc 10 T5): `Instrument | Lot | Asset | Entitlement | Installed`, by category and significance with price as a hint. Lot lineage is append-only; coarsening rules past the starter pack are E7's.
- **D-E6-13 Every quote declares the clock its expiry is on** (answer 11); a stale definition version or observed revision refuses deterministically rather than re-pricing.
- **D-E6-14 The public projection is aggregate and shares the existence-hiding renderer** (answers 11, 20; doc 10 §14): no ids, no counterparties, bucketed depth.
- **D-E6-15 NPC market makers and campaign budgets are explicit and exhaustible** (doc 10 7A): a maker that runs out stops quoting, and nothing recapitalizes it without an attributed act.
- **D-E6-16 Payment never edits a sheet** (answer 12): settlement writes an `InstalledAugmentation`; the ruleset's own installation action — later — writes the sheet.
- **D-E6-17 The run board is ruleset-side, not `mush-market`** (answers 20, 24): a run offer is a contract with an audience notice, not a listing of goods, and E6a ships two cuts before `mush-market` exists. Moving it later is a projection change, not a data migration.
- **D-E6-18 `report_stolen` is a `mush-economy` action** (answer 14, 08e §4 item 6) — generic, Shadowrun-motivated, relaxing E3 invariant 19 for the title owner, an authorized insurer, or an authority; `Encumber` otherwise stays staff-only.
- **D-E6-19 The run's issuer is its payer** (M1). `@contract/amend` is issuer-only and A′ keeps only the amending party's acceptance, so a fixer-issuer would refund the payer's escrow at every `+run/crew accept` and leave a `Listed` entry standing on nothing. Personal: poster = payer, from their own account. Org: `scope=` makes the org the issuer; the posting officer holds `contract.draft`, `contract.accept` and `economy.account.post` on the scope. A fixer is `roles.author`/`representative` with a fee row, never the issuer. The roles rule in §3.3 (owner, 0.3) governs which roles may coincide.
- **D-E6-20 E6a settles by attestation only** (M2): outcome conditions are `Attestation`, `evidence=attest` is forced, `adjudicator=` defaults to `party:<payer>` (the shipped rig's shape, `e4b2.rs:209`), and `+run/post` refuses `before`/`after`/`fact` rows — `Fact` resolves `None` with no `sr.*` provider, no canon kind is registered until E6b, and a bare `Before` row is `Ready` the moment the last runner accepts and the sweep would pay it. The other routes arrive with E6b (§4.2).
- **D-E6-21 `ApprovalRule` gains a journal-derived rolling `window { total, period_secs }`** (DL-10, 08g §12.1 item 2): the account's debits plus placed holds over the trailing period, a refund never re-opening it; over the window ⇒ `NeedsApproval` on the threshold's own path. It closes the ≤-threshold drip against an org treasury (M3). No default — an account without a `window` keeps today's behaviour; organization run offers stay on.
- **D-E6-22 A crewed run's deadline disputes, never refunds** (M7): posted with `on_expiry = Dispute`, so a Johnson who simply never attests cannot recover the escrow by silence — bound parties are paged and agree; an unbound (`Listed`/`Crewing`) run refunds regardless. The board's `expires_at` is the contract's `consent.deadline` on `RealUtc` (M8): nothing else expires an `Offered` contract holding escrow, and `page_stale_contracts` ignores `Offered`.
- **D-E6-23 The lifestyle schedule's `maximum_catch_up` is 1, and a downgrade is a replacement** (M11, G12): one fire is one attempt, so a downtime backlog can never be the second miss; the `sr.month` clock's own `catch_up` (3) forgives on the clock first; the second miss replaces the record at the new tier through `create_obligation` with `unpaid_periods` carried, and `+lifestyle/set` is refused in arrears.

## 9. Constants

`MAX_PARTY_SIZE` 8 · `MAX_PARTY_INVITES` 16 · `MAX_PARTY_NAME_BYTES` 48 · `PARTY_IDLE_TTL` 7 d ·
`MAX_OPEN_RUNS_PER_PRINCIPAL` 4 (inside E4's `MAX_OPEN_CONTRACTS_PER_ISSUER` 16) · `BOARD_CAP` 128
open listings · `BOARD_PAGE` 20 · `BOARD_TTL` 14 d — **the contract's `consent.deadline` on
`RealUtc`** (M8), posted with `on_expiry = Dispute` (M7) · `MAX_RUN_OUTCOMES` 5 (inside E4's
`MAX_OUTCOMES` 8) · lifestyle tiers `street|squatter|low|middle|high|luxury`
(`crates/ruleset-shadowrun/src/data.rs:225-232`), one downgrade step per unpaid period, arrears
notices capped at 3 then silent until the level-2 escalation · **`LIFESTYLE_CATCH_UP` 1** — the
lifestyle schedule's `maximum_catch_up` (M11, D-E6-23) · `MAX_OUTCOME_NOTE_BYTES` 240 ·
`QUOTE_TTL_LIVE` 1 h (`RealUtc`) · `QUOTE_TTL_REMOTE` 24 h · `MAX_LISTINGS_PER_VENDOR` 64 ·
`MAX_LOT_PARENTS` 16 · `MAX_CATALOG_ENTRIES_PER_PACK` 512 · `DEPTH_BUCKETS` 8. No longer a
constant: `STALE_CONTRACT_AGE_SECS` (7 d) becomes `[economy].stale_contract_age_secs` and the paging
skips a contract with an unlapsed deadline (DL-08). `ApprovalRule.window` has **no default** — an
account without one keeps today's threshold-only behaviour (DL-10).

**`sr.month` and the two catch-ups (G12, D-LBK-7).** `sr.month` is created at runtime by `@sr/clock`
over `create_cadence` (`api.rs:1365-1425`) and stays a legacy `EconomicClock::Cadence(id)` until the
convergence registration makes it a declared cadence (`[economy.cadences]`, 08h §2 Q6). Two caps
apply, and both are said so neither is mistaken for the other: the clock-level `catch_up` (3, §11 q1)
*forgives* on the clock (`clock.rs:276-295`); the lifestyle schedule's `maximum_catch_up` (1) decides
*attempts* in one fire.

**Not constants — `[shadowrun.economy]` config** (answer 16): `enabled`, `instrument`,
`lifestyle_clock`, `lifestyle`, `player_run_offers`, `organization_run_offers`, `chargen_shopping`,
`catalog_paths`, plus `lifestyle_costs` per tier, `quote_ttl_live`/`quote_ttl_remote`,
**`fence_rate_immediate`/`fence_rate_consign`/`fence_delay_secs`**, `active_offer_cap`, and
`chargen_carryover_cap` (parsed but **unapplied** until shopping ships — answer 17). Parsed host-side
like every other block: `mush-config` `ShadowrunCfg` (the `EconomyCfg` pattern,
`crates/mush-config/src/config.rs:334-358`) → `mush_server::config_ext::shadowrun_config`
(`crates/mush-server/src/config_ext.rs:41` is the precedent) → `ShadowrunRuleset::with_config`
(`crates/ruleset-shadowrun/src/lib.rs:126-135`, which has **no external caller today**;
`modules.rs:315` constructs with `::new()`).

**Partial-switch table** (G9; E5c fold D2's `careers=false` precedent). Every row says what it stops
and what it never touches; no row ever touches an account, a balance or a live contract:

| Switch | `false` stops | Never touches |
|---|---|---|
| `enabled` | registers the verbs, creates nothing, evaluates nothing (E5c dev. 1) | existing accounts, balances, live contracts and obligations — they settle through E4 as before |
| `player_run_offers` | `+run/post` from a personal account, refused in the off-mode words | listings by orgs or staff; live runs |
| `organization_run_offers` | `scope=` posts | live org runs — they settle |
| `lifestyle` | no lifestyle obligation is created; existing ones are cancelled by `@sr/migrate` (never by boot); tiers stay cosmetic | accounts, balances, arrears already posted |
| `chargen_shopping` | parsed, inert until E6c | — |
| `fence_rate_*` | parsed, inert until E6b | — |

## 10. The owner's answers (08e §3, 2026-09-03 — binding)

1. `sr.nuyen` at scale 0; after chargen the account is authoritative, the sheet renders a derived balance; `system:sr.chargen` funds starting characters.
2. Karma stays separate — progression currency on its existing approval flow, never economic money.
3. Lifestyle is an automatic E4 obligation on an explicit `sr.month` campaign clock; autopay default; one failed period → notice/grace, a second may downgrade; downtime or a clock jump must never cause multiple surprise downgrades; physical housing stays separate.
4. Players, authorized org members, staff, player STs and campaign systems may create runs; **nobody from an uncontrolled faucet**; a Johnson is the visible representative, not necessarily payer or author, and the contract records the accountable principal.
5. The team is an ephemeral party, not an organization or principal; acceptance freezes a roster snapshot and payout shares; substitution needs an amendment and affected-party consent; payouts settle to individual accounts.
6. A typed scene-outcome / attestation seam — sealed scene outcome, authorized attestation, deterministic condition, or composite; **the offer names its adjudicator and permitted evidence before acceptance**; prose alone never releases escrow.
7. Freeze the split when the roster locks; exact shares must sum; **no unspecified remainder to the fixer**; separate frozen slots for payout, fee, advance, reserve, bonuses/penalties; fixed advance first, receipts later.
8. A bounded, data-driven outcome table in frozen terms — `failure`/`partial`/`success` as defaults, never hard-coded; each row names payouts, expense treatment and asset consequences.
9. Representation is catalog policy: important/serialized/restricted/upgradeable/stealable goods become assets; ammunition, drugs, reagents and commodities become bounded stacks; licences and programs may be entitlements; cyberware becomes an installed record retaining provenance.
10. A small, representative, source-controlled starter catalog; full catalogs are versioned packs; entries carry stable ids, definition versions, baseline price, availability, legality, representation kind, stackability, tags and rules references; vendors never mutate entries.
11. Vendor accounts, quotes and contracts; stock, a registered market source, or delayed procurement; **quote expiry declares its clock** (~1 h live, ~24 h remote); scarce or restricted goods use an engine-resolved recorded test; ST attestation only for authored availability.
12. Cyberware and clinics are a later slice; **payment alone never edits the sheet**; completion invokes a ruleset-owned installation revalidating Essence, prerequisites, custody, clinic capability, payment and sheet revision atomically; Karma costs are never bypassed.
13. Deterministic first-pass fence pricing with strict visibility; **title ≠ possession** — a fence may pay for custody while title and the theft claim stay attached; laundering is later and explicit.
14. A typed `report_stolen` by the title owner, an authorized insurer, or an authority, with public / restricted / covert visibility; later transfers generate evidence, not guilt; heat and notoriety later.
15. Configurable delayed fence settlement (24–72 h, shown in the offer); **the fence reserves or escrows before taking custody**; immediate-for-less vs consignment-for-more; notices only on custody, settlement, cancellation or dispute.
16. `[shadowrun.economy]` owned by the Shadowrun configuration (`enabled`, `instrument`, `lifestyle_clock`, `player_run_offers`, `organization_run_offers`, `chargen_shopping`, `catalog_paths`, plus quote expiry, offer caps, fence delay and lifestyle costs).
17. Chargen shopping and the 5,000¥ carry-over cap with **one live cash authority**: approval atomically opens the account, mints the priority allotment, pays chargen vendors, creates inventory/titles/installed records and leaves no more than the cap. **Until shopping exists, keep current behaviour and do not apply the cap.**
18. Migration is idempotent and inspectable, funded by `system:sr.migration`: a dry run shows each planned mint; the commit binds the old sheet value and revision, mints once, and stops treating the sheet number as authoritative; re-running returns the original result.
19. Cuts by actual prerequisite, and **no team vaults**: E6a after E4b2 (accounts, migration, lifestyle, representation, quotes, direct vendors, chargen shopping); E6b after the attestation seam (board, authored runs, parties, frozen rosters and splits, outcome settlement, basic clinics); E6c after knowledge support (belief-sensitive fences, heat, notoriety, investigations, title laundering).
20. A neutral `mush-market` crate owning bounded catalogs, listings, quotes, orders, expiry and vendor interfaces; the economy keeps accounts, contracts, title and vaults; rulesets own prices, availability, legality, item meaning and content.
21. A lightweight party subsystem — leader, accepted members, pending invitations, optional roles, revision, lifecycle, expiry. Not a principal; owns no money or title.
22. Separate run author, payer, Johnson, facilitator and adjudicator; they may coincide, but the data model must not assume it.
23. LLMs draft; authorities commit. An LLM ST may generate seeds, dialogue, complications and proposed outcomes; a registered campaign policy may authorize bounded recurring offers from a capped system budget; the LLM cannot invent funds, alter frozen terms, or settle on narration.
24. Bound the player-created run board: escrow required at publication, active-offer caps per principal, expiry of abandoned listings, and public / faction-visible / invitation-only runs.

## 11. Open questions (defaults chosen — **accepted by the owner 2026-09-03**; q2 resolved to E5b; all binding as written)

Only what the twenty-four answers and doc 10 leave open. Each default is what the build takes unless
the owner says otherwise.

1. **`sr.month`'s source and period.** Default `CadenceSource::WorldIc`, a 30-IC-day period, `catch_up_cap` 3. Answer 3 requires an explicit campaign clock and forbids a literal calendar; it does not pick the source. A `Manual` cadence advanced by staff is the alternative. *(0.3, G12: `sr.month` stays a legacy `EconomicClock::Cadence(id)` until D-LBK-7's convergence; this `catch_up` forgives on the clock, the lifestyle schedule's `maximum_catch_up` 1 decides attempts — §9.)*
2. **Who owns `@econ/cadence`.** Resolved: **E5b ships it** in `mush-economy` (its build brief already does — housing assesses on a cadence); E6a tests E4b1's B6/S2 against real clocks.
3. **08e answer 19's E6c list vs doc 10 §15.** Answer 19 puts belief-sensitive fences, heat, notoriety, investigations and title laundering in E6c "after knowledge support"; doc 10 §14–§15 replaces that gate with the **safe market projection**, which supplies no beliefs. Default: **E6c is the market**, and the five belief-dependent items move past E6 (§1.1). A real divergence between two binding documents, flagged rather than silently resolved.
4. **Where the run board lives.** Default `ruleset-shadowrun` (D-E6-17); `mush-market` listings are goods, a run is a contract with an audience notice.
5. **Whether a party may hold more than one live run.** Default **no** — `Locked { contract }` is singular, which keeps the frozen roster unambiguous.
6. **Fence rate shape.** Default: two configured permille rates against the catalog baseline, modified by declared legality/demand factors — config, not constants (answer 16, §9).
7. **Whether `+run/post` may name an org account the poster cannot view.** Default **no**, matching E4b2's `@contract/pay` rule (a listed party adds their own consideration — A1's fix, 08c §5.1).
8. **Starter catalog size.** Default ≤ 64 entries in one `sr.core` pack — enough to prove all five representations and both availability paths (answer 10's "representative, never exhaustive").
9. **Whether chargen shopping ships in E6a or E6c.** Default **E6c**, with E6a shipping only the cash authority (§3.5). Answer 19 lists shopping under E6a, but there is no catalog to shop from until `mush-market` exists, and answer 17 explicitly sanctions "keep current behaviour" meanwhile.
10. **Insurer identity for `@asset/stolen`.** Default: an org holding a new `asset.insure` capability on the asset's scope; "an appropriate authority" is staff in E6.

## 12. Review outcome

**Draft 0.1 → 0.2 (2026-09-03, one pass).** Two amendments: (1) the `scene.outcome` seal is no longer a
per-run staff gate — the contract's named adjudicator signs, staff conferral is the fallback (§4.1,
D-E6-7); (2) `@econ/cadence` is E5b's (§11 q2). Confirmed as correct against the tree: D-E6-4 (a
`System` principal can never be a contract party), the entities-named-at-seal rule, the doc 10 §15
re-gating of E6c on the safe market projection, and escrow-at-accept via the payer's own accept.
Open for the owner: §11 q1, q3–q10.

**Draft 0.2 → 0.3 (2026-09-04): the three-pass review** — adversarial money-safety, code-grounding at
HEAD `ce43714`, and design-fit against 08 / 08a / 08c / 08d / 08e / 10 (08g §11 item 8, DL-02).
M1–M19 and G1–G12 folded; the roles rule amended (owner, §3.3). What the review overturned: §2's
"whole list" (E6a needs five economy seams that do not exist — A′, the rolling window,
`create_obligation`, the contract seams, `InstrumentDisplay`); D-E6-4's "never a party" (a `System`
payer is unfundable, not unofferable — M4); D-E6-7's economy-side `sealed_by` check and `scene:{uid}`
source (M14); D-E6-3's downgrade path (replacement; catch-up 1 — M11); faction boards as an
`Audience` (M5); a deterministic E6a settlement route (M2); the issuer ≠ payer assumption (M1). Owner
additions: the roles rule, DL-04/DL-05/DL-08 and the M4 fold as E6a items, D-E6-19..23. Eight drifted
`file:line` cites corrected. The E6a brief (`e6a-brief.md`) carries the same corrections.
