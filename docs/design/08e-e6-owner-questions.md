# E6 — Shadowrun run-and-pay: owner questions before planning

**Status:** questions only (2026-09-03), grounded in two seam surveys of the tree at `78d25f7`
(main + K merged). Each question carries the default the plan will take if unanswered. The plan
(`08e`, later) records the answers verbatim as binding, the way 08d §10 does for E5.

Design: doc 08 §9.4 (catalogs, quotes, provenance), §11.2 (the Shadowrun slice), §14.2 (no
universal market), §18-E6. Builds on E4's contracts, holds, `After`, and cadence clocks (08c) and on
E3's title/vault/custody. **E6 cannot start before E4b2 ships.**

## What the tree says today

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
- Neither shipped ruleset creates a world `Thing` for gear; E3 titles arbitrary `ObjId`s, so gear
  objects would be ordinary things.
- `SceneRecord` has **no outcome/objective field**; `scene.concluded` canon carries participants,
  rounds, anchor, `source: "scene:{uid}"` — not success/failure (`mush-scenes/src/lib.rs:301-329`).
  `SceneUid` is monotonic and reboot-stable.
- The knowledge model doc 08 §9.4 leans on ("a fence evaluates provenance through the buyer's
  available knowledge") is **unbuilt**: `Claim`/`Belief`/`Evidence` (`docs/scenes_factions_knowledge.md`
  §2) exist only in design; shipped `LoreEntry`/`IntelRecord` are free-text and person-keyed.
- Journal-derived provenance **does** exist: `AcquisitionRecord { transaction, at }`, the
  `AssetEffect` title chain, `CustodyEffect`s (transaction-level actor only), `Encumbrance::Disputed`,
  and canon scopes `Public|Restricted|Secret`.
- Orgs are staff-minted (08a decision 1); a team stash is an org vault; `@econ/pay` is arity one by
  design — two-party trades are E4 contracts.

## Questions

### A. Money model

1. **Personal mode.** Shadowrun is a cash game. Default: `personal_mode = exact`; instrument
   `sr.nuyen` at **scale 0** (SR5 has no minor units); the sheet's `nuyen` field becomes a *view* of
   the account (derived at render, never written); `system:sr.chargen` mints starting funds.
   Confirm code, scale, and the sheet field becoming derived.
2. **Karma stays a rating**, sheet-side, with its job-approval spend flow — never money. Confirm.
3. **Lifestyle as an E4 obligation.** SR5 monthly costs (Street 0, Squatter 500, Low 2,000, Middle
   5,000, High 10,000, Luxury 100,000¥) in a config table; one obligation per character on a
   cadence clock `sr.month`, paid to `system:sr.lifestyle` (or a landlord org if the character
   rents an E5 property). Failure → E4 Grace → Delinquent → **programmatic downgrade one tier**
   after `N` delinquent periods with a notice (decision 25), never eviction or teleport. Default:
   lifestyle is obligation + enum in E6; a physical residence via E5 housing is optional and
   separate. Confirm the downgrade rule and `N` (default 2).

### B. Runs

4. **Who offers a run.** Default: an ST offers it through a Johnson NPC (`PrincipalId::Object(npc)`)
   fronting for an org (corp, fixer network) whose account funds escrow; `+run/offer` (ST) drafts an
   E4 contract with the runners as parties; runners `+run/accept` (each accept places nothing —
   the Johnson's consideration is the escrow; runners have no consideration unless the terms say
   so). Players cannot self-create run contracts. Confirm.
5. **The team as a principal.** Default: an ad hoc team is **not** an org — the contract's split
   pays individual accounts; a persistent crew that wants a shared stash gets a staff-minted org
   (decision 1). Alternative: a capability-gated "form crew" that mints a small org. Which?
6. **The run outcome fact.** Default: settlement rests on an **ST attestation** (`@contract/attest`).
   Optional addition: a scene outcome field (`+scene/outcome success|partial|failed`, on
   `mush-scenes`) that commits canon with `source: scene:{uid}` and the outcome in `detail`, which
   E4c's source-specific attestation can match. In scope for E6, or attestation-only?
7. **Split and expenses.** Default: the split is declared in the terms at offer (equal permille
   unless the Johnson specifies), remainder to the fixer; **expenses** are a capped line in the
   terms, reimbursed from escrow before the split against attested receipts (a runner's own
   payments with `cause` referencing the run). Confirm.
