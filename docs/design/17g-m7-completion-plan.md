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
runs one yet (§7, C1 as built).

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

**What players can reach.** Only the gateway's generic `+actions` /
`+act <verb> with <JSON>` (`crates/mush-gateway/src/bridge_common.rs:532-727`), and the
web client's legal-action form, which submits by offer id and so also reaches a shared
verb that `+act` refuses as ambiguous; only for registered providers:
- Filing by hand needs eleven internal parameters (`mush-roll/src/commerce/filing.rs:731-751`).
- `@mind/publish` refuses ("not available yet", `tacsim/src/minds/workshop/commands.rs:435`).
- `@mind/fork` is the local draft copy, not the paid fork.
- `SIM MIND /filed` lists owner licences only.
- The typed protocol (`mush-proto` `MindRequest`) covers authoring only.
  `Query::MarketOriginalMinds` exists, but no client sends it.

| Feature | Owner code | `MindCommerce` builder | Production | Player entry today | This plan |
|---|---|---|---|---|---|
| Personal filing and owner licence | yes | in `prepare` | launch only | `+act` with 11 ids | C1, C2 |
| Personal original sale and listings | yes | `with_original_purchases` / `_source_purchases` | format-2 launch (C1c: v2 source and v3 entitlement) | `+act` at the counter for v3 and derived; v2 `market-original-mind-*` by the web form only | C1, C2 |
| Paid licence resale | yes | `with_paid_mind_resales` | format-2 launch (C1c), House Pins fenced by the House admission (C1a) | web form only (`+act` refuses the shared `paid-mind-resale`) | C1, C2 |
| Charter transfer and PendingTransfer | yes | `with_charter_transfers`, `with_pending_charter_transfers` | format-2 launch (C1c) | `+act` with JSON | C1, C2 |
| Forks, derived minds, royalties | yes | `with_authored_forks`, `with_dependent_owner_filings`, `with_derived_updates` | format-2 launch (C1c) | `+act` with JSON | C1, C2 |
| Personal renewal and Grace | yes | `with_mind_renewals`, `with_mind_renewal_service` (C1a) | format-2 launch (C1c) | `+act` with JSON (v2 Pins do not renew) | C1, C2 |
| Owner attestation (personal) | yes | `with_owner_attestations` (C1a) | format-2 launch (C1c) | `+act` with JSON | C1, C2 |
| Withdrawal | yes | `with_author_withdrawal` (C1a) | format-2 launch (C1c) | `+act` with JSON | C1, C2 |
| Parent re-qualification | yes | `with_owner_parent_qualification` (C1a) | format-2 launch (C1c) | `+act` with JSON | C1 |
| Registry recall and revocation | yes | `with_registry_recall`, `with_registry_revocation` | format-2 launch (C1c); offered to granted operators only | `+act`, for an operator granted with `@org/grant` | C1, C2 |
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
  - **Possible flake**: `initialized_active` retries only a `disposition=busy` submit,
    not the typed `LegalActionReadError::Busy` from `Running::offer`'s own
    `legal_actions` read; C1a's, C1b's and C1c's mush-server tests all reuse it. A small
    retrying wrapper fixes it.
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
