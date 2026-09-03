# E6 — Shadowrun run-and-pay: owner questions and answers

**Status:** questions asked and **answered 2026-09-03**. The answers in §3 are binding design
constraints for the E6 plan (`08f`, to be drafted after E4b2 ships) the way 08d §10's are for E5;
they override doc 08 §11.2 where the two differ. Grounded in two seam surveys of the tree at
`78d25f7` (main + K merged). §4 lists what the answers change upstream in E4 (08c).

Design: doc 08 §9.4 (catalogs, quotes, provenance), §11.2 (the Shadowrun slice), §14.2 (no
universal market), §18-E6. Builds on E4's contracts, holds, `After`, and cadence clocks (08c) and on
E3's title/vault/custody.

## 1. What the tree says today

- `ruleset-shadowrun` has **no** economy integration: `nuyen` is a sheet number (0..50,000,000,
  `schema.rs:298`), `lifestyle` a cosmetic enum (`street|squatter|low|middle|high|luxury`,
  `data.rs:225-232`) with no cost or upkeep, `gear`/`weapons`/`armor` are free-text sheet entries
  ("a Map of owned items, not a costed catalogue", `chargen.rs:345-348`). The only price literals are
  the chargen priority table `resource_nuyen: [450_000, 275_000, 140_000, 50_000, 6_000]`
  (`data.rs:1164-1177`). Karma spends go through a job approval (`+karma/spend`); nuyen has no such
  flow. The only NPC ability is `sr_roll`. No `[shadowrun]` config block exists; `ShadowrunRuleset::new()`
  uses defaults (`mush-server/src/modules.rs:177-219`).
- **Nothing** in the workspace is a shop, vendor, quote, catalog, price, listing, fence, heat, or
  notoriety (repo-wide grep, homonyms excluded). `CatalogRef` exists on `EconomicAsset` but nothing
  ever sets it (`commands.rs:1436` passes `None`).
- Neither shipped ruleset creates a world `Thing` for gear; E3 titles arbitrary `ObjId`s.
- `SceneRecord` has **no outcome/objective field**; `scene.concluded` canon carries participants,
  rounds, anchor, `source: "scene:{uid}"` — not success/failure (`mush-scenes/src/lib.rs:301-329`).
  `SceneUid` is monotonic and reboot-stable.
- The knowledge model doc 08 §9.4 leans on ("a fence evaluates provenance through the buyer's
  available knowledge") is **unbuilt**: `Claim`/`Belief`/`Evidence` (`docs/scenes_factions_knowledge.md`
  §2) exist only in design; shipped `LoreEntry`/`IntelRecord` are free-text and person-keyed.
- Journal-derived provenance **does** exist: `AcquisitionRecord { transaction, at }`, the
  `AssetEffect` title chain, `CustodyEffect`s (transaction-level actor only), `Encumbrance::Disputed`,
  and canon scopes `Public|Restricted|Secret`.
- Orgs are staff-minted (08a decision 1) — which does **not** stop members of a staff-minted player
  corporation from publishing runs under properly scoped authority; `@econ/pay` is arity one by
  design — two-party trades are E4 contracts.

## 2. The questions as asked (2026-09-03)

Twenty questions with defaults were put to the owner; the owner's answers below supersede the
defaults and add four decisions the questions missed (21–24). The original question text is kept in
the git history of this file (`bbddca6`).

## 3. The owner's answers — binding

> "This one should be much more materially economic than WoD. Exact balances, purchases, services,
> escrow, and player employers all fit Shadowrun's gameplay loop."

1. **Exact personal money.** Register `sr.nuyen` at scale 0. After chargen the economy account is
   authoritative and the sheet renders only a derived balance; during chargen `nuyen` remains a
   temporary resource budget until approval. `system:sr.chargen` funds starting characters.
2. **Karma stays separate** — progression currency with its existing approval flow, never
   transferable economic money.
3. **Lifestyle is an automatic E4 obligation on an explicit `sr.month` campaign clock** — not
   unrestricted real time or generic IC time. Autopay is the default. One failed period → notice /
   grace; a second unpaid campaign period may downgrade lifestyle. Server downtime or a large clock
   jump must never cause multiple surprise downgrades. Physical housing remains separate.
4. **Players may create runs.** A personal employer may escrow their own funds; an organization
   member may create one using an account and scope they are authorized to spend from, including
   ordinary approval thresholds; staff, player STs, and campaign systems may also create runs.
   Nobody may create a funded run from an uncontrolled faucet. **A Johnson is the visible
   representative, not necessarily the payer or author**; the contract records the actual
   accountable principal and reveals it only per visibility policy.
5. **The run team is an ephemeral party, not an organization or economic principal** — a durable,
   lightweight party record that survives disconnects and restart but expires when the run settles
   or is abandoned. Members join, leave, or swap while assembling; **acceptance freezes a roster
   snapshot and payout shares**; later substitutions require a contract amendment and affected-party
   consent. Payments settle directly to individual accounts. Persistent crews and corporations
   remain staff-approved organizations with accounts and vaults.
