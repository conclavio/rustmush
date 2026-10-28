# 17h — M8, contraband and procurement: implementation plan

**Draft 0.2, 2026-10-28 — the owner's rulings (§5: Q-M8-1..7 at their defaults), M8a and M8c
started.** Draft 0.1 was planned read-only from `main` at `2cbe9f7c` (the PR #58 merge), with 17g
cuts C0–C3 and doc 23 cuts S.0–S.3 merged, C4 in flight in its own session, and no M8 code in the
tree. This note is the implementation plan for doc 17 §22's M8 — "contraband and procurement, gated
on IC sorties" — in the shape of 17g: what the owner has ruled, where the tree stands against each
M8 item (§2, from four read-only inventories of the crates at that commit), the cuts (§3), what is
deferred and why (§4), the decisions the owner is asked for (§5), the risks (§6) and the ledger (§7).

The one thing the reader should take from this note: **M8 as doc 17 wrote it cannot start whole**,
because its gate needs a mind flying IC (doc 23 §10 tier 2, cut S.4) and doc 23 placed S.4 "inside
D-UM-47's cut" — the drone phase, a mining station, which doc 17 §22 lists *after* M8. §1 proposes
the resolution: split tier 2 into the sealed mind aboard a rig (built here, as M8e / doc 23 S.4a)
and the thinking mind (the drone phase proper, still after M8), and defer the one evidence channel
that needs an IC fight (testimony) to the fight. Everything else in M8 is buildable on the tree as
it stands, and most of it is independent of the sorties question.

## 1. The gate, restated, and the two knots

Doc 17 §22 M8 (verbatim at `17-unit-minds.md:1716-1722`): the governed bit (an ungoverned core alone
skips D-UM-17's floor check, its unlawful arming journalled), ungoverning refits, heirloom and grey
provenance, renewal attestation, custody inspection verbs, black-box seal lines, seizure, fine,
revocation, Tycho's cover condition, the `minds.unwarranted` canon event and the Red Dies front; the
quirk claim and `minds.claimed`; NPC procurement with the diversity cap. **Gate:** observer-rule
leak tests prove an ungoverned core is learned only through the five evidence channels; the zero-PC
test in which an NPC org re-equips from the Roll on its own budget with no charter over the cap.

Doc 23 §10 restates "IC sorties exist" as two tiers and says M8 needs both:
- **Tier 1 (S.1–S.3, built):** a rig's and a ship's epoch open and close with a `SortieId` and a
  `MindSealed`/`SortieClosed` pair in a durable per-hull black box; a rig closed with it out stays
  open and flagged; a re-run tick mints nothing.
- **Tier 2 (S.4, not built):** a drone flying a held mind opens with a real four-owner Open and
  closes with a real Close, its seal line carries the mind, the core and the licence state, and
  M8's leak and quirk-claim tests have a seal line to read.

**Knot 1 — tier 2 is the drone phase.** Doc 23 §9 S.4: "lands inside D-UM-47's cut, not before";
§13: "no such hull flies IC until the drone phase, and the drone phase is D-UM-47's own cut"; doc 20
§17 reserves the drone phase (asteroid fields, drone bays, the mining station's `scan()`, `sound()`,
`goto()`, `mine()`, `recover()`, a `mining` role doctrine, a twin fixture) and doc 17 §22 "After M8"
lists "the first non-combat station (mining or cargo) … under D-UM-47 and D-UM-48". Read together:
M8 waits on a cut that waits on M8. (Doc 17 "After M8", as S.0 amended it at `:1748-1755`, already
says the custody half "lands on the IC-clock pull ticks as doc 23's cuts S.1–S.4 *before* M8"; the
split below is that sentence made buildable.)

And one fact under both knots: **no IC hull carries a core today.** `tv_prospector`, `dray`,
`dray_p` and `halcyon` (`crates/tacsim/content/units/`) have no core section and no core component;
the only cored chassis are the arena's (the line tanks, the bipeds, the heavy, the scout, the drone,
`cores.json`), and the refit's root family is three line-tank roots (`core_replacement.rs:1-35`).
Every rig seals Class 0, the ROM. So before a core can be inspected, ungoverned or sealed with a
mind on a surface excursion, a rig has to be able to carry one: content (a core section on the TV)
and a refit root family for it. That is M8b's first step (§3).

What tier 2 actually needs, against what the drone phase is: tier 2 wants a *seal line that names a
mind, a core and a licence*, written by a real Open and closed by a real Close, on a production
host, so that an inspection or a black-box read has something to find. It does not need the mind to
think. The drone phase is where a mind first *thinks* IC (doc 23 §6's table), which needs the
station. **The proposal (Q-M8-1): split S.4.**
- **S.4a — the sealed mind aboard** (this plan's M8e): a rig with a core component and an operator
  who holds a licence to a filed mind seals that mind at the excursion's launch — the claim book
  accepts the IC `SortieId`, the surface finish mints the `StartPermit` and `CloseEvent` through the
  four-owner composition instead of the test fixtures, admission counts the site's pool, the black
  box line carries the mind, the core and the licence state, the `abi` drop on resume is wired —
  and the rig's excursion then runs exactly as today, on its fill plan, the mind sealed and dormant
  (the hull has no station the mind could act on; doc 17 §12's **NONE** mode, under a seal). The
  honest minimum that gives M8 its seal line.
- **S.4b — the thinking mind** (the drone phase proper, D-UM-47's mining station): stays after M8
  as doc 17 §22 has it. Nothing in M8 reads whether the mind thought.

The alternative is to build the drone phase first (a train of its own, doc 20 §17's size) and start
M8 after it. The lead recommends the split: M8's evidence and consequence machinery is a Registry
and economy programme that does not care whether a sealed mind acted, and the drone phase is a
surface-mining programme with its own owner questions (fields, bays, the leash).

**Knot 2 — testimony needs a fight.** The fourth channel is "a hit by a contact the victim's own
sensors never classified". Hits exist only in the arena (`EventKind::Hit`, `arena/ledger.rs:302`;
no classified bit on it; `Classified` is its own line at :182, so the fact is rebuilt by ordering
the tape). IC, nothing shoots: doc 16c §3.1 H1 refuses the first IC fight until the owner settles
destruction, who may start one and the proving ground; doc 23 §8 keeps it out; the surface and port
black boxes have no hit lines. **The proposal (Q-M8-2):** M8 builds four of the five channels and
records testimony as the fight's cut (H1 lands it, reading the arena tape's `Classified`/`Lost`
order against `Hit` for the victim), with the leak tests written against the four and the fifth
named as owed in the record. The alternative — build the arena-side half now (a `Hit` carrying
`classified_by_victim`) so H1 only wires it — is small and is offered as M8g's optional first step.

## 2. Where M8 stands (verified at `2cbe9f7c`)

Four read-only inventories of the crates, one per area; the headline per item, with the anchor the
builder will start from. Paths are under `crates/`.

### 2.1 The core, the governed bit, the seal

| Item | State | Where |
|---|---|---|
| `core` content kind, `ComponentRole::Core`, `tacsim.core` capability payload with **`governed: bool`** (default true) | **built** | `tacsim/src/definitions/core.rs:35-119` (`CoreDef`, `profile_payload`), `definitions/unit.rs:1946-1977` (lowering), `arena/core.rs:382` (`read_core` → `CoreHardware { component, class, governed }`; a missing bit reads true) |
| Shipped cores | all governed | `tacsim/content/cores/cores.json` (doctrine I, line II, command III) |
| `governed = false` anywhere | tests only | `tests/shard_2/extensions.rs:11113-11207` (operator pack `house.core.relay`, an ungoverned II), `tests/shard_4/core_replacement.rs:380` (an ungoverned III refused by replace), tamper edits in `match_/seal.rs:1136` and `arena/hash.rs:3024`; `ic_sortie/audit.rs:94-98` is the production seal-against-hardware check, not a test site |
| What the seal does with the bit | **partial** | `match_/seal.rs:543-558`: an ungoverned core skips only `OverClass` (the class becomes `min(rated, core, cap)`). **D-UM-17's "an ungoverned core alone skips the floor check" is absent at all three of D-UM-17's points**: the seal (`seal.rs:673-679`, the stock-preset path), the arming's landing (`arena/conditions.rs:567-573`, `condition_is_lawful` in `resolve_arming`) and the cited fire (the warrant re-checked per shot). No test seals an ungoverned core. |
| Which IC hulls carry a core | **none** | `content/units/` (`tv_prospector`, `dray`, `dray_p`, `halcyon`: no core section); `seal_rom_for` seals every rig Class 0 (`surface/black_box.rs:741-768`); the refit's roots are the three line tanks (`definitions/core_replacement.rs:1-35`, recipe `core_refit` "Governed line-tank core fitting") |
| Unfiled digest, unlicensed line, licence state at the seal | upstream of the seal, IC path only | IC execution requires a filed certificate (`minds/ic_sortie/execution.rs:88-125`; `validate` at :140-149 refuses a non-certificate origin and a non-`Packed` id). **The surface dispatch reads no mind licence at all** (the ROM seal only). The mind-licence state is `TermPhase` (`mush-roll/src/commerce/mod.rs:314`) plus `RevokedTerm` and `RevisionStatus.recalled_by`; `port/licence.rs` is the Freeport's *prospecting* licence (doc 20 SM.5), not a mind licence. |
| `degrade_core` / `destroy_core`, `DropCause::{Strikes, Core, Abi}` | **built** | `definitions/unit.rs:380-401`, `arena/damage.rs:985-1060`, `arena/mind.rs:253-282`; the IC `abi` drop is appended at `ic_sortie/execution.rs:189-243` on install |
| Pack listing prints "governed"/"ungoverned" | built | `tacsim/src/commands.rs:1199-1203` — the only player-facing word for the bit |

### 2.2 IC hardware and provenance

| Item | State | Where |
|---|---|---|
| `tacsim.electronics` lots, `DieClass { I, II, III }` as `die.c1/c2/c3`, `Lineage::Origin` from `certified_core_die.v1`, `OriginPolicy { importer, scope, depot, agents, grades, monthly_quotas }` | **built** | `mush-industry/src/core_production/{records.rs:16-27,193,213; config.rs:26}`, `mush-inventory/src/pool.rs:1846`; the importer is a generic `OrgId` the rigs name "Basin" (`tacsim/src/hardware/flow.rs:223-384`, `launch/deployment.rs:63`; `monthly_dies` [64, 16, 4] in `tacsim/content/hardware/governed_hardware.json`) |
| Governed integration (`core_integrate`) and the refit work order | **built** | `mush-economy/src/core_integration/*`, `mush-industry/src/core_production/{integration,refit}/*` (offer, accept, advance, `install_core_component`, cancel; `FITTING_MINOR` 10000); installation through `replace_component` (`mush-tactics/src/unit/replacement.rs:129`) and `tacsim/src/definitions/core_replacement.rs` (`GovernedLineCores`, **governed only**) |
| `UnitMutationOp::InstallComponent` | absent by design | `mush-tactics/src/unit.rs:1171`; `tests/shard_4/core_replacement.rs:526-534` pins that it does not deserialize. The install is `replace_component` + `SetComponentAsset`. Doc 17 §9's sentence is overtaken; this note does not add the op. |
| `InstalledComponent.asset`, core title births | **built** | `mush-tactics/src/unit.rs:335-353`, `mush-economy/src/managed_title.rs:20` (`TitleBirthKind`, a private enum: a provenance read needs a public accessor) |
| The refit's authority | built, governed only | `core_production/refit/authority.rs:100-124`: one configured factory owned by the provider org, the facility capability `core_refit`, `require_core_delivery`. There is no "charter" object; doc 17's `core_integration` charter is this capability. `core_production/authority.rs` is the importer's origin check. |
| A core's removal | **absent** | the refit always installs an incoming core; the displaced one becomes `AssetPosition::Detached { order, from_hull }` (`refit/audit.rs:275-296`). No path takes a hull to Class 0. |
| The industry journal of a refit | partial | the `ReplacementJournal` of mush-tactics (`refit/policies.rs:64`, `refit/capacity.rs:22`); nothing records *who* integrated a core on its title |
| `core3_end_user` entitlement, `Legality::Restricted` | partial | `mush-economy/src/core_integration/{subject.rs:11, authority.rs:200-215, delivery.rs:81-95}` checked for Class III, provisioned only in tests; `Legality` exists in mush-market (`record.rs:173-201`) and no tacsim content sets it |
| **The ungoverning refit, heirloom dies, grey dies** | **absent** | docs only (`17-unit-minds.md:866,1463,1717`) |

### 2.3 The Registry: states, recall, revocation, attestation, facts

| Item | State | Where |
|---|---|---|
| Term phases | partial | `TermPhase { NotEffective, Active, Grace, Lapsed }` (`mush-roll/src/commerce/mod.rs:314`); **Recalled** is per filed revision (`RevisionStatus.recalled_by`, `records.rs:298`; `RatingStanding::Recalled`, `rating.rs:1444`); **Revoked** is per licence term (`registry_revocation/records.rs:206`, `RevokedTerm`). No single state enum, by design. |
| The licence holder's type | character only | every family's holder is a character `ObjId`: `house_purchase/records.rs:31`, `entitlement/records.rs:30`, `paid.rs:108`, `holding_reader.rs:53`, `dependent_owner.rs:340`; the dispatch holding refuses an org title (`owner_holdings.rs:382`, `title_owner != PrincipalId::Object(actor)`, the comment at :125 saying why); `@mind/attest`'s command layer gates on `own_filed` (`market_commands/author.rs:360`) |
| Recall | **built** | `commerce/registry_recall*` (capability `roll.registry.recall`, the 5-s notice drain at `notice.rs:310-376` that aborts on one bad holding — 17g §4's follow-up), `@registry/recall <filed>`, `@mindrecall/status` |
| Revocation of one term | **built** | `commerce/registry_revocation*` (capability `roll.registry.revoke`; Active or Grace only; the generation dead for good; a successor only after `grace_end`; closes a pending charter sale and a paid resale with the escrow refunded), `@registry/revoke <licence>`, `@mindrevoke/status` |
| **Revocation of a principal's licences for a term** | **absent** | nothing acts per principal; no duration but the term's own Grace end |
| `@registry` as a command | **built** | `tacsim/src/minds/bulk/registry_service.rs:37-404` (`/rate`, `/cancel`, `/recall`, `/revoke`; `/continue`, `/reconform` GOD-only); operators through `RegistryAuthority { organization, scope, operators }` (`launch/deployment.rs:134-145`; the Registry is a real org here) and a crate-private `authority::observe` per capability (`registry_recall/authority.rs:82`, `registry_revocation/authority.rs:82`: operator ∈ list, org and scope Active, a live `mush_org::authorize_in_world` grant) — a new verb needs its own capability and twin |
| Personal owner attestation | **built** | `with_owner_attestations`, `@mind/attest <filed>`, `owner_attestation/provider.rs:185-433` (no pending charter, not revoked, title held, **not a House birth** (:256), head not withdrawn or recalled, reconformance current, the term due, a fork's parent live). **It reads no core and no black box: attestation as a detection channel is absent.** |
| **House owner attestation** | **absent** | 17g §1 ruling 1 deferred it behind C0's 365-day term (`with_house_owner_terms`, `module.rs:534`); the provider refuses a House birth; an org principal has no authority path (`HouseFilingAttribution` is attribution, not authority) |
| Fact kinds | partial | provider `minds`: `filed` (public), `licence.lapsed`, `licence.transferred` (scoped; E4 contract conditions are the only consumers) at `commerce/facts/protocol.rs:13-16`; the test at :160 pins that **`seized`, `unwarranted`, `claimed` do not parse** |
| Canon, fronts, clocks | no minds kind | `mush-faction/src/sim.rs:53-158` (`Front`, `Clock.advance_on: Option<canon kind>`); tacsim commits canon through `mush_canon::commit` in three places (`composition/econ_canon.rs:240`, `port/site.rs:2093`, `relay_foundry.rs:1249`) and none is a minds kind; fronts are made in code, not content (`relay_world.rs:76-90`, the "Terms of Service" front); **no Red Dies front** |

### 2.4 Consequences in the economy

| Item | State | Where |
|---|---|---|
| `Encumbrance::{Lien, Disputed { claimant, reason }, Lease, Collateral}`; `ReportStolen` records `Disputed` | **built** | `mush-economy/src/record.rs:749-761`, `api.rs:415-470`, staff `@asset/{lien,release,stolen,transfer,title}` |
| Encumbering or re-titling a mind charter or a core | **blocked** | `managed_title.rs:119-141` refuses `Title`, `TransferTitle`, `Encumber`, `SetCatalog`, `Abandon`, `WriteOff` on managed kinds `Charter` and `Core` for everyone, staff included ("a future managed transfer needs a separate private closed plan proof"); the charter transfer refuses any encumbrance (`managed_charter_transfer/native.rs:216`). **Seizure needs a new managed path.** |
| `issue_title` | **built** | `api.rs:2816`; issuers registered by `with_title_issuers`; tacsim registers `tacsim.registry` (`port/hull.rs:80`) and the filing issuer (`composition/mod.rs:300-345`) |
| An obligation as a **fine** | **partial** | `create_obligation` (`api.rs:2045`) needs a Ruleset cause and the actor's *own* debit authority; `ObligationSpec` (`api.rs:1759`) is a standing charge with an `EconomicSchedule` and a grace; `ObligationCause { Contract, Staff, Ruleset }` (`obligation.rs:229`); **no involuntary one-shot debit of a sanctioned principal** |
| A per-account budget | built, wall-clock | `ApprovalRule.window: Option<ApprovalWindow>` (`record.rs:919-925`; `@account/approval … window=<total>/<secs>`), a rolling spend cap per account in seconds, not IC months |
| Hull cover, Tycho | test-composed only | `tacsim/src/underwriting/*` (`CoverageTermsV1`, whose comment at `terms.rs:50-52` says the exclusion policy is a product binding *separate from* the terms), `minds/ic_sortie/host/coverage.rs` (`CoverageCloseEvidence`, `WithheldDeparture`, both constructors `#[cfg(test)]`), `with_hull_coverage` not in launch (17g §3 "Production: no"); the decision kinds are mush-economy's `CoverageDecisionKindV1 { Destroyed, CleanClose, Cancelled, ExpiredClean }` with `pays()` (`managed_coverage/decision.rs:9`); qualification already requires `seal.governed` (`underwriting/qualification.rs:141`, `coverage.rs:746`); precedent for "Tycho voids cover" is `mush_sim::relay::cover_voided` (`mush-sim/src/relay.rs:128`) |
| The Registry's principal | four roles, no one actor | the Registry **org** (`RegistryAuthority.organization`, `launch/deployment.rs:134-145`), the deployment's `registry_issuer: SystemPrincipalId` (charters; `:100`), the levy sink `registry: Recipient`, the hull issuer `tacsim.registry`; `PlayerMarket.clerk` is the market's service actor, not the Registry |

### 2.5 The epoch, the black box, the seal line

| Item | State | Where |
|---|---|---|
| `SortieId::{Packed, Ic { hull, at_ic }}` | **built** (S.1) | `arena/mind.rs:307-417` |
| The ROM seal over a `TacticalUnit` | **built** (S.1) | `match_/seal.rs:473-531` (`IcSealConfig`, `seal_rom_for`) |
| The per-hull black box | **built** (S.1–S.3) | `tacsim/src/surface/black_box.rs` (1605 lines): `log:hull.<n>` on `TacsimStore.black_boxes`, `BlackBoxLine::{MindSealed, SortieClosed}`, `EpochHost::{Surface, Port}`, `Admitted { class, host, count }`, `BlackBoxParticipant` registered in `port_tick.rs:672-732`; cap `[tacsim.surface].black_box_max_lines` 4096, **no retention** (a full box refuses and pages) |
| **A read path** | **absent** | `TacsimStore::{black_box, black_box_lines, black_box_len, black_box_dark}` (`store.rs:391-426`) decode the **RAM open tail only**; the sealed history is `black_box::history(docs, hull)` (:671) over the `log:hull.<n>:<k>` documents the caller fetches from the store (an async read); both are Rust seams called only by tests; nothing in mush-proto, mush-server or mushd reads a box; `@arena/report` reads the arena tape. **S.1's owed "store-read completion … no verb until M8's inspection".** |
| The seal line's fields | **partial** | arena `EventKind::MindSealed { unit, sortie, mind, source (16 hex), card_digest, class, governed, licence, principal }` (`arena/ledger.rs:579`), built by `MindSeal::sealed_line()` (`arena/mind_seal.rs:130-143`); the box line is assembled by its own `black_box::sealed_line` (:581) from `SealFacts::of` (:187), itself read off the arena line, and adds `at_ic`, `host`, `hangar`, `site`, `seal`, `seal_digest`, `fallback`, `admitted` (`black_box.rs:232-277`). **`licence` is a label only** (`profile_seal.rs:106`: "doctrine", "personal draft" or the filed licence id) — doc 17 §13's "licence validity" is not recorded. |
| The claim book and the four-owner Open/Close | **test-bounded** | `ic_sortie/state.rs:36-109` refuses an IC id at load and requires a packed `next_transition`; `execution.rs:106-108` seals `Packed(sortie)` "until S.4" and `validate` (:140-149) refuses a non-`Packed` id and a non-certificate origin; `records.rs:193-199` and `audit.rs:63` require the packed kind; `HostTransition` (`participant.rs:24`) is private with no constructor, so `StartPermit`/`CloseEvent` (:27, :32) cannot be minted in production; `fixture_start`/`fixture_close`/`fixture_withheld`/`fixture_covered_*` are `#[cfg(test)]` (:500-591); `with_ic_minds(Vec<ConfiguredIcBase>, …)` (`composition/mind_commerce.rs:352-375`; the bases are interior fixed domains, `mush-sim/src/ic_exclusion.rs:286`) has only test callers. The doc comment at `black_box.rs:89-90` says S.4 adds the drone host. |
| Where epochs open and close | **built** | surface: `surface/act.rs:9203-9244` (`epoch_move`, inside the finish's **prepare**, with no `World` in hand, `black_box.rs:705-712`), `open_epoch` (`black_box.rs:828`), `surface/recovery.rs:845-1138`; port: `port/epoch.rs:55-1069`, `tacsim.ship_epoch` |
| Observer-rule harness | **built** | `mushd/tests/sim_lenses/observer_rule.rs` (five witnesses, positive control then absence, `ask_whole` + sentinel, `no_sixteen_digit_word`), `mush-contract-tests::projection_json_leaks*` |

### 2.6 Procurement

| Item | State | Where |
|---|---|---|
| The mind market, the sell side | **built**, differently shaped | typed signed-offer families in `mush-market/src/mind_{house,entitlement,trade,renewal,house_renewal,paid_resale}/` and `mush-roll/src/commerce/house_*`; the `CatalogSource`/`AvailabilityProvider`/`DeliveryProvider` route doc 17 §16 describes exists as traits (`mush-market/src/module.rs:37-86`) with only test catalogues and one real `AvailabilityProvider` (`CopratesMembership`, `tacsim/src/composition/coprates_membership.rs:124-145`, not in production composition) |
| The launch catalogue | **built** | Meridiani Field Series, Coprates Drill, Safe Conduct as shipped source, envelope and manifest in `tacsim/content/minds/house/*`, imported by `LaunchRecipe` (`launch/content.rs:21-80`); the Doctrine Library is free pages (`minds/doctrine_library.rs`, `@mind/doctrine`), not a House; `NamedHouses { meridiani, coprates, tycho }` are deployment coordinates with an org treasury (`launch/deployment.rs:36-50`) |
| The Republic standing rule | **built as a seller's rule** | `CopratesAdmission` (`launch/admission.rs:66-102`) is a `HouseBuyerAdmission` scoped to the Coprates House's own offers (`issuer != self.issuer → Ok`): a live `FactionStandingProvider.eval("member", republic)` on the buying character (`ObjKind::Player`, not GOING), repeated at House renewal and paid resale Accept/Fund. It is the 2026-10-18 ruling, not D-UM-54's buyer-side standing filter. |
| The rating | **partial** | the certificate (`commerce/rating.rs:980`): per-class `ClassSummary { outcome: ClassOutcome { Passing, BudgetExceeded }, cases, meter, peak_case }` (:911) — "Passing" means within the ops budget and holds no win count; the per-case `TrialOutcome { Win, Loss, Draw, Timeout }` sits on `CaseResult.terminal` (:596) in the result pages `RatingResultRef` names, not on the certificate; **no score and no "per credit"** |
| **An organisation as a mind holder** | **absent, refused by design** | every family's holder is a character `ObjId` (§2.3's row); the dispatch holding refuses an org title (`owner_holdings.rs:382`); the arena fill is keyed by the side lead's character (`match_/holdings.rs:1-33`); the buyer's flow is counter-bound Quote → Pay from the actor's *personal* account → Complete (`market_commands/market.rs:38-48, 225`) with no org-wallet payer and no tick-driven buyer |
| Org budgets | partial | `GrantLimit { max_amount }` per operation (`mush-org/src/record.rs:202`); the economy's per-account `ApprovalRule.window` (§2.4) is a rolling wall-clock cap; no IC-month budget in economy, org, faction, npc or market; the faction "pressure budget" is deferred (`mush-faction/src/sim.rs:16`) |
| An NPC purchasing loop | precedent only | the port houses' funded standing ore bids (`tests/shard_3/orders.rs:2800,2865`, `shard_2/plumbing.rs:2573`); Occator's dispatcher mines and delivers in kind with no market call (`port/yard.rs`, `operator_principal` → `Organization(house)`), its rigs coreless |
| **Procurement, the diversity cap, the zero-PC test** | **absent** | docs only |

### 2.7 What this adds up to

Three of the five channels have most of their substrate (the black box, the seal, the live core
read, attestation's provider); none of them has a verb, a validity bit or a hardware check, and no
IC hull has a core to read. The crime has no mechanism (no ungoverning, no heirloom, no grey, no
cored rig). The consequences have their nouns (Disputed, obligation, revocation) and each is
blocked at the exact place M8 needs it (managed titles, involuntary obligations, per-principal
revocation, no core removal). Procurement has a seller and no buyer: an organisation cannot hold a
mind licence at all, which is also what House owner attestation needs, so one cut unblocks both.
The zero-PC story therefore has two halves — the purchase (M8f on M8c) and the flight (M8e) — and
the gate's "re-equips" is met by the purchase plus the next seal carrying the licence.

## 3. The plan

Run each cut through the established cut loop (builder, lenses, fixer, gate, judge; one commit and
PR per cut off `main`; the briefs under `plan-data/m8-contraband/cuts/<cut>/brief.md`, the common
rules in `plan-data/m8-contraband/common-m8.md` inheriting doc 23's `common-s.md` in full: `-j 8`,
no second workspace build, test only what changed, the eight-section report, no `docs/design/`
edits by a builder, no "sortie" on a player surface). The gate per cut is the cut's own tests on the
box plus the PR's CI shards; the long stories (M8f's and the M7 subset) run only on the CI dispatch.
Sizes are the lead's estimates against 17g's cuts (C0 small, C1 medium-large, C2 large).

Dependency order: **M8a ∥ M8c** (independent: surface/commands; roll) → **M8b** (after M8a, since
D-UM-42 says no contraband cut ships before its detection channel exists) → **M8d** (on M8a's
inspection record and M8b's crime), **M8e** (on M8a's validity word, M8b's cored rig and M8c's org
holder) and **M8f** (on M8c) → **M8g** (on all). Two cuts can run at once on the two VPS boxes and
this one; the owner's 2026-10-26 word allows it.

### M8a — The reads: the black-box verb and custody inspection (medium; independent)

The two deterministic channels, and S.1's owed read. No crime exists yet and no IC hull has a
core; the cut reads the rigs as they are (Class 0, the ROM) and a cored arena hull in a test world,
and pins what the reads say, so that M8b's cored and ungoverned cases slot into the same tests.

- **The seal line's validity word.** `MindSeal::sealed_line()` (`arena/mind_seal.rs:130`) takes a
  new parameter `licence_state: SealLicenceState { Doctrine, Draft, Active, Grace, Pending, Lapsed,
  Revoked, Recalled }` — a parameter, **never a `MindSeal` field**, so `MindSeal::digest()`, the
  claim book's `seal_digest` and the audit do not move — and the arena `MindSealed` line and the
  box line (through `SealFacts::of`) carry it; `black_box::sealed_line` passes it through. One
  function maps a mind licence's state to the word from `TermPhase`, `RevokedTerm` and
  `RevisionStatus.recalled_by`. Today only the IC execution resolves a mind licence at a seal
  (`execution.rs:88-125`), so M8a wires the word there and writes `Doctrine` for every ROM seal;
  M8e's surface dispatch reads the state through the same function. The strict-shape test moves
  (`s1_the_two_line_shapes_are_strict`); the arena's pinned tapes move once and are re-blessed.
- **The owner's read, `+hull/box <hull> [n]`.** A player verb (a `CommandSpec` with help and a
  `legacy-handlers.toml` stanza) printing the last *n* lines (default 8, at most 32) of a hull's
  black box to its title holder, its leasing operator or staff, one line per record: the kind, the
  IC instant, the host and hangar, the mind's name and class, "governed"/"ungoverned", the licence
  word, the close kind and recoverer. Refusals name nothing (the observer rule): a hull the actor
  cannot read answers as a hull that has no box. No sixteen-hex-digit word; the seal digest is
  shown as its first eight. The read is `black_box::history` over the `log:hull.<n>:<k>` documents
  fetched from the store — an async store read in the command's reply path, the market panel pulls'
  shape — joined to the RAM open tail; a dark box says so. Spelling is the owner's (§5 question 3).
- **Custody inspection, `@registry/inspect <hull>`.** A Registry operator's verb under a new
  capability `roll.registry.inspect` with its own `authority` twin (the recall's and revocation's
  are crate-private per capability), deterministic, reading in one txn: the live core (`read_core`:
  component, class, governed; "no core" for a Class 0 hull), the core title's birth and lineage
  (through a new public accessor over `TitleBirthKind` and the lot's `Lineage::Origin` or its
  absence), the hull's open epoch and its seal line (the seal digest against the live hardware,
  `audit.rs:94-98`'s check made a read), and writes an `InspectionRecord { hull, core asset, class,
  governed, provenance word, seal digest, licence word, at_ic, operator }` to a new Registry family
  `roll_registry_inspections` (bounded, like the revocations). The reply is the record in words.
  The hull need not be in a hangar: inspection in the field is doc 17 §10's "custody inspection
  (deterministic, reads the bit and the provenance)".
- **A provenance word.** One function, `provenance_of(core asset) -> Provenance { Certified
  { importer, grade }, Heirloom, Grey, Unknown }`, read from the title birth and lineage; M8a
  returns `Certified` or `Unknown`; M8b adds the other two.
- Gate: a rig's excursion inspected in the field and at the pad reads "no core", `Doctrine`; a
  cored line tank in the test world reads `Certified`, governed; the owner's `+hull/box` shows the
  open and close lines; a bystander, the other operator and a pilot read nothing (the observer-rule
  pattern of `sim_lenses/observer_rule.rs` applied to the two verbs, the positive control first);
  the record survives a restart; the cost lane's write census moves by the inspection row only.

### M8b — The crime: the cored rig, the ungoverning refit, heirloom and grey dies, the ungoverned seal (large; after M8a)

- **The cored rig (first).** Content: a `core` section on `tv_prospector` (the chassis the
  excursion drives; empty by default, so every shipped rig still seals Class 0 and no fixture
  moves), and a second refit root family beside `GovernedLineCores`' three line-tank roots —
  `SurfaceRigCores`, the TV's roots per class — so the existing governed `core_refit` can fit a
  certified core to a rig at Stickney. `seal_rom_for` already reads the core live, so a fitted rig
  seals its class with no code change at the seal. A dray stays coreless (a ship is driven by the
  port tick, not a station) until a cut wants otherwise. The content digest pins move once.
- **The ungoverning refit.** A second recipe, `core_ungovern`, with its own facility capability
  `core_ungovern` (doc 17's "facility without a `core_integration` charter" is a facility that
  holds this capability and not `core_refit`; the deployment check refuses a facility with both),
  the refit's authority (`core_production/refit/authority.rs:100-124`) generalised over the two
  capabilities, priced and timed like a fitting, leaving `ReplacementJournal` rows and a
  **provenance note** on the core's managed title (`CoreProvenanceNote { Ungoverned { facility,
  at_ic } }`, written by the refit's completion), and rewriting the installed core's capability
  payload `governed: false` through the existing `replace_component`/`SetComponentAsset` path with
  the root families carrying an ungoverned twin of each class. `core_replacement.rs:90`'s "not the
  exact governed stock class" and `tests/shard_4/core_replacement.rs:380` move: the ungoverned
  payload is refused by the governed recipe and accepted by this one.
- **Heirloom and grey lots.** Two die sources beside `certified_core_die.v1`: `enclave_heirloom.v1`
  (a lot whose `Lineage::Origin` names no importer and whose grade is `die.h`, integrating as an
  ungoverned Class III) and a **grey** lot (`tacsim.electronics` with no `Lineage::Origin` at all,
  integrating as an ungoverned core of the lot's declared class). Integration of either runs only
  through the ungoverning recipe's facility. Staff issue them (`issue_heirloom_die`,
  `issue_grey_die` actions beside `issue_core_die`); no vendor sells them in this cut (§4: the
  Tether's back room is its own design).
- **The ungoverned seal and the three floor points.** With `governed == false`, D-UM-17's floor
  check is lifted at all three points — the seal (`seal.rs:673-679`), the arming's landing
  (`condition_is_lawful` in `resolve_arming`, `arena/conditions.rs:567-573`) and the cited fire —
  and the IC execution admits an unfiled personal draft (`authored: None` with the draft's source
  digest; `validate` at `execution.rs:140-149` lifted for an ungoverned seal) and an unlicensed
  line (the licence word `Draft`/`Lapsed`/`Revoked` on the line, the seal taken); over-class is
  already admitted. Every such admission is a black-box line field, never a notice — detection is
  evidence only (D-UM-42). In the arena nothing changes in play: every pod core is governed
  (D-UM-41), pinned by a test.
- **Unlawful arming journalled.** The arming's ledger line (or a new `Armed` line if arming is not
  journalled today — the builder checks) and `Fired` gain `unlawful: bool`, true when the cited
  condition is under the seal's floor or on an unclassified contact and the seal is ungoverned;
  `state_hash` does not fold them (record fields); `each_new_row_field_moves_the_state_hash` is
  unchanged. Unreachable in play until the fight and cheap to pin now.
- Gate: a rig fitted at Stickney seals its class and reads `Certified` to M8a's inspection; a rig
  refitted at the ungoverning facility reads ungoverned with the provenance note and
  "ungoverned" on its next seal line; a heirloom integrates as an ungoverned III and a grey as its
  class; the governed facility refuses both; the governed recipe refuses an ungoverned payload (the
  moved pin); the arena seal admits an unfiled draft on an ungoverned core and refuses it on a
  governed one, with the line field proving which; the panic budget and the content digest pins
  re-blessed with reasons.

### M8c — Organisations as holders: House owner attestation and the org-held licence (medium-large; independent)

The one cut that unblocks both the deferred 17g §4 item and procurement. It is not one line: every
commerce family records its holder as a character `ObjId` (§2.3), so the holder becomes a typed
value across the families. Pre-launch, no migration (the standing rule: no migrations for data
that does not exist).

- **The holder.** `Holder { Character(ObjId), Organization { org, acting: ObjId } }` replaces the
  bare `ObjId` in the licence, holding, paid, dependent-owner and house-purchase records and their
  readers, serde-tagged so a character holder's bytes are unchanged (the pinned digests hold). An
  org acts through a character granted the scoped capability `roll.holder.org` by `@org/grant` on
  the org's scope (the shape of `roll.registry.recall`); that character projects the org's charter
  titles and licences into a dispatch holding (`owner_holdings.rs:382`'s refusal becomes the
  capability check) and acts for the org at the market — buy, renew, attest, accept a transfer —
  with the org's treasury as payer (the buyer flow's Pay step gains an org-account payer behind
  the same grant). The acting character is recorded on every operation. The alt refusal extends
  to "one account's characters acting for one org" through the lobby's `AccountId` oracle. The
  rejected alternative — a steward character holding *for* the org — would make the Roll's records
  lie about who holds, which the Registry's reads (M8a's licence word, M8d's revocation per
  principal) cannot afford.
- **House owner attestation.** `owner_attestation/provider.rs:256`'s House refusal becomes a
  branch: a House birth attests through `@mind/attest <filed>` by a character holding the org
  capability (`market_commands/author.rs:360`'s `own_filed` widened to the org's filed view), using
  `house_terms` (C0's 365-day term) for the new generation; the C0 runway note in 17g §3 is closed.
  The chain, title and reconformance checks apply unchanged, the title check reading the org as
  owner.
- **Attestation as a channel.** mush-roll cannot read tacsim's black boxes (a different crate and
  an async chunk store), so the channel is a **push**: at every IC seal the black-box participant
  also writes one row to a bounded roll family `roll_licence_seal_marks { licence, hull, at_ic,
  governed }` (a second line in the same bundle; the cost lane's census moves by one write per
  launch, re-blessed in M8e where the first IC seal lands). Attestation (personal and House) reads
  the licence's marks for the ending term and refuses when any is ungoverned, the reason on the
  refusal and the scoped fact `minds.licence.unattestable` published — "an ungoverned core cannot
  attest". In M8c the family, the read and the refusal land with a test that seeds a mark; the
  first production mark is M8e's.
- Gate: a House's owner licence attested at day 365 by its curator, refused by a character without
  the grant, refused for a second character of the curator's account; an org buys a listed line
  with its treasury and holds the licence as `Organization`; every existing commerce story is
  byte-identical for character holders; a personal attestation refused on one seeded ungoverned
  mark with the reason.

### M8d — The consequences: seizure, fine, revocation for a term, the facts and the front (medium-large; on M8a, M8b)

- **Seizure.** A new managed-title path with its own closed plan proof (the `managed_title.rs`
  comment's condition): `@registry/seize <hull>` on a hull whose latest `InspectionRecord` (M8a)
  reads ungoverned or `Grey`/`Heirloom` provenance marks the core title `Disputed { claimant: the
  Registry org, reason: inspection id }` and removes the core from the hull through a **removal
  order** — new work: the refit's shape with no incoming core, the hull to Class 0 and the core
  `AssetPosition::Detached` into the Registry's custody (today the refit only ever installs, the
  displaced core going `Detached { order, from_hull }`); `@registry/forfeit <inspection>` after
  `[tacsim.minds].seizure_hold_ic_days` (default 3) transfers the title to the Registry org
  (`RegistryAuthority.organization`, an org that already holds a treasury; §5 question 4 for the
  choice against the `registry_issuer` system principal). Both are Registry operator verbs; both
  write `minds.seized` (public, the hull named, the mind not).
- **The fine.** `ObligationCause::Sanction { inspection }` and `create_sanction_obligation` in
  mush-economy: a one-period `ObligationSpec` (one due instant, the obligation's existing grace
  and delinquency handling after it) debiting the sanctioned principal's account under the
  Registry's own capability, not the actor's debit authority, paid to the levy sink;
  `@registry/fine <inspection> <amount>` with a cap `[tacsim.minds].fine_cap_minor` (default 10×
  the filing fee). An account that cannot pay carries the obligation; nothing is minted.
- **Revocation for a term.** `RevokedPrincipal { principal, from, until, inspection }` beside
  `RevokedTerm`: every licence the principal holds (character or org, M8c's `Holder`) is revoked
  as today (the existing per-term path called per licence, the pending sales closed and refunded),
  and no new licence may be written to the principal until `until`
  (`[tacsim.minds].revocation_ic_days`, default 90, one audit term). `@registry/revoke` grows a
  `principal=` form.
- **Tycho's cover condition.** The exclusion is a **product binding**, as `terms.rs:50-52`'s
  comment has it, not a terms field: `CoverageProductV1` gains `exclusions: Vec<CoverageExclusion>`
  with the first member `UngovernedCore`, and mush-economy's `CoverageDecisionKindV1` gains
  `Voided { inspection }` (`pays()` false) raised when an `InspectionRecord` reads ungoverned on
  the covered hull within the term. Underwriting stays test-composed (§4); the rule and its test
  land so that the live story inherits it.
- **The facts and the canon event.** `minds.seized` (public), `minds.unwarranted` (public: an
  inspection found an ungoverned core, or an `unlawful` arming line reached a black box — the
  second unreachable until the fight), `minds.claimed` (scoped, M8g) added to the `minds`
  provider; the protocol test at `facts/protocol.rs:160` inverts. `minds.unwarranted` is also
  committed as a canon event through `mush_canon::commit`, the way `port/site.rs:2093` does.
- **The Red Dies front.** In code, the `relay_world.rs:76-90` precedent: a front on the
  Republic's board made through the `mush_faction` APIs at world build, with a clock whose
  `advance_on` is `minds.unwarranted` (§5 question 5 for the board, the length and the end); the
  first clock in the tree to advance on a minds event. No content pin moves.
- Gate: an inspected ungoverned rig is seized and dropped to Class 0, its core titled to the
  Registry org after the hold, the fine an obligation on the operator's account due once, the
  operator's licences revoked and a purchase refused until the term ends, the covered hull's cover
  voided in the test composition, the three facts readable by an E4 condition, the front's clock
  one step on; a governed rig cannot be seized (the verb refuses by the record's word); every
  write survives a restart.

### M8e — The sealed mind aboard: doc 23 S.4a (medium; on M8a, M8b, M8c)

Recorded in both ledgers (here and doc 23 §14). Pending the owner's Q-M8-1. This also closes 17g
§4's "IC claims … in live play" deferral for the claim half (cover stays test-composed).

- The claim book accepts `SortieId::Ic` at all five sites (`state.rs:36-109`,
  `execution.rs:106-108` and `validate` at :140-149, `records.rs:193-199`, `audit.rs:63`): the
  host's id carried through, the ordinal kept for the transition order.
- A production `HostTransition` constructor behind the surface finish. `epoch_move` runs in the
  finish's prepare with no `World` (`black_box.rs:705-712`), and a tick submits nothing
  authoritative, so the Open rides the finish's bundle the way the black box does: a second
  `OwnerParticipant` beside `BlackBoxParticipant` whose prepare mints the `StartPermit` for a rig
  whose `TacticalUnit` carries a core and whose operator holds a dispatch holding for a filed mind
  (personal, or the org's through M8c), and whose apply runs `IcHost::enqueue_start`; the Close the
  same at `Recover`/`Home`. The fixtures `fixture_start`/`fixture_close` in `ic_integration_tests`
  are replaced by a production-host twin of its stories. `with_ic_minds(Vec<ConfiguredIcBase>, …)`
  joins the launch composition behind `[tacsim.minds].ic_minds` (default on when a launch path is
  set), the launch supplying each site's bay as a configured IC base.
- Admission against the site's pool at the launch: **recorded, never refused** (D-23-8, Q-23-4,
  accepted 2026-10-27: the allotment is recorded until a mind thinks, and this mind is dormant);
  the line's `admitted` carries the count as S.1 built it. A seat is taken at the seal and freed
  at the close (D-UM-39).
- The seal line names the mind, the core, the class, the governed bit and M8a's licence word (the
  surface dispatch now resolving the mind licence's state through M8a's function); M8c's seal
  mark written in the same bundle; the `abi` drop on resume (`execution.rs:189-243`'s append) wired
  to the excursion's reboot path with the sealed record keeping the old ABI.
- The rig's excursion runs as today: the mind is sealed and dormant (no station). No fill plan
  changes; the cost lane re-run with the two added writes per launch (the claim, the mark) and
  re-blessed. Long-leg snapshots (doc 23 S.4's last item) are not built: a dormant mind has no
  state beyond its seal; they land with the thinking mind (§4).
- Gate: tier 2 of doc 23 §10 as restated by Q-M8-1 — an excursion by a cored rig with a held mind
  opens with a real four-owner Open and closes with a real Close across a restart; a lapsed licence
  seals Doctrine and says so only to the operator; a pending transfer refuses the seal by name;
  the pool count is recorded past the allotment and nothing is refused; the ungoverned rig's seal
  (M8b) carries the draft and the word; the seal mark reaches the roll family.

### M8f — Procurement: the NPC org buys from the Roll, with the diversity cap (large; on M8c)

- **The rating score.** The certificate holds no win count (`ClassOutcome` is the ops budget), so
  the score reads the result pages `RatingResultRef` names: `RatingScore = wins × 1000 / cases`
  over `CaseResult.terminal` for the revision's lowest passing class, computed at ranking time and
  cached per `(revision, rating id)` in the tick's own bounded table, never stored on the Roll;
  `score_per_credit = score × 1000 / price_minor`.
- **The procurement policy.** `ProcurementPolicy { org, scope, budget_minor_per_ic_month, standing:
  Option<faction>, cap_permille (default 400), roles }` on the deployment beside `NamedHouses`; a
  tacsim tick `tacsim.procurement` (the recall notice drain's shape; the port houses' bid loop's
  budget discipline) that, once per IC month per policy, reads the player market's listings
  (`ListingsPage`, `mush-market/src/mind_commands.rs:485`), filters to sealed/open/open-resale
  lines the org's standing admits (membership is per character, `mush-faction/src/data.rs:572`, so
  the org's standing is its backing faction's, §5 question 4), ranks by score per credit, and buys
  through the typed buyer path as the org holder (M8c's acting character with the treasury as
  payer; the tick is GOD and submits nothing authoritative itself, so the buy is a queued
  submission as the acting character, the Occator dispatcher's shape) until the budget or the cap
  is reached — the cap counted as **hulls per charter** (D-UM-54) over the org's titled hulls, so
  an org of ten hulls may hold a charter for at most four of them.
- **Only listed lines.** 17f's fixture made real: a filed-but-unlisted mind (D-UM-56) is never
  bought; the test seeds one beside a listed one and asserts the purchase.
- **The budget.** The economy's per-account `ApprovalRule.window` is a rolling wall-clock cap and
  cannot count an IC month, so the spend is recorded as `ProcurementLedger` rows (the E0 ledger)
  and the budget is a cap on the month's rows, never a mint; a treasury that cannot pay skips the
  month (the ore-bid precedent). The approval window stays as the account's own safety.
- Gate: the zero-PC story's first half — a world with no player character, a House with a budget
  and ten titled hulls, three listed lines and one unlisted; after one IC month the House holds
  licences for the best two per credit within its budget, for at most four hulls of the best, the
  unlisted line untouched, every purchase a balanced split with the levy; a second month at a
  spent budget buys nothing; a restart between months changes nothing. Runs on the M7 story
  harness, so this story waits on C4's green (`player_market_commands_story` is red at
  `7c65c59f`, dispatch 40667921671).

### M8g — The claim, the leak tests and the gate (medium; on all)

- **The quirk claim.** `@mind/claim <hull> [<hull>…] <text>`: an author (a charter title holder)
  files testimony naming hulls their own sensors saw (the hulls must appear in the author's own
  earned contact history — the knowledge rows of doc 20 §8 or the arena's `Detected` for their
  pilot; the engine refuses a hull the author never earned, with the observer-rule sentence);
  `minds.claimed` (scoped to the Registry) and a `ClaimRecord` the Registry operators read with
  `@registry/claims`; the reply is a receipt and confirms nothing (D-UM-55). An inspection or a
  box read by the Registry is what establishes the fact (M8a).
- **The leak tests.** In `sim_lenses/observer_rule.rs`'s pattern: an ungoverned core on a rig
  (M8b) sealing an unfiled draft (M8e) is learned by nobody — the bystander, the other operator,
  a pilot, staff's non-Registry reads, the web — and by the four built channels only: the owner's
  box read, the Registry's inspection, the attestation refusal, the claim's inspection. Testimony
  is named as owed (Q-M8-2).
- **The gate stories, dispatch-only on CI** (doc 23 S.5 folded here): tier 1 (the existing
  shard tests promoted to one story), tier 2 (M8e's), the zero-PC story's two halves (M8f's
  purchase, then the House's rig sealing the bought mind on its next excursion through M8e, the
  black box line naming the mind and the org principal), the leak battery.
- **The record.** Doc 17 §22's M8 and "After M8" amended (the split, the deferred channel), §24's
  as-built entry; 17b's M8 row; doc 23 §9 S.4/S.5 and §10 restated per Q-M8-1; 17f's M8 sentences
  closed; this note's §7.

## 4. Deferred: each needs a recorded ruling

- **Testimony, the fifth channel.** With the fight (16c H1). The arena-side half (a `Hit`
  carrying whether the victim had classified the shooter) is offered inside M8g as optional.
- **The drone phase proper (S.4b).** The mining station under D-UM-47 and D-UM-48, after M8 as doc
  17 §22 lists it; doc 20 §17's fields and bays with it; doc 23 S.4's long-leg snapshots on the
  surface tick with it (a dormant mind has nothing to snapshot beyond its seal).
- **A cored ship.** M8b cores the TV only; the dray stays Class 0 until a cut wants a ship's core.
- **The Tether's back room as a vendor.** Who sells heirloom and grey dies, at what price, under
  what cover: a lore and market design (doc 11 §4.6). M8b seeds them by staff.
- **Hull cover in production.** 17g §4's "IC claims and hull cover in live play": M8d lands the
  exclusion rule in the test composition; a live Tycho waits on the region host.
- **Seizure of hulls.** M8d seizes cores. A hull titled to a principal under revocation keeps its
  title; doc 20 §17's D-20-7 lifting and salvage are the fight's.
- **The region sim task** (doc 23 D-23-6's simulation half) and the `MIND_ABI` upgrade window for
  open IC seals.
- **`Legality` on tacsim content** (Class III as `Restricted`, the `core3_end_user` entitlement
  seeded in content rather than tests): small, and not a channel; a follow-up of M8b if the owner
  wants the Class III scarcity live.
- **17g §4's other follow-ups** (Registry pacing, the shared instrument lifecycle) stand.

## 5. Owner decisions

**Ruled by the owner, 2026-10-28:** Q-M8-1 and Q-M8-2 accepted at their defaults; Q-M8-3 to
Q-M8-7 taken at their defaults. The M8a and M8c briefs were written and the cut loop started the
same day (§7).

**Two more owner rulings, 2026-10-28, raised by the M8a and M8c reviews (both at the lead's
defaults, "for now"):** (a) a charter tenant (the Dray-P's pilot, who holds neither the hull's
title nor its lease) does not read the hull's black box with `+hull/box`; the readers are the
title holder, the current lessee (from their own lease's start only) and staff. (b) The licence
seal-mark ring keeps 64 marks per licence, evicting the oldest **governed** mark first, so an
ungoverned flight inside a term is never washed out by later governed ones; an ungoverned mark is
evicted only when the whole ring is ungoverned.

1. **Q-M8-1, the tier-2 split.** S.4a (the sealed, dormant mind aboard a rig) built as M8e now;
   the thinking mind (the drone phase) after M8; doc 23 §9's "lands inside D-UM-47's cut, not
   before" amended to say so; the claim half of 17g §4's "IC claims and hull cover in live play"
   closed by it. **Default: yes.** The alternative is the drone phase first and M8 after it.
2. **Q-M8-2, testimony deferred to the fight.** M8 ships four channels and names the fifth owed;
   doc 17 §22's gate sentence reads "the four built channels" until H1. **Default: yes.**
3. **Q-M8-3, the verb spellings.** `+hull/box` (owner read), `@registry/inspect|seize|forfeit|
   fine|claims`, `@registry/revoke principal=`, `@mind/claim`. §3 C2's rule of 17g (`@mind/*`
   author, `+minds/*` holders, `@registry/*` Registry) is kept; `+hull/*` is new, for a hull's
   owner or operator. **Default: as proposed.**
4. **Q-M8-4, the org as a holder, its standing, and the Registry's own principal.** The typed
   `Holder` across the commerce families with a scoped `roll.holder.org` grant (M8c), the org's
   faction standing read as its backing faction's membership for procurement's filter, and the
   Registry **org** (`RegistryAuthority.organization`) rather than the `registry_issuer` system
   principal as the claimant and the forfeited core's owner. **Default: as proposed**, the standing
   read the lead's call.
5. **Q-M8-5, the Red Dies front.** Which board (default: the Republic's, the grey dies being
   Martian), the clock's length (default 6) and what its end does (default: nothing but the canon
   line, until the owner names a consequence).
6. **Q-M8-6, the numbers (provisional, per the owner's standing word on economy numbers).**
   `seizure_hold_ic_days` 3; `revocation_ic_days` 90; `fine_cap_minor` 10× the filing fee; the
   procurement budget per House 2,000cr per IC month; the cap 400‰; the pool allotment per site
   as Q-23-4 records it. None holds a build; each is a config key.
7. **Q-M8-7, C4 and the stories.** M8f's and M8g's stories build on the M7 story harness, which is
   red on the market-commands story at `main`'s tip. M8a–M8e do not wait on C4; M8f's story does.
   **Default:** build M8a–M8c now, M8d/M8e as they unblock, and land M8f's story once C4 is green.

## 6. Risks

- **The managed-title path for seizure** is the one new economy primitive with its own proof
  obligation; it is why M8d is medium-large and not medium. If the proof turns out to need the
  "private closed plan" machinery the comment names, M8d splits and seizure lands second.
- **Org holders touch every commerce family.** M8c's typed `Holder` runs through house purchase,
  entitlement, paid, dependent-owner and holding readers and every story that reads one. The
  serde tagging must keep a character holder's bytes identical or every pinned digest moves; the
  cut's tests must include every reader, or a later story finds the gap (17g's C2b lesson).
- **The seal mark is a second cross-crate write per launch.** It is bounded and tiny, and it is
  what lets mush-roll's attestation stay synchronous; if the cost lane objects, the mark can be
  folded into the black-box participant's own chunk and read by tacsim on roll's behalf — a
  trait seam instead of a push, and a later cut's problem.
- **Three new primitives.** Seizure's managed-title path, the core removal order and the one-shot
  sanction obligation are new economy and industry machinery, not wiring; they are why M8d is
  medium-large. Any one of them turning out to need its own design conversation splits M8d.
- **Doc 23's pinned sentence** "lands inside D-UM-47's cut, not before" is moved by Q-M8-1. If
  the owner keeps it, M8e and M8g wait on the drone phase and M8a–M8d, M8f proceed.
- **The story harness is red.** M8f's story cannot be proven until C4 lands; building M8f's code
  before its story is a known deviation from the cut loop and is named as such in its brief.
- **Content digest pins** move in M8b (the TV's core section, the recipes, the lots); each
  re-blessed with its reason, never silently. M8d's front is code and moves none.

## 7. Ledger

Nothing landed. The first entries are M8a and M8c, each on its own branch (`claude/m8-<cut>`) off
`main`, each a PR; M8b follows M8a.
