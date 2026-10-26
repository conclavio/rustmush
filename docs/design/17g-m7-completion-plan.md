# 17g — M7 completion: the player market, its commands, and the House-term stopgap

**Draft 0.1, 2026-10-25.** Planned read-only from `claude/m7-remerge` at `c123b069`, the
integration-green M7 line merged with `main` through doc 20 cut SM.12a (PR #5). This
plan finishes M7 as doc 17 §22 and 17f §1 define it. 17f stays the implementation
record; this note is the plan for what remains. Section 5 lists the owner decisions
it still needs.

## 1. The owner's rulings (2026-10-25)

1. **House owner attestation is deferred.** Instead, a House-filed mind's free
   owner's licence runs **365 IC days**, about 15.2 real days at the deployed
   IC rate of 24 (`deploy/dev*/instance.env` `IC_RATIO=24`), plus the existing
   3-day Grace. That is enough runway for the playtest worlds.
   - Paid House Pins and paid House renewals stay at 90 days.
   - Personal owner licences stay at 90 days. They get real owner attestation
     through the player market (§3, C1), because that owner already exists.
2. **Player-to-player sales ship in M7.** They are the reason for M7's
   commercial work, and skipping them leaves the market without its main
   interface.
3. **The operator commands (17f M7.11) ship in M7.** Without them most of the
   built commerce code is unreachable.

## 2. Where M7 stands (verified at `c123b069`)

**Built and story-tested, composed only in tests.** Every player-to-player owner
exists in mush-roll, mush-market and mush-economy and is exercised by the
integration stories:
- personal original sale, paid licence resale
- charter transfer and PendingTransfer
- forks, derived minds and parent royalties
- personal renewal and Grace, owner attestation, withdrawal
- Registry recall and revocation
- IC claims and hull cover

Production composes none of this.

**What production composes.** `mushd` boots through `mush_server::boot::game_modules`,
which composes mind commerce only through the launch path. `game_modules_with_mind_commerce`
has no production caller, and boot refuses it beside a launch path
(`mush-server/src/boot.rs:344`). Two consequences:
- With `[tacsim.minds].launch_path` unset, no commerce is composed at all. That is
  the default, and nothing under `deploy/` sets it.
- With a launch path in Active mode, `LaunchComposition::prepare`
  (`crates/tacsim/src/launch.rs:92-120`) composes:
  - the House chain: House filings, the one-seat House Pin with Coprates admission,
    House renewals and their service
  - Conduct trials, Registry reconformance and mind facts
  - the governed hardware: certified cores, integration and refit
  - one side effect: `MindCommerce::prepare` always registers personal filing
    (`roll-file-original-mind`)

**What players can reach.** Only the gateway's generic `+actions` /
`+act <verb> with <JSON>` (`crates/mush-gateway/src/bridge_common.rs:532-727`), and only
for registered providers:
- Filing by hand needs eleven internal parameters (`mush-roll/src/commerce/filing.rs:731-751`).
- `@mind/publish` refuses ("not available yet", `tacsim/src/minds/workshop/commands.rs:435`).
- `@mind/fork` is the local draft copy, not the paid fork.
- `SIM MIND /filed` lists owner licences only.
- The typed protocol (`mush-proto` `MindRequest`) covers authoring only.
  `Query::MarketOriginalMinds` exists, but no client sends it.

| Feature | Owner code | `MindCommerce` builder | Production | Player entry today | This plan |
|---|---|---|---|---|---|
| Personal filing and owner licence | yes | in `prepare` | launch only | `+act` with 11 ids | C1, C2 |
| Personal original sale and listings | yes | `with_original_purchases` / `_source_purchases` | no | none | C1, C2 |
| Paid licence resale | yes | `with_paid_mind_resales` | no | none | C1, C2 |
| Charter transfer and PendingTransfer | yes | `with_charter_transfers`, `with_pending_charter_transfers` | no | none | C1, C2 |
| Forks, derived minds, royalties | yes | `with_authored_forks`, `with_dependent_owner_filings`, `with_derived_updates` | no | none | C1, C2 |
| Personal renewal and Grace | yes | **missing** | no | none | C1, C2 |
| Owner attestation (personal) | yes | **missing** | no | none | C1, C2 |
| Withdrawal | yes | **missing** | no | none | C1, C2 |
| Parent re-qualification | yes | **missing** | no | none | C1 |
| Registry recall and revocation | yes | `with_registry_recall`, `with_registry_revocation` | no (reconformance recall only) | none | C1, C2 |
| House owner attestation | **not built** | — | — | — | deferred; C0 stopgap |
| Paid Follow licences | **not built** (paid is Pin only) | — | — | — | deferred (§4) |
| IC claims, hull cover | yes | `with_ic_minds`, `with_hull_coverage` | no | none | conditional (§4) |

## 3. The plan

Run each cut through the established cut loop: builder, three lenses, fixer, gate
runner, judge, one commit and push per sub-cut, with the ledger kept here. Each
cut's gate runs the full sweep, workspace clippy with `-D warnings` and the four
`mind-commerce-acceptance` stories (about four hours on CI).

### C0 — The House owner's 365-day term (small; independent)

There is no House-only term setting today. One `CommerceConfig.terms` serves:
- House, personal and dependent owner licences, and owner attestation
  (`mush-roll/src/commerce/module.rs:35`, used at :729, :794, :1393, :1420, :1448, :1589, :1941)

The launch constant `content::TERM_SECONDS` also feeds the paid House Pin and its
renewal (`tacsim/src/launch.rs:80, 106-111`). The roll-wide bound
`MAX_TERM_SECONDS` (`commerce/mod.rs:26`, checked in `TermWindow::new` and its
decode) refuses anything over 90 days.

Work:
- Add a separate owner-term bound, e.g. `MAX_OWNER_TERM_SECONDS` of 365 days, beside
  the paid 90-day bound. Use it only on the owner-term paths:
  - `InitialFilingTerms::new` (`filing.rs:44`)
  - the owner window's `TermWindow` construction and decode
  - the market House subject's `owner_window` (`mush-market/src/mind_house/records.rs:43`)
  - the House catalogue reader (`mush-roll/src/commerce/house_catalogue/reader.rs:283`)

  The mush-core copies (`7_776_000` in the charter-pending, paid-resale, House-renewal
  and IC-claim wires) never see a House owner window and stay as they are.
- Add `house_terms` to `CommerceConfig`, used by the House filing action and builder
  (`module.rs:729, :1420`), and pass it through
  `MindCommerce::with_house_filings` (`tacsim/src/composition/mind_commerce.rs:436`).
- Add a 365-day House owner-term constant in `tacsim/src/launch/content.rs`, passed
  from `launch.rs:93`. The paid House terms keep `TERM_SECONDS` (90 days).
- Move the pins that assume the owner term is shorter than the paid term:
  - `house_purchase_integration_tests.rs:1119, 1188-1189`
  - `house_renewal_integration_tests.rs:137-151, 398`
  - `hardware/flow/launch/story.rs:467-471` (its "still in Grace" comment)
- Update 17f §2.2 ("at most 90 IC days") and `docs/help/house-minds.md`.

Notes:
- The House filing action's definition digest changes. Pre-launch, that means a
  fresh store at the next deploy.
- Existing House filings keep their immutable 90-day windows, so a running world
  re-files or reboots fresh.
- Gate:
  - A House filing's owner term is 365 days, its Grace 3.
  - A paid House Pin and its renewal stay at 90.
  - Personal and dependent owner terms stay at 90.
  - The catalogue sells through day 365 and refuses after Grace.
  - Boot and restart pass.

**The runway is real but finite.** Plan House owner attestation (§4) before any world
nears 350 IC days after its House filings, which is about 14.5 real days at rate 24.

### C1 — Compose the player market in production (medium-large; three sub-cuts)

**C1a, builder completeness.** Add `MindCommerce` methods for four owners. Their
Roll, Economy and Market configuration already exists; copy the fixture wiring at
`original_purchase_integration_tests.rs:446-478, 553-568, 703-715, 794-796`.

| Owner | Roll | Economy | Market |
|---|---|---|---|
| Personal renewals and their service | `CommerceConfig::with_mind_renewals` / `with_mind_renewal_service` | `with_mind_renewals` / `with_mind_renewal_service` | `with_mind_renewals(RenewalConfig)` |
| Withdrawal | `with_author_withdrawal` | — | — |
| Owner attestation (personal) | `with_owner_attestations` | — | — |
| Parent re-qualification | `with_owner_parent_qualification` | — | — |

Thread them through `configure_roll`, `configure_economy` and `configure_market` in
`tacsim/src/composition/mod.rs`.

**C1b, deployment configuration.** `LaunchDeployment` (`launch/deployment.rs:82-91`)
has no fields for the player market. Version it to `format: 2` with:
- the personal quote, entitlement and renewal policies (venue: the deployment's venue)
- the service actor and offer windows for pending charters and resale (the tests use `GOD`, 60 s)
- the Registry recall/revocation policy `{issuer, organization, scope, operators ≤ 32}`

Version 1 deployments keep booting as House-only.

**C1c, compose in Active mode.** In the launch Active branch, after the House chain,
compose in the order the builder enforces:
1. `with_original_source_purchases`. This is the v2 path; forks refuse on v1.
2. `with_authored_forks`, then `with_marked_registry`, then `with_dependent_owner_filings`, then `with_mind_entitlements`.
3. `with_compatible_updates` and `with_derived_updates`.
4. `with_charter_transfers`, then `with_registry_revocation`, then `with_pending_charter_transfers`, then `with_paid_mind_resales`.
5. `with_registry_recall`.
6. C1a's renewal, withdrawal, attestation and re-qualification owners.

Also:
- Grant `roll.registry.recall` and `roll.registry.revoke` to the Registry operators at
  Initialize. Today they are vocabulary only (`composition/mod.rs:1010-1012`).
- Update the launch HELP (`launch.rs:16`).
- Set `launch_path` in the dev deploy templates, applied only at the owner's next
  deploy.

Gate:
- First, a mush-server `game_modules`-level test that boots the full player-market
  roster through production composition (none exists today). Mirror the
  `ic_integration_tests` roster (`tacsim/src/minds/bulk/ic_integration_tests.rs:588-612`).
- Split and all-in-one boot parity with the new owners.
- Restart.
- The four CI stories, still green.
- Every new offer listed by `+actions` to the right principal and to no one else.

### C2 — Operator commands, line mode (large; three sub-cuts)

Each command wraps an existing legal action. The server builds the internal
parameters (shell request, revision and rating digests, charter name, payer) from
the actor's own holdings. Clients never supply an actor, an identity or a digest,
and no new authority is added. Purchases and title trades still confirm through the
normal market and E4 contract path. Section 5, question 1 puts the spellings to the
owner; the proposal follows the existing `@mind` switch grammar (`MindRef::parse`,
`name@rev`).

| Proposed command | Wraps | Who |
|---|---|---|
| `@mind/rate <name>@<rev>` | the Registry rating a filing requires (`rating_job` / `rating_id` / `rating_digest`); check whether a separate rating verb is needed or filing drives it | author |
| `@mind/file <name>@<rev>[=<charter>]` | `roll-file-original-mind` (`roll-file-dependent-mind` for a fork) | author |
| `@mind/publish <filed> price=… form=sealed\|open\|open-resale seats=… [term=…]` | `original-list-v3` / `market-original-mind-offer` (`derived-*-v1` for derived) | charter owner |
| `@mind/update <filed> <name>@<rev>` | `roll-update-original-mind` / `-derived-mind` | charter owner |
| `@mind/withdraw <filed>@<rev>` | `roll-withdraw-authored-mind` | charter owner |
| `@mind/fork <licence>=<name>` | `roll-fork-paid-mind`. The existing `/fork` local copy keeps a different spelling. | open-form holder |
| `@mind/attest <filed>` | `roll-attest-owner-mind` | owner, when due |
| `+minds` / `+minds <listing>` | `Query::MarketOriginalMinds Listings` readback | anyone |
| `+minds/buy <listing>` | quote, then fund (`original-*-v3`), confirmed through E4 | buyer |
| `+minds/holdings` | the licence readback (owner and paid), with term, Grace and lapse. Absorbs the `/filed` paid-holdings follow-up. | holder |
| `+minds/renew <licence>` | `mind-renewal-*` | paid holder |
| `+minds/resell <licence> price=…` | `paid-mind-resale`. Split its shared verb into offer and fund so `+act` never meets the ambiguity guard. | paid holder |
| `+minds/charter <filed>=<buyer> price=…` and `/accept` | `roll-offer-charter-transfer` / `roll-accept-charter-transfer` and the pending-charter verbs | charter owner and buyer |
| `@registry/recall <filed>=<reason>`, `@registry/revoke <licence>=<reason>` | `roll-registry-recall` / `roll-registry-revocation` | Registry operators only |

Contracts every new verb meets:
- `legacy-handlers.toml`: the `@mind` stanza's "Publish remains closed until M7"
  note is rewritten.
- `help_commands.rs` and its allowlist: every `@`, `+` and capitalised token in
  `docs/help` must resolve.
- The `@mind` help pins at `arena_bridge.rs:6041` and `tests/cockpit.rs:16694`.
- The panic budget.
- The observer and privacy batteries: no source in any reply, error or event.

Rewrite `docs/help/personal-minds.md` and `house-minds.md` around the commands.
`+act` stays available.

Gate: §9 story 1 driven entirely through the commands, line mode, two accounts.
Then the same with an alt pair (refused), stale terms, tiny amounts and a restart.

### C3 — Typed actions for the TUI and web (medium-large; owner's scope call, §5 question 2)

17f M7.11 asks for "the same typed actions in web/TUI without requiring clients to
synthesize command text". Work:
- Either add commerce `MindRequest` variants, or route the clients' existing
  `LegalAction SubmitSelf` through server-built parameter shapes. Validate in
  `InvokeRequest::validate` with a closed serde grammar, and regenerate the TS bindings.
- Use `Query::MarketOriginalMinds` in a web market panel and a TUI market and holdings
  screen, at the 120-column floor.
- Give the TUI's legal-actions panel typed submit, not a prefilled `+act` line.

Line mode stays first-class, so this cut can follow C2's ship.

### C4 — Acceptance and the record (medium)

17f M7.12 on the production composition and the commands:
- §9 stories 1 (commercial author), 2 (private and derivative), 3 (seat and term),
  5 (authority and adversarial), 6 (launch catalogue) and 7 (facts), each with a restart.
- Story 4 (hardware) through the launch's governed hardware.
- `cargo test -p mush-bot` green on every new sentence.
- The cross-host deterministic cases.
- Registry load beside live arenas, reported.

The existing four CI stories become a subset. Update the 17b M7 row, 17f's status
and doc 17 §22.

## 4. Deferred: each needs a recorded ruling

- **House owner attestation.** C0's runway; also an M8 prerequisite. Build it before
  any world needs more than about 350 IC days.
- **Paid Follow licences.** Paid writers support Pin only
  (`personal_paid_holdings.rs:248-249`). This was parked in Codex's log along with
  the original-major producer, the House and dependent major variants, and scoped
  House author updates and source access.
- **IC claims and hull cover in live play.** These need an IC region host that doesn't
  exist (17f §10). They stay composed in tests, named as the conditional
  integration, and are not presented as a passed live story.
- **Follow-ups 17f already lists:**
  - the recall notice drain aborting on one bad holding (`registry_recall/notice.rs`)
  - Registry pacing, 3–7 s per lease, which real players will feel
  - the shared instrument lifecycle design conversation (six copies of "a sale")

## 5. Owner decisions

1. **Command spellings.** §3 C2's table is a proposal: `@mind/*` for author acts,
   `+minds/*` for holders and buyers, `@registry/*` for Registry staff.
2. **C3's scope.** Typed TUI and web inside M7, or line mode first with typed
   clients right after?
3. **Player-market tuning (17f §2.3, still provisional):**
   - the filing fee and levy, venue share and royalty cap
   - the resale and pending-charter service actor and offer windows
   - who the Registry operators are
4. **Paid Follow.** Defer, as the parked list has it?
5. **Still owed from 17f:** D-UM-56, and confirmation of Q-UM-18's written default.
6. **Merge order.** Recommended: merge PR #5 once its dispatched gates and CodeQL are
   settled, then build C0–C4 on `main`. The other order means a second long-lived
   branch while `main` moves hourly, and this line has already been re-merged five
   times.

## 6. Risks

- **No production-roster boot test.** C1's first act is to add one. The player-market
  owners have only booted in fixtures.
- **Builder ordering.** The constraints are real: v1 against v2 purchases, House
  two-seat Pins before House renewals, revocation before pending charters.
- **Digest and config changes.** C0's filing action, C1's deployment format 2 and
  C2's split resale verb mean a fresh store at deploy, which is acceptable pre-launch.
- **The 2026-10-24 dead-code pass** (`2a83f8b8`) deleted superseded wrapper ladders
  that nothing called. The builder path above is intact, and the tests compose through
  it. A wiring step that wants an old rung restores it from git.
- **Privacy.** Every new reply crosses the observer battery. Sealed-source minds must
  never print source, memory or a digest that would act as an oracle.
- **Load.** Registry pacing and the queue's fairness with live players are unmeasured.
  Report them under C4, in line with the owner's report-only timing ruling.