6. **A generic typed scene-outcome / attestation seam is in scope.** A run may settle from a sealed
   scene outcome, an authorized player/ST attestation, a deterministic mechanical condition, or a
   composite. **The offer names its adjudicator and permitted evidence sources before acceptance**;
   runners consent to that authority when accepting. An LLM may narrate, facilitate, and propose an
   outcome, but prose alone cannot release escrow — deterministic recorded facts or an authorized
   human attestation must.
7. **Freeze the split when the roster locks**, not necessarily at first offer: the offer defines
   the split method or slot shares; once the party accepts, exact recipient shares freeze and must
   sum correctly. **No unspecified remainder to the fixer.** Model separate frozen terms for runner
   payout, fixer/arranger fee, expense advance, reimbursable expense reserve, and bonuses/penalties.
   Prefer a fixed expense advance first; receipt-based reimbursement once attestations are mature.
8. **A bounded, data-driven outcome table in frozen terms.** Ship `failure`, `partial`, `success`
   as the default three but never hard-code the list. Each outcome names exact payouts, expense
   treatment, and any asset consequences; exceptional bonuses are separate conditions.
9. **Representation is catalog policy, not a price line.** Individually important, serialized,
   restricted, upgradeable, or stealable gear becomes an economic asset/object; ammunition, drugs,
   reagents, and commodities become **bounded inventory stacks**; licenses and programs may be
   **entitlements**; cyberware becomes an **installed augmentation record** after installation,
   retaining acquisition and clinic provenance. Price may influence the default; category and
   gameplay significance decide. Otherwise a shop charges for what a player could type onto the
   sheet for free.
10. **A small, representative, source-controlled initial catalog** proving weapons, armor, drugs,
    programs, a deck, magical goods, services, and cyberware — never an exhaustive transcription.
    Full catalogs are versioned content packs from deployments or licensed sources. Entries carry
    stable IDs and definition versions, baseline price, availability, legality, representation
    kind, stackability, tags, and rules references. Vendors produce offers from the catalog; they
    never mutate canonical entries.
11. **Vendor accounts, quotes, and contracts — but not every vendor needs pre-existing stock.** A
    vendor may sell an item held in its storefront vault, a commodity supplied by a registered
    market source, or a delayed procurement/delivery contract. **Quote expiry declares its clock**:
    ~1 h for an active in-person session, ~24 h for asynchronous/remote offers. Stocked legal items
    need no availability test; scarce or restricted goods use an **engine-resolved, recorded
    Shadowrun test**; ST attestation is for exceptional/authored availability only.
12. **Cyberware and clinics are a later slice, and payment alone never edits the sheet.** The
    service contract purchases and schedules the procedure; completion invokes a ruleset-owned
    installation action that revalidates Essence, prerequisites, asset custody, clinic capability,
    payment, and sheet revision atomically. Routine legal installation is deterministically
    validatable (no mandatory staff homework); configured restricted procedures may require ST
    approval. Buying a spell formula or a teacher's service never bypasses the Karma cost.
13. **Deterministic first-pass fence pricing with strict visibility**: a fence prices only
    provenance it may observe or the seller presents; hidden journal information must not leak
    through an unusually low quote. **Title ≠ possession**: a thief generally cannot transfer valid
    title; a fence contract may pay for custody/possession while the original title and theft claim
    stay attached. Laundering title is a later, explicit process — never an accidental result of
    payment.
14. **A typed `report_stolen` action** replaces generic "mark disputed": the current title owner, an
    authorized insurer, or an appropriate authority may file it with public, restricted, or covert
    visibility. Later custody transfers and purchases generate evidence; they do not establish
    guilt. Heat and notoriety remain later features.
15. **Configurable delayed fence settlement**: 24–72 h real time is acceptable when shown in the
    offer; the fence must **reserve or escrow the payout before taking custody**; offer "immediate
    sale for less" vs "delayed consignment for more". Notices only on custody, settlement,
    cancellation, or dispute state changes.
16. **`[shadowrun.economy]`**, owned by the Shadowrun configuration, at least:
    ```toml
    [shadowrun.economy]
    enabled = true
    instrument = "sr.nuyen"
    lifestyle_clock = "sr.month"
    player_run_offers = true
    organization_run_offers = true
    chargen_shopping = true
    catalog_paths = []
    ```
    plus quote expiry, active-offer caps, fence delay, and lifestyle costs.
17. **Chargen shopping and the 5,000¥ carry-over cap — with one live cash authority.** The
    unapproved sheet holds a temporary budget and purchase plan; approval atomically creates the
    account, mints the priority allotment from `system:sr.chargen`, pays chargen vendors for
    selected goods, creates inventory/titles/installed records, and leaves no more than the
    configured carry-over. Until shopping exists, keep current behavior and do not apply the cap.
