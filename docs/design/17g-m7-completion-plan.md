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

Later the same day, with PR #5 merged to `main` (`5d00f978`) and C0–C4 started:

4. **§3 C2's command table is approved as proposed**: `@mind/*` for author acts,
   `+minds/*` for holders and buyers, `@registry/*` for Registry staff (§5
   question 1).
5. **C3 is in M7.** The owner asked for C0–C4, so typed TUI and web actions ship
   with M7 rather than after it (§5 question 2).
6. **Player-market tuning takes provisional defaults** (§5 question 3), per the
   owner's standing word on economy numbers: reasonable defaults now, tuned in
   play, never a reason to hold a build. Each default C1 picks is listed in §7.

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

Production composed none of this before C1. Since C1c (`9002169e`) a format-2 named
launch in Active mode composes all of it except IC claims and hull cover; no instance
runs one yet (§7, C1 as built). Since C2 (`bfc1b34e`–`3e4f7668`, PR #30, §7, C2 as
built) the same launch composes the line-mode command slot; the table says which
commands reach each owner and which are held.

**What production composes.** `mushd` boots through `mush_server::boot::game_modules`,
which composes mind commerce only through the launch path. `game_modules_with_mind_commerce`
has no production caller, and boot refuses it beside a launch path
(`mush-server/src/boot.rs:344`). Two consequences:
- With `[tacsim.minds].launch_path` unset, no commerce is composed at all. That is
  the default, and nothing under `deploy/` sets it.
- With a launch path in Active mode, `LaunchComposition::prepare`
  (`crates/tacsim/src/launch.rs:89-179` at `9002169e`) composes:
  - the House chain: House filings, the one-seat House Pin with Coprates admission,
    House renewals and their service
  - Conduct trials, Registry reconformance and mind facts
  - the governed hardware: certified cores, integration and refit
  - one side effect: `MindCommerce::prepare` always registers personal filing
    (`roll-file-original-mind`)
  - for a format-2 deployment only (C1c), the player market after the House chain:
    the seventeen builder steps of §7's C1 entry, paid resale under the House buyer
    admission; IC claims, hull cover and paid Follow are not composed
  - since C2 (§7, C2 as built), with the player market: the `MindCommandService` slot
    (`TacsimRuleset.mind_commands`), Roll's command kinds folded into the
    `original_owner_holding` v2 read, and the market's `mind_market_commands` read,
    registered but ungranted while the owner-read caps stay at 32/32, so every `+minds`
    form and `@mind/publish` answer `NOT_OPEN`

**What players can reach.** Before C2, only the gateway's generic `+actions` /
`+act <verb> with <JSON>` (`crates/mush-gateway/src/bridge_common.rs:532-727`), and the
web client's legal-action form, which submits by offer id and so also reached the four
shared verbs that `+act` refused as ambiguous until C2a split them one per kind; only
for registered providers:
- Filing by hand needs eleven internal parameters (`mush-roll/src/commerce/filing.rs:731-751`).
  Since C2a `@mind/rate <name>@<rev>` and `@mind/file <name>@<rev>` build them from the
  actor's own state.
- `@mind/publish <filed>` (C2c) lists a filed mind, but it and every `+minds` form answer
  `NOT_OPEN` until the lead raises the owner-read caps (§7, C2 as built).
- `@mind/fork <licence>=<name>` is the paid fork since C2a; `/localcopy` is the local
  draft copy.
- `SIM MIND /filed` lists owner licences only.
- The typed protocol (`mush-proto` `MindRequest`) covers authoring only.
  `Query::MarketOriginalMinds` exists, but no client sends it.

| Feature | Owner code | `MindCommerce` builder | Production | Player entry today | This plan |
|---|---|---|---|---|---|
| Personal filing and owner licence | yes | in `prepare` | launch only | `@mind/rate`, `@mind/file` (C2a); `+act` with 11 ids | C1, C2 |
| Personal original sale and listings | yes | `with_original_purchases` / `_source_purchases` | format-2 launch (C1c: v2 source and v3 entitlement); the `+minds` slot and the ungranted market read (C2c) | `@mind/publish <filed>` and `+minds` (C2c), `NOT_OPEN` until the cap raise (§7); `+act` at the counter for v3 and derived; v2 `market-original-mind-list`, `-quote` and `-fund` by `+act` since the C2a split | C1, C2 |
| Paid licence resale | yes | `with_paid_mind_resales` | format-2 launch (C1c), House Pins fenced by the House admission (C1a) | `+act` with the split `paid-mind-resale-offer`, `-accept` and `-fund` (C2a); `+minds/resell` and `/resell/accept` held `NOT_OPEN` (C2d) | C1, C2 |
| Charter transfer and PendingTransfer | yes | `with_charter_transfers`, `with_pending_charter_transfers` | format-2 launch (C1c) | `+act` with JSON; `+minds/charter` and `/charter/accept` held `NOT_OPEN` (C2d) | C1, C2 |
| Forks, derived minds, royalties | yes | `with_authored_forks`, `with_dependent_owner_filings`, `with_derived_updates` | format-2 launch (C1c) | `@mind/fork <licence>=<name>`, `@mind/update <filed> <name>@<rev>` (C2a); `+act` with JSON | C1, C2 |
| Personal renewal and Grace | yes | `with_mind_renewals`, `with_mind_renewal_service` (C1a) | format-2 launch (C1c) | `+act` with JSON (v2 Pins do not renew); `+minds/renew`, `/pay`, `/decline` held `NOT_OPEN` (C2d) | C1, C2 |
| Owner attestation (personal) | yes | `with_owner_attestations` (C1a) | format-2 launch (C1c) | `@mind/attest <filed>` (C2a); `+act` with JSON | C1, C2 |
| Withdrawal | yes | `with_author_withdrawal` (C1a) | format-2 launch (C1c) | `@mind/withdraw <filed>` (C2a); `+act` with JSON | C1, C2 |
| Parent re-qualification | yes | `with_owner_parent_qualification` (C1a) | format-2 launch (C1c) | `+act` with JSON | C1 |
| Registry recall and revocation | yes | `with_registry_recall`, `with_registry_revocation` | format-2 launch (C1c); offered to granted operators only | `@registry/recall <filed>`, `@registry/revoke <licence>` (C2d) and `+act`, for an operator granted with `@org/grant` | C1, C2 |
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
- the service actor and offer windows for pending charters and resale (the tests used `GOD`,
  60 s; as built the actor is a deployment-named clerk, never #0 or `GOD`, §7)
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
  Initialize. Today they are vocabulary only (`composition/mod.rs:1010-1012`). As built,
  the grant is the baseline `@org/grant`, a runbook step; the code grants nothing (§7).
- Update the launch HELP (`launch.rs:16`). As built, `HELP` is unchanged and bare
  `@mind/launch` appends a deployment-rendered market line (§7).
- Set `launch_path` in the dev deploy templates, applied only at the owner's next
  deploy. As built, it is documented and commented, not live (§7, a departure).

Gate:
- First, a mush-server `game_modules`-level test that boots the full player-market
  roster through production composition (none exists today). Mirror the
  `ic_integration_tests` roster (`tacsim/src/minds/bulk/ic_integration_tests.rs:588-612`).
- Split and all-in-one boot parity with the new owners.
- Restart.
- The four CI stories, still green.
- Every new offer listed by `+actions` to the right principal and to no one else.

### C2 — Operator commands, line mode (large; planned as three sub-cuts, built as four, §7)

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

As built (§7, C2 as built): the table's spellings with the plan's departures.
`@mind/publish <filed> form=sealed|open|open-resale [royalty=<per mille>] [transfer=no]`
takes `price=`, `seats=` and `term=` only at the market's values; `+minds` reads the v3
and derived listings through the market's own `MarketCommandReader`, not v2's
`Query::MarketOriginalMinds`; the four shared verbs are split one per kind; recall and
revocation take no reason. `+minds/buy` and its steps, the paid half of `/holdings`,
`/renew`, `/resell`, `/charter` and both accepts are built and held at `NOT_OPEN` until
the owner-read cap raise, the index sub-cut and the renewal seam land.

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

## 7. Ledger

### C0 — the House owner's 365-day term (2026-10-25, branch `claude/m7-c0-house-term`)

As built, against §3 C0:
- **Two bounds, one window type.** `mush_roll::commerce::MAX_OWNER_TERM_SECONDS`
  (365 days) sits beside `MAX_TERM_SECONDS` (90). `TermWindow` carries owner and
  paid windows alike, so its own constructor and decode take the longer owner
  bound, and `PaidTermRecord::validate` holds a paid window to 90 through the new
  `TermWindow::term_seconds`. Every paid product, config and record that checks
  `term_seconds` keeps `MAX_TERM_SECONDS` unchanged. `InitialFilingTerms::new`
  takes the owner bound.
- **The House term is its own setting.** `CommerceConfig::with_house_owner_terms`
  (set once; the House filing otherwise files under the personal `terms`) feeds
  the House filing action's definition and its builder.
  `MindCommerce::with_house_owner_terms` must precede `with_house_filings`, which
  fixes that definition. The launch sets `content::HOUSE_OWNER_TERM_SECONDS`
  (365 days); the paid House Pin and its renewal keep `TERM_SECONDS` (90).
- **mush-core's 90-day copies stay.** The charter-pending wire's window bound is
  the personal owner term's: charter transfers and pending charters refuse a House
  birth (`economic.house.is_some()`), so they never carry a House owner window.
  Its comment now says so.
- **The pins.** The House purchase and renewal integration fixtures file House
  minds under their own explicit 90-day term and test protocol mechanics: an
  issuer lapse is not inherited by a granted Pin, and a renewal runs while the
  issuer is in Grace. Those cases stay valid and are what House owner attestation
  will need, so they were left on the fixture term rather than re-choreographed.
  C0's gate is on the production composition instead, in the named launch story
  (`hardware/flow/launch/story.rs`): the Coprates owner term is 365 days with 3
  of Grace, the paid House Pin 90 with 3, the issuer is Active (no longer in
  Grace) at the Pin's due date, and the catalogue quotes one second before day
  365 and during Grace and refuses at the Grace end.
- **Docs.** 17f's term rule names the exception; `docs/help/house-minds.md` gives
  the 365/3 and 90-day terms.
- **Deploy note.** The House filing action's definition digest moves with its
  term, so the next deploy of a launch world takes a fresh store (§6).

### C1 — plan (2026-10-25, branch `claude/m7-c1-player-market`)

As planned against §3 C1 in three sub-cuts, revised after two review lenses (plan:
plan-data/m7-completion/cuts/C1/plan.md, §10 dispositions). **C1a, builder
completeness, the resale fence and the first act.** `MindCommerce` gains
`with_mind_renewals(RenewalPolicy)`, `with_mind_renewal_service`,
`with_author_withdrawal`, `with_owner_attestations` and
`with_owner_parent_qualification`. They are shaped like the House renewal pair and
threaded through `configure_economy` and `configure_market`; the Roll config carries
the flags. Composing paid resale beside the launch's transferable Coprates Pin would
let a Republic member resell it to a non-member, because the resale reader never
consults `HouseBuyerAdmission`. So C1a adds `PaidResaleWriter::with_buyer_admission`,
checked at the buyer's Accept and Fund for House-born licences.
`with_house_minds_and_admission` stores the admission and `with_paid_mind_resales`
installs it. This is the conservative reading of the 2026-10-18 ruling, proven in the
House resale fixture inside the original-purchase CI story and sent to the owner. 17g
§6 risk 1's test lands here: a mush-server lib test boots the whole roster through
`boot::game_modules_with_mind_commerce`, through Initialize, the real
`roll-initialize-commerce` receipt, Active and a restart. The order follows the real
guards: v2 source purchases before forks; forks, the marked Registry and dependent
filing before entitlements; renewals after entitlements; pending charters after
charter transfers; re-qualification after dependent owners; the House admission before
paid resale. Revocation-first and resale-after-personal-purchases are conventions, not
guards: `with_registry_revocation` rebuilds both configs anyway.

**The census reads the as-built projection rules, which are the no-oracle design.**
Most player-market offers go to every live player, or to every player at the counter,
never only to holders. So the census has five classes. Recall and revocation go to
granted operators only. The service steps appear only with a live intent, so they are
absent in an empty world (a vacuous check, stated as such). The non-refund v3/derived
and v2 list and quote steps are counter-gated. The v2 Expire step is God-only.
Everything else is generic. The privacy property is byte-identity: projected actions,
minus their offer ids, are identical across same-room principals. The story adds the
invocation side: God's resale plan on the buyer's House Pin refuses with the same
bytes as for a nonexistent licence. **C1b, format 2.** `LaunchDeployment` gains an
optional
`market: PlayerMarket { clerk, registry_authority: { organization, scope, operators ≤ 32 }, terms: MarketTerms }`.
Format 1 forbids it and serializes byte-identically, so its `launch_deployment`
identity does not move and it keeps booting House-only. The terms carry tuning only.
Doc 17's 90-day term, 3-day Grace and 50‰ levy stay content constants; the personal
venue cut is 0, and licences are transferable policy-wide.
`tacsim::launch::market::policies` builds the typed policies; the entitlement policy's
own validation still runs at Active composition. Finalize checks existence, ownership
and kind only, like the House check. The providers re-check org and scope status,
GOING and grants on every offer, so the Registry's kill switch (a suspended org, a
retired scope) never becomes a boot refusal. C1b adds no command: the review replaced
`@mind/launch registry` with the baseline `@org/grant`, which already makes the
exact-scope, non-delegable grant through `mush_org::api::submit`. A new tacsim writer
of org authority would have broken legacy-handler rule 1. The first production boot of
the named launch through `game_modules` uses production commands only, under a stop
rule.

**C1c** composes the market after the House chain in the Active branch. Bare
`@mind/launch` prints a deployment-rendered market line, which pays the remainder to
the charter owner (to the seller on resale), says v2 Pins do not renew, and says the
shared verbs wait for C2. For God only, a read-only readiness line asks
`authorize_in_world`, so an ineffective grant reads as lacking. C1c moves the named
launch story to format 2 with the grant at Initialize, proves parity by a real Active
`build_engine` boot on the same store with equal offer route tables, and measures a
reboot with changed terms, pinning whichever outcome it finds.
**Provisional defaults (ruling 6):** a personal Pin and its renewal at 400cr (one
policy-wide price, as built); a 300 s quote; a 3 IC-day renewal offer; a one-real-day
resale offer; 3 IC days to settle a pending charter; the clerk a deployment-named
character, never #1; one operator, the owner's staff character, granted with
`@org/grant`. **Departures:** the boot test and the resale fence land in C1a.
`launch_path` is documented rather than live in the templates, because bring-up is
two-phase by construction (finalize needs identities only a booted world can create,
and `instance.env` cannot say unset). That departure needs the lead's OK before C1c,
and the runbook opens by saying the dev worlds compose no mind commerce until an owner
decision makes the path live. **Gate reading:** C1 proves boot, restart and listing
with an empty market. Per-step restart and unknown-acknowledgement recovery for the
new owners belong to C4. **For C2:** split `paid-mind-resale` and the
`market-original-mind-*` shared verbs, and note that the web legal-action form already
reaches them. Two personal listing protocols are live. The price and transferability
are policy-wide, so `@mind/publish … price=…` and an author's non-transferable mark
need owner changes. **Owner questions** (to 17g §5): a live `launch_path` mechanism,
and whether a resale counts as a new purchase for Coprates.

Sub-cuts:
- **C1a** MindCommerce builder completeness, the Coprates fence on paid resale, and the
  production-roster boot test
- **C1b** LaunchDeployment format 2 and its market identities
- **C1c** Compose the player market in the launch Active branch, and C1's gate

Provisional defaults (17g section 1 ruling 6):
- Personal Pin price 400cr (40,000 minor), one policy-wide price as built for original
  v2, original v3 and derived v1 listings (MarketTerms.price_minor)
- Personal renewal price 400cr (MarketTerms.renewal_price_minor)
- Personal quote window 300 real seconds, as the House quote (MarketTerms.quote_seconds)
- Personal renewal offer window 3 IC days, 259,200 s (17f §2.3;
  MarketTerms.renewal_offer_ic_seconds)
- Paid-resale offer window 86,400 real seconds, the MAX_OFFER_SECONDS ceiling
  (MarketTerms.resale_offer_seconds)
- Pending-charter settlement window 3 IC days, 259,200 s (D-UM-40
  transfer_settle_ic_days, Q-UM-18 written default; MarketTerms.charter_settle_ic_seconds)
- Personal term 90 IC days and Grace 3 IC days, from content::TERM_SECONDS /
  GRACE_SECONDS (doc 17 limits), not deployment fields
