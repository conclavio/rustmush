# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Not built.

## MW.1

Not built.

## MW.2

The two market defects ruled to be fixed before the go-live (doc 44 §7.1; Q-44-14, Q-44-15;
second review, §4c rows 8 and 9; doc 17g §7). Every Q-44 question it rests on is ruled.

**Built.**

- **A read the Roll owns says whether a listing can still be sold.**
  `crates/mush-roll/src/commerce/withdrawal/sellable.rs`, reached only as
  `EntitlementPurchaseWriter::sellable` (the writer the market already holds). Given what a
  listing froze of its seller's standing (`ListedSubject`), it answers `Sellable::Yes` or
  `No(Unsellable)` for a batch of up to 32 listings:
  - `Withdrawn`, `Recalled`: the listed revision's status row (recall is told first).
  - `SellerLicence`: the seller's owner licence is past Grace, has moved to another term, is
    revoked, or its holder is barred by an inspection for now.
  - `ParentLicence` (a derived listing): the seller's qualifying parent licence (the one the
    child was filed under, or a committed replacement) is not theirs, is past Grace, is not an
    open-resale term, or is revoked; or the parent source is recalled.
  - `CharterMoved`: the charter's title is not the seller's at the listed title revision, or the
    seller is no live character.
  - `Unproved`: a row the answer needs is missing or does not match. The market treats it as
    unsellable.
  It reads at most 4 rows for an original listing and 12 for a derived one, in at most 12
  batched reads a call. The rows are read through the market's own read lens: `install` adds
  seven read-only family grants to `market` (`status`, `licences`, `revoked_terms`,
  `operations`, `owner_parents`, `owner_qualification_heads`, `owner_qualification_operations`).
  The market holds no handle to any of them.