18. **Existing-character migration is idempotent and inspectable**, funded by `system:sr.migration`
    (not the chargen faucet): a staff dry run shows each planned mint; the committed migration binds
    the old sheet value and revision, mints once, marks the migration complete, and stops treating
    the stored sheet number as authoritative; re-running returns the original result.
19. **Cuts by actual prerequisite**, and **no team vaults** (an ephemeral party owns neither an
    account nor a vault):
    - **E6a after E4b2:** nuyen accounts + migration, lifestyle, catalog and inventory
      representation, quotes, direct vendors, chargen shopping.
    - **E6b after the E4 attestation seam (E4c):** run board, player/org/ST-authored runs, parties,
      frozen rosters and splits, outcome settlement, basic clinics/services.
    - **E6c after knowledge support:** belief-sensitive fences, heat, notoriety, investigations,
      title laundering.
20. **A neutral `mush-market` crate** owning bounded catalogs, listings, quotes, orders, expiry, and
    vendor interfaces. Economy keeps accounts, contracts, title, vaults; rulesets own prices,
    availability tests, legality, item meaning, and content. WoD financing and TacSim requisitions
    reuse it without Shadowrun's retail model.
21. **A lightweight party subsystem**: leader, accepted members, pending invitations, optional
    roles, revision, lifecycle, expiry. Not a principal; cannot own money or title. A run contract
    consumes a frozen party snapshot.
22. **Separate run author, payer, Johnson, facilitator, and adjudicator.** They may coincide, but
    the data model must not assume it — a player-run corporation hires runners while another player
    ST, or an automated campaign, facilitates.
23. **LLMs draft; authorities commit.** An LLM ST may generate run seeds, Johnson dialogue,
    complications, and proposed outcomes; a registered campaign policy may authorize bounded
    recurring offers from a capped system budget. The LLM cannot invent funds, alter frozen terms,
    or settle escrow on narration alone.
24. **Bound the player-created run board**: escrow required at publication, active-offer caps per
    principal, expiry of abandoned listings, and public / faction-visible / invitation-only runs.

## 4. What these answers change upstream (E4, 08c)

Recorded here and mirrored into 08c so E4b2/E4c build the right seams before E6 needs them:

1. **Outcome-indexed frozen terms (answers 7–8).** E4a is building `ContractTerms` with one split.
   E4b2 generalizes: `ContractTerms.outcomes: Vec<OutcomeTerms { name, when: Vec<condition idx>,
   payouts: Vec<PayoutShare>, expense_treatment, asset_consequences }>` (bounded, data-driven;
   `success`/`partial`/`failure` as defaults), each with **exact** shares that must sum, **no
   implicit remainder**; E4a's single split is the degenerate one-outcome case. Separate named term
   slots for payout, fee, expense advance, reserve, bonuses/penalties.
2. **Roster lock and amendment (answers 5, 7).** A contract may be `Offered` with slot shares and
   an unfrozen party; **acceptance by the party freezes the roster snapshot and exact shares** into
   the consent hash; later substitution is an **amendment** that re-hashes the consent surface and
   requires re-acceptance by affected parties only. E4b2.
3. **Named adjudicator and permitted evidence (answer 6).** The consent surface names the
   adjudicator (`AttestorSet`) and the permitted evidence sources (the condition kinds allowed);
   runners consent to that authority at acceptance. E4a's `TypedCondition`/`AttestorSet` already
   carry this; E4b2 makes the offer spell it out and refuses attestation from anyone else.
4. **Separate roles (answer 22).** `EconomicContract` records author, payer (the accountable
   principal whose account escrows), visible representative (Johnson), facilitator, and adjudicator
   as distinct fields with independent visibility. E4b2 record shape; E6b fills them.
5. **A sealed scene outcome as evidence (answer 6).** `mush-scenes` gains a typed outcome record
   committed as canon with `source: scene:{uid}` and the outcome in `detail`, which E4c's
   source-specific attestation matches. Owned by E6b (or E4c if the seam needs a real producer to
   test against — decide at E4c).
6. **`report_stolen` as a typed action (answer 14).** Relaxes E3 invariant 19 for the title owner /
   an authorized insurer / an authority via a dedicated action that places `Encumbrance::Disputed`
   plus a scoped canon event — an economy change; E6a (generic, Shadowrun-motivated).
7. **Fence escrow-before-custody (answer 15)** is an E4 hold placed by the fence's own authority
   before the `DepositInto` condition can be satisfied — expressible with E4b2 as planned; note it
   in the E6 plan's fence flow.
8. **Party subsystem (answer 21)** is new and game-neutral — a small `mush-party` crate (E6b), not an
   economy record.
9. **Inventory stacks and entitlements (answer 9)** are new representations: stacks as bounded
   `(owner, catalog code, count)` records owned by `mush-market` (or `Commodity` instruments —
   decided in the E6 plan), entitlements via doc 08's `InstrumentKind::Entitlement`.