- Registry levy 50 permille on personal sales and renewals to the deployment's Registry
  recipient, from content::LEVY_PERMILLE (doc 17's Compact levy), not a deployment field
- Venue cut 0 on personal listings and renewals (17f §2.3 direct author listing); venue
  name the launch counter's
- Personal licences transferable, policy-wide (D-UM-40 default); the author's
  per-listing non-transferable mark is not built and is flagged for C2
- Filing fee 100cr unchanged (content::FILING_MINOR), personal and House
- Aggregate parent royalty cap 300 permille unchanged (code constant
  MAX_ROYALTY_PERMILLE), not configurable in C1
- Service actor for pending charters and paid resale: a deployment-named live character
  (runbook name Market Clerk, created with @newchar on the staff account), never #1
  (refused at decode)
- Personal and House renewal services act as #1 by their fixed as-built protocols
  (service_actor 1); not configurable in C1
- Registry operators: deployment-named, 1 to 32, strictly ascending; runbook default one
  operator, the owner's staff character (#1 on a fresh store)
- Registry authority: a Registry org with a review scope, distinct from the three Houses
  and the hardware importer
- Operator grants: roll.registry.recall and roll.registry.revoke, exact scope,
  non-delegable, no limit, no validity window, made with the existing @org/grant by #1
  at Initialize (runbook step; no new command)
- God-only read-only readiness line under bare @mind/launch reports each operator as
  ready or lacking, through mush_org::authorize_in_world
- Coprates fence on resale: a resold House Pin's buyer must pass that House's buyer
  admission (Coprates: current Republic membership) at Accept and Fund; conservative
  reading of the 2026-10-18 ruling, owner question queued for 17g §5
- Finalize checks the market identities for existence, ownership and kind only;
  org/scope status, GOING and grants are runtime states the providers check, so the
  Registry kill switch never refuses a boot
- Deployment price ceiling at decode: 100,000,000 minor (1,000,000cr) for price_minor
  and renewal_price_minor
- Format-2 deployments require the Registry recipient to be a system principal (the
  personal price policy's recipient type)
- Market help line wording: remainder to the charter owner (to the seller on resale), v2
  market-original-mind-offer Pins do not renew, shared verbs wait for the C2 commands
- launch_path is documented and commented in deploy/instance/mushd.toml.tmpl (under the
  commented [tacsim.minds] header), with deploy/instance/launch.json.example and a
  deploy/README.md runbook; not set live (needs the lead's OK and an owner decision);
  dev worlds compose no mind commerce until it is live
- A terms change on an initialized format-2 world: behaviour measured by C1c test 7 and
  recorded as found; a change after licences exist is untested
- A world initialized under format 1 is not promised to boot under format 2; the market
  starts on a fresh store

### C1 — as built (2026-10-26, branch `claude/m7-c1-player-market`, PR #14)

As built, against §3 C1 and the plan above: three sub-cuts, each one commit pushed to
the branch, then one close sweep. PR #14 is open and not merged; the four M7 stories run
on the CI dispatch before the merge (below).

**Landed.**
- **C1a** (`af096dbf`): C1a (17g C1 — plan, sub-cut 1 of 3) landed on
  `claude/m7-c1-player-market` over 46d35d5b, as commit af096dbf. **The builder.**
  `MindCommerce` gains the following methods.
  - `with_mind_renewals(RenewalPolicy)`: refuses "personal renewals require one Active
    entitlement composition" in Initialize, on a second call, or without entitlements or
    the personal purchase participants. It builds `RenewalConfig::new(manifest, purchase
    participants, RenewalWriter::new(), policy)` and registers its 5 actions through
    `CommerceConfig::with_mind_renewals`.
  - `with_mind_renewal_service()`: refuses "personal renewal service already
    configured", "personal renewal service requires renewals" or "personal renewal
    service action".
  - `with_author_withdrawal()`, `with_owner_attestations()` and
    `with_owner_parent_qualification()`: thin delegations whose Roll guards (Active,
    once, and dependent owners for re-qualification) are the refusals.

  `MindCommerceState` gains `renewals`, `renewal_actions`, `renewal_service` and
  `house_admission`. `configure_economy` installs `with_mind_renewals(manifest,
  [economy, retention], actions)` and then `with_mind_renewal_service` after the House
  branches. `configure_market` installs `with_mind_renewals(config)`. `configure_roll`
  and `configure_tacsim` are unchanged.

  **The Coprates fence on paid resale.** This is the conservative reading of the
  2026-10-18 ruling.
  - `PaidResaleWriter::with_buyer_admission(Arc<dyn HouseBuyerAdmission>)` is set once,
    before registration ("paid resale buyer policy must be configured once before
    registration"). `has_buyer_admission()` reports it.
  - `BirthProof.house_issuer` and `PaidResaleFacts.house_issuer` (private,
    `skip_serializing_if` None) are `Some` only for `Birth::House`.
  - `Mode::Admission(facts, Option<Arc<dyn HouseBuyerAdmission>>)` calls
    `policy.check(world, issuer, buyer)` after `check_fresh` at the buyer's Accept and
    Fund. A refusal is the provider's existing uniform refusal, with no new sentence or
    variant. Personal births, the seller's Offer and the service steps are untouched.
  - `with_house_minds_and_admission` stores the admission and refuses "house buyer
    admission must precede paid resale" when given `Some` after paid resale.
    `with_paid_mind_resales` installs the stored admission on its writer, so the
    launch's `CopratesAdmission` reaches resale with no launch change.

  **The risk-1 boot test.** The new `mush-server` lib file
  `world_task/submission_disposition/tests/player_market.rs` seeds through public
  commands only: `@account/system`, `@account/open`, `@org/create Registry`, `@org/scope
  create Registry=review`, `@org/grant` of `roll.registry.recall` and
  `roll.registry.revoke` to the operator, and `@teleport`. It boots plain, then
  Initialize, where GOD is offered and submits a real `roll-initialize-commerce`. It
  then boots the full House-plus-player-market roster Active through
  `boot::game_modules_with_mind_commerce`, takes a census, restarts, and finds the
  census identical. The roster is `player_market_commerce`, in C1c's order, with no
  owner fix needed. `player_market_offers_follow_the_projection_rules` asserts:
  - class A, recall and revocation, for the granted operator only;
  - class B, the four service verbs, absent (vacuous in an empty world);
  - class C, the eight non-refund `original-*-v3` and `derived-*-v1` verbs plus the
    List, Quote, Fund and Grant kinds of the two `market-original-mind-*` verbs, at the
    counter only;
  - class D, the v2 Expire kind, for GOD only (vacuous here);
  - class E, every other C1 verb plus the RetireListing, RetireQuote and Refund kinds
    exactly once for every principal;
  - privacy, as byte-identity of author, buyer and accountless, and of clerk and
    operator once class A is removed.

  The census tells the kinds apart by the routed action id. As measured, the counter
  trio has 38 C1 entries each (`market-original-mind-offer` 4,
  `market-original-mind-purchase` 3), GOD and the clerk 26, and the operator 28. The
  `paid_purchase` harness's `runtime` and `store` become `pub(super)`, several items are
  widened to `pub(in …tests)`, and `Running::routes()` is added.

  **Tests added.**
  - Builder: `personal_renewals_are_explicit_active_entitlement_bound_and_once`,
    `personal_renewal_service_requires_renewals_and_registers_its_close`,
    `withdrawal_attestation_and_requalification_are_explicit_active_once`,
    `requalification_requires_dependent_owner_filing`, and
    `full_player_market_chain_registers_every_owner_in_builder_order`. The last asserts
    30 refs plus 4 Roll ids, refuses both real-guard swaps, and gets an identical
    definition set from the revocation reorder.
  - `paid_resale_inherits_the_house_buyer_admission`.
  - mush-roll: `resale_buyer_admission_is_optional_and_configured_once`.
  - The House resale fixture's `AdmissionProbe` and `refused_admission`, inside
    `genuine_registry_original_purchase_grant_refund_and_paired_recovery`. The armed
    (issuer, buyer) Accept refuses byte-identically to an Accept on a nonexistent offer
    token. The probe is consulted with only that pair, and never before the buyer's
    Accept. The admitted path then runs unchanged.

  The gate is green: fmt, clippy, strict rustdoc, mush-roll 284+2, tacsim 3605/0/16,
  mush-server 570/0/3, contract tests 138/0/8, and the story passed by name in 1321.30
  s. No gate moved.
- **C1b** (`823ca9c4`): `tacsim::launch::LaunchDeployment` gains a last field `market:
  Option<PlayerMarket>` (`serde(default, skip_serializing_if)`), so a format-1 document
  serializes to its eight pre-C1 keys and its `launch_deployment` descriptor does not
  move; `PlayerMarket { clerk, registry_authority, terms }`, `RegistryAuthority {
  organization, scope, operators }` and `MarketTerms { price_minor, renewal_price_minor,
  quote_seconds, renewal_offer_ic_seconds, resale_offer_seconds,
  charter_settle_ic_seconds }` (all `deny_unknown_fields`, re-exported from
  `tacsim::launch`) with `MarketTerms::PROVISIONAL` =
  40_000/40_000/300/259_200/86_400/259_200; `LaunchDeployment::validate` admits only
  `(1, None)` or `(2, Some)` ("launch deployment identity differs") and for format 2
  refuses "launch deployment player market differs" unless the Registry recipient is a
  system principal, the clerk is neither #0 nor GOD, the operators are 1..=32 strictly
  ascending with no #0, the Registry org is no House org and not `hardware.importer`,
  both prices are in 1..=100_000_000 (private `MAX_MARKET_PRICE_MINOR`), each window is
  within `MAX_QUOTE_SECONDS`, `MAX_RENEWAL_OFFER_IC_SECONDS`, `MAX_OFFER_SECONDS` and
  `MAX_CHARTER_SETTLEMENT_IC_SECONDS`, and `market::policies` succeeds; new
  crate-private `tacsim::launch::market::{MarketPolicies, policies}` builds the eight
  typed policies (`OriginalQuotePolicy` with venue cut 0, `EntitlementPolicy` with venue
  `{0, None}`, transferable, content term/Grace/levy, validated `RenewalPolicy`,
  `PendingCharterTransferConfig::new(clerk, settle)`, clerk, resale window,
  `RegistryRecallPolicy` and `RegistryRevocationPolicy` on `registry_issuer`) and
  returns `None` for format 1; `Registration::actual_identities` refuses "named launch
  market identities disagree" unless the Registry org exists, the scope exists and
  belongs to it, and the clerk and every operator are `ObjKind::Player`, never checking
  status, GOING or grants (the kill switch), and `Registration::deployment()` is added
  for C1c; stop-rule fix older than C1:
  `tacsim::composition::NAMED_LAUNCH_HARDWARE_CAPABILITIES` (`industry.core_die.issue`,
  `industry.core.integrate`, `industry.core.refit`) is chained into
  `org_capability_vocabulary()`, because the production `@org/grant` refused the named
  launch's hardware grants (vocabulary only, nothing granted);
  `deploy/instance/launch.json.example` is a format-2 Initialize document with
  `PROVISIONAL` terms; tests: `market.rs` `format_one_has_no_market_policies`,
  `provisional_terms_build_every_policy`,
  `distinct_terms_land_in_their_own_policy_fields`; `hardware/flow/launch.rs`
  `setup_with(format)`, `registry_market`, `grant_registry`,
  `named_launch_format_two_is_strict_and_format_one_identity_is_unchanged`,
  `named_launch_format_two_checks_market_identities_at_boot` (a retired Registry scope
  reboots), interim `named_launch_format_two_composes_no_market_owner_yet`, and
  `the_org_vocabulary_admits_every_named_launch_grant`; mush-server `player_market.rs`
  `launch_world` and
  `named_launch_format_two_initializes_and_activates_through_game_modules` (the first
  named-launch boot through `boot::game_modules`: Initialize, the operator's two
  `@org/grant`s, `roll-initialize-commerce` and the three industry initialize offers,
  then Active twice with GOD offered `issue-core-die` and no initialize verb);
  `boot_parity` `check_config_preflights_the_bounded_named_launch_document` gains its
  format-2 cases, plus `the_shipped_launch_example_decodes_to_the_provisional_terms`; no
  version, IPC, digest-pin, help, legacy-handler, state-slot or shard change.
- **C1c** (`9002169e`): C1c composes the player market in the named launch's Active
  branch: `LaunchComposition::prepare` computes `market::policies(&deployment)?` before
  `Registration::new` and, after `.with_house_mind_renewal_service()?` and only when it
  is `Some(p)`, chains `with_original_source_purchases(p.quote)`, `with_authored_forks`,
  `with_marked_registry`, `with_dependent_owner_filings`,
  `with_mind_entitlements(p.entitlement)`, `with_compatible_updates`,
  `with_derived_updates`, `with_charter_transfers`,
  `with_registry_revocation(p.revocation)`, `with_pending_charter_transfers(p.pending)`,
  `with_paid_mind_resales(p.clerk, p.resale_offer_seconds)`,
  `with_registry_recall(p.recall)`, `with_mind_renewals(p.renewal)`,
  `with_mind_renewal_service`, `with_author_withdrawal`, `with_owner_attestations`,
  `with_owner_parent_qualification`, with a comment separating the six guards from the
  two conventions; format 1 composes exactly the House chain, Initialize composes no
  market owner, paid resale inherits `CopratesAdmission`, and IC claims, hull cover and
  paid Follow are not composed; the `cfg_attr(not(test), allow(dead_code))` allowances
  on `MarketPolicies` and `Registration::deployment` are gone. `tacsim::launch` gains
  `pub(crate) fn market_help(&PlayerMarket) -> String` (rendered from `MarketTerms` and
  `content::{FILING_MINOR, TERM_SECONDS, GRACE_SECONDS, LEVY_PERMILLE}` through private
  `credits` and `window`; with `MarketTerms::PROVISIONAL` it is the brief's line byte
  for byte) and `pub(crate) fn registry_readiness(&World, &PlayerMarket) -> String`
  (`Registry operators: #<n> ready|lacks recall|lacks revocation|lacks recall and
  revocation`, ascending, comma-joined, one read-only `mush_org::authorize_in_world` per
  `REGISTRY_RECALL_CAPABILITY`/`REGISTRY_REVOCATION_CAPABILITY`, `amount: None`); bare
  `@mind/launch` (`Workshop::dispatch`, private `launch_market_lines`) prints `HELP`,
  then the market line to any actor when the `LaunchService` slot is owned by
  `crate::NS` and its deployment has a market, then the readiness line to GOD only, and
  `HELP` alone otherwise. mush-core `MAX_OWNER_PARTICIPANT_REQUESTS` is 256 (was 128;
  the format-2 Active launch declares 156, format 1 91), pinned by
  `composition_accepts_256_distinct_requests_and_refuses_257` and the new
  `registration_accepts_256_allowed_actions_and_refuses_257`. Tests: tacsim `setup()` is
  `setup_with(2)`; `named_launch_real_initialization_cold_activation_and_private_import`
  checks the 17 `MARKET_ACTIONS` are absent at Initialize and present at Active; the
  interim `named_launch_format_two_composes_no_market_owner_yet` is replaced by
  `named_launch_format_one_stays_house_only` (no market action, no
  `GENERIC_MARKET_VERBS` or Registry verb projected, `@mind/launch` is `[HELP]`);
  `named_launch_registry_offers_reach_only_the_granted_operators` (projection via
  `Engine::legal_action_query` and `project_legal_actions`: nobody, then only the
  granted operator, then nobody after `@org/scope retire` and a reboot, with a positive
  control, and GOD's readiness line tracking it);
  `named_launch_readiness_names_what_each_operator_lacks` (two operators, both mixed
  arms, the join); `named_launch_market_help_states_the_deployment_terms` (buyer `[HELP,
  provisional]`, GOD plus readiness, tuned `123.50cr`/`2 IC hours`/`1 hour`, the 22
  class-E verbs projected on format 2); the tacsim rig's `logins(r, d)` puts the clerk
  and operators on a `launch-staff-two` login; the story
  `named_coprates_real_save_rate_house_sale_and_membership_fences` runs on format 2 with
  `grant_registry` at Initialize and `resale_privacy` after the 40000 check
  (paid-mind-resale projections byte-identical for holder, GOD and operator minus
  `offer`; the holder's `offer_paid_mind_resale` plan prepares; GOD's refusal on the
  held licence equals that on a missing one: `ProviderRejected: This paid licence
  transfer is unavailable or has changed.`), green in 4376.94 s; mush-server
  `player_market.rs` gains `initialized_launch`, `launch_active`, `census_routes` and
  `scratch`, C1b's launch test now asserts the granted operator is offered both
  `OPERATOR_GATED` verbs,
  `named_launch_composes_the_player_market_with_parity_and_restart` (six-principal
  census at the venue room passes `projection_rules`, a restart is identical, and a real
  Active `boot::build_engine` on the same store has an equal-length offer route table
  resolving every census route to the same `DefinitionRef`), and
  `named_launch_market_terms_change_across_a_reboot` (an empty initialized world reboots
  Active on `price_minor: 45_000` and `renewal_offer_ic_seconds: 172_800`,
  `@mind/launch` shows `450cr` and `open 2 IC days;`). Docs and deploy: the closing
  paragraph of `docs/help/personal-minds.md`, one sentence in the `@mind`
  legacy-handlers note, a commented `# launch_path =
  "/opt/rustmush/<instance>/data/launch.json"` stanza under `# [tacsim.minds]` in
  `deploy/instance/mushd.toml.tmpl` (comments only), and the `deploy/README.md` section
  "Named launch and the player market (17g C1)" (the no-commerce consequence, what
  format 2 adds, a 12-step bring-up from a fresh store with `@org/grant` operator
  grants, the kill switch, the provisional defaults, the terms-change finding, the
  format-1-to-2 fresh-store rule).

**Evidence.**
- **The loop.** The plan was revised once, after a feasibility lens and an adversarial
  lens; all fifteen findings were adopted (plan §10). Each sub-cut then ran §3's loop: a
  builder, three review lenses, a fixer, a gate runner and an independent judge, then
  one commit and one push. The judge's verdict was ship-with-notes all three times. The
  judge's texts give no must-fix counts. They name the review fixes they carried: in
  C1a, the census's routed `action` key and its per-kind classes, after a false comment
  had skipped the generic Retire and Refund kinds; in C1b, GOD's positive Active
  assertions, from its fix round. The close sweep needed no repair.
- **C1a's gate**: fmt for tacsim, mush-server and mush-roll; workspace clippy with `-D
  warnings`; strict rustdoc; mush-roll 284 + 2; tacsim 3,605 passed, 0 failed, 16
  ignored; mush-server 570/0/3; contract tests 138/0/8; and the original-purchase story
  by name in 1,321.30 s, carrying the fence case. No gate moved.
- **C1b's gate**: the same three static gates; the focused `launch::`,
  `hardware::flow::launch`, `player_market` and `boot_parity` targets; tacsim
  3,612/0/16; mush-server 572/0/3; contract tests 138/0/8.
- **C1c's gate**: the same three static gates; the focused `launch` and `player_market`
  targets; tacsim 3,615/0/16; mush-server 574/0/3; mushd `boot_composition` 6/0,
  mush-config `shipped_configs` 13/0 and mush-server `provisioning_smoke` 4/0; contract
  tests 138/0/8; and the named Coprates story on format 2 by name in 4,376.94 s (about
  73 minutes).
- **The close sweep** (round 1, green) at `9002169e`, over the five crates changed since
  `40db0019`, the merge base with `origin/main` (mush-contract-tests, mush-core,
  mush-roll, mush-server, tacsim): `cargo fmt -p <crate> -- --check` for each; `cargo
  clippy --workspace --all-targets -j 8 -- -D warnings`; `RUSTDOCFLAGS="-D warnings"
  cargo doc --workspace --no-deps --document-private-items -j 8` (21 m 23 s); `cargo
  test -p mush-contract-tests -j 8 --no-fail-fast`, 138 passed, 0 failed, 8 ignored
  (`scale_harness`) across 31 binaries. The only warning in the clippy and doc logs is
  mush-net-web's known build-script note (no `web/dist/index.html`).
- **The PR's CI.** The sweep opened PR #14. Its run `40368266688` tested the merge ref
  of `9002169e` with `main`, which had moved past `40db0019` through the SM.12 train,
  the T.0 ten-shard reshard (PR #12) and PR #11 (`git merge-tree`: no conflicts). It
  concluded success on attempt 3, every job green, `tacsim-integration-1..4` included.
  Attempts 1 and 2 each failed only `test (rest-mushd)`, each on a different known flake
  that passed in the other attempts:
  - `mushd --test observer_rule`
    `nothing_of_the_shooting_crosses_to_the_world_or_to_the_target` (attempt 1: a
    timeout waiting for "Hit on the "). It is on the load-sensitive list, a sibling in
    the same binary failed on CI before this cut (run `40253195539`), and it passed
    alone locally, 32 of 32 in 200 s.
  - `mushd --test web_e2e` `spa_if_built_and_healthz_are_served` (attempt 2: `/healthz`
    answered `ok: false`, `web_e2e.rs:999`). `main` failed the same assertion in push
    run `40225850544` on 2026-10-25.

  C1 changes nothing under `crates/mushd`, `crates/mush-gateway`, `crates/mush-net-web`
  or `web/`.
- **Not run.** The four M7 stories as a set, on the CI dispatch: the PR run skips the
  `M7` matrix job, and plan §5 requires all four before the merge. Two ran locally
  inside the sub-cut gates (the original-purchase story in C1a, the named Coprates story
  in C1c); the server persistence story and the Conduct story have not run on any C1
  build. There was no hand-played session on this build and no deploy, and `launch_path`
  is live on no instance. `cargo test -p mush-bot` was in no gate; the bot reads none of
  the new `@mind/launch` lines.

**Deviations and residuals.**
- **From §3 C1 (plan §1, the hand-off of plan §9).**
  - The production-roster boot test landed in C1a, not C1c, because C1a is the first
    sub-cut that can compose the whole roster. §6 risk 1 is closed by it, by C1b's first
    named-launch boot through `game_modules` and by C1c's composed-market census.
  - The Coprates fence on paid resale landed in C1a, a builder-and-owner change that C1c
    inherits unchanged.
  - `launch_path` is documented and commented, not live, in the deploy templates, where
    §3 C1c said to set it. Bring-up is two-phase by construction: finalize needs
    identities only a booted world can create, `check_config` refuses a missing file,
    and `instance.env` cannot say "unset". So after C1 merges the dev worlds compose no
    mind commerce at all, C0's House chain included, until an owner decision makes the
    path live. The plan made this departure conditional on the lead's acceptance before
    C1c; that acceptance is not recorded in the build's inputs (residual below).
  - The Registry operators are granted with the baseline `@org/grant`, a runbook step at
    Initialize, so C1 adds no command and writes no grant code (legacy-handler rule 1).
    The capabilities stay vocabulary in the composition, as §3 C1c found them.
  - The launch `HELP` constant is unchanged. Bare `@mind/launch` appends a
    deployment-rendered market line, and a God-only readiness line.
  - The service actor is a deployment-named clerk, never #0 or GOD, where §3 C1b said
    the tests use GOD and 60 s; the provisional resale window is 86,400 s.
  - The runbook's clerk is `MarketClerk`, created with `@newchar`: the
    provisional-defaults list above says `Market Clerk`, which `validate_character_name`
    refuses.
  - C1c test 7's outcome: an empty initialized format-2 world reboots Active on changed
    terms and `@mind/launch` shows them. A terms change after filings, quotes or
    licences exist is untested; the runbook says to treat it as needing a fresh store.
    That is the owner flag.
- **C1a.** (1) `resale_routes::exercise` gained a parameter, `admission:
  Option<(&AdmissionProbe, HouseIssuer)>`, because the House resale offer is made inside
  it; the two personal callers pass `None` and no assertion changed. (2) The brief's
  'stub that refuses one buyer while an AtomicBool is set' became `AdmissionProbe`,
  which refuses exactly the armed (issuer, buyer) pair and records every consultation;
  this is stronger, because it also binds the fence to the licence's own House issuer
  and the accepting buyer, and proves nothing consults it before the buyer's Accept. (3)
  Test 7 fences Accept only: with no open paid claim, Fund refuses on the claims check
  first, so the fence at Fund is unproven until C1c's composed story. (4) The 17g C1
  design note's 'God's resale plan on the buyer's House Pin refuses with the same bytes
  as for a nonexistent licence' was built, as the brief specifies, as the buyer's Accept
  refusing with the same bytes as an Accept naming a nonexistent offer token. (5) The
  census adds a routed `action` key to every entry, and checks classes C, D and E per
  kind for `market-original-mind-offer` and `market-original-mind-purchase`; this was
  the review fix for a false comment that had skipped their generic Retire and Refund
  kinds. Test 9 also asserts that the counter removes nothing, and that every C1 offer's
  route resolves in `Running::routes()`. (6) Clerk and operator are characters of one
  second staff login account, `player-market-staff-two`, because the plan names none.
  (7) Bounded `Busy` retries were added to `create_player`, `legal_actions` and the
  Initialize offer, after a first full-suite run failed with `Error: Busy`; the typed
  Busy from `Running::offer`'s internal `legal_actions` read is still not retried (a
  residual). (8) The builder tests reuse `house_engine(c, true)` as the market-composing
  helper rather than adding a duplicate. (9) Deferred to C1c or the owner: re-scoping
  the `CopratesAdmission` descriptor (`fresh_house_purchase_and_renewal`), because the
  brief forbade a launch change; and, for 17g §5, two owner questions: whether a resale
  counts as a new purchase for Coprates, and the Fund-to-service-settle window that is
  not re-checked. To remove the fence: delete the `with_buyer_admission` install in
  `with_paid_mind_resales`, the ordering refusal in `with_house_minds_and_admission`,
  and the second field of `Mode::Admission`.
- **C1b.** (1) Stop-rule fix older than C1: `org_capability_vocabulary()` lacked the
  three capabilities the named launch's governed hardware authorizes, so the production
  `@org/grant #1=Basin industry.core_die.issue scope=#31` refused "Unknown capability"
  and the named launch could never be set up through `game_modules`. tacsim's rig
  composes an org module with no vocabulary, so it never saw this. The fix is about 17
  lines adding `NAMED_LAUNCH_HARDWARE_CAPABILITIES`, vocabulary only, with the unit test
  `the_org_vocabulary_admits_every_named_launch_grant`. The refusal came from seeding
  the launch, not from the launch boot itself; the fix was accepted under the stop rule
  because without it the launch cannot be set up. (2) The one changed existing
  assertion: `mush-server` `the_org_vocabulary_covers_every_economy_capability` adds
  `NAMED_LAUNCH_HARDWARE_CAPABILITIES` to its containment loop and its exact-length sum.
  This follows from (1), and it extends the test rather than weakening it. (3) The two
  mush-industry capability ids `industry.core.integrate` and `industry.core.refit` are
  literals, because mush-industry exports no constants for them; exporting them is
  deferred. (4) Test 3 proves each identity refusal on a fresh `setup_with(2)` rig's
  first launch boot, not on a reboot after a successful Initialize, because a refused
  `Rig::restart` consumes the world; the rigs are deterministic. A reboot-after-boot
  case with a destroyed clerk is deferred to C1c. (5) Test 5 adds two things the brief
  did not ask for: an INTERIM check that the granted operator is offered no
  `OPERATOR_GATED` verb (C1c replaces it), and positive Active assertions for GOD
  (`issue-core-die` offered, no initialize verb) added in the fix round. It also still
  prints the deployment with `eprintln!`. (6) `created()` now parses the leading digits
  after its marker, and C1a's busy-retry loop moved into `submit_initialize`. Both keep
  their behaviour, to cover more acknowledgement formats. (7) Deferred to C1c: the
  example's ids are illustrative and its `registry_issuer` is `hardware.factory` (the
  runbook must say to replace them); the terms-change reboot behaviour (C1c test 7); the
  org-existence clause shadowed by the ownership clause, and the positive 32-operator
  and multi-operator boot cases (test-coverage notes). (8) 17g §3 C1's "the tests use
  GOD" service actor is superseded by the brief: the clerk is never #0 or #1.
- **C1c.** (1) mush-core `MAX_OWNER_PARTICIPANT_REQUESTS` went from 128 to 256, a core
  change taken under plan §8's local-fix allowance. With the market composed, the
  format-2 Active launch declares 156 owner-participant requests and the old bound
  refused the boot. The constant also caps each registration's `allowed_actions`, so
  that cap doubled too, and a new `register()` 256/257 test covers it. The 156/91
  figures are a measurement that no test pins. The lead's acceptance must be recorded,
  and 17f's 128/129 passage folded to 256/257. (2) The tacsim rig's `logins(r, d)` puts
  the clerk and every operator on a `launch-staff-two` login, because the paid-resale
  writer requires distinct personal accounts and check (b) refused without it. This is
  fixture-only and mirrors `launch_world`. (3) The brief's precondition, the lead's
  acceptance of plan §1 departure 3, was not recorded before the build. The builder
  followed brief §3's default: `launch_path` is commented in the template and the
  runbook opens with the no-commerce consequence. The committer records the acceptance
  or the opposite ruling; the opposite ruling changes only the comment stanza and the
  README opening. The owner's live-path mechanism question goes to 17g §5. (4) The
  runbook departs from the brief's bring-up so that it works as written. It adds a
  `launch.faucet` system account that mints one 100cr filing fee into each House
  treasury, and a second login (`create LaunchAgent <password>`) for the second origin
  agent. It creates the clerk and the operator as `@newchar MarketClerk` and `@newchar
  RegistryOperator` on that login, because `validate_character_name` refuses the brief's
  and 17g §7's `Market Clerk` (17g §7 to fold). It also says to take `launch_path` out
  of the live config before a reset. (5) Test 4 checks the tuned terms by calling
  `market_help` on a decoded copy rather than through a rebooted `@mind/launch`.
  Dispatch under non-provisional terms is proven by mush-server test 7. (6) The market
  line is brief-verbatim. It omits `roll-retire-authored-draft`, which `+act` also
  refuses as ambiguous; this is noted in `market_help`'s doc. Naming it is the lead's
  call, and it is on C2's split list. (7) The market line, and GOD's readiness line,
  also print on a format-2 Initialize boot, because the brief's dispatch condition has
  no mode. No test covers this. (8) The template stanza is six comment lines, not four
  or five, and it separates `odds_floor_permille`'s description from its key. (9)
  Deferred: a terms change after records exist is untested, and test 7's refusal branch
  is unexercised. The readiness line reads `ready` for a GOING operator, whom the
  providers refuse. Story check (c) holds partly by the provider's uniform refusal, with
  GOD naming the buyer's `seller_account`. The `CopratesAdmission` descriptor scope
  still says `fresh_house_purchase_and_renewal` (C1a residual), and a House-born resale
  Fund and a personal-born resale under the admission are untested end to end. The
  README overstates test 7 ('every offer is listed as before') and lists personal filing
  among format 2's additions, though it predates C1. `activate(.., true)` is unused, and
  C1b's `NAMED_LAUNCH_DEPLOYMENT` eprintln is kept.
- **Residuals still open** (the three sub-cuts' lists merged; closed items dropped:
  C1b's two interim pins were replaced in C1c, and C1c's runbook tells operators to
  replace every id in the example and that `registry_issuer` names a system principal of
  their choice).
  - **The lead's acceptance of `launch_path` documented, not live**, is unrecorded. The
    lead records it before the merge, or rules the other way, which reshapes only the
    template stanza and the README opening. Until the path is live for an instance, that
    dev world composes no mind commerce, not even C0's House chain.
  - **`MAX_OWNER_PARTICIPANT_REQUESTS` 128 → 256** needs the lead's recorded acceptance.
    It doubles the per-registration `allowed_actions` bound too. The format-2 Active
    count (156; format 1, 91) is a measurement no test pins, so C2 and C3 owners spend
    the 100-request headroom silently until a boot refuses. 17f's N51 passage now notes
    the change.
  - **The Coprates fence on resale.** The `CopratesAdmission` descriptor
    (`tacsim/src/launch/admission.rs`) still declares `scope:
    fresh_house_purchase_and_renewal` although it now also gates resale; re-scoping it
    moves the descriptor's digest and waits on the owner's ruling. `publication.rs`'s
    comment overstates parity: a fresh House purchase re-checks at Grant, while a
    pending resale Fund is settled later by the service (`Mode::Service`) without a
    re-check, so a buyer who leaves the Republic between Fund and settle still receives
    the Pin (benign under `historical_paid_use: false`). The fence's only behavioural
    proof runs inside the `#[ignore]`d original-purchase story and covers Accept only:
    Fund on a House-born licence, and a personal-born resale passing with a refusing
    admission installed, are untested end to end (C2 or C4).
  - **The census runs in an empty Active world.** Classes B and D are vacuous, and
    byte-identity cannot detect a holdings-dependent oracle; `projection_rules` should
    be repeated after seeding a filed mind, a listing and a held licence (C2 or C4).
    Clerk and operator are characters of one login account in both rigs
    (`player-market-staff-two`, `launch-staff-two`), so account independence rests on
    the counter trio. Story check (c) holds partly by construction (one uniform refusal,
    and GOD's plan names the buyer's `seller_account`); a variant where GOD names his
    own personal account would make the licence lookup decisive (C2 or C4). The pointer
    and balance checks after `refused_admission` follow a prepare-only refusal and
    cannot fail; the evidence is the (Err, Err) pair, the probe record and the
    byte-equal refusals.
  - **Possible flake** (RESOLVED 2026-10-27): `initialized_active` retried only a
    `disposition=busy` submit, not the typed `LegalActionReadError::Busy` from
    `Running::offer`'s own `legal_actions` read, and `paid_purchase`'s own offers
    retried nothing. It materialised once, on PR #29's CI run 40440170786:
    `public_commerce_setup_accepts_generic_committed_receipts` was answered `busy` on
    `roll-reserve-mind-charter` right after its Active boot, because the world task
    refuses an `AuthoritativeAction` at once while any commit is pending, and every
    periodic module tick is armed against the loaded world's clock at engine
    construction, so the first Timer wake after a boot fires them all and its flush is
    the process's first commit (reproduced locally under parallel nextest). The
    retry lives in the harness: `Running::offer` resubmits a `Busy` submission (17g C2
    close) and its `legal_actions` read a `Busy` read (C2a), both bounded at 50 tries;
    PR #34 re-asks `connect`'s and `command`'s barrier and readiness probes the same
    way, keeps the owned-action poll going on a `Busy` status read, and drops
    `submit_initialize`'s own loop.
  - **The market line.** It names only `paid-mind-resale` and the
    `market-original-mind-*` offers as waiting for C2, though
    `roll-retire-authored-draft` also projects two offers to every player and `+act`
    refuses it as ambiguous; naming it is the lead's call, and it is on C2's split list.
    Bare `@mind/launch` also prints the market line, and GOD's readiness line, on a
    format-2 Initialize boot, when no market owner is composed; no test pins that
    output. The readiness line reads `ready` for an operator flagged GOING, whom the
    providers refuse: it reports grant effectiveness, not offer reachability. C2 must
    update the line's last sentence in its three pinned places.
  - **Terms changes.** C1c test 7 pins only the reboot-boots outcome in an empty market,
    and its refusal branch is unexercised; a change after filings, quotes or licences
    exist is untested. The README's "every offer is listed as before" says more than
    test 7 asserts (only `projection_rules`), and its format-2 list names personal
    filing, which predates C1.
  - **Coverage gaps from C1b**: the org-existence clause of `actual_identities` is never
    exercised alone (the ownership clause refuses first); no valid 32-operator list is
    decoded, and the boot check never sees more than one operator; the identity refusals
    run on a first launch boot, never on a reboot after a destroyed clerk.
    `NAMED_LAUNCH_HARDWARE_CAPABILITIES` holds two mush-industry ids as literals,
    checked by tests against the same literals, so a rename there is caught only by
    mush-server's `player_market` test 5; and the vocabulary test's exact-length pin
    trusts the production constant, so a fourth id would widen `@org/grant`'s vocabulary
    unnoticed.
  - **Hygiene**: C1b's `eprintln!("NAMED_LAUNCH_DEPLOYMENT …")` is kept; `activate(..,
    true)` is never called, and the story repeats that sequence by hand; the template
    stanza is six comment lines and separates `odds_floor_permille`'s description from
    its key.
  - **The named story** takes about 73 minutes locally, is `#[ignore]`d and off the
    default CI shards, and runs with the other three on the dispatch before the merge.
- **For §5 (not yet folded there):** the live `launch_path` mechanism (an empty value
  meaning unset, or a per-instance overlay); whether a resale counts as a new purchase
  for the Coprates fence, with the unchecked Fund-to-settle window beside it. The lead's
  two acceptances above are the lead's, not the owner's.

### C2 — plan (2026-10-26, branch `claude/m7-c2-operator-commands`)

As planned against §3 C2 in four sub-cuts (plan:
plan-data/m7-completion/cuts/C2/plan.md), revised after two review lenses (§11
dispositions) and refreshed at C1's landed tip `ecc26f0e` (main with the merge
`3d584827`, PR #14, and the owner's test-strategy cuts T.0–T.3). **One mechanism.**
Every new command is a line-mode `Ctx` handler that reads only the actor's own committed
or public state: tacsim's library and bulk runs, Roll's own-only `RollCommandReader`,
the market's bounded `MarketCommandReader`, and mush-core's new read-only
`ShellReservations::open_request`. It then submits exactly one existing legal action
through `Ctx::submit_authoritative`, keyed by verb, subject, IC second and an ordinal
(the industry idiom). The server's projection resolves the actor's own offer, so no
command adds authority and every invocation refusal is the driver's one line. Renewals
are the one front over existing code: `+minds/renew` calls the renewal services'
factored `player_step`, which `@mindrenewal` and `@housemindrenewal` keep calling
byte-identically, and the service still grants or refunds a funded offer. The commands
open only where Active commerce composes the player market (entitlements), so a
House-only launch says the market is not open. Four shared verbs split one per kind:
paid resale, v2's offer and purchase, and the two authored-retirement versions. The
census goes to 45 verbs; only the presentation changes, so no digest moves. C2 adds no
participant request (156 of 256 stays) and two owner-read services with two grants; the
refresh's static count puts the format-2 launch at 24–29 of the 32 before C2, so C2a's
composition test prints the exact figure and the fallback (fold Roll's kinds into
`OriginalOwnerReader`, then the lead raises the caps) is pre-decided.

**What the as-built protocols forced.** The approved nouns stay: `<filed>` for update,
attest, publish, charter and recall, with licence, line and charter derived server-side
from the actor's title. Nine departures go to the lead (17g §5, carried by the
committer): withdraw takes `<filed>` without `@<rev>`; publish's `price=`, `seats=` and
`term=` are optional and accepted only at the market's values (policy-wide as built);
`+minds` reads the v3 and derived listings, not v2's `Query::MarketOriginalMinds`;
`+minds/buy` is four explicit steps; resale names its buyer, with `/accept` under both
`/resell` and `/charter`; both accepts list the offers naming the actor and submit only
the one whose frozen price the buyer repeats with `price=`; renewal has `/pay` and
`/decline` only; recall and revocation take no reason; the story stops at IC holdings
(flying the mind is C4's story 1). **C2b is the only owner change, on the money paths,
with no command.** It adds four indexes whose audits do not depend on the clock: the
per-buyer entitlement index (8 quotes, 8 purchases; removal by the operation's own
buyer); one paid-holdings row per held licence, written at all five pointer-holder
writers (the two renewal grants replace the pointer with the same holder and write no
row), with no cap; and the (buyer, licence) and (buyer, charter) named-offer indexes (8
live offers each). It also adds the author's `transfer=no` mark, which only narrows and
moves no record format. Charter transfers by command always use the pending protocol:
one protocol and one index, correct whether or not sorties are out. A resale's Accept or
Fund follows the provider's own open-claim test.

**The gate.** §9 story 1 through the commands needs one genuine Registry rating, so it
cannot fit the focused gate. It is the `#[ignore]`d mush-server lib story
`world_task::submission_disposition::tests::player_market_story::player_market_commands_story_two_accounts_alt_stale_tiny_and_restart`,
the fifth `mind-commerce-acceptance` matrix cell (`name`, `package`, `test`, as the four
are), and grows with a restart after each part: C2a rate, file and a refused attest; C2c
publish, the alt refused at quote, a 30-second quote that expires and is refused by the
provider, a 400cr sale with its split, an open `transfer=no` sale and a paid fork; C2d
two offers and a price-confirmed 0.21cr resale, the marked licence's refused resale, a
not-due renewal, withdrawal, a clerk-settled charter, revocation and recall. Focused
tests cover every command's wrong-principal refusal (under a placeholder comparison
rule), the bystander, delimited privacy checks, the format-1 not-open case and the
120-column widths. Since T.3 a crate's integration tests are group binaries, so every
run line is `--lib <filter>` or `--test <group> <module>::` (`--test shard_6
cockpit::…`, `--test commerce_composition mind_commerce_composition::`), C2 adds no
integration test file (its new tests are lib modules), and each sub-cut's gate is the
focused local runs (about 15 minutes) ending with `scripts/vps-gate.sh --wip --runner
conclav-dev`, the workspace sweep on the owner's 32-core VPS runner; the PR's
fifteen-cell CI runs on every push and the five M7 stories on the lead's dispatch after
C2b. C1a's typed-Busy residual is fixed in `Running::offer` (`legal` and
`submit_initialize` already retry). No IPC, proto, TS, gateway or web change. C2 adds
four families and two owner-read definitions; the fresh store is already required.
Provisional defaults (ruling 6) are listed in plan §7. Owner questions for 17g §5:
per-listing prices, seats and terms; a recall and revocation reason; House catalogue
purchases by command; an offers-to-you list; the gate's sim half moving to C4.

Sub-cuts:
- **C2a** The command adapter, the shared-verb split, the author commands and the
  story's first part
- **C2b** The owner indexes on the money paths and the author's transfer mark
- **C2c** The market commands: listings, purchases, publication and holdings
- **C2d** Renewals, title trades, Registry operators, the help rewrite and the story's
  last part

Provisional defaults (17g section 1 ruling 6):
- Pending-charter offer window (valid_for_seconds) 86,400 real seconds, the protocol's
  MAX_OFFER_SECONDS, matching the paid-resale offer window
- Charter transfers by command always use the pending-charter protocol (one protocol and
  one buyer index, correct with or without sorties out); the immediate charter offer
  stays reachable through +act
- Listing royalty 0 per mille unless stated; accepted 0 to 300; 0 required on a sealed
  listing
- Listings are transferable unless the author writes transfer=no; the mark only narrows
  the policy-wide transferability, never widens it
- @mind/publish's price=, seats= and term= are optional and accepted only at the
  market's current values (the policy price, 1 seat, 90 IC days); per-listing terms are
  deferred
- Resale and charter price bounds 0.01cr to 1,000,000cr (1 to 100,000,000 minor, C1b's
  deployment ceiling), written `<whole>[.<one or two digits>][cr]`
- Accepting a named offer requires price= equal to the offer's frozen price; without it
  the command lists the offers naming the actor and submits nothing; an equal-price tie
  takes the offer expiring last
- Per-buyer entitlement index bounds: 8 live quotes and 8 open purchases per character
  (v2's MAX_BUYER_QUOTES); expired quotes pruned at the buyer's next quote
- Named-offer index bounds: 8 live offers per (buyer, licence) for resale and per
  (buyer, charter) for pending charters; a ninth refuses only the seller's Offer
- Paid-holdings index: one row per held paid licence, no per-holder cap (no new refusal
  on a money path); renewal grants keep the holder and write no row
- Index boot audits are clock-free: never require an indexed quote or offer to be
  unexpired
- Listing pages of 20 rows; holdings show up to 20 owner rows and 20 paid rows, each
  section paged by its own cursor
- Listing code: exactly the first 8 characters of the publication token; a two-listing
  collision refuses AMBIGUOUS and points to +act
- Charter name defaults to the mind's library name (1 to 80 bytes, the Roll bound)
- Payer and seller account: the actor's one personal tacsim.credit account; none or
  several refuse ACCOUNT (a deliberate narrowing of 17f §2.3's buyer choice, which stays
  on +act and C3)
- Idempotency discriminator: `mind.<verb>.<subject>.<IC second>.<per-second ordinal>`,
  the industry command idiom
- The listing detail discloses the Registry levy always and the venue cut and parent
  royalties when nonzero
- Renewals: +minds/renew shows status or requests an offer once due by the service's own
  due rule; /pay and /decline only; the existing renewal service grants or refunds a
  funded renewal
- Registry recall and revocation commands take no reason, because the as-built actions
  carry none
- House catalogue purchases stay on +act house-* (no per-buyer House index); House Pins
  are listed, renewed and resold by command
- Width maxima for the 120-column test: 8-digit numbers, 24-character listing and
  20-character holdings name columns, 12-character prices, class III, the server's
  longest player name, four-digit minutes
- The gate story's deployment uses quote_seconds 30 (a test value) and PROVISIONAL terms
  otherwise; its tiny-amount case is a 0.21cr resale beside a dearer 5.00cr offer
- Owner-read capacity: the static count (24–29 of 32 services before C2) is taken as
  sufficient for C2's two services and two grants; C2a's composition test prints the
  exact figure and the fallback (fold into OriginalOwnerReader, then the lead raises
  both caps to 48) applies only on a refused boot
- Each sub-cut's workspace sweep runs on conclav-dev through scripts/vps-gate.sh --wip
  --runner conclav-dev as the gate's last command; local runs stay focused (about 15
  minutes)
- C1's provisional defaults are unchanged: 400cr Pin and renewal, 300 s quote, 3 IC-day
  renewal window, one-day resale window, 3 IC-day charter settlement, the
  deployment-named clerk, one operator

### C2 — as built (2026-10-27, branch `claude/m7-c2-operator-commands`, PR #30)

As built, against §3 C2 and the plan above: four sub-cuts, each one commit pushed to the
branch, then one close sweep with one repair commit (`3e4f7668`). PR #30 is open, not a
draft and not merged; the four M7 stories and the fifth, the command story, run on the
CI dispatch before the merge (below). The cut's landed shape is narrower than the
plan's: the author's `@mind` commands and the Registry pair work on a format-2 launch,
every `+minds` form and `@mind/publish` answer `NOT_OPEN` until the lead raises the
owner-read caps, and the four owner indexes of C2b wait for a mush-core ruling, with the
renewal front and the named-offer accepts held behind them.

**Landed.**

- **C2a** (`bfc1b34e`): C2a — as built (2026-10-26, branch
  `claude/m7-c2-operator-commands`, uncommitted on `09b9dcdb`; gate 2 green on 21
  commands, VPS sweep run 40458491284 on conclav-dev 11,180 tests 0 failed). **The split
  (plan §2.3)**: `mind_paid_resale::actions::presentation(Kind)` gives
  `paid-mind-resale-offer|-accept|-fund`; `mind_trade::provider::verb(Kind)` gives
  `market-original-mind-list|-quote|-retire-listing|-retire-quote|-expire`;
  `mind_trade::purchase_provider::verb(action_id)` gives
  `market-original-mind-fund|-grant|-refund`;
  `authored_retirement::provider::verb(&DefinitionRef)` gives
  `roll-retire-authored-draft-v1` for version 1 and keeps `roll-retire-authored-draft`
  for version 2; labels, help and every digest unchanged; `Kind` enums widened to
  `pub(crate)`; tests `shared_verbs_are_split_one_per_kind` (eleven distinct verbs plus
  a real v2 projection carrying no old verb) and
  `retirement_versions_project_distinct_verbs`. Pinned strings moved in
  `player_market.rs` (`C1_VERBS` 36→45, `SPLIT_BY_KIND` and `fn action` removed,
  `COUNTER_KINDS`/`GOD_KIND`/`GENERIC_KINDS` by verb, new `SPLIT_AWAY` census error),
  `paid_purchase.rs` (list, quote, fund), `launch/story.rs` `resale_privacy` and the
  `launch.rs` positive control (the three-verb set), `GENERIC_MARKET_VERBS` 22→26,
  `PROVISIONAL_MARKET_LINE` and `crate::launch::market_help` (`a
  market-original-mind-list listing`; `Until the +minds commands exist, use +actions and
  +act <verb> with <JSON object>.`). **mush-core**:
  `ShellReservations::open_request(&self, &OwnerReadContext) ->
  Result<Option<RequestUid>, OwnerReadError>` (own `open` row via `action_state`, format
  and requester checked, `Integrity` on mismatch); `Registry::owner_read_counts() ->
  Option<(usize, usize)>` over `OwnerReadRegistry::counts()`; test-only
  `OwnerReadContext::for_test`; test `open_request_reads_only_the_callers_open_shell`;
  caps still 32/32. **The Roll read, folded (plan §2.4 fallback 1)**:
  `mush_roll::commerce::command_reader` holds `RollCommandReadRequest::{open_shell(),
  own_filed(FiledRevisionId)}`, `RollCommandReadResult::{OpenShell{request:
  Option<RequestUid>}, OwnFiled(OwnFiledView)}` (`open_shell()`, `into_own_filed()`),
  `OwnFiledView` getters `filed, charter, line, owner_licence, dependent, name,
  owner_phase`, `OwnerPhase::{Active{days_left}, Grace{days_left}, Lapsed}` (whole IC
  days rounded up, NotEffective reads Active), `pub(super) MAXIMUM_ROWS = 64` and
  `pub(super) fn read(ctx, roll, families, shells: Option<&ShellReservations>, request)`
  (every kind `Unavailable` without `shells`; `own_filed` keys the filed→charter→birth
  operation→owner licence chain on `charter_transfer::history::current` for the actor
  and `owner_attestation::history::current` for the term, every failure `Refused`); no
  service, descriptor or grant of its own. `holding_reader::OriginalOwnerReader` carries
  it: `ReadKind::Command`, `OriginalReadRequest::command(..)`,
  `OriginalReadResult.command` (`skip_serializing_if` none, old encodings
  byte-identical) with `into_command()`, field `commands: Option<ShellReservations>`,
  `register(.., expected, commands)`, descriptor `original_owner_holding` **version 2**
  with config member
  `"commands":{"own_only":true,"maximum_rows":64,"account_ids":false}`.
  `CommerceConfig.mind_commands` set by `with_mind_entitlements` through crate-private
  `with_mind_commands()`; `holding_reader::register` moved from
  `register_active_readers` into `register` after `register_filing_declarations`
  (Active-only as before) passing `config.mind_commands.then(|| shells.clone())`.
  `command_actions::RollCommandActions::capture(&DefinitionRegistry)` with `reserve()`,
  `file(dependent)`, `update(derived)`, `withdraw()`, `attest()`, `fork()` matching
  owner `roll` kind `action` ids `reserve_initial_filing_shell`, `initial_filing`,
  `dependent_initial_filing`, `update_original_mind`, `update_derived_mind`,
  `withdraw_authored_mind`, `attest_owner_mind`, `fork_paid_mind` at version 1 (unique
  match or `None`). Tests
  `command_reader_answers_only_for_the_actors_own_shell_and_filed_minds` (real second
  Player; own shell present, absent for another, consumed after the real filing; exact
  `OwnFiledView` JSON; forbidden words; foreign and missing both `Refused`),
  `command_kinds_are_closed_without_the_player_market`,
  `owner_phase_rounds_whole_ic_days_up_and_reads_grace_and_lapse`,
  `command_actions_capture_only_registered_actions`; `filing_tests` gained
  `TacsimReads`, `Fixture::new_with_reads(credit, reads, commands)`,
  `try_engine`/`engine_with`, widened `Fixture`, `config`, `token`, `request`.
  **tacsim**: `minds::market_commands::{mod, words, author, tests}`; `MindCommandService
  { roll: OwnerReadHandle<OriginalOwnerReader>, attempts: Arc<Mutex<(i64,u64)>> }`
  registered by `market_commands::register(reg, handle)` from `TacsimRuleset::register`
  when `TacsimRuleset.mind_commands` is set (by `MindCommerce::configure_tacsim` only
  when `mode == Active && state.entitlements.is_some()`), refusing without a Registry, a
  differing manifest or the entitlement holdings;
  `match_::original_access::register_entitlements` now returns its
  `OwnerReadHandle<OriginalOwnerReader>`; `service(ctx)` refuses `NOT_OPEN` unless
  tacsim owns the slot; `next_key` → `mind.{verb}.{subject}.{at:x}.{ordinal:x}`;
  `submit(ctx, verb, subject, action, parameters, request)` (principal, then `BUSY`,
  then the key, then the request line, then one `submit_authoritative`);
  `personal_account` (unique Active unscoped `tacsim.credit` account of
  `PrincipalId::Object(actor)`, else `ACCOUNT`). `author`: `SWITCHES`
  rate/file/update/withdraw/attest/fork, `USAGE_RATE_FILE`/`USAGE_FILED`/`USAGE_FORK`
  (the HELP lines), parsers `number`, `revision_reference`, `charter_name`,
  `file_arguments`, `update_arguments`, `fork_arguments`, builders `reserve_parameters`,
  `file_parameters`, `update_parameters`, `withdraw_parameters` (decimal string),
  `attest_parameters` (number), `fork_parameters` (decimal string licence, canonical
  name), `rate_refusal(&BulkError, held)`; subjects `reserve`/`file` `<mind>.<rev>`,
  `update` `<filed>.<mind>.<rev>`, `withdraw`/`attest` `<filed>`, `fork` `<licence>`.
  `words` verbatim per the brief plus `RATING_UNAVAILABLE` and `fill()`.
  `workshop::commands`: `SWITCHES` +5, `HELP` +3 lines after `/localcopy`, the six
  switches dispatch to `author::author` after the principal check, the arm is
  `"localcopy"` alone; `workshop::{MindRef::name, saved_revision}`. `registry_service`:
  `start_revision(ctx, mind, number)` factored from `start`, `completed_rating(ctx,
  &Revision) -> Result<Option<RatingRef>, BulkError>`, `holds_rating(&Runtime, ObjId)`
  (pub(super)) and `rating_held(&World, ObjId)` (pub(in crate::minds));
  `bulk::world::Preparing.kind: WorkKind` set at the Proving, Conduct, Registry and
  reconformance sites. `lib.rs` `TacsimRuleset.mind_commands: Option<DefinitionRef>`;
  `composition/mind_commerce.rs` sets it. Tests `author_arguments_parse_strictly`,
  `command_keys_differ_per_retype_and_fit_the_key_grammar`,
  `rate_refuses_busy_only_for_the_characters_own_registry_job`,
  `holds_rating_is_only_the_characters_own_registry_job`,
  `the_command_service_is_composed_only_with_the_player_market` (slot and no read of its
  own: (true,false); holdings v2; prints 18 services / 17 grants for the
  composition-only chain),
  `hardware::flow::launch::author_parameters_satisfy_the_registered_providers` (all 8
  parameter objects resolve through
  `legal_action_query`/`project_legal_actions`/`resolve_legal_action` and pass
  `validate_invocation`; prints the format-2 launch at **31 services / 32 grants of
  32/32**), the two `@mind` help pins. **mush-server tests**: `tests.rs` declares
  `player_market_commands` and `player_market_story`; `paid_purchase/runtime.rs`
  `Recorder` keeps `SendTarget`, `Seen { own, others, to }` with `extend` and
  `reaching(ObjId)`, `Running::{command_seen, take_seen}`, private `legal_actions`
  retrying typed `Busy` (50 tries, 20 ms) used by `offer`; `player_market_commands.rs`
  helpers `same_refusal`, `assert_private`, `Known`, `World`, `world`,
  `world_with_terms`, `active`, `saved`, `await_own`, `until_own`, `open_shells`,
  `QUEUED`, and tests `author_commands_refuse_alike_for_foreign_and_missing_targets`
  (buyer, alt, author; exact /fork pin with `Queued.`),
  `author_command_replies_reach_only_the_actor_and_name_no_digest` (positive `say`
  control, per-recipient checks, settle windows, the Registry's later line),
  `file_reserves_a_shell_then_waits_for_a_rating_across_a_restart`,
  `file_and_rate_read_only_the_actors_own_revision_and_shell`,
  `rate_queues_and_cancels_a_genuine_registry_preparation`,
  `author_commands_are_not_open_on_a_house_only_launch` (format 1),
  `comparison_rule_masks_only_the_typed_number`; `player_market.rs` gained
  `initialized_launch_format` and `initialized_launch_with(dir, format, MarketTerms)`
  and widened its helpers; `player_market_story.rs` holds the `#[ignore]`d part 1 (mint,
  save, reserve, NOT_RATED, rate with the typed `rated()` wait, file with exact pins,
  one filing row, one owner licence, no open shell, the 100cr fee, a refused attest, the
  buyer's and alt's foreign-versus-missing probes for all four commands, `durable()`
  across a restart) with the `// C2c:` and `// C2d:` markers. **Contracts, help, CI**:
  the `@mind` legacy note's C2a sentence; `docs/help/personal-minds.md` "Filing a mind";
  the fifth `mind-commerce-acceptance` entry `player market commands`; `#![cfg(test)]`
  fences on the three new external test modules for the I13 census.
- **C2b** (`337915c4`): C2b landed partially: the author's per-listing transfer mark
  (D-UM-40, C2 plan §2.5, 17g §1 ruling 6's default "transferable unless marked, the
  mark only narrows") and the paid-resale reader's transferability gate; no owner index,
  family, writer or money path moved. mush-market `mind_entitlement`: `offers::List` is
  `pub(super)` and gains a private `transferable: bool` with `#[serde(default =
  "transferable_unless_marked")]` (`true`) under `deny_unknown_fields`, so the List
  request of both entitlement protocols (`OriginalV3` `market-original-mind-list`,
  `DerivedV1`) accepts an optional `"transferable": <bool>` and every existing caller is
  byte-identical; `List::choice(&self) -> ProductChoice` (`pub(super) const fn`);
  `Form::paid` is `const fn`; new `config::ProductChoice { form: PaidForm,
  royalty_permille: u16, transferable: bool }` (`pub(super)`, Copy, Eq) with
  `ProductChoice::frozen(&EntitlementProductV1)` reading a listed product's form,
  downstream royalty (0 when sealed) and own transferability; new
  `EntitlementConfig::terms(minimum, form, transferable) -> Result<PaidTerms, String>`
  returning `personal_terms(minimum, form, self.policy.transferable && transferable)`;
  `EntitlementConfig::product(&self, f: &EntitlementSaleFacts, choice: ProductChoice)`
  now delegates to new `product_for(protocol, &EntitlementSubjectV1,
  &FiledRoyaltyTermsV1, choice)` with `f.protocol()`, `EntitlementSubjectV1::capture(f)`
  and `f.royalty()`, the same inputs as before; the List branch builds
  `self.config.product(facts, request.choice())`; `current_listing` recomputes with
  `ProductChoice::frozen(&l.product)`, so a marked listing stays current under a
  transferable policy and an unmarked one still lapses when the policy closes
  (`policy_ref` still lapses every listing on a policy change); new
  `offers::parameters(kind: Kind) -> Result<Vec<ProjectedParameter>, String>` (the
  mind_renewal idiom) used by `project_offer`, whose List schema is `owner_licence`
  (Text 20), `form` (Choice sealed|open|open_resale), `royalty_permille` (Integer
  0–300), all required, plus `transferable` (Boolean, `required: false`) via new
  `actions::optional(id, kind)`; Quote stays the single required `listing` (Text 32).
  The mark is frozen in the listing row's product, carried by the quote's product and
  digest, handed to the grant by `publication::terms`, and equals the renewal template
  (`EntitlementProductV1::validate`), so a marked licence renews non-transferable; no
  record format, definition, policy payload or digest pin moved. mush-roll:
  `commerce::paid_resale::reader::require_transferable(&PaidTermRecord) -> Result<(),
  PublicationError>` (`pub(crate)`) is the extracted gate `observe` calls after
  `acquired::current`, `Unavailable` on a non-transferable term, behaviour unchanged;
  `commerce::renewal::tests::birth()` delegates to new `birth_with(terms: PaidTerms)`
  with identical terms. Tests:
  `mind_entitlement::tests::the_transfer_mark_narrows_and_never_widens` (test 6: the
  serde default and refusals of `"no"`, `0`, `null` and an unknown field; the List and
  Quote schema shapes and core's supplied/required rule for the marked and unmarked
  request; `product_for` over the fixture subject for all three forms: the mark gives
  non-transferable terms and renewal template, changes the product digest, `frozen`
  reproduces each listed product exactly, a `false` policy gives `false` for either mark
  and the unmarked product no longer recomputes to itself; a quote over the marked
  product validates with its frozen digest and `publication::terms` is non-transferable;
  a half-widened product and a product swapped under the frozen digest both fail
  validation; three mutants killed: `frozen` or `choice` returning `true`, and the
  optional parameter dropped) and
  `commerce::renewal::tests::the_marked_licence_cannot_be_offered_for_resale` (test 12:
  `personal_terms(.., false)` terms through `birth_with`, the renewed generation 2 keeps
  them, `require_transferable` returns `Unavailable` and the `PaidResaleBinding`
  `observe` builds returns `Mismatch` for both generations, the unmarked control passes
  both gates, and a renewal binding over widened predecessor terms is refused). Gate
  green: fmt, workspace clippy, strict rustdoc, `mush-market --lib` 106, `mush-roll
  --lib` 290, tacsim `composition::mind_commerce` 43, `registry_integration_tests` 14 (2
  ignored), mush-server `player_market` 12 (the story ignored), `commerce_composition`
  4, the three contract tests, and the conclav-dev sweep run 40465337439 on tree
  bf006b46: 11182 tests, 0 failed. No existing assertion changed; no command, tacsim,
  IPC, proto, TS, gateway, web, shard, state-slot, family or panic-budget move.
- **C2c** (`e54604ba`): **mush-market**: new `pub mod mind_commands`
  (`crates/mush-market/src/mind_commands.rs`): `MarketCommandReader:
  OwnerReadService<State = MarketStore>` over the entitlement listing family, descriptor
  `owner_read` `mind_market_commands` v1 (payload `format 1, listings_per_page 20,
  code_characters 8, code_rows 2, scan_rows 64, scans 8, seller false, account_ids
  false, digests false, manifest`, referencing the manifest; allowed and dependencies
  `{tacsim}`; an `owner_read` definition validator), registered by `module.rs` inside
  the entitlements branch right after `register_with_house_definitions`
  (`mind_commands::register(reg, families.clone(), config, gate)`);
  `MarketCommandReader::request(reg, manifest)` is TacSim's grant call and has no
  caller; request kinds `at_counter()`, `listings(Option<CommitToken>)`,
  `listing_by_code(&str)`, `listings_after(&str)` (Serialize only), results `AtCounter {
  here }`, `Listings(ListingsPage { rows, next })`,
  `ListingByCode(CodeLookup::{One(view), Ambiguous})`; `answer(source, here, config,
  request)` holds the counter rule and the `current_policy` filter and `read` delegates
  to it over the `Lens` (`scan_action_state` with the key-equals-publication Integrity
  check); `listings` scans at most 8 x 64 rows and, when the budget ends before the
  family, hands back the last scanned key as `next` (a page may be empty and still
  continue); `listings_after` resolves a `MORE` code to the first key starting with it,
  current or not, in one 2-row scan then pages with 7 scans; `by_code` is one 2-row
  scan: none or one non-current `Refused`, one current `One`, two `Ambiguous`; `is_code`
  (exactly 8 of `0-9a-f`); `MarketCommandActions::capture(&DefinitionRegistry)` and
  `entitlement(Protocol::{Original, Derived}, Step::{List, Quote, Fund, Grant, Refund})
  -> Option<DefinitionRef>` matching owner `market`, kind `action`, id
  `{list|quote|purchase|settle|refund}_{original|derived}_mind` at version 3 / 1;
  `PublicListingView` (13 fields with const getters and `code()`), `ListingForm`, consts
  `LISTINGS_PER_PAGE`, `CODE_CHARACTERS`, `CODE_ROWS`. Crate-private seams:
  `Families::listings()`, `EntitlementGuard::current` and `current_policy` (the policy
  half of `current_listing`, factored; `current_listing`'s refusal unchanged)
  `pub(crate)`, `EntitlementListingV1::command_view()`, `mind_entitlement::tests`
  `pub(crate)` with `listing_fixture`, `command_config(price_minor)`,
  `current_listing_fixture`. **mush-roll `commerce::command_reader`**: kinds
  `OwnHoldings { section: HoldingSection::{Owner, Paid}, after: Option<u64> }` and
  `CharterNames { charters }` on the folded `original_owner_holding` v2 read;
  `RollCommandReadRequest::{own_holdings, charter_names}`;
  `RollCommandReadResult::{OwnerHoldings(OwnerHoldingsPage), CharterNames { names }}`
  with `into_owner_holdings()`, `into_charter_names()`; `OwnerHoldingView { filed,
  charter, owner_licence, dependent, name, owner_phase, selling }` (filed =
  `LineRecord::head`, `selling` = a `Funded` `charter_pending_heads` state only),
  `OwnerHoldingsPage::{rows, next}` (asset-id cursor), `CharterName::{charter, name}`;
  `OWNER_ROWS_PER_READ` 3, `OWN_ASSETS_PER_READ` 16, `MAX_CHARTER_NAMES` 20 (more is
  `Limit`, unknown charters left out); `Paid` is `Unavailable`; private `owner_holdings`
  over `assets_of_after(PrincipalId::Object(actor))`, `page`, `pending_sale`, `funded`,
  `charter_names`. **tacsim**: `MindCommandService.market:
  Option<OwnerReadHandle<MarketCommandReader>>`, `None` everywhere;
  `minds::market_commands::market` (`+minds` registered unconditionally in
  `arena_bridge::register` right after `workshop.register` under the Authoritative
  class, `ArgShape::Raw`, `SWITCHES = [next, holdings, buy, pay, complete, refund]`,
  `HELP` the brief's two lines; dispatch by the sorted switch set: `[]` list or detail,
  `["next"]`, `["holdings"]`, `["buy"]` and `["buy","pay"|"complete"|"refund"]` parse
  the code then `handles` then `NOT_OPEN`, else `HELP`; `handles` requires the slot and
  the market handle; `list`, `next`, `show`/`page_lines` (`EMPTY` only at the true end,
  else `HEAD`, rows, `FOOT`, `MORE`), `detail`/`detail_lines`, `buy` built but held
  under `#[allow(dead_code)]`, `publish` (parse, `NOT_OPEN`, `COUNTER`, `TERMS` against
  the launch deployment's `market.terms.price_minor` and `personal_term_days()` 90,
  `NO_FILED` through `author::own_filed`, protocol Derived when dependent, `PUBLISHING`,
  `submit` with `list_parameters(owner_licence, form, royalty, transferable)`),
  `publish_arguments`/`PublishArguments`/`PublishRefusal`, `credits_minor` (0.01 to
  1,000,000), `terms_match`, `quote_parameters`, `Shown`, `row_text`, `detail_text`,
  `percent`, `days`, `form_word`, `class_word`, `row_flags`, `USAGE_PUBLISH`);
  `market_commands::holdings` (`Holdings::{All, Owner(u64), Paid(u64)}`,
  `holdings_arguments`, `holdings`: `All` and `Paid` refuse `NOT_OPEN`, `Owner(cursor)`
  aggregates up to 12 Roll reads into 20 rows, `owner_lines` prints `HOLD_HEAD`, rows
  and `HOLD_MORE owner <cursor>` and never `HOLD_EMPTY`, `owner_line`, `owner_row`,
  `phase_words`); `words`: the brief's words verbatim (`COUNTER`, `EMPTY`, `HEAD`,
  `ROW`, `FOOT`, `MORE`, `NO_LISTING`, `AMBIGUOUS`, `DETAIL`, `TERMS_LINE`, `SPLIT`,
  `QUOTING`, `PUBLISHING`, `TERMS`, `BAD_FORM`, `BAD_ROYALTY`, `BAD_TRANSFER`,
  `HOLD_HEAD`, `HOLD_OWNER`, `HOLD_MORE`) and `words::pending` (`HOLD_EMPTY`,
  `YOUR_QUOTE`, `QUOTE_EXPIRED`, `YOUR_ESCROW`, `ASK`, `NO_QUOTE`, `PAYING`,
  `NO_PURCHASE`, `COMPLETING`, `REFUNDING`, `HOLD_PAID`, `HOLD_WAIT_HEAD`, `HOLD_WAIT`);
  `fill` gains `{key:<n}`/`{key:>n}` padding and `cut`; `author::own_filed`
  `pub(super)`; `@mind` `HELP` gains `@mind/publish <filed> form=sealed|open|open-resale
  [royalty=<per mille>] [transfer=no] (at a counter)` after the fork line and the old
  publish refusal is removed, `/publish` dispatching to `market::publish` after the
  principal check; `launch::credits` `pub(crate)`, `launch::personal_term_days()`;
  `market_help`'s last sentence is `Read the listings here with +minds and your licences
  with +minds/holdings; help personal-minds lists every market command.`;
  `market_commands::mod` declares `#[cfg(test)] mod tests` last. Widths pinned: `ROW`
  106, `DETAIL` 86, `TERMS_LINE` 76, `SPLIT` under 100, `HOLD_OWNER` 80, `HOLD_PAID`
  114, `HOLD_WAIT` 96. Owner-read counts on the format-2 Active launch: services 32,
  grants 32 (pinned). **Contracts and help**: the `+minds` stanza in
  `legacy-handlers.toml` (verbatim, between `+market` and `+move`), the `@mind` note's
  C2c sentence in place of "Publish remains closed until M7.",
  `docs/help/personal-minds.md`'s "Listing and buying" section verbatim. **Tests**:
  `market_command_reader_is_bounded_and_seller_free` (paging 20/20/5, scans of 64, the
  2-row lookup, Ambiguous, Refused, malformed codes, a 600-stale-row run with the cursor
  at the 512th key and `listings_after` past it, the exact key set and privacy words),
  `market_command_actions_capture_the_entitlement_steps`,
  `market_command_read_answers_the_counter_rule_and_the_policy` (AtCounter, Refused away
  from the counter before any scan, the older-price listing hidden and refused by code
  but usable as a cursor), `command_reader_pages_own_holdings_of_every_kind` (one owner
  row, the cursor, another player's empty page, Paid Unavailable, names as exact JSON,
  the head moved so the charter drops), `a_funded_pending_head_is_a_sale`,
  `own_holdings_page_cuts_at_three_rows_and_sixteen_assets`,
  `market_rows_fit_one_hundred_and_twenty_columns`, `fill_pads_to_the_named_width`,
  `publish_arguments_parse_strictly`, `an_empty_page_that_goes_on_prints_its_cursor`,
  `market_parameters_satisfy_the_registered_providers` (in `hardware/flow/launch.rs`:
  (32, 32), the read registered, the slot's market None, List with and without the mark,
  Quote, Fund, Grant and Refund of both protocols resolved and validated),
  `market_commands_are_not_open_on_a_house_only_launch` (test 10),
  `market_replies_reach_only_the_actor` (test 9 on format 2: room speech as the positive
  control, no line reaches another principal, `assert_private` against every account id,
  the `NOT_OPEN`/`BAD_FORM`/help replies pinned for re-blessing), the two
  `@mind/publish` help pins. The `// C2c:` story marker is untouched. Gate round 1
  green: 19 steps, the conclav-dev sweep 11195 tests, 0 failed.
- **C2d** (`b3ce5a4f`): **C2d — as built (2026-10-27, branch
  `claude/m7-c2-operator-commands`, uncommitted on `e54604ba` until this commit).**
  mush-market: both renewal services factor the player branch of `command` into a
  private `step(ctx, &Service, LicenceId, Switches)` and expose `pub fn player_step(ctx:
  &mut Ctx<'_>, licence: LicenceId, step_kind: RenewalPlayerStep) ->
  Result<RenewalPlayerOutcome, CmdError>` from `mind_renewal` and `mind_house_renewal`
  over one set of types in the new `mind_renewal::player` (`RenewalPlayerStep::{Status,
  Request, Accept, Decline}` where `Request` sets both the request and retry switches;
  `RenewalPlayerOutcome::{Status(RenewalStatusView), Queued, Unresolved}`;
  `RenewalState::{None, Drafted, Funded, Granted, Closed}`; `RenewalStatusView` with
  `state()`, `term_end_ic()` and `Option` getters `price_minor`, `instrument`,
  `offer_expires_ic`, `term_seconds`, `grace_seconds`, `registry_permille`,
  `venue_permille`, `parent_royalty_permille`; crate-private `Switches::{of,
  of_command}`, `personal_status_line`, `house_status_line`, `outcome_line`,
  `QUEUED_LINE`, `UNRESOLVED_LINE`); `@mindrenewal` and `@housemindrenewal` call `step`
  with `Switches::of_command(&ctx.switches)` and print through `command_line(outcome)`,
  byte-identical to the pre-C2d sentences by test 1's oracle
  (`renewal_player_step_keeps_the_service_commands_byte_identical`, every state of a
  real operation row for both services, the queued and unresolved lines, the switch
  mapping, and the view's frozen price, instrument, expiry and term);
  `status_view(term_end_ic, Option<&MindRenewalOperationV1>)` builds the view (the House
  one with parent royalty 0); the service's admission, retained entry, key, submission
  and continuation are unchanged; `mind_commands` gains `pub enum ResaleKind { Offer,
  Accept, Fund }` and `MarketCommandActions::resale(ResaleKind) ->
  Option<DefinitionRef>` (owner `market`, kind `action`, ids
  `offer_`/`accept_`/`fund_paid_mind_resale`, version 1; test 3
  `market_command_actions_capture_the_resale_kinds`). mush-roll: the folded
  `original_owner_holding` v2 read gains `RollCommandReadRequest::own_paid(LicenceId)`
  answering `RollCommandReadResult::OwnPaid(PaidHoldingView)` (`into_own_paid()`;
  getters `licence`, `charter`, `house`, `name`, `form() -> PaidForm`, `transferable`,
  `phase() -> OwnerPhase`, `revoked`, `selling` (a Funded resale head), `renewal() ->
  Option<RenewalMark>` with `pub enum RenewalMark { Offered, Paid }` serialized
  snake_case) by point reads of the licence's own rows (`paid_licences` pointer named by
  the actor, else `Refused`; `paid_terms`; `charters`; `house_allocations` by the birth
  operation; `revoked_terms`; `paid_resale_heads`; the personal or House renewal head's
  pending operation), every failure `Refused` so a foreign licence reads like a missing
  one; `pub(super) const fn renewal_mark(drafted, funded)`; `read`'s signature is
  unchanged; `RollCommandActions` gains `pending_charter_offer()`
  (`offer_mind_charter_transfer` v2; v1 is the immediate protocol and is not captured),
  `pending_charter_fund()` (`fund_mind_charter_transfer` v1), `recall()`
  (`registry_revision_recall` v1) and `revocation()` (`registry_term_revocation` v1);
  test 4 `command_reader_trade_kinds_answer_only_their_own_party` (own, foreign and
  missing over a written pointer and term; the renewal mark `Offered` then `Paid`
  through real `renewal_heads` and `renewal_operations` rows; no digest, account,
  contract, consent or operation text) and test 5
  `command_actions_capture_charter_and_registry_actions`. tacsim: `market::SWITCHES`
  grows to 11 (`renew`, `decline`, `resell`, `charter`, `accept`), `HELP` gains the
  brief's three lines, and `command` dispatches `["renew"]`, `["pay","renew"]`,
  `["decline","renew"]` to `renew::renew(ctx, text, Renew::{Status, Pay, Decline})`,
  `["resell"]` to `trade::resell`, `["charter"]` to `trade::charter`,
  `["accept","charter"] | ["accept","resell"]` to `trade::accept`; `renew` (new):
  `pub(super) own_paid(ctx, u64) -> Result<PaidHoldingView, CmdError>` (`NO_LICENCE`,
  `NOT_OPEN` without the slot), `renew` checks the number (help), `own_paid`, then
  refuses `NOT_OPEN` at `pub(crate) const RENEWALS_OPEN: bool = false`; behind it the
  full flow is built (House flag picks the service; `RENEW_NOT_DUE` by
  `world_ic_now().at < term_end_ic` with `days` left; `RENEWING` printed only on
  `Queued`; `RENEW_OFFER`+`RENEW_HOW` via `offer_lines`/`offer_text(licence,
  price_minor, term_seconds, grace_seconds, expires_ic, now_ic) -> [String; 2]` and
  `hours_left` rounded up never below 0; `RENEW_FUNDED`; `/pay` and `/decline` need
  `Drafted` (`NO_RENEWAL`), `RENEW_PAYING` with the frozen price, `RENEW_DECLINING`;
  `Unresolved` is `BUSY`); `trade` (new): `TradeArguments { subject, player, price_minor
  }`, `AcceptArguments { subject, price_minor: Option<i64> }`, `TradeRefusal::{Usage,
  Price}`, strict `trade_arguments` (`<n>=<player> price=<credits>`, names may hold
  spaces, `price=` last) and `accept_arguments`, `amount(minor)` in the `price=`
  spelling, `player()` resolving `me`, `#<dbref>`, `*name` or a name to a live non-GOING
  Player (`NO_PLAYER`); `resell` and `charter` check shape and price (help,
  `BAD_PRICE`), the slot (`NOT_OPEN`), the player, then `own_paid` (`NO_LICENCE`) or
  `author::own_filed` (`NO_FILED`), then refuse `NOT_OPEN` at `pub(crate) const
  NAMED_OFFERS_OPEN: bool = false`; behind it they capture `resale(Offer)` /
  `pending_charter_offer()`, the personal account, print `RESELLING`+`RESELL_TELL` /
  `CHARTERING`+`CHARTER_TELL` and submit `resale_offer_parameters(LicenceId, ObjId, i64,
  AccountId)` = `{"licence": u64, "buyer": u32, "price": i64, "seller_account": u64}` or
  `charter_offer_parameters(CharterId, ObjId, AccountId, i64)` = decimal strings with
  `"valid_for_seconds": "86400"` (`CHARTER_OFFER_SECONDS`); `accept` parses then refuses
  `NOT_OPEN`; the buyer's plan is built and tested but uncalled: `NamedOffer { offer,
  price_minor, expires_at_utc }`, `Named::{Resale, Charter}`, `AcceptPlan::{Show,
  Refuse, Submit(NamedOffer, String)}`, `accept_plan(named, subject, &[NamedOffer],
  Option<price>, now_utc)` (expired dropped, (price, expiry) order, an offer line plus
  its confirming line each, `NO_OFFER`/`NO_CHARTER_OFFER`,
  `NO_OFFER_AT`/`NO_CHARTER_OFFER_AT` then the listing, an equal-price tie submits the
  one expiring last, `ACCEPTING`/`FUNDING_CHARTER`), `offer_lines`, `minutes_left`,
  `resale_accept_parameters(CommitToken, AccountId)` = `{"offer", "payer": u64}`,
  `charter_fund_parameters` = `{"offer", "payer": "<decimal>"}`; `registry` (new):
  `RegistryCommand::{Recall, Revoke}`, `registry(ctx, which, usage)` checks the number
  (`@registry` help), the slot and the captured action (`REGISTRY_OFF`), then prints
  `RECALLING`/`REVOKING` and submits `recall_parameters(u64)` = `{"filed": "<decimal>"}`
  or `revocation_parameters(u64)` = `{"licence": "<decimal>"}` with no operator
  pre-check (the projection offers the actions only to granted operators, so a
  non-operator reads the driver's line like a missing number); `bulk::registry_service`
  adds `recall` and `revoke` to both switch lists, dispatches them first in `command`,
  and `pub(crate) const HELP` is the brief's line split in two at its second clause;
  `holdings::{paid_flags(transferable, revoked, selling, renewal), paid_row,
  paid_line(&PaidHoldingView)}` build the paid row with `; no resale`, `; revoked`, `;
  selling`, `; renewal offered`/`; renewal paid` but nothing prints them (the paid
  section is still `Unavailable`); `words` carries the brief's 18 verbatim words and
  `words::named` the 10 accept-side ones; tests 6 `trade_arguments_parse_strictly`, 7
  `accept_selects_the_offer_the_buyer_confirms`, 8
  `trade_lines_fit_one_hundred_and_twenty_columns` (`CHARTER_OFFER` widest at 109; paid
  rows at most 114 with `; no resale; renewal offered`; both help texts),
  `renewal_offer_lines_round_hours_up` (pins `!RENEWALS_OPEN`), and 9
  `trade_parameters_satisfy_the_registered_providers` in `hardware::flow::launch` (all
  seven parameter objects resolve against the projection and pass `validate_invocation`;
  the buyer is offered neither Registry action). mush-server
  (`player_market_commands.rs`): `known_accounts`, `REGISTRY_HELP`, `REGISTRY_OFF`,
  `MINDS_HELP` re-blessed with the three lines; tests 10
  `registry_commands_refuse_alike_for_every_principal_on_unknown_targets` (buyer, God
  and operator all read `[request, Queued., driver]`; `1` and `999` alike under
  `same_refusal`; a malformed number is the help), 11
  `trade_commands_refuse_without_own_records` (`NO_LICENCE`, `NO_FILED`, `NO_PLAYER`,
  `BAD_PRICE`; the accepts `NOT_OPEN`), 12 `trade_replies_reach_only_the_actor`
  (bystander and `assert_private`, `LaunchAlt` exempt on seller lines only), 13
  `trade_commands_are_not_open_on_a_house_only_launch` (`NOT_OPEN`; `REGISTRY_OFF`); the
  story's `// C2d:` marker carries a comment that part 3 waits. Contracts and docs: the
  `+minds` legacy note carries the brief's sentence plus a sentence recording that every
  C2d form refuses `NOT_OPEN` until the seam and the indexes land;
  `docs/help/personal-minds.md` has the brief's "Renewing and trading" section, operator
  and `+act` paragraphs and the "Purchases, resales and transfers" sentence;
  `docs/help/house-minds.md` has the brief's paragraph plus a closing paragraph naming
  the `house-renewal-*` actions where `+minds/renew` is not open; `deploy/README.md` has
  the brief's operator paragraph; the operator sentence was checked against 17f's recall
  and revocation effects and holds. No family, IPC, proto, TS, gateway, web, shard,
  state-slot or panic-budget change; `@mindrenewal` and `@housemindrenewal` sentences
  unchanged; the bot reads none of these lines. Gate: fmt, workspace clippy, strict
  rustdoc and every focused run green first time; the conclav-dev sweep (run
  40490994233, 11,208 tests) had one unrelated mushd relay_bridge Busy flake that passed
  alone.
- **The close repair** (`3e4f7668`): CI's "M7 memory-size limits" step
  (`clippy::large_stack_frames`, 64 KiB under `.github/clippy-m7`) refused two C2d test
  functions: mush-roll
  `command_reader::tests::command_reader_trade_kinds_answer_only_their_own_party`
  (69,262 bytes) moves its drafted and funded `RenewalOperationV1` construction into a
  `renewal_rows` helper that takes `filed` and `revision` from the pinned
  `RevisionPolicy` the test now builds once; mush-market
  `mind_renewal::tests::renewal_player_step_keeps_the_service_commands_byte_identical`
  (142,944 bytes) splits into `personal_status_is_byte_identical` and
  `house_status_is_byte_identical`, their seven-row tables from `personal_rows` and
  `house_rows` as a `Vec`, the personal table building one funded row and reusing it.
  Every assertion is unchanged. The mush-server `paid_purchase` harness's `offer` helper
  retries the typed `ActionSend::Busy` (refused before durable ownership; a fresh
  request sequence per call, so the retry is exact) up to 50 tries 50 ms apart, each
  under `STEP`, logging `event=busy_retry`; every other disposition fails as before.

**Evidence.**

- **The loop.** The plan was revised once after a feasibility lens and an adversarial
  lens (23 findings, plan §11) and refreshed read-only at `ecc26f0e` (plan §12, the test
  targets since T.3). Each sub-cut then ran §3's loop: a builder, three review lenses, a
  fixer, a gate runner and an independent judge, then one commit and one push. The
  judge's verdict was ship-with-notes all four times. The judge's texts give no must-fix
  counts. They name the review fixes they carried: in C2a, the owner-read fold (the
  builder stopped at 32 services and 33 grants against the 32/32 caps, as the brief
  required; the repairer applied plan §2.4's pre-decided fallback 1), the `RATING_BUSY`
  narrowing to the character's own Registry job (reviewer F2) and the file-level
  `#![cfg(test)]` fences after the I13 census (`action_state_republish`) read a
  test-side `install_composed_action` as production code; in C2b, the builder stopped on
  the four indexes (mush-core's terminal-key and `exact_puts` checks refuse every
  same-transaction index write) and the reviewers' must-fix for a mush-market lib test
  pinning the List-to-product wiring and `current_listing`'s frozen recompute; in C2c,
  the fix round held `+minds/buy` with its steps (C2C-3), widened the listing cursor and
  added `ListingsAfter` (F4) and kept `HOLD_EMPTY` off a paged owner section (F3,
  C2C-4); in C2d, the fix round held the renewal front at `RENEWALS_OPEN` (C2D-R1, F1,
  T1), removed Roll's `ResaleStep` oracle (C2D-R2, C2D-R3), recorded the over-width paid
  row (C2D-R4) and restored house-minds.md's `house-renewal-*` paragraph (C2D-R5, F3,
  T6).
- **C2a's gate** (gate 1 failed on the owner-read capacity the plan had counted as
  tight: `hardware::flow::launch` 5 of 10 and mush-server `player_market` 7 of 12 with
  "owner read exceeded its bounded envelope", every rerun identical, and
  `action_state_republish` on the census; gate 2 after the fold and the fences, green on
  21 commands): fmt for mush-core, tacsim,
  mush-roll, mush-market and mush-server; workspace clippy with `-D warnings`; strict
  rustdoc for the five; mush-core `reserved_shell` 11/0; tacsim `minds::market_commands`
  3, `composition::mind_commerce` 43, `hardware::flow::launch` 10/0/1, `minds::workshop`
  27, `minds::bulk::registry_service` 9, `arena_bridge::tests::m1b5` 1 and `--test
  shard_6` `cockpit::m1b5_mind_set_reads_sets_and_refuses_by_name` 1; mush-roll lib
  289/0; mush-market lib 105/0; mush-server `player_market` 12/0/1 (the story ignored)
  and `--test commerce_composition` 4/0; the five contract targets 28/0; and the
  conclav-dev sweep, run 40458491284: 11,180 tests, 0 failed, 279 binaries.
- **C2b's gate**: fmt for mush-roll, mush-market and mush-server; workspace clippy;
  strict rustdoc for mush-roll and mush-market; mush-market lib 106/0; mush-roll lib
  290/0; tacsim `composition::mind_commerce` 43 and `registry_integration_tests` 14/0/2
  (the table's three `minds::bulk::{entitlement_integration_tests, paid_resale,
  charter_pending}` filters matched no test and passed vacuously; C2c's and C2d's gates
  dropped them); mush-server `player_market` 12/0/1 and `commerce_composition` 4/0; the
  three contract targets 17/0; the sweep, run 40465337439: 11,182 tests, 0 failed.
- **C2c's gate** (19 steps, green first time): fmt for tacsim, mush-roll, mush-market
  and mush-server; workspace clippy; strict rustdoc for the four; mush-market
  `mind_commands` 3 and `mind_entitlement` 10; mush-roll `commerce::command_` 7; tacsim
  `minds::market_commands` 7, `composition::mind_commerce` 43, `hardware::flow::launch`
  11/0/1, `minds::workshop` 27, `m1b5` 1 and the `shard_6` cockpit test 1; mush-server
  `player_market` 14/0/1 and `commerce_composition` 4/0; the four contract targets 23/0;
  the sweep, run 40477037205: 11,195 tests, 0 failed.
- **C2d's gate** (green first time locally): fmt for the same four crates; workspace
  clippy; strict rustdoc; mush-market `mind_renewal` 22, `mind_house_renewal` 9,
  `mind_commands` 4 and `mind_paid_resale` 3; mush-roll `commerce::command_` 9,
  `commerce::charter_pending` 2 and `commerce::paid_resale` 4; tacsim
  `minds::market_commands` 11, `composition::mind_commerce` 43,
  `minds::bulk::registry_service` 9 and `hardware::flow::launch` 12/0/1; mush-server
  `player_market` 18/0/1 and `commerce_composition` 4/0; the four contract targets 23/0;
  the sweep, run 40490994233, 11,208 tests with one failure, mushd `--test
  durable_process`
  `relay_bridge::r6b_real_bridge_delivers_only_to_arrived_observers_and_deduplicates_provider_retries`,
  a Busy-while-CommitPending flake at the test's first relay line on the 32-thread
  runner (no mushd, gateway, comsys or relay file changed in C2); it passed alone on the
  first rerun.
- **The close sweep**, round 1 at `b3ce5a4f`: the local steps were green (fmt for the
  six crates changed since `origin/main`, workspace clippy, strict rustdoc,
  mush-contract-tests), but the PR's CI run 40495069512 failed: `test (lint)`'s "M7
  memory-size limits" step (`clippy::large_stack_frames` at 65,536 bytes under
  `.github/clippy-m7`) refused the two C2d test functions above, and `test
  (rest-server)` failed
  `world_task::submission_disposition::tests::paid_purchase::public_commerce_setup_accepts_generic_committed_receipts`
  with `disposition=busy` on its first attempt (the job's re-run passed 591/0/4). The
  repair commit `3e4f7668` followed; the six-crate memory-size step re-run locally with
  `--keep-going` exits 0, and all 31 `submission_disposition` tests pass (2 ignored).
  Round 2 at `3e4f7668`, green: `cargo fmt -p <crate> -- --check` for
  mush-contract-tests, mush-core, mush-market, mush-roll, mush-server and tacsim; `cargo
  clippy --workspace --all-targets -j 8 -- -D warnings` (6.18 s warm); `RUSTDOCFLAGS="-D
  warnings" cargo doc --workspace --no-deps --document-private-items -j 8` (5 m 01 s, 48
  crate docs); `cargo test -p mush-contract-tests -j 8 --no-fail-fast`, 139 passed, 0
  failed, 8 ignored across 33 binaries. The only warning in the clippy and doc logs is
  mush-net-web's known build-script note (no `web/dist/index.html`).