8. **Partial success and failure.** Default: the terms carry a three-outcome payout table
   (full / partial / fail → forfeit to the Johnson's account); the ST attests which outcome
   occurred; terms are frozen at offer, so no renegotiation without a new contract. Confirm.

### C. Gear and purchases

9. **Gear as objects.** Default: items above a catalog threshold (default 5,000¥) become titled
   `Thing`s (weapons, armor, drones, vehicles, commlinks); minor gear stays sheet text; cyberware
   is never an object (it is essence on the sheet). Where is the line?
10. **The catalog.** Default: a TOML data file in `ruleset-shadowrun` (the weather-climate pattern),
    loaded at boot: `code, name, category, baseline_nuyen, availability, legality (legal | restricted
    | forbidden)`; `CatalogRef { ruleset: "shadowrun", code }` set on titled objects. Who authors
    and maintains the initial list?
11. **Buying.** Default: from an NPC vendor/fixer that owns an org account and a storefront vault;
    `+buy <code> from <vendor>` yields a deterministic **quote** (baseline × availability/legality/
    relationship modifiers, expiry 1 h real time); accepting the quote creates an E4 contract that
    settles at once (payment, title transfer, custody withdraw from the vendor's vault). A quote is
    a short-lived contract offer. Confirm, and whether availability rolls (SR5 Negotiation tests)
    are deterministic rolls through `mush-dice` or ST-attested.
12. **Cyberware and clinics.** Default: **E6b** — a clinic is a service contract whose settlement
    files the sheet change through the karma-style job approval; E6a ships gear only. Confirm.

### D. Fences and heat (the half that waits for the knowledge model)

13. **Fence pricing without beliefs.** Default for E6a: a fence offers a deterministic fraction of
    baseline (30–50% by legality) modified by **journal-derived** provenance the fence may
    legitimately see at its canon scope: a `Disputed` encumbrance, an `asset.stolen` canon event,
    a title chain with no contract sale behind it. Belief-based pricing is E6b after the knowledge
    model exists. Confirm the split.
14. **Reporting theft.** Default: the title owner may `+report stolen <obj>` → `Encumbrance::Disputed`
    on their own asset (invariant 19 relaxed for the title owner) + a `Secret`-scope canon event;
    a fence that can see it refuses or discounts; buying a disputed item produces evidence against
    the buyer. Notoriety / Street Cred / Public Awareness as sheet scalars are **absent today** —
    E6b ruleset work. Confirm.
15. **Delayed settlement.** Default: fenced sales settle via E4 `After { deposit, delay }` with a
    per-fence real-time delay (24–72 h) and the goods custody-locked in the fence's vault meanwhile.
    Confirm the clock (real time) and range.

### E. Config, chargen, migration

16. **`[shadowrun.economy]`** mirroring WoD's: `personal_mode = "exact"`, `lifestyle = true`,
    `cadence = "sr.month"`, `catalog = "<path>"`, fence parameters — parsed host-side and passed
    into `ShadowrunRuleset` (no config plumbing exists for SR today). Confirm shape.
17. **Chargen.** Default: approval mints the priority-table nuyen from `system:sr.chargen`; a new
    shopping stage buys catalog gear within the budget (major items → titled objects); SR5's 5,000¥
    carry-over cap is enforced only once shopping exists. Confirm.
18. **Existing characters.** Default: staff-triggered `@sr/economy migrate` mints each sheet's
    `nuyen` into the new account from `system:sr.chargen` with attribution, then the field derives
    from the account. Confirm.

### F. Boundaries and sequencing

19. **Two halves.** E6a (runs, lifestyle, catalog, vendor quotes, team vaults) after E4b2; E6b
    (fence beliefs, heat, clinics, notoriety) after the knowledge model. Confirm.
20. **A neutral market crate.** Default: quotes, catalogs, vendors, and the storefront-vault flow
    live in a game-neutral `mush-market` crate (doc 08 §14.2: markets are ruleset modules over
    quotes, holds, contracts, title, custody, routes); `ruleset-shadowrun` supplies catalog data and
    pricing policy; E5's "financing" and E7's requisitions reuse it. Confirm.
