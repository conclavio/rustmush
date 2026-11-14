# 47. The close-out — cut notes

One section a unit of [doc 47](47-close-out.md) §6. A unit's session replaces its own "Not built."
line with what it built, under these headings: **Survey**, **Built**, **Tests**, **Divergences**,
**Not built**. Each session edits only its own section, so two units' notes never touch the same
lines.

## CO.1

Not built.

## CO.2

Not built.

## CO.3

Not built.

## CO.4

Not built.

## CO.5

Not built.

## CO.6

Not built.

## CO.7

Not built.

## CO.8

Not built.

## CO.9

Not built.

## CO.10

Not built.

## CO.11

The defect sweep (doc 47 §5). Taken in the order the unit was given: D1, D2, D5, D6, D7, D8.
D3 and D4 wait for CO.2 and are not taken. Paths are under `crates/tacsim/` unless given.

**Survey.**

- **D1, the tonne for 3 kg, is the ledger's rule, not a defect.** The burn is charged in
  `space::tick::HullPropellantLedger::record_burn_kg`. The hold's `stow.propellant` pool is
  whole tonnes, and the ledger keeps it at `floor(live_kg / 1_000)` (`SF.R4`, `D-35-6`): the
  first kilogram of a fresh tonne debits it whole and the other 997 kg stay live in the
  ledger, which the bubble reads (`effective_live_kg`) and which survives a bubble's close
  and a restart (`SF.R11`, `SF.R21`; the cap never drops a ledger with a part-tonne owed,
  `SF.R27`). MW.21 read the pool, 9 t to 8 t; her live mass was 8,997 kg. No rounding error
  and no unit mix-up. What a player could call a defect is that the hold reads 8 t while the
  ship holds 8,997 kg; that is the integer-tonne inventory, and changing it is a design call.
- **D2.** `+ship/mind`'s read of a sealed hull was one sentence of 154 columns for the Salt
  Moth (`port::ship_mind::MIND_ABOARD`). Filled at a 32-byte hull name (`MAX_HULL_NAME_BYTES`),
  a 24-byte mind and a ten-digit licence, five more of the verb's lines passed 120:
  `MIND_PREVIEW` (215), `MIND_HOLDS_HER` (139), `MIND_VOIDED` (137), `MIND_UNFIT` (131),
  `MIND_OFF_PREVIEW` (130), `MIND_SEALED` (128), and the refused read, which joined two
  sentences on one row. A mind's name has no cap of its own, so rewording alone cannot hold
  the floor. **The absent-unit notices** (120 to 196 columns) are in `field/absent.rs`, a file
  CO.2 is changing: they are not touched here (below).
- **D5.** The retry list is `Minding::homebound` in `field/absent.rs`, a module state the host
  fills at `on_out` and never writes to the store. Every line of a fix is in that file, which
  is CO.2's.