- **The PR's CI.** PR #30 ("M7 C2: Operator commands, line mode") was opened with the
  design note and was already not a draft. Its run 40501574368 on `3e4f7668` ran from
  12:09:34Z to 12:31:56Z and concluded success: all 24 gating jobs passed and the 8
  dispatch-only jobs were skipped. `main`'s own last three runs (40484066481,
  40450170234, 40440474128) are success too. C2 changes nothing under `crates/mushd`,
  `crates/mush-gateway`, `crates/mush-net-web`, `crates/mush-proto` or `web/`.
- **Not run.** The four M7 stories and the fifth cell,
  `player_market_story::player_market_commands_story_two_accounts_alt_stale_tiny_and_restart`,
  on the CI dispatch before the merge: the PR run skips the `M7` matrix job, and the
  command story is compiled (clippy all-targets) but has never run, holds only its part
  1 (C2a), and its `// C2c:` and `// C2d:` markers carry no part 2 or part 3. The plan's
  dispatch after C2b was not made: C2b moved no writer, family or balance, so there was
  nothing money-path to exercise. There was no hand-played session on this build and no
  deploy; `launch_path` is live on no instance. `cargo test -p mush-bot` was in no gate;
  the bot reads none of the changed sentences.

**Deviations and residuals.**

- **From §3 C2 and the plan above (plan §10, the hand-off).**
  - Four sub-cuts, not §3's three: the plan split the owner indexes and the transfer
    mark out as C2b, a money-path change with no command.
  - The command table is the approved one (ruling 4) with the plan's nine departures,
    and every command is the one mechanism of the design note; but the Roll read is not
    a service of its own: plan §2.4's fallback 1 folded its kinds into
    `holding_reader::OriginalOwnerReader` (`original_owner_holding` v2), so there is no
    `RollCommandReader`, no `mind_commands` descriptor and no second Roll grant. The
    format-2 Active launch measured 31 services and 32 of 32 grants after C2a (the
    plan's static count said 24–29), and C2c's `MarketCommandReader` took the 32nd and
    last service slot without a grant. No cap was raised, as the briefs required.
  - The owner indexes (plan §2.5) are not built. mush-core's typed-state checks refuse
    every same-transaction index write the plan assumed, so C2b landed only the transfer
    mark and the resale transferability gate, and C2c and C2d built every form that
    needs an own-record lookup up to the point where it would read one, then held it:
    `+minds/buy` and its steps, bare `+minds/holdings` and the paid section,
    `/resell/accept` and `/charter/accept` all answer `NOT_OPEN`; `+minds/resell` and
    `+minds/charter` also, by C2c's rule that no command makes an offer no command can
    take.
  - The renewal front (plan §2.1) is held at `renew::RENEWALS_OPEN = false`: the renewal
    services' `RenewalReader` handle and market-owned families refuse a tacsim dispatch
    context, so `player_step` cannot be a plain cross-module call. `@mindrenewal`,
    `@housemindrenewal` and `+act` remain the renewal path.
  - The gate story has part 1 only and is unrun; the alt pair, the stale quote, the tiny
    resale and the restarts of parts 2 and 3 wait for the grant and the index sub-cut.
  - `@mind/publish` lists an already filed mind (`NO_FILED` otherwise) and never files
    first; `@mind/file` is reserve-then-file with the actor's own completed rating from
    `@mind/rate`.
  - The `@mind/launch` market line's last sentence is C2c's (`Read the listings here
    with +minds and your licences with +minds/holdings; help personal-minds lists every
    market command.`), moved in its pinned places; the verb it names for a non-renewing
    Pin is now `market-original-mind-list`.
- **C2a.** (1) Capacity: the brief said stop and report on an owner-read refusal and
  never raise a cap; the builder stopped (32 services / 33 grants against 32/32, the
  "WHAT THE EARLIER SUB-CUTS LEFT" section carrying the lead's decision was missing from
  the brief), and the repairer applied plan §2.4's pre-decided fallback 1 rather than
  wait: Roll's command kinds are folded into `holding_reader::OriginalOwnerReader`
  (`original_owner_holding` v2 with a `commands` config member; per the common rules a
  changed definition is a new version, and no test pins its digest or version), so
  `RollCommandReader`, its `mind_commands` v1 descriptor and its grant do not exist,
  tacsim's slot reads through the grant `register_entitlements` already holds,
  `holding_reader::register` moved into `register` after the filing declarations
  (Active-only as before), and the kinds are gated at read time by `with_mind_commands`
  (`Unavailable` otherwise, test `command_kinds_are_closed_without_the_player_market`);
  no cap was raised, the launch is at 31 services / 32 of 32 grants, and the consequence
  for C2c (its `MarketCommandReader` cannot compose without the lead's cap raise or a
  fold) is the lead's decision before C2c. (2) Test 4 lives on the launch rig
  (`hardware/flow/launch.rs`), not `composition/mind_commerce/tests.rs`, because
  providers project nothing in an unbooted composition; test 3's counts are the
  composition-only chain's (18/17), test 4 prints the launch's. (3)
  `MindCommandService.attempts` is `Arc<Mutex<..>>` because `Registrar::state` requires
  `Clone`. (4) `submit` takes a sixth argument, the request line, printed after every
  check so a refusal never follows a request line. (5) mush-core gained
  `Registry::owner_read_counts` and `OwnerReadRegistry::counts` (read-only counts for
  tests 3 and 4) and a test-only `OwnerReadContext::for_test`, beyond the one accessor
  the brief named. (6) Test 6 omits the dependent line and the charter transfer the
  brief required (mush-roll has no dependent-filing or E4 transfer fixture; tacsim's
  live inside the ignored genuine-Registry flows); the foreign caller is a real second
  Player; a residual the lead owns (the C2d story's charter step, the dependent route at
  C2d or C4). (7) Test 9 in the fast suite compares two missing numbers (a filing needs
  a genuine rating), re-documented as the missing-path and masking check; the foreign
  half for /update, /withdraw, /attest and /fork by the buyer and the alt is in the
  unrun story. (8) Story part 1 runs reserve, NOT_RATED, rate, file instead of plan §5's
  NOT_RATED, rate, reserve, file, because §6's algorithm reserves whenever no shell is
  open; every pin includes the engine's best-effort `Queued.` line between the request
  line and `Done.`/the driver's line (C2c and C2d pins must too). (9) RATING_BUSY is
  narrowed to the character's own Registry job (reviewer F2): `Preparing.kind:
  WorkKind`, `holds_rating`, `rating_held`, `rate_refusal`; every other Busy and error
  prints the unavailable line. (10) `@mind/fork` requires the slot (NOT_OPEN on format
  1), consistent with the brief's test 13. (11) `initialized_launch` gained
  `initialized_launch_format` and `initialized_launch_with` for test 13 and the story's
  `quote_seconds: 30`. (12) The three new external test modules carry a file-level
  `#![cfg(test)]` fence after the I13 census (`action_state_republish`) read a test-side
  `install_composed_action` as production code. (13) `Seen` gained `to`, `extend` and
  `reaching` beyond the brief's `{own, others}` so test 10 checks observers by
  recipient. (14) `legal`, `submit_initialize` and `initialized_active` are unchanged,
  as the brief predicted. (15) The brief's `#[cfg(test)] mod tests;` sits at line 30 of
  `market_commands/mod.rs`, before the production code (as `workshop/mod.rs` does); move
  it to the end in C2c. (16) Not done by any builder, since nothing under docs/design is
  theirs: 17g line 75 ("`@mind/fork` is the local draft copy") is now false, doc 17
  ~1895 and 17g lines ~84, 487, 563-564, 798, 921 name the old shared verbs, and the
  §2.4 estimate of 24-29 services is replaced by the measured 31/32; for the record
  writer.
- **C2b.** (1) Brief items 1–4 (the four owner indexes `market_mind_entitlement_actors`,
  `roll_mind_paid_holdings`, `market_mind_paid_resale_buyer_offers`,
  `roll_mind_charter_pending_buyer_offers`, their writers, audits,
  `paid_holdings::page`, the delegations and the four family registrations) and tests
  1–5 and 7–11 are not built, because mush-core's typed-state checks refuse every
  same-transaction index write the brief and plan §2.5 require:
  `terminal_capacity::prepare`'s terminal branch (`terminal_capacity.rs:744-765`)
  refuses any key outside the reservation's targets ("unreserved terminal key"), which
  covers the entitlement Grant and Refund removals, the three grant inserts and the
  resale service settlement; `paid_resale/transition.rs` `exact_puts` (lines 13–31,
  called at Offer :133 and Accept/Fund/terminals :276) and
  `charter_pending/transition.rs` `exact_puts` (:82, called at :155 and :187) require
  the transaction's whole mutation set to equal a fixed list of puts, which covers items
  3 and 4 at every step and item 2's immediate-Accept swap; the plan's premise that the
  indexes copy v2's `ActorOffers` missed that v2's index is written only at Quote and
  Fund, never in a terminal or exact-set transition; mush-core is outside C2b's files
  and the common rules forbid improvising a design, so the builder stopped and reported,
  and the lead must rule between admitting a closed index-family list in the terminal
  branch and both `exact_puts` callers, reserving the holding row at funding (which
  alone reaches only item 2's grant inserts), or taking the indexes off the money
  transactions (against 17g's same-transaction rule); the Quote and Fund writes of item
  1 were deliberately not built alone because the index audit ("indexed operation ⇒
  Funded") would refuse every boot after the first grant without the terminal removal.
  (2) Consequently no family-set or definition pin moved and `cargo test -p mush-roll
  --lib commerce::paid_holdings` matches nothing; C2c and C2d have no bounded own-record
  read until the indexes land, and the plan's dispatch of the five M7 stories after C2b
  has no money-path change to exercise, so the gating dispatch should follow the index
  sub-cut. (3) `EntitlementConfig::product` takes a `ProductChoice` rather than the
  brief's `(f, form, rate, transferable)` positional signature, and the product body is
  split into `product_for` over a captured subject and royalty chain, because
  `EntitlementSaleFacts` has no constructor outside Roll and the reviewers' must-fix
  required a mush-market lib test to pin the List-to-product wiring and
  `current_listing`'s frozen recompute; `product` passes the same inputs as before, so
  no production behaviour changed. (4) Test 12 lives in `commerce::renewal::tests`
  (where the birth → draft → fund → grant helpers are), not under
  `commerce::paid_resale`, and exercises the extracted `require_transferable` and the
  resale binding rather than a full `observe`, which needs a granted licence and its
  proofs in a real-engine harness; the end-to-end marked sale and refused resale are
  C2c's and C2d's story legs. (5) The List and Quote parameter vectors were factored
  into `offers::parameters(kind)` and `actions::optional` was added, small seams the
  brief does not name, so the projected schema is pinned (dropping the optional
  parameter would make core refuse every supplied mark). (6) Stale design text is left
  for the record writer, since builders may not edit docs/design: doc 17 §24 (about line
  1879) and 17g §2 (lines 457–458) say the per-listing mark is not built; 17g §3's C2
  paragraph (line 1021) and plan §2.5/§3.2 say C2 adds four families in the same
  transaction with no mush-core file; `crates/tacsim/src/launch/market.rs:17-18` says
  the mark is not built (C2c, which touches tacsim, should amend it). (7) The gate
  table's three tacsim filters for `minds::bulk::{entitlement_integration_tests,
  paid_resale, charter_pending}` match no test and passed vacuously; the files are
  `#[path]` helpers driven by the ignored original-purchase story, and the table needs
  correcting.
- **C2c.** (1) TacSim does not request the market read: the format-2 Active launch holds
  31 services and 32 grants before C2c, so the registration takes the 32nd service and a
  request reproduces `OwnerReadError::Limit`; the brief's Gates section says to stop and
  report and never raise a cap, so `MindCommandService.market` is `None` in every
  composition and every `+minds` form and `@mind/publish` refuse `NOT_OPEN` until the
  lead raises `MAX_OWNER_READ_SERVICES` and `MAX_OWNER_READ_GRANTS` (plan 2.4 fallback
  2) and one `MarketCommandReader::request(reg, manifest)` in `TacsimRuleset::register`
  stores the handle; tests 7 and 8 and story part 2 cannot pass and are not written (the
  `// C2c:` marker stands), and 17g 7's and plan 2.4's "two services, two grants" counts
  should read services 32, grants 32 with TacSim's market grant held back. (2)
  `OwnPurchase` and `OwnOpen`, Roll's `Paid` section (`Unavailable`) and
  `PaidHoldingView`, `+minds/buy/pay`, `/buy/complete`, `/buy/refund`, the own-purchase
  line under `+minds <code>`, the paid and waiting rows and bare `+minds/holdings` are
  not built because C2b's WHAT-LEFT section (which outranks the brief) says the
  per-buyer entitlement index and the per-holder paid index do not exist; bare
  `+minds/holdings` and `paid <cursor>` refuse `NOT_OPEN` rather than print a list that
  leaves the paid licences out, and `HOLD_EMPTY` and the other index words wait in
  `words::pending`. (3) `+minds/buy <code>` is held with the steps above (the built
  `buy` under `#[allow(dead_code)]`) because `QUOTING` promises that `+minds <code>`
  shows the quote, which only the own-purchase line can, and a quote no command can pay
  only expires (fix round, C2C-3). (4) Roll answers at most 3 owner rows and examines 16
  assets per `OwnHoldings` read, inside the folded read's declared `maximum_rows` 64
  (each row costs `own_filed`'s point reads), and TacSim aggregates up to 12 reads into
  the brief's 20 rows. (5) `selling` on an owner row means only a `Funded`
  pending-charter head, not an open unfunded offer, because `CharterOffer` is keyed by
  operation and no bounded per-charter lookup exists (C2b's planned index is keyed by
  buyer and charter, so it would not provide one either). (6) `ListingsPage::next`
  widened and `ListingsAfter` added (fix round, F4): when the 8-scan budget ends on
  insert-only listings a policy change left stale, the cursor is the last scanned key
  and `+minds/next` resolves it current or not, so a page can be empty and still print
  `HEAD` then `MORE`; `EMPTY` only at the true end. (7) A paged owner section never
  prints `HOLD_EMPTY` (fix round, F3 and C2C-4): `HOLD_HEAD`, the rows and `HOLD_MORE`
  while the reads continue; `HOLD_EMPTY` is the bare form's, which waits for the paid
  index. (8) Test 6 lives in `hardware/flow/launch.rs` beside C2a's test 4, not in
  `composition/mind_commerce/tests.rs`, because the providers project only in a booted
  launch; it still runs under the gate's `hardware::flow::launch` filter. (9) `+minds
  <code>` omits the own-purchase line rather than always printing `ASK`, which would be
  false for an actor with a quote. (10) `price=0`, `seats=0`, `term=0` and malformed
  values print the usage line rather than `TERMS` (`number` and `credits_minor` reject
  0); a well-formed non-matching value gets `TERMS`. (11) The help page, the market line
  and the legacy `+minds` note are the brief's verbatim texts and so describe commands
  that refuse `NOT_OPEN` today, and the old `+act` purchase table is gone from
  `personal-minds.md`; whether these land ahead of the grant is the lead's call with
  (1). (12) Test 1 lacks the `OwnPurchase`/`OwnOpen` cases and test 3 the dependent,
  paid, House Pin, revoked and neighbour-holds-rows cases (no fixture or not built); the
  read-path seam is tested through `answer` over an in-memory source with the real
  `current_policy`, while `Lens::scan` and the real venue gate wait for a granted
  composition. (13) `by_code` answers `Ambiguous` for two matching rows whether or not
  both are current (the brief's literal "two"), where plan 2.7 says two current
  listings. (14) The unrequested service keeps the 32nd and last service slot, which the
  builder flagged as unsure; nothing else in C2 adds a service.
- **C2d.** (1) `ResaleOffers` (market) and `CharterOffers` (Roll), the live
  `/resell/accept` and `/charter/accept`, test 2
  `market_command_reader_lists_only_the_actors_resale_offers` and story part 3 are not
  built: C2b's per-(buyer, licence) and per-(buyer, charter) offer indexes do not exist
  and no composition grants tacsim the market read, and C2b's WHAT-LEFT section outranks
  the brief; both accepts parse then refuse `NOT_OPEN`, so test 11 asserts `NOT_OPEN`
  for `/resell/accept 999` and `/charter/accept 999` in place of `NO_OFFER` and
  `NO_CHARTER_OFFER`. (2) The seller forms `+minds/resell` and `+minds/charter` run
  every check and then refuse `NOT_OPEN` at `trade::NAMED_OFFERS_OPEN = false` instead
  of submitting, by C2c's rule (reviewer C2C-3) that no command makes an offer no
  command can take; test 12's seller-line exemption is therefore never exercised and the
  constant is unpinned. (3) `+minds/renew`, `/renew/pay` and `/renew/decline` are held
  at `renew::RENEWALS_OPEN = false` after `own_paid` (fix round, reviewers C2D-R1, F1,
  T1): `player_step` reads through the market's own `RenewalReader` handle (requester
  `market`) and market-owned families, which mush-core refuses to a tacsim dispatch
  context (`OwnerReadError::Authority`, `require_owner`), so every held licence would
  read the service's uniform refusal; the owner-read caps are 32/32, so tacsim cannot
  take its own read; a seam that runs the step under the market's identity is the lead's
  ruling, and the dispatched test that reaches `RENEW_NOT_DUE` waits with it. (4) Roll's
  `ResaleStep` kind was built then removed in the fix round (reviewers C2D-R2, C2D-R3):
  `paid_resale::reader::observe` checks no offer naming the buyer, so the kind was an
  existence and open-claim oracle on any transferable licence rather than own-only as
  the folded read's descriptor promises, and no mush-roll fixture reaches a granted
  transferable licence for a positive test; the index sub-cut rebuilds it with the
  offer's committed (buyer, licence) binding as evidence, and `holding_reader.rs` and
  `observe`'s visibility are back to base. (5) `@mindrenewal` and `@housemindrenewal`
  call the private `step` with `Switches::of_command` rather than the public
  `player_step`, so the commands keep their own switch semantics (`/request` alone never
  retries); byte-identity is proven by test 1's oracle. (6) The seller forms check shape
  and price, then the slot, then the player, then the actor's own record, where the
  brief ordered own record, player, price: the only order in which test 11 can show
  `NO_PLAYER` and `BAD_PRICE` without own records (a player's existence is public); on a
  format-1 launch a bad price now answers `BAD_PRICE` before `NOT_OPEN`. (7)
  `@registry`'s help is the brief's one line split at its second clause into two lines
  (`…; @registry/cancel` newline `@registry/recall <filed> and …`), dropping the `; `
  after `@registry/cancel`, to keep the 120-column floor; the brief's line is 147
  columns, not the 136 the code comment says. (8) `docs/help/house-minds.md` keeps the
  brief's paragraph and adds a closing paragraph naming the `house-renewal-*` actions
  for launches where `+minds/renew` is not open (fix round, reviewers C2D-R5, F3, T6),
  because the brief's text removed the only House renewal path that works on a
  House-only launch; `personal-minds.md`, `deploy/README.md` and the brief's legacy-note
  sentence still describe `/renew`, `/resell`, `/charter` and the accepts as working
  while they answer `NOT_OPEN`, and the legacy note gained a final sentence recording
  the hold. (9) Test 9 `trade_parameters_satisfy_the_registered_providers` lives in
  `tacsim::hardware::flow::launch` beside C2a's and C2c's, not in
  `composition/mind_commerce/tests.rs`, because the providers project nothing in an
  unbooted composition; the gate list runs it. (10) Tests 10 and 11 compare two missing
  numbers (`1` and `999`) rather than a real filed number or a licence another character
  holds, because the fast suite has no genuine rating; the real-target `same_refusal`
  checks are owed to story part 3. (11) `+minds/holdings` paid rows with the renewal
  suffix are built (`paid_line`) but not printed, the paid section being `Unavailable`
  until the per-holder index; the reachable `; no resale; revoked; renewal offered` row
  (a revocation admits a matching pending renewal) is 121–122 columns in Grace and is
  left out of test 8's legality filter, owed a ruling on the flag words or the
  `HOLD_PAID` layout (reviewer C2D-R4). (12) Test 1 proves the sentences through
  `command_line` over `status_view` and the switch mapping, never through a dispatched
  `player_step` or the `@…renewal` commands; dispatch-level proof rests on the long
  renewal stories' substring checks. (13) Test 4 proves the House, revoked and selling
  flags only negatively (no House catalogue, revocation or funded-resale fixture in the
  module); the renewal mark is proven through real rows. (14) `OwnPaid` was built
  although C2b's WHAT-LEFT listed it among the kinds without a bounded read: it is point
  reads keyed by the licence the player typed, so it needs no index. (15) Small doc
  inaccuracies left standing: `words::named`'s doc names a nonexistent `trade::select`
  (it is `accept_plan`); `personal-minds.md` line 34 is not rewrapped;
  `command_kinds_are_closed_without_the_player_market` was not extended to `OwnPaid`.
- **Residuals still open** (the four sub-cuts' lists merged; closed items dropped: C2a's
  `#[cfg(test)] mod tests;` placement, which C2c moved to the end of
  `market_commands/mod.rs`; C2b's three vacuous gate filters, which the later gates
  dropped; the `tacsim/src/launch/market.rs` comment on the mark, which C2c amended; the
  stale design text the judges listed for the record writer, folded by this entry and
  its §2, §3, 17f and doc 17 §24 edits).
  - **The owner-read caps.** The format-2 Active launch is at 32 services and 32 grants
    of 32/32 (pinned by
    `hardware::flow::launch::market_parameters_satisfy_the_registered_providers`), with
    the `mind_market_commands` read registered and ungranted, so
    `MindCommandService.market` is `None` in every composition and every `+minds` form
    and `@mind/publish` answer `NOT_OPEN`. The lead raises `MAX_OWNER_READ_SERVICES` and
    `MAX_OWNER_READ_GRANTS` in `mush_core::read_service` (plan §2.4 fallback 2) as their
    own commit recorded here, then one `MarketCommandReader::request(reg, manifest)` in
    `TacsimRuleset::register` stores the handle and the (32, 32) pin is re-blessed.
    Until then `Lens::scan`'s key check, the real `EntitlementGuard` and `next_after`
    against the live store run in no test. C3 and C4 add no owner read without the count
    being redone.
  - **The index sub-cut.** The four indexes (`market_mind_entitlement_actors`,
    `roll_mind_paid_holdings`, `market_mind_paid_resale_buyer_offers`,
    `roll_mind_charter_pending_buyer_offers`), their audits and `paid_holdings::page`
    are refused by mush-core's terminal-key check (`terminal_capacity.rs:744-765`) and
    both `exact_puts` checks (`paid_resale/transition.rs`,
    `charter_pending/transition.rs`). The lead rules between (a) admitting a closed list
    of index families, puts and deletes, in the terminal branch and both `exact_puts`
    callers; (b) reserving the holding row at funding, which alone reaches only the
    grant inserts; or (c) taking the indexes off the money transactions, against 17g's
    same-transaction rule. Any ruling changes both `exact_puts` functions and the resale
    swap, and mush-core's capacity and transition tests then join the money-path review.
    Waiting on it: `OwnPurchase` and `OwnOpen`, Roll's `Paid` section and
    `PaidHoldingView`'s rows, `+minds/buy`, `/buy/pay`, `/buy/complete`, `/buy/refund`,
    the own-purchase line under `+minds <code>`, `HOLD_WAIT` and bare `+minds/holdings`
    (`HOLD_EMPTY` waits in `words::pending`); `ResaleOffers` (market) and
    `CharterOffers` (Roll), a rebuilt `ResaleStep` bound to the offer's committed
    (buyer, licence) evidence, the live accepts and the flip of
    `trade::NAMED_OFFERS_OPEN` (unpinned: flipping it by accident leaves the suite
    green); the brief's missing test cases (C2c test 1's `OwnPurchase`/`OwnOpen`, test
    3's dependent, paid, House Pin, revoked and neighbour-with-rows cases; C2d test 2
    and test 11's `NO_OFFER` and `NO_CHARTER_OFFER`); and story parts 2 and 3. The
    gating dispatch of the five M7 stories should follow the index sub-cut, not C2b.
  - **The renewal seam.** Before `RENEWALS_OPEN` can flip the lead rules how
    `+minds/renew` reaches `player_step` under the market's identity: a
    market-registered command or continuation the handler hands the step to, a mush-core
    delegated read, or a cap raise plus a tacsim renewal read. Until then
    `+minds/renew`, `/pay` and `/decline` print `NO_LICENCE` or `NOT_OPEN` only, and no
    test reaches `player_step` through a dispatched `Ctx`.
  - **Text ahead of code.** `docs/help/personal-minds.md` ("Listing and buying",
    "Renewing and trading"), the `@mind/launch` market line, the `+minds` legacy note's
    brief sentence and `deploy/README.md`'s operator paragraph describe commands that
    answer `The mind market is not open here.`, and the old `+act` purchase table is
    gone from the help page; only the legacy note's last sentence and house-minds.md's
    restored `house-renewal-*` paragraph record the hold. Harmless pre-launch and
    undeployed, but the branch must not reach `main` or dev1 before the grant and the
    index sub-cut land, or the lead gates the prose until then.
  - **The command story.** Its first dispatch must confirm that the filing row
    serializes `filed` and `licence` as the `number` helper reads them, that the
    filing's commit-bound line is the generic `Done.`, that the launch world holds
    exactly one `rating_jobs` row after `@mind/rate`, that `Registry work finished.`
    reaches the author within `STEP` of Roll's Completed row, and that the buyer's and
    alt's foreign probes read alike under `same_refusal`. The fast suites compare two
    missing numbers only (C2a test 9; C2d tests 10 and 11 with `1` and `999`): the real
    foreign-versus-missing proofs for `/update`, `/withdraw`, `/attest`, `/fork`, the
    buyer's recall of a real filed number and the buyer naming the author's licence are
    owed to the story. So are the dependent line (`update_derived_mind`) and the owner
    licence following the title after an E4 charter transfer (after
    `+minds/charter/accept` the new holder's `@mind/attest <filed>` reaches the driver's
    line and the old owner reads `NO_FILED`), at C2d's story step or C4.
  - **The transfer mark's coverage.** No non-ignored real-engine test drives a marked
    List → Quote → Fund → Grant or a full `observe` refusal of a granted marked licence;
    the lib tests pin `product_for`, `frozen`, the schema and both reader gates, and
    dropping the `require_transferable(term)?` call in `observe` while the binding gate
    stays would leave test 12 green (`Mismatch` instead of `Unavailable`). Under a
    policy of `transferable: false` a List with `"transferable": true` is accepted and
    silently narrowed; the `@mind/publish` reply should print the listing row's frozen
    transferability, not the request's.
  - **Widths.** A revocation admits a pending renewal whose binding matches the birth,
    so a `transfer=no` licence in Grace can carry `; no resale; revoked; renewal
    offered` at 121–122 columns; C2d test 8's legality filter leaves that combination
    out, and it and C2c's `market_rows_fit_one_hundred_and_twenty_columns` disagree on
    whether selling plus a renewal mark is legal. A ruling on the flag words or the
    `HOLD_PAID` layout is owed before any paid row prints.
  - **Reader and command edges.** `by_code` answers `Ambiguous` when the two rows
    sharing a code include a stale one (plan §2.7 says two current listings), so `+minds
    <code>` refuses a current listing and reveals that a stale one existed; `selling` on
    an owner row means only a `Funded` pending-charter head, never an open unfunded
    offer (offers are keyed by operation); `price=0`, `seats=0`, `term=0` and malformed
    values print the `@mind/publish` usage line rather than `TERMS`; `CharterNames`
    answers any charter's public name for any actor while the folded read's descriptor
    says `own_only: true` (no leak; names are public), and nothing enforces its
    `maximum_rows` 64 (an `OwnHoldings` read charges per examined asset within the 512
    context budget); `trade::player()` accepts `me`, the seller's own name or dbref as
    the buyer, which once `NAMED_OFFERS_OPEN` flips would read `RESELLING`/`CHARTERING`
    then the driver's line (refuse `NO_PLAYER` locally before the request line);
    `@registry/recall` and `/revoke` route through `market_commands::submit`, whose
    principal check answers `NOT_OPEN` for a non-player or unconnected actor; `BUSY` is
    checked after the handler's own-state reads, so a pending request may print
    `NO_FILED`, `NOT_RATED` or `ACCOUNT` first (messages only); the idempotency ordinal
    is RAM-only and keys carry the IC second, so a stopped IC clock could regenerate a
    pre-restart key after a restart and read `(already done)`;
    `mind_trade::purchase_provider::verb` projects any action id but FUND and REFUND as
    `market-original-mind-grant` and `authored_retirement::provider::verb` any version
    but 2 as `-v1`, so a fourth purchase kind or a version 3 would silently share a verb
    (match GRANT and version 1 explicitly; C4).
  - **Coverage notes.** `command_kinds_are_closed_without_the_player_market` was not
    extended to `OwnHoldings`, `CharterNames` or `OwnPaid` (`read`'s early return covers
    them; a refactor moving it would go unnoticed); C2d test 4 proves the House, revoked
    and selling flags only negatively, and test 1 never drives `player_step` or the
    `@…renewal` commands through a `Ctx` (dispatch-level proof rests on the long renewal
    stories' substring checks); the `+minds` read forms skip plan §2.1's principal check
    (public or actor-scoped data only); whether the gateway, TUI or web client renders
    an optional Boolean offer parameter is unverified (C3 owns typed clients).
  - **Hygiene.** `market_help`'s doc comment still explains the replaced C2a sentence;
    `words::named`'s module doc names a nonexistent `trade::select` (it is
    `trade::accept_plan`); `registry_service::HELP`'s comment says the brief's one line
    is 136 columns (it is 147) and the two-line split dropped the brief's `; ` after
    `@registry/cancel`; `personal-minds.md` line 34 is not rewrapped; the `@mind` legacy
    note's M6 sentence still lists `/fork` among the local-copy switches while its C2a
    sentence makes it the paid fork.
  - **The sweep flake.** mushd `durable_process`
    `relay_bridge::r6b_real_bridge_delivers_only_to_arrived_observers_and_deduplicates_provider_retries`
    is in neither common-m7.md's load-sensitive list nor nextest's retry override; add
    it to one or teach the test to retry Busy.
- **For §5 (not yet folded there):** the plan's owner questions: per-listing prices,
  seats and terms; a recall and revocation reason; House catalogue purchases by command;
  an offers-to-you list; the gate's sim half moving to C4. The lead's three decisions
  above (the cap raise, the index ruling, the renewal seam) and whether to gate the help
  prose are the lead's, not the owner's.

### C2e — plan (2026-10-27, branch `claude/m7-c2e-completion`)

As planned against §3 C2 in four sub-cuts (plan: plan-data/m7-completion/cuts/C2e/plan.md), revised after two review lenses (§9 dispositions FE-F1..14, AD-F1..13) and refreshed read-only at `563259b7` (main: PR #30 merged as `8aaf1866` with the C2 holds in place, PR #32's shard moves), on the cut's own branch `claude/m7-c2e-completion`; the committer opens the draft PR with this note and the ledger gains `### C2e — plan` after `### C2 — as built`. **What C2 left and why.** C2 landed every `+minds` and `@mind` command, the renewal services' factored `player_step`, the recall and revocation captures and the market's bounded command read, but three mechanisms could not close inside it: the owner-read roster had no free grant (32 services and 32 grants on the format-2 Active launch, the caps at 32), so `MindCommandService.market` is `None` in every composition and every read form refuses `NOT_OPEN`; mush-core's typed-state checks refuse every same-transaction index write (the terminal branch admits only the two exact `extra_terminal_key`s, both `exact_puts` validators compare a fixed Put-only key set), so none of the four owner indexes exists and the buy steps, the paid rows and the named offers have nothing to read; and `+minds/renew` dispatches under tacsim while each renewal service reads through the market's own handle and families, so the front is held at `RENEWALS_OPEN = false`. The lead's five rulings close them without new authority: R1 raises the two declaration caps 32 → 48 as the first commit on its own (heap `Vec` lengths outside the `owner_read_services.v1` digest, no memory bound) and then requests the ONE market read grant, which puts the launch at (32, 33) — a grant, not a service — and moves only the two unpinned fingerprints; R2 option (a) admits a CLOSED, `const`, per-transition table of index families and their exact puts or deletes in the terminal branch and in both `exact_puts` callers and the entitlement validator, each expected key derived from the transaction's own committed record exactly as the two existing admissions derive theirs, nothing at a resale or charter terminal and nothing at revocation, recall, lapse or the renewal grants (the holdings family joins no protocol's `own()` filter and is delegated to market beside `paid_licences`; a full store refuses a Grant put and never a Refund); R3 runs the renewal step under the market's identity as a market-registered `front` continuation that the market delegates to tacsim at composition, core gaining one owner-declared, one-grantee, freeze-checked delegation (`MAX_CONTINUATION_DELEGATES` = 8) and a narrow acceptance rule (`actor == self.actor` and a dispatching connection: never another actor, a tick, a continuation or a `@wait`-queued line), the market arm performing the read, the step and the `+minds` reply after the step because Roll's mirror carries no offer price or expiry and a request line printed before an in-memory hand-off would promise a payment a dropped continuation never makes; R4 shortens the paid row's renewal words to `; renew offer`/`; renew paid` so the widest LEGAL combination (Grace, `; no resale; revoked; renew offer`) is 118 ≤ 120 under one legality rule stated once in `paid_flags`' doc and shared by the two width tests; R5 keeps every help sentence C2 wrote.

**The order and the rule of fixtures.** C2e (the caps, the grant, the live read forms, the three edges: current-rows-only ambiguity inside the unchanged 2-row window, `TERMS` for zero terms, the publish line carrying the row's frozen flags, plus C2's hygiene list and the `relay_bridge` retry) → C2f (R2 and the four indexes, writers, clock-free audits and `paid_holdings::page`, the money path only, no surface) → C2g (`OwnPurchase`/`OwnOpen`/`ResaleOffers`/`charter_offers` over the indexes, the paid section and `HOLD_WAIT` regardless of listing currency, the own-purchase line after `NO_LISTING` too, `+minds/buy` and its three steps keyed by the quote or operation token, R4, `ResaleStep` rebuilt on the offer's committed `(buyer, licence, price, expires)`, `NO_PLAYER` for an offer to oneself) → C2h (R3 in core and market, tacsim's `renew.rs` reduced to its synchronous refusals and the hand-off, story parts 2 and 3 at the `// C2c:` and `// C2d:` markers, the last doc edges). No fast fixture can file a mind (a filing needs a genuine Registry rating, `REGISTRY_GUARD` 4 h), so every listing- or licence-dependent dispatched assertion lives in the `#[ignore]`d story, which stays the fifth `mind-commerce-acceptance` cell and is dispatched by the lead after C2f and after C2h, never by a builder or a gate; the fast production-composition tests cover `EMPTY`/`COUNTER`/`NO_LISTING`/`NO_FILED`/`NO_QUOTE`/`NO_PURCHASE`/`NO_LICENCE`/`NO_OFFER` through the real granted reads and the principal and observer rules, and the in-crate rigs (mush-market's `CapturingMarket` engine, mush-roll's renewal and charter rigs, mush-core's synthetic transitions) prove every writer, audit and admission, admitted ⇒ exactly the expected row and a stray put still refused. **Digests.** The grant and the four families move `owner_read_services.v1`, `state_family_registry.v<N>` and the `mechanics_registry.v10` fold, all verified unpinned outside mush-core's own unit tests and visible only in `@stats`: builders report the values before and after and re-bless only a pin they find. Nothing moves `IPC_VERSION`, the TS bindings, the legacy-handlers baseline (one note's text), state-slot or module-ns owners, the panic budget or the tacsim shard list (no new `crates/tacsim/tests/*.rs`); gateway, web/ and mush-proto are untouched; a pre-C2f store with paid licences fails the new index boot audit, acceptable pre-launch. Every gate is the post-T.3 shape: per-crate fmt, workspace clippy, the M7 memory-size step exactly as ci.yml's lint cell runs it (the regenerated 48-way quota fixture is linted by it), strict rustdoc, focused `--lib` filters (the launch pins are `hardware::flow::launch::`, never a `--test hardware` that no longer exists), the one group target `--test commerce_composition mind_commerce_composition::`, the contract targets each sub-cut can move, and last the conclav-dev sweep.

Sub-cuts:
- **C2e** The owner-read cap raise, the market read grant, the live +minds read forms
- **C2f** R2 and the four owner indexes: the money path
- **C2g** The paid rows, the buy steps, R4, the named offers
- **C2h** The renewal seam, the stories, the edges

Provisional defaults (17g section 1 ruling 6):
- MAX_OWNER_READ_SERVICES and MAX_OWNER_READ_GRANTS 48 (the format-2 Active launch then composes 32 services and 33 grants); mush-core
- MAX_CONTINUATION_DELEGATES 8 per composition, one grantee per handle, checked at freeze; mush-core
- Renewal front continuation delay 0 ms; tacsim renew.rs
- The market's player-front switch is `front` (@mindrenewal/front <licence> request|accept <op>|decline <op>; the House twin the same); not documented to players (R5)
- Paid-row renewal words `; renew offer` and `; renew paid` (R4); tacsim holdings.rs
- Widest legal HOLD_PAID row pinned at 118 columns at the two-digit phase maxima (Grace, `; no resale; revoked; renew offer`); the 120-column floor stays the pin
- by_code ambiguity counts current rows only, within the unchanged 2-row CODE_ROWS window (a current listing behind two stale prefix-sharing rows is a stated residual)
- relay_bridge (group durable_process) retries 1 in the load-sensitive nextest override; the committer adds it to common-m7's list
- Index page sizes and buyer-offer bounds as the C2b brief set them (8 live quotes, 8 open purchases, 8 live offers per (buyer, licence|charter); no per-holder cap on paid holdings)
- A pre-C2f store with paid licences fails the index boot audit; a fresh store is acceptable pre-launch (no migration)
- The buy steps' idempotency subject is the quote or operation token, not the listing code
- @mindrenewal under a pending authoritative submission prints UNRESOLVED_LINE (submit reports whether it sent)
- Each sub-cut's workspace sweep runs on conclav-dev through scripts/vps-gate.sh --wip --runner conclav-dev as the gate's last command; local runs stay focused (about 15 minutes); the five M7 stories are lead-dispatched after C2f and after C2h