- **The counter and the House agent's list ask it.** In `crates/mush-market/src/mind_commands.rs`
  "current" is now two tests, the policy (`current_policy`) and then the Roll's answer, for
  `Listings`, `ListingsAfter`, `ListingByCode` and `ProcurementListings` (which now shares
  `Listings`' page loop, `page`). A dead listing leaves `+minds`, the `mind.market` panel and
  the agent's list; its code is no longer found (`+minds <code>`, `+minds/buy <code>`); it still
  serves as a `MORE` cursor, as a policy-stale listing does. The read's descriptor
  `mind_market_commands` is version 3.
- **The Roll's answers are paid for inside the read's own allowance.** Core gives an owner read
  256 reads and 512 rows and fails the whole read past either; the listing scans alone could
  spend every row. The page loop counts each scan and each listing asked about (`Budget`) and,
  when what is left cannot pay for the next, ends the page with the last listing it decided as
  the cursor. A page that meets a long run of dead listings can therefore be short, or empty,
  and still say `MORE`; following the cursor reaches what lies behind them.
- **The Roll's renewal readers carry a lasting-refusal verdict.** `RenewalStop` in
  `crates/mush-roll/src/commerce/renewal/reader.rs`, used by `house_renewal/reader.rs` as well:
  `Recalled`, `Withdrawn` (the pinned revision's status row; one more row a licence) and, for a
  personal licence, `OwnCharter` (the holder now owns the charter it is sold under, so the
  holder would be buyer and seller at once; from the economy's title, no row). It is derived at
  every read and never stored. `Inventory` leaves a stopped licence out; `Status` answers it.
- **Both renewal services stop on it.** In `mind_renewal/service.rs` and
  `mind_house_renewal/service.rs`: the scan admits no stopped licence; a draft entry admitted
  before the stop arose ends at its next turn; the player's `request`, `accept` and `retry`
  answer the lasting line and submit nothing (a decline of an open offer still goes through; a
  funded offer still runs to its grant or refund). Nothing in the service's memory decides it,
  so the next scan and a restart's first scan are told the same by the Roll.
- **`+minds/renew` says why**, in the ruled words (`player::RENEW_STOPPED`): "This licence cannot
  be renewed: <reason>. Nothing further will be tried." The reasons are "the Registry recalled
  this mind", "its author withdrew this mind" and "you now own this mind's charter".

**Tests.**

- `mush-roll`, `commerce::withdrawal::sellable::tests` (six): each of the four ways a listing
  dies and `Unproved`, the Grace boundary, the parent choice (bound licence, replacement, a
  later owner with none), and the declared row costs. `commerce::renewal::reader::service_tests`
  (two new): the three lasting causes and their order; an entry that is not stopped serialises
  as before.
- `mush-market`, `mind_commands::tests` (four new): a listing the Roll calls unsellable leaves
  `Listings`, `ListingByCode` and `ProcurementListings`, is never asked about when the policy
  already dropped it, and still serves as a cursor; a dead row does not count toward a code's
  ambiguity; over 200 dead derived listings no read spends more than core's allowance, every
  cursor moves forward and the one live listing behind them is reached; a full page followed
  only by dead listings is the last page. `mind_renewal::tests` (one new): the ruled sentence
  for each cause, exhaustively, and the House type is the same type.
- The two stories are rewritten (`player_market_story.rs`, `player_market_typed.rs`). The typed
  story: after the typed `Withdraw` the `mind.market` panel has no rows and the empty counter's
  notice. The command story: both listings read on the counter within the owner term's Grace;
  after `@mind/attest` they are gone and the code is not found (see Divergences); after
  `@mind/withdraw` the counter is still empty, no quote row is added, and `+minds/renew` answers
  the withdrawn line with no renewal operation opened; after the recall and the restart it
  answers the recalled line, again with none opened. Both run on the CI dispatch only.

**Divergences.**

- **An owner's attestation takes that owner's listings off the counter.** A listing freezes the
  owner term it was made under, and a quote has always refused a listing whose frozen subject
  differs from the Roll's current facts (`mind_entitlement::offers::current_listing`). So once
  `@mind/attest` opens the owner's next term, every listing made under the earlier term could
  already not be quoted; it used to stay on the counter and now leaves it, and the author has to
  list again. The same holds for any change of the charter's title revision. The command story
  pins it. Whether a listing should survive its owner's attestation is a rule of the quote, not
  of this unit, and is the owner's to decide.
- **"Expired" is not a terminal state here.** The brief named recalled, withdrawn and expired.
  The renewal protocol renews an expired predecessor on purpose (the reader's descriptor says
  `expired_predecessor_permitted`), there is no renewal deadline anywhere in the Roll, and the
  service only begins once a term has ended. Leaving lapsed licences out would turn the service
  off. `OwnCharter` is added instead: it is the second cause doc 17g §7 recorded for the fault.
- **A lasting refusal is known before a draft is sent, not learned from one.** A refused draft
  tells the service nothing (the provider answers one uniform refusal and the observation is
  `NoIntent` with no payload), so the lasting causes are read from the Roll's rows. The service
  therefore tries a recalled licence not once but never; an entry it already held ends at its
  next turn.
- **The reason phrases are a default.** The sentence is the owner's ruling; the three phrases
  that fill "<the Registry's reason>" are this unit's wording.
- **The counter's answer is slimmer than a quote's.** It proves no ancestry, root, rating or
  royalty evidence, so a listing it shows can still refuse a quote (a re-conformed rating, a
  product the policy now computes differently, a buyer-specific refusal).
- **Fingerprints move.** The seven read grants enter the state-family registry's fingerprint and
  the descriptor's version enters the mechanics registry's. Neither is pinned in the tree;
  `tacsim/src/hardware/flow/launch.rs`' check of the read's version is updated to 3.

Two of these would take real work to undo: the budgeted page loop (the old loop is gone, and
`ProcurementListings` now shares the new one), and the choice to read the Roll inside the
market's read rather than from the caller, which is why the answer has to be slim.

**Not built.**

- No back-off for a passing refusal. A draft that refuses for a cause that is not one of the
  three (a derivative waiting on its parent author's renewal, a short wallet, a pending resale,
  an officer without the grant, and also any fault this unit does not classify: a charter an
  organisation holds, a retired House purchase template, an integrity mismatch) is still retried
  once a real second, as before, and the holder still reads the uniform line.
- The House agent keeps no memory of a dead listing it had already quoted; it is simply not
  handed the listing again, and its open quote expires as any quote does.
- No change to the quote path, to `+minds/holdings`, or to tacsim's commands.

**Not pinned by a story.**

- No engine-level test drives the renewal service over a recalled or withdrawn revision; the
  command story on the CI dispatch is the only run that does, and only for the personal service.
  The House service's stop has no story at all.
- "A derivative waiting on its parent's renewal is still retried" holds by construction (it has
  no `RenewalStop`, and the retry path is untouched) and has no test of its own.
- `ParentLicence`, `SellerLicence` by revocation or bar, and `CharterMoved` are pinned by the
  Roll's unit tests on the decision alone; no story lists a derived mind and then lapses its
  parent.
- The House agent's list is pinned over the in-memory listing family; the procurement stories
  (`m8_world`) run it over real rows without a dead listing among them.

## MW.3

Not built.

## MW.4

Not built.

## MW.5

Not built.

## MW.6

The Registry's pace as a file key, and the two notice drains that stopped on one bad holding
(doc 44 §7.1, Q-44-41; second review, §4c rows 4 and 10). Built on Q-44-41's default, which is
open: the key exists with today's value as its default, and a dev box sets it low.

**Built.**

- **`[tacsim.minds].registry_pulse_ms`.** `mush_config::TacsimMindsCfg.bulk_maintenance_ms` was
  `serde(skip)`, so no file could set the pace of a Registry lease; only the acceptance stories'
  harness could, in code. It is now a file key under the name `registry_pulse_ms` (the Rust
  field keeps its name and is not a key). Absent, it is 1,000 ms, production's second, as doc
  43c records was decided. `MindsConfig::validate` (`crates/tacsim/src/config.rs`) refuses a
  value outside 10 to 60,000 by the key's name, ahead of the bulk model's own check. The key is
  documented, commented, in `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl`, and in
  the key table at the head of `crates/mush-config/src/config.rs`. `tacsim::MindsConfig` itself
  still skips the field in its own serde; the host's mapping (`mush_server::config_ext`) is the
  path a file takes.
- **Both notice drains skip one bad holding.** In `crates/mush-roll/src/commerce/registry_recall/notice.rs`
  and `registry_revocation/notice.rs`, `tick` returned the first error from `own` before the
  cursors were written back, so the next tick read the same page, met the same holding and
  stopped again: every notice behind it waited for ever. Now an error that is one holding's own
  (`one_holding`: `Missing`, `Mismatch`, `Invalid`) skips that holding, emits nothing for it,
  logs it once (`Runtime::note_skipped`, bounded like the remembered notices) and lets the
  cursor pass. An error that is the inventory's (`Unavailable`, `Stale`, `Capacity`) still stops
  the drain where it is, before anything is emitted, to be tried again. `page` had the same
  fault one level up: a row that failed `validate` failed the whole page. Such a row is now
  carried with no recipient, so the cursor passes it.

**Tests.** `mush-config`: `m1b1_the_minds_block_parses_absent_written_and_refuses_a_typo` gains
the key (absent is 1,000; `registry_pulse_ms = 50` parses; `bulk_maintenance_ms` is an unknown
field). `mush-server`: `config_ext`'s refusal table gains 5 and 60,001, each refused by the key's
name, and the mapper carries 50 through to the layer's config. `mush-roll`: one test in each
notice module, `a_bad_holding_is_skipped_and_an_unavailable_inventory_is_not`. Local: `cargo test
-p mush-config` 141 + 14 passed; `cargo test -p mush-roll notice` 3 passed; `cargo test -p
mush-server --lib config_ext` 18 passed; `cargo test -p mush-contract-tests` green; clippy with
warnings denied on mush-roll, mush-config, tacsim and mush-server.

**Divergences.** The unit's row asks for "the measured time a rating takes at both paces". It
was not measured in this cut. The figures on file are `ci.yml`'s (about 1,070 leases a
certificate, about three hours a story at 1,000 ms) and the acceptance stories' (minutes at
their 50 ms). MW.1's bring-up is where a dev box's rating is first timed.

**Not built.** Registry pacing beyond the pulse: a lease still takes 3 to 7 s of work at the
stock pace, and nothing here shortens the work a lease does. The key is not set in
`deploy/dev1` or `deploy/dev2`; MW.1 sets it when it renders the launch.

**Not pinned by a story.** The drains end to end: no rig corrupts one holding and shows the
holder behind it getting their notice. The tests pin which errors skip and which stop, and that
the skipped keys stay bounded. A row that fails `validate` in `page` is not pinned at all.

## MW.7

Not built.

## MW.8

Not built.

## MW.9

Not built.

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

## MW.14a

Not built.

## MW.14b

Not built.

## MW.15

Not built.

## MW.16

Not built.

## MW.17

The last decision in the staff view of an NPC (doc 44 §7.1, Q-44-30; first review, finding 8;
second review, §4c row 19).

**Built.** In `crates/mush-npc`:

- **A turn records what woke it.** `NpcHostState::begin_turn` (`runtime.rs`) drains the mailbox,
  so it is the one place that knows; it now writes a `LastDecision` on the NPC's runtime: the
  correlation, when it began, and for the first `LAST_DECISION_WOKE_MAX` (8) stimuli of the batch
  their kind (`Stimulus::kind_name`) and whom each was about. `fire_dialogue` (`module.rs`) adds
  how many public facts and recalled memories the snapshot showed the model
  (`note_decision_context`); guarded and secret facts never cross the async boundary, so the
  count is of public ones.
- **A completion records what the turn came to.** `NpcCompletion::on_complete` writes a
  `DecisionResult`: the model answered with these actions (their tool names), the model did
  nothing, the profile's fallback line with the staff-facing reason (capped at 160 characters),
  or the payload did not decode. A completion for another turn changes nothing
  (`finish_decision`).
- **`@agent/show` prints it**, in two lines after the runtime line (`LastDecision::lines`), and
  `docs/help/npc.md` describes them. The verb is Builder and up, as before.

The record holds kinds, ids and counts. It holds no prompt, no reply and no stimulus text, in
keeping with the audit's rule (`docs/help/llm.md`).

**Tests.** `runtime::tests::mw17_the_last_decision_records_what_woke_a_turn_and_how_it_ended`
(the record, the cap, another turn's completion, the two lines carrying no text) and
`mw17_staff_read_what_woke_a_turn_and_whether_the_model_or_the_fallback_spoke`
(`crates/mush-npc/tests/agents.rs`): through `@agent/show`, a turn in flight, the same turn
answered by the model, the next turn starved of budget and answered by the fallback, and a
player refused the verb. No test makes a provider call. Local: `cargo test -p mush-npc` 150 +
72 + 34 passed; clippy clean with warnings denied.

**Divergences.**

- **The NPC decision call still carries no NPC.** The row asks for `decision_llm.rs` to pass its
  NPC so the per-NPC budget and `@llm/kill <npc>` reach a decision. It cannot be done at that
  call site: the worker lease it is handed (`mush_core::decision::DecisionWorkerLease`) is
  public-safe by design, "no world or private witness reference can cross the async boundary",
  and names a controller and a decision, not an object. Carrying an audit attribution on the
  lease is a change to that contract in `mush-core` and belongs to its own cut.
- **"The knowledge tiers the NPC was allowed"** is reported as a count of public facts and of
  recalled memories. The snapshot carries no other tier, so there is nothing else to report.

**Not built.** The record does not survive a restart (it is runtime state, like the mailbox).
It keeps one turn, not a history. It does not cover a summarise turn or a scene-turn decision
(`npc_decision`), only dialogue turns.

**Not pinned by a story.** A completion whose payload does not decode (the `Unreadable`
result) through `@agent/show`.

## MW.18

Not built.

## MW.19

Not built.

## MW.20

Not built.

## MW.21

Not built.

## MW.22

Not built.

## MW.23

Not built.

## MW.24

Not built.

## MW.17b

Not built.
