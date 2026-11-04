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

## RS.0 — the vocabulary (2026-11-03)

Row: §6 RS.0; Q-38-29. PR [#385](https://github.com/conclavio/rustmush/pull/385), base `a2ef4075`.

**Built.** `crates/mush-bot/src/mine/salvage.rs`, mounted as `mine::lane::salvage` beside CR.0's `crew` (the
`#[path]` sibling under `lane`). The captain's (`field::in_vocabulary`) and the escort's
(`escort::in_vocabulary`) closed vocabularies both admit `salvage::in_vocabulary`, each line in closed form:

| Line | Form admitted | Whose verb |
|---|---|---|
| `+tv/<n> pickup`, `+tv/<n> scrap` | `[<object>] [go]`, `<n>` 1–99, `<object>` a decimal object number, no leading zero | doc 36 GC.5, GC.6 (shipped) |
| `+salvage`, `/claim`, `/contest`, `/inspect`, `/report`, `/authorize` | `+salvage/claim <hull> [go]`; `/contest <hull>[=recovery\|procedure\|right] [go]`; `/inspect <hull>`; `/report <hull>=<who> [go]`; `/authorize <hull>=<who> [until=<n>] [go]` and `/authorize <site>/<block>=<who> [until=<n>] [go]` | doc 36, RS.3 (shipped) |
| `+salvage/retrieve`, `/jobs`, `/accept` | `/retrieve <hull>[=<port>] [fee=<cr>] [go]`; `/jobs`; `/accept <job> [go]` | RS.5 (Q-41-9; "the bot learns the three") |
| `+salvage/report … registry` | `/report <site>[=<who>\|#<hull>] registry` | RS.7 (Q-41-24) |
| `+bay`, `/quote`, `/order`, `/fit`, `/strip`, `/scrap`, `/cancel`, `/berth`, `/stow` | `+bay <hull>`; `/order <hull>=<work>[,…]` (the seven kinds or a section or part word); `/fit <hull>=<slot> <lot>` (`lot#` and 16 hex digits); `/strip <hull>=<part>`; `/stow <hull>[=<carrier>]`; the rest `<hull>` | GC.8a/b (shipped); RS.4 the bay-to-bay `/stow` |

A `<hull>` is `#<id>` or a name (letters, digits, spaces, `-`, `'`, `.`, at most 48); a `<who>` `#<id>` or a
plain name; a site, a port, a slot, a section or a part a lowercase word. Nothing beginning `@`, and no line
over 160 bytes or with a line break.

`salvage::answer` reads the server's answers as one of fourteen `Answer`s: the claim's (not in custody,
previewed, filed, filed already), the contest's (previewed on any ground, filed, no ground for any of the five
reasons, window closed), the rig's scrap (previewed, done, no remains, not the title holder) and pickup
(previewed or queued, no crate). `rs0_the_salvage_answers_match_the_game` holds each to tacsim's own
`surface::claim` and `surface::salvage` constants through the development dependency (`lines`' precedent),
and `rs0_the_salvage_lines_are_closed` the forms both ways.

**Taken already, not repeated.** `+field/void` and the four-digit address parser (`lines::is_addr`) are doc
40 FR.0's; `+sortie/condition` is doc 42 CR.0's; `+tv/<n> tow|load|release` are GC.4's and FR.0's. Doc 39's
IN.11 row names `+bay/order replace` as its vocabulary: RS.0's `+bay/order` form admits `replace` with the
other six kinds, so IN.11 reads it rather than adds it.

**Story.** `crates/mush-bot/tests/mining_cohort.rs`, `rs0_the_bot_types_the_salvage_words_on_urvara` (512) and
`rs0_the_bot_types_the_salvage_words_on_ahuna` (256), one body (`salvage_smoke`), `#[ignore]`d and run by name
(`cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored rs0_ --nocapture --test-threads 1`): the
production root booted in the test binary, the bot's own lane over its own SSH session in line mode, the
canary's two phases around the rate lift. The lane kits up at Ceres, lifts to the site and launches rig 1; then
the bot types story (b)'s `+salvage/claim <hull>`, `+salvage/contest <hull>=recovery` and `+tv/1 scrap`, each
asserted in `field::in_vocabulary` before it goes, and `salvage::answer` classifies each answer. **Passed on
the owner's Windows box, 2026-11-03:** Urvara Reach launched at +321 s, the three answered at +330 s (no
salvage custody names the hull; no remains lie within a cell of rig 1); Ahuna Flank launched at +352 s,
answered at +356 s; 687 s for both.

**Divergences.**

- The row's acceptance reads the lines on a site with nothing recovered: the answers classified are the
  refusals (`NOT_IN_CUSTODY`, `NO_REMAINS`). The filed claim, the contest preview and the break-up are read
  by the same reader against the game's own sentences in the unit test; RS.3's `salvage_close` story and
  RS.9's whole stories play them on a real casualty.
- RS.5's and RS.7's forms are admitted before the game has them, at doc 41's ruled grammar, as FR.0 admitted
  `+faction/stance` before FR.1. `+bay/return` (doc 39 IN.8) has no ruled grammar yet and is left to IN.8;
  `+debts` and `+debts/pay` (RS.5) and the tender's `+tv/<n> stores open|unjam|stabilize|rearm` (RS.1) are
  outside the row and left to those cuts.
- No lane plays the lines yet; the stories grow them from RS.1 on.

**Cell cost (rule #14).** **The delta is two new tests of 0.02 s in all, in the `bot` cell; no other cell gained
or lost a test.** Measured from the JUnit of #385's CI run
[41760031062](https://github.com/conclavio/rustmush/actions/runs/41760031062) on head `3424a895`, against
main's last CI run [41754225803](https://github.com/conclavio/rustmush/actions/runs/41754225803) on `c0d123a9`.
Main since then (`a2ef4075`) differs from `c0d123a9` only in `40-cut-notes.md`, so the two runs differ only by
this cut.

- **New tests**: `mine::lane::salvage::tests::rs0_the_salvage_answers_match_the_game` (0.01 s) and
  `…::rs0_the_salvage_lines_are_closed` (0.008 s), both `bot`. The two SSH smokes are `#[ignore]` and run by
  name only.
- **Each cell's test count**: the same in both runs except `bot` (736 → 738).
- **Summed minutes per cell, main → PR**: `bot` 11.80 → 12.13; `bot-stories` 26.58 → 28.24; `rest-server`
  48.83 → 65.47; `rest-server-boundary` 20.90 → 36.84; `tacsim-integration-7` 48.51 → 27.53;
  `tacsim-integration-6` 45.24 → 24.80; total 637.12 → 673.04.
- **These movements are host noise, not this cut.** They run both ways, from −21.0 to +16.6 minutes, in cells
  whose tests are identical in both runs (`bot`'s +0.33 is 0.02 s of new tests and 19.8 s of noise).

No cell's wall moved because of RS.0, so nothing moves shard (rule #3).
