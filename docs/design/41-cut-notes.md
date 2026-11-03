# 41. Cut notes — restoration and the salvage loop

One `## RS.n` heading per cut of [41-restoration-and-the-salvage-loop.md](41-restoration-and-the-salvage-loop.md)
§6, in the order they landed: what was built, the story that proves it, where the PR departed from the row,
and the cell's cost (doc 43 rule 2.3 #14).

## RS.3 — the claim's close (2026-11-03)

Rulings: Q-41-20..23, Q-41-28, Q-38-19, Q-S13, Q-S14; round-2 items 41-1, 41-8, 41-10, 41-12a (§4b).

**Built.**

- **`claim_window_s` from config.** `TacsimRecoveryCfg.claim_window_s` (`mush-config`, default 172,800 real
  seconds) is mapped by `mush-server/src/config_ext.rs` and refused at boot outside one second .. a week; the
  stories keep setting `TacsimRecoveryConfig` directly.
- **Who answers for a title** (`surface/claim.rs` `answerers_of`). A character's title is theirs; an
  organization's is its live rig lessee first, then its active members and its live grantees, unfiltered by
  capability (Q-41-28: Occator's only grantee is its lessor agent, an Npc). The pin carries the list
  (`TenurePin.answerers`, written only for an org title, so a character's pin prints as before) and
  `answering` is its first. Filing tells everyone who answered at the attach and everyone who answers now,
  then the landholder; the notice is mailed one letter a recipient, so a post that refuses an agent costs no
  one else theirs.
- **The narrowed award.** `mush_economy::award_salvage_title` refuses an organization's hull only when the
  economy sees a lease or charter on it (an `assignment`, a `Lease` or `Collateral` encumbrance); tacsim's
  tick refuses an org hull under a live rig lease or a programme charter (`leased_or_chartered`). An org hull
  with neither is awarded (story: Occator's unleased spare goes to its salvager).
- **`+salvage/authorize`** (Q-41-21). `<hull>=<who> [until=<UTC seconds>] [go]` by whoever answers for the
  title submits Org's own `unitgrant_issue` for `unit.recover_load` on the hull (`runtime::submit_from`, the
  personal-title parent). `<site>/<block>=<who> [until=<IC seconds>] [go]` by the block's live claim holder
  writes a `LandGrant` on the site's record through the tow action's `claim` op (`ClaimOrder::AuthorizeBlock`,
  keyed by the block's place), bound to the claim's holder and registration instant so a new holding voids it.
  Both are read at the attach into the pin (`owner_grant`: the grant id and its grantor, found by Org's
  `Authorize` and `List` reads as the recoverer; `landholder_authorized`), and `open_claim` feeds them to
  `evaluate_recovery_entitlement`. The load's unlawful-recovery log honours either.
- **The contest kinds** (Q-41-23). `+salvage/contest <hull>[=recovery|procedure|right] [go]`; the bare form
  keeps the automatic choice (any answerer may contest on bare title). `procedure` is for whoever answered for
  the title and rests on `procedure_defect`: an answerer (or the landholder) not told, a pin missing or another
  tug's, the box's last `recovered_to_base` line naming another tug than `custody.recovered_by` or another base
  than `custody.carrier` — never the claimant; an empty box is no ground. `right` rests on `right_of_grantor`:
  the pin's `owner_grant`, filed only by its grantor and only when the recoverer is the claimant, carried as
  `SuperiorSpecificRight { contract_id: "grant/<id>", obligor: claimant }`. The prepare re-checks each ground
  against the record (the box defect is the command's reading of the box, the registry's). Notices say why the
  claim was vacated.
- **Repairs at risk** (Q-41-20, Q-S14). The `+bay` quote under a Salvor right prints
  "At your risk: the claim on her is open, and a failed claim repays nothing." first. The claims' tick turns a
  salvor's completed order (Done, `Right::Salvor`, the custodian's, closed since the recovery) into an
  `AtRiskRepairEntry` (job lines; fee plus goods at the goods table's reference; the close instant) and writes
  it as the custodian (`ClaimOrder::AtRisk`): onto the filed claim, or onto `SalvageCustody.at_risk`, which the
  filing carries into the claim. The award and vacate notices list the entries; nothing is repaid.
- **Hot parts** (§4.6, item 41-12a). `BayHull.claim_lots` lists a lot `+bay/strip` mints while the actor holds a
  filed custody on the hull (dormant: a salvor never strips on the ruled table). On a vacate the tick marks the
  claim's lots (`strip::claim_vacated`) and flags them one act at a time (`strip::flag_next_vacated` →
  `strip::vacate_claim`, which drops the lot from the list in its own write; a sweep over
  `TacsimStore::bay_vacated_lot_hulls` resumes the list each tick). The mush-tactics
  `mark_stripped_component_contraband_on_vacated_claim` is deleted.

**Story.** `crates/tacsim/tests/shard_2/salvage_close.rs`, `rs3_story_b_the_claims_close_on_urvara_reach` (512)
and `…_on_ahuna_flank` (256), one body: Brask Freight's hull `a` (T its grantee) off L's block without
authority — T's `procedure` finds no defect, T's `right` no right, T's `recovery` vacates; T's own hull `c`,
recovered under T's `+salvage/authorize` and L's block delegation (both on the pin) — lawful, L's `right` is no
ground, T's `right` vacates; Occator's spare `b` — the lessor agent is told, contests on bare title, the claim is
awarded to P. The at-risk line in the quote, the written entry and the vacate notice:
`shard_1/bay_36a.rs` `b36a_rs3_the_salvors_repair_is_at_risk_and_reported` (the bay works only docked at a
workshop port, so it rides 36a's salvor-at-Ceres fixture). Unit tests: `surface::claim::tests::rs3_*` (the
procedure reading, the right, the at-risk list, the land grant). Re-run unchanged and green: `field_gc5`,
`field_gc6`, `bay_gc8`, `review_36a`, `bay_36a`.

**Divergences.**

- The landholder's delegation lives on the site's record (`SiteRecord.land_grants`), not inside the block's
  `BlockClaim`: the tenure family is written by industry's tenure bundle (`port/claims.rs` `stage`), and the
  site record is the claim's own action's. It is bound to the claim instance the same way.
- The at-risk entry is written by the claims' tick from the completed order, not inside `complete`'s write: the
  bay record and the site record are written by two actions. The tick writes it before anything settles, once.
- `procedure`'s "the filing registry against the site's" has nothing to compare: every lane site files at the
  one lane registry.
- `right`'s retrieval-contract source is RS.5's (no retrieval contract exists yet); RS.3 builds the authorization
  source. The hull form of `+salvage/authorize` issues on a personal title only: an org hull's grant needs a
  delegable org grant parent, which `@org/grant` cannot make; the story's `right` variant uses T's own hull.
- Story (b)'s vacated hull's return, the port's berth "held for T", the fine and the quarantine are RS.4 and
  RS.7; the freeze-paused window stays proven by `field_gc6` at 512.
- Story parties: a second salvager P takes `b`, since O's Dray-P bay holds two casualties.

**Cell cost (rule #14).** shard_2 gains the two story tests (local Windows debug: 32.4 s at 512, 29.7 s at 256);
shard_1 gains `b36a_rs3_…` (bay_36a's 14 tests ran in 19.9 s). The PR run's JUnit deltas are recorded on the
PR.