- **D6: the flake is the test's, not the Registry's.** In `minds::bulk::registry_service`
  the ack that says "Registry work finished." sets `Phase::Finished`, and `continue_owned`
  drops the entry in the same command under the same lock; from then `holds_rating` is false
  until a new job is queued. So a `RATING_BUSY` after "finished" means a new job exists. The
  test made it and missed it: `Running::command` returns every line recorded since the
  command was sent, and on a loaded box late lines share the drain (the engine's `Queued.`
  and the driver's `Done.` for the earlier `@mind/save`, an inventory provider's "Dissolved
  lot#…", an obligation's line). The re-rate loop compared the whole drain with
  `["Registry preparation queued."]`, so a reply of `["Done.", "Registry preparation
  queued."]` did queue a job and was not counted; every later rate met that job and was
  refused `RATING_BUSY` until `STEP` (120 s) ran out. Both 160 s timeouts reproduced open with
  exactly that reply and then about 600 refusals. The test's other exact matches failed the
  same way, and a loaded world sometimes sheds a line ("The world was busy too long for that
  line; please retry in a moment."), which broke the cancel step.
- **D7.** `docs/design/45-cut-notes.md` held `## AP.11` twice. Commit `e45e47669` ("the cut note
  names the second merge of main") appended a whole new copy rather than editing the first;
  the merge of main that followed left the copies in the other order. The two agreed for 210
  lines. The first copy (the newer) had the second merge's bullet, but its last 13 lines were
  AP.10's (its CI bullet naming `mush-campaign`'s census, and a "Not verified" list naming
  `end_practice`), pasted over AP.11's own. Nothing links to the second copy's anchor
  (`#ap11-1`); doc 45 links `#ap11`, which is the one kept.
- **D8.** MW.19's item 3: the author's session reads the engine's `Queued.`
  (`mush_core::engine`, `drain_authoritative_submit`) and the world task's fallback `Done.`
  (`mush-server` `world_task.rs`, `CommandSubmitDrive::Staged`) for every submission a module
  makes as that player from a tick, the Registry's steps among them (4,294 lines a rating).
  Item 4: `+tv/<n> launch mind=filed:<licence> go` answers as a plain launch does, and nothing
  names the mind the claim seals aboard.

**Built.**

- **D2: `+ship/mind` at the floor.** A `fit` in `port::ship_mind` wraps each row at the
  crate's 118 columns (`port::site_map::wrap`, continuation rows indented two) and is applied
  to everything the verb and its arm say that carries a player's name: the read, both
  previews, and the arm's four notices (sealed, voided, refused, unsealed). The sentences
  break their own rows where a sentence ends (`MIND_ABOARD`, `MIND_UNFIT`, `MIND_PREVIEW`,
  `MIND_OFF_PREVIEW`, and the refused read), so the bot's needles each sit in one row.
  `MIND_HOLDS_HER` is said unwrapped by the yard and the refit (`bay_work/command.rs`,
  `refit.rs`, CT.7's files), so it is reworded to fit as it stands: "The {name} is held as she
  is while a mind is sealed aboard: +ship/mind off, at her berth." The bot's needles for
  `MIND_UNFIT`, `MIND_OFF_PREVIEW` and `MIND_HOLDS_HER` follow (`mush-bot`,
  `mine::minds::ship_mind`).
- **D7.** One `## AP.11`: the newer copy, its "Main moved twice" bullet kept, with AP.11's own
  CI bullet and "Not verified, and not covered" list put back in place of AP.10's lines.
- **D8, item 4: the operator is told which mind is aboard.** The surface tick's claim arm says
  `surface::excursion::CLAIM_ADMITTED` to the operator ("The Registry admits licence
  {licence}'s claim on #{tv}." and, on its own row, "{mind} is sealed aboard at Class {class}
  until the rig is home.") on the visit that acknowledges the claim's Open and submits the
  queued launch behind it (`surface::mind_claim::step`). It is said once: the Open's ticket is
  removed on that visit, and the line is said only when one was removed. Live when the
  operator is connected, queued for their login when not (`NoticeLevel::Trip`, as
  `CLAIM_REFUSED` is). A launch refused after it is still told `CLAIM_CANCELLED`. The bot's
  claim table has a row for it.

- **D6: the test matches the line it needs.**
  `mush-server` `player_market_commands::rate_queues_and_cancels_a_genuine_registry_preparation`
  reads the author's own lines (`command_seen`), looks in each reply for the one line it is
  checking, resends a line the world shed (`admitted`, within `STEP`), and waits for "retained
  and queued" and "finished" with `until_own`, which counts a line inside the command's own
  reply. Test-only; no production line moved.

**Tests.**

- **D1** `domain_world::voyage::co11_a_part_tonne_burn_debits_a_whole_tonne_and_keeps_the_rest_live`
  pins the rule with MW.21's numbers: 3 kg off 9 t debits one tonne, the live mass is 8,997 kg
  before and after the debit lands, the next 997 kg debit nothing and the kilogram after them
  opens the next tonne. It passes on main: there was nothing to fix.
- **D2** `port::ship_mind::tests::co11_every_ship_mind_line_fits_120_columns`: every sentence
  the verb says, filled at the worst case above, is at most 120 columns a row (through `fit`
  where the verb wraps, as it stands where it does not); the Salt Moth's read is the two rows
  MW.21 should have read; a 60-byte mind still wraps. On main the Salt Moth's read is one row
  of 154 and the test fails. MW.13's story (`domain_field::mw13_ship_mind`) still matches its
  lines byte for byte.
- **D8, item 4** m8_world's `claimed_launch` (the helper the licensed-launch stories share:
  `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`,
  `s4b2_a_claim_given_up_does_not_hold_the_next`, and MW.5's filed-mind story in
  `c4_filed_mind_field`) asserts the operator is told `CLAIM_ADMITTED`, filled with the
  claim's own seal, exactly once across the fires that open the claim, said or queued, and
  that each row fits 120. On main no such line exists and the assertion fails.

- **D6** on conclav.dev, at `bb6a24e8` (`claude/doc47-close-out`), 24 to 32 copies of the test
  binary at once (`--exact`), load average 40 to 89 with other sessions' builds running:
  before, 251 of 264 passed (13 failed, 4.9 %: two the reported 160 s `RATING_BUSY` timeout,
  the rest stray lines in exact matches); with the fix, 192 of 192 passed. The harness keeps
  its store in memory (`PurchaseFaultStore`), so no Redis was needed. The failure is a race of
  wall time, not a logic the fix could fail on main deterministically; the before and after
  counts are its test.

**Divergences.** None from doc 47's rows. D1 is closed by a reading and a pinning test, not a
fix (the unit's row allows "named with the reason").

**Not built.**

- **D6's neighbours.** The same stray-line flake can hit the shared `world()` launch setup
  (one run failed "one acknowledged wallet creation … got []" with the shed line in the
  reply) and every other test in `player_market_commands.rs` that matches a whole
  `command()` reply. Not touched; the same `admitted`/`command_seen` idiom fixes each.

- **D2's absent-unit notices** (`field/absent.rs`, 120 to 196 columns). CO.2 is rewriting those
  notices (R-47-2, R-47-3); a width test written now would be written against sentences about
  to change. Left for CO.2 or for a CO.11 follow-up after it lands: the same check, each
  notice filled at a five-digit unit and the longest posture word, at most 120 a row.
- **D3 and D4** wait for CO.2 (doc 47 §6, "Collisions to watch").
- **D5, the recover owed across a restart.** Left: the list and every line of a fix are in
  `field/absent.rs` (CO.2's). The fix is one of two. (a) Write `Minding::homebound` to the
  store with the field's other records and read it back at `at_resume`; or (b) rebuild it at
  `at_resume` from what survives: a unit out of the field, standing beside its carrier's
  ramp, whose rig record names an absent pilot and no recover act. (b) needs no new key and
  is the smaller; it is the one this note would take. Either wants MW.7's restart story
  extended by a reboot between `on_out` and the recover's landing.
- **D8, item 3: the engine's `Queued.` and `Done.` for a submission the player did not type.**
  Left, as too large for a sweep. The fix is in the engine's submit path: mark an
  `AuthoritativeSubmit` made outside a command dispatch (`Ctx::submit_authoritative` with no
  dispatch stamp), skip the receipt `Queued.` for it in `drain_authoritative_submit`, and skip
  the world task's fallback `Done.` (keeping a provider's own summary). `AuthoritativeSubmit`
  is a public struct with about fifty literals across nine crates' tests, the change reaches
  every story that reads a tick's lines, and MW.19's nightly job is built to go red the day the
  lines stop (`scripts/bot-minds-lane.sh`, "KNOWN DEFECT GONE"), so the marker comes out in
  the same change. It wants its own unit, with the stories' and the bot's runs.

## CO.12

Not built.
