# 44. Minds, wired through the game — cut notes

One section a unit of [doc 44](44-minds-wired-through-the-game.md) §7. A unit's session replaces its
own "Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**, and the open Q-44 defaults it was built
on. Each session edits only its own section, so two units' notes never touch the same lines.

## MW.0

Licensed minds composed on a live server (doc 44 §7.1, Q-44-39; second review, §4c row 1). Built
with MW.22 below as one change: the owner ruled that neither reaches main without the other.

**Built.**

- **Every Active launch composes the claim path.** `tacsim::launch::LaunchComposition::prepare`
  (`crates/tacsim/src/launch.rs`) now ends its Active branch with
  `with_ic_minds_in_bays(Vec::new())`, after the House chain and the player market, for a
  format-1 and a format-2 deployment alike. An `Initialize` launch composes none of it, as
  before. So on a production boot `claim_path_composed` is true once the launch is Active: a
  licensed `+tv/<n> launch mind=filed:<n> go` opens its claim in the bay before the ramp, the
  field's `claim_seal` finds the claim and admits the rig under its own seal, a licence's seats
  are counted at `go`, and the reader refuses an Open while a resale is pending.
- **One function, shared by construction.** `mush_server::boot` already called `prepare` and
  nothing after it. The switch the rigs alone could reach,
  `LaunchComposition::with_ic_minds_in_bays`, is deleted, and the M8 world's `factory`
  (`crates/tacsim/tests/domain_field/m8_world.rs`) calls `prepare` and nothing after it. No
  world can now compose a launch without the claim path or with something production lacks, so
  every M8 story runs on the production composition. `m8_world_with_claims` and `factory_for`
  are gone; `m8_world_field` remains as the M8 world on a chosen site.
- **Nothing else had to be composed with it.** Doc 17g §4 deferred "IC claims and hull cover in
  live play" for want of an IC region host. The host exists since doc 23's S.4b (the surface
  tick's claim arm and the IC field). `with_ic_minds_in_bays` asks only for an Active commerce.
  Hull cover asks for the claim path, not the other way about, and stays uncomposed.
- **The shared story helpers follow the claim.** `m8_world::launch` and `recover` pump the
  claim arm where a launch opened a claim, so doc 17h's stories read as before.

**Tests.** Fast, on the production composition (`domain_field::m8_world`):
`m8g_the_combined_world_boots_active` now asserts the claim path is composed, and every M8
world asserts, at its own `Initialize` boot, that none is composed there. **The server's own
boot shows it too**: `mush-server`'s player-market tests boot the server's module list from a
launch document (`game_modules` with `[tacsim.minds].launch_path`), and their pinned count of
read services and grants rose from 41/44 to 42/45, which is the claim path's admission reader
and TacSim's grant to it (`player_market_commands.rs`, `player_market_typed.rs`; the same pin
in `tacsim`'s `hardware::flow::launch` rose from 40/43 to 41/44; the caps are 64/64). The acceptance stories are `#[ignore]`d (each takes a genuine Registry rating) and run on
the CI dispatch: `s4b3_a_licensed_mind_flies_a_field_at_256` and `_at_512` (a filed mind on a
rig its pilot holds title to opens a claim, is admitted to a field under its own seal and card,
and the one seat refuses the second rig), and
`s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten for MW.22 (its
step 3 is the launch refused while the licence's resale is pending). Results are under MW.22.

**Divergences.**

- **A leased rig and a House hull are not refused.** The row's last clause ("a leased rig and a
  House-titled hull are refused by name") describes MW.0 alone. Under Q-44-39 it never reaches
  main alone, and MW.22 admits both. What is refused by name is a hull the pilot has no
  authority over (MW.22).
- **No change to `mush-server`'s code**, only a comment at the call: the function it calls
  changed.

**Not built.** Hull cover and paid Follow stay uncomposed (17g §4). The DT launch
(`+sortie/launch`) still takes no `mind=` word, so a detachment's flight opens no claim (doc 23
§17.9, 23-10). A cored ship opens no claim (D-23-12).

**Not pinned by a story.**

- **A changed mind changing what the rig does.** The field stories pin that the rig flies the
  claim's own seal, mind and card and not the ROM's, with no drop and no fault. No story flies
  two different filed minds on one rig and compares what it did; MW.7's filed-mind story is the
  first that flies a mind written to behave differently.
- **A rig launched on a world the server binary booted.** `mush-server`'s tests boot its own
  module list over an Active launch and count the claim path's reader, and the M8 world flies
  rigs on what `prepare` returns, but no one test does both. MW.1's bring-up is where a rig is
  first launched on a box.
- **A definition-set change at deploy.** The claim path adds its owners' definitions to an
  Active launch's manifest, so a store written by an earlier build does not boot under this
  one. The dev boxes are wiped at every step; nothing migrates.

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

Docs only (2026-11-11). Each ruling is written where its question was first asked, as "Ruled
2026-11-11 (doc 44 Q-44-n)"; no question was deleted and no surrounding text rewritten.

- `17-unit-minds.md`: D-UM-56 (Q-44-9); Q-UM-12, Q-UM-14, Q-UM-18 (Q-44-10).
- `17g-m7-completion-plan.md`: §3 C4 (Q-44-15); §4's paid Follow and §5 item 4 (Q-44-11); §5
  item 3 (Q-44-5); §5 item 5 (Q-44-9, Q-44-10); §7's renewal-retry finding (Q-44-14).
- `17h-m8-contraband-procurement-plan.md`: the heirloom (Q-44-12); the House term (Q-44-13).
- `36-ground-combat-after-the-field.md`: Q-36-41 (Q-44-7); Q-36-43 (Q-44-21).
- `03-llm-integration.md`: the status line, from `LlmFeature` and its callers (§4 G), with Q-44-29
  and Q-44-6.
- `27-ic-field-operations-design-and-survey.md`: Q-H5 (Q-44-35 to Q-44-38).
- `42b-surrender.md`: the yielded hull that keeps firing on a busy exit (§4c row 22), checked in
  `field/raiders.rs`.
- `23-ic-sorties.md`: §14 S.2's owed list, a wrecked ship's epoch (§4c row 26), checked in
  `port/epoch.rs`.
- `README.md`: the 17g row.

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

One applier and one view for every space mind, the trace-string and memory hooks removed, and
the interdictor's stand-down hull by hull (doc 44 §7.1, Q-44-24; first review finding 6, second
review row 16). PR #551. Paths are under `crates/tacsim/src/space/` unless given.

**Built.**

- **The one applier (`applier.rs`, new).** `apply_intents` turns one think's intents into
  bubble commands for every actor: the customs corvette and the raider (`picket.rs`), the
  interdictor (`tick.rs`) and the backup mind of a hull whose crew has no link (`backup.rs`).
  Those were four hand-written mappings. What each intent means in a bubble is one table, at
  the head of the file:
  - `lock(id)`: her lock is the ship that row names.
  - `helm(word)`: the direction of this think's burn. `speed(n)`: its strength, `n` cm/s², as
    far as her drive gives; below nought, the other way. With no helm word the burn is toward
    her lock; with neither, nothing. `speed(0)` and `hold()` cut the drive. A burn stands
    until a later think changes it.
  - `emcon(true)` is COMBAT; `emcon(false)` is the quietest posture the seat allows.
  - `fire(id, slot)`: the mount `id` fires at her lock. Each `fire` of a tube is one torpedo of
    one salvo. A shot with no lock is dropped: the applier never picks a target.
  - Every other intent is dropped.
- **Seats.** The applier is one function; a `Seat` says what its actor may do. `Seat::PATROL`
  and `Seat::RAIDER` fire as their doctrine decides. `Seat::ABSENT` is a hull flown for an
  absent crew: every lock and shot is dropped (`FireRule::Never`), and every burn but one
  that takes her drift against the ship stopping her off (`BurnRule::Toward`), so doc 35 §11
  `B-4` holds whatever source thinks: she never fires and never runs a picket under power.
- **The one view (`view.rs`, new).** `space_mind_view` builds what every space mind reads.
  Beyond the arena's view it carries a `SpaceView`, hung on `MindView::space`:
  - her own velocity as a vector, her pitch, what her drive gives now, her reaction mass, the
    change of speed her tank holds and the reserve she must keep, and which of the four
    emission postures she holds;
  - the stop a picket or an interdictor has on her (`none`, `hailed`, `hove_to`, `cleared`,
    `ran`), the contact that holds it, and the furthest of those words a picket has spoken on
    open radio in the bubble (a hull that holds no contact still hears it);
  - for each contact: its elevation, its velocity as a vector, the posture its emissions read
    as, and the stop it is under.
- **Contact ids.** Each observer has a `ContactBook`, kept in its mind's record
  (`corvette_contacts`, `raider_contacts`, `interdictor_contacts`, `mind_contacts`). A ship is
  numbered the first time that observer's sensors hold it and keeps the number for the life of
  the bubble; its designator letter is the number's. A ship she has never detected has no
  number and moves no other. The view's `lock` is now the hull's own lock; it used to be the
  nearest contact whatever she was locked on.
- **Four kernels, and the mind ABI at `api=8`** (`crates/tacsim/src/minds/kernels.rs`,
  `mod.rs`): `space(v)` and `space_contact(v, id)` answer the rows above as maps, `()` on the
  ground; `course(bearing_mdeg, elevation_mdeg)` and `course_of(dx, dy, dz)` answer the helm
  word for a direction. No getter, intent, refusal or snapshot format moved.
- **The hooks are gone.** No Rust reads a trace word. `sync_corvette_cleared_stage_in_memory`
  (the write) and `read_corvette_stage_from_memory` (the read) are deleted: the corvette reads
  each contact's stop from its view, and the picket's own rules (hove to under the heave-to
  speed, run past the boundary) are settled on the bubble's state in `settle_picket_stops`.
  With them went what Rust decided for the minds: the raider's uncloak, the interdictor's
  forced posture and its fixed burn, a default target for a shot with no lock, the raider's
  press attack and the interdictor's tube gate. Each is in its doctrine now.
- **The doctrines** (`crates/tacsim/content/minds/`), all four rewritten to steer and fire by
  intents alone:
  - `kirkwood_picket.rhai` reads each contact's stop; it keeps one word of memory, that it has
    hailed.
  - `belt_raider.rhai` lurks dark until the radio says customs have finished with a carrier,
    lights up, and on the next think opens on the nearest hostile customs have finished with.
    It presses only a prey that is radiating for a fight, as the Rust hook did.
  - `interdictor_patrol.rhai` hails on its first think of a hull that is not radiating for a
    fight. After that it reads the hull's velocity: one whose change of velocity is against
    its drift past the interdictor is slowing, and the interdictor holds its fire and its
    drive for it. Any other hull it closes on, along the line to it, and fires at.
  - `linkdead_return.rhai` computes its own brake from the view: the way the stopping ship
    moves against her, sized to end her drift in the step ahead.
- **The stand-down, hull by hull, on state** (`tick.rs` `settle_interdictor_stand_down`;
  `crates/tacsim/src/arena/party.rs` `Relations::stand_down`). A carrier under the heave-to
  speed against an interdictor that still stands is stood down: the interdictor's party and
  hers clear their marks of each other and forget that the other fired, both ways, and no
  third party's book moves. `ActiveInterdictionBubble::stood_down` records each hull once. It
  no longer sets the bubble's `cleared`, which let every carrier go.
- **The contract test** (`crates/mush-contract-tests/tests/engine_contracts/space_mind_memory.rs`):
  nothing under `space/` names `encode_memory`, `decode_memory` or `mush_mind::memory`, and
  every assignment to a mind's memory field has its own think's `memory` on the right.

**What the survey had wrong, and what it did not see.**

- "Three older appliers" were three NPC mappings and MW.8's seat, so four. The drone has
  none: its think's intents are discarded (`mining.rs`, `commands.rs`) and its yield is a
  timer. Not changed here; MW.12.
- The memory hook was two hooks, a write and a read. The read drove the inspection status on
  a path only the unit tests reached (`carriers` empty); production always passed carriers.
- **Nobody could heave to against the interdictor.** It un-ghosted, burned at the carrier at
  10 m/s² from its first think and never stopped, and a carrier enters its bubble 45 m/s off
  it with a drive that gives under 1 m/s². Its doctrine stood down a target that was not
  moving, which never came true. "A Dray can heave to against the interdictor" therefore
  needed the interdictor's doctrine as well as the Dray's: it hails first and holds for a
  hull that is slowing.
- The detection of a contact never depended on its index (the object ids handed to
  `evaluate_space_contact_detection` are not read by the resolver). Only the ids and the
  designator letters leaked.
- **What an ABI move costs.** A seal's digest covers its ABI and its source, and the arena
  folds each seal into its state hash. So an ABI move re-pins: the nine default seal
  identities (`match_/seal.rs` `m5rules_default_seal_identities_are_pinned`); the stock
  mind's digest, because `tests/domain_arena/content.rs` holds its manifest to `API_VERSION`
  and its header says the API; and the arena pins that fold a Standard seal (six tests in
  `tests/domain_arena/` `minds.rs`, `motor.rs` and `reflex.rs`: three state hashes and the
  `hash` of 84 pinned ledger lines), 58 values re-pinned from one Linux run by script; and
  the four rows of the cross-platform corpus (`minds_rhai.rs` `M2_CROSS_PLATFORM_PINS`),
  whose test is `#[ignore]`d and runs only in CI's `tacsim-minds` cell. Nothing an arena
  does moved: the ledger lines differ in `hash` and in the `mind_sealed` lines' `source`
  alone, and the corpus's four stopping times are as they were. The three house minds' envelopes and manifests and the
  acceptance mind's source name the ABI too.
- A changed burn is applied by the bubble as a one-second impulse on the step it changes, on
  top of the thrust it then integrates (`bubble.rs` `command_burn_mmps2`). A doctrine that
  re-aims every think therefore gets about twice what it asked. Not changed; the interdictor
  and the raider ask for their closing burn once and let it stand.
- Relation changes (`set_party_relation`, and now `stand_down_parties`) are made on the bubble
  directly, not through the tape, so a replay from a snapshot does not reproduce them. That
  was so before this unit.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **The ABI moved to `rhai=1.26.0;api=8;stock=3`**, once. Every sealed mind, compile cache key and resume check carries it | **Expensive.** Doc 45's AP.3 moves it again; nothing here needs another move |
| 2 | Space facts reach a script through kernels that answer maps, not through getters on `View` and `Contact`. A getter needs a row in the keyframe parity table and a field on the ground keyframe, and the ground has neither | Medium: it is API. A script reads `space(v).stop`, not `v.stop` |
| 3 | **The helm word.** `helm(mdeg)` carries one number; in space it is the bearing plus a whole turn for each tenth of a degree of elevation. A plain bearing is a level burn, and the arena, which reads a heading modulo a turn, sees the bearing. Scripts use `course` and `course_of`, never the packing. The cleaner shape is a pitch on the intent, which is a new field on a carried enum and a snapshot format move in the arena | Low for scripts; the format move is the cost of the cleaner shape |
| 4 | The stand-down is the host's rule, as a customs clearance is. A doctrine cannot refuse it, because no intent names it. Each hull is stood down once; one that fires afterwards is hostile again and is not offered a second | Low |
| 5 | The interdictor hails first and holds its fire for a hull that is slowing. A hull already radiating for a fight is not hailed. With nobody hostile in sight it shows itself at STANDARD, so it is never an unseen blocker. This changes play: a hull that brakes is not fired on, and its first volley is one think later | Low: it is the doctrine |
| 6 | `emcon(false)` is the quietest posture the seat allows: GHOST for a raider, STANDARD for the others. It used to be two meanings in two mappings. A mind cannot choose PASSIVE: the intent is a switch | Low |
| 7 | A hail and a clearance are open radio: every hull in a bubble reads the furthest word spoken (`space(v).heard`), whether or not it holds the ship spoken to. It is how the raider knows when to light up | Low |
| 8 | The absent seat lets through only a burn that heaves her to, and none when nothing is stopping her. Stricter than MW.8, which braked for her; never looser | Low: one rule in `backup.rs` |
| 9 | One heat reflex stays in Rust, the same for every seat that may fire: a hull over 650 per mille vents before she shoots. It was 600 for the corvette | Low |

**Left, and for which unit.**

- **Return fire in space (Q-44-38).** Not built. The absent seat refuses every lock and shot.
  The seam is `FireRule` in `applier.rs`; the follow-up adds the variant that asks MW.7's
  rule, and a record of which unit fired on which in a bubble (`Relations::fired` is by
  party). It follows MW.7.
- **MW.10** (a doctrine by digest): every seal is still built with the compiled digest. The
  unit test selects a revision by building the seal with another digest and handing the think
  a store that holds its source.
- **MW.11** (thinks off the world thread): every think is still
  `MindPool::shared(1).submit().wait()`. The raider now thinks while she lurks, so a picket
  bubble has two thinks a step from the moment it opens. Relation changes are not on the tape.
- **MW.12** (drones): untouched; the drone's view has `space: None`.
- **MW.13** (a player's mind on a ship): the seat rules it needs are `Seat` and `BurnRule`.
  Q-35-B4 is still the owner's call.
- **No intent names** a stand-down, a coolant vent, one of four postures or the nose's
  attitude. Each needs a variant on a carried enum, so each waits for the arena's next
  snapshot format move. Deferred, with no unit.
- The view's mount rows are still placeholders (100 rounds, 850 per mille). They wait for the
  mounts' redesign.

**The acceptance, and what holds each.**

| The row asks | Held by | Before the cut |
|---|---|---|
| A doctrine changed to hold where it used to close makes its actor hold | `tick.rs` `interdictor_tests::a_doctrine_changed_to_hold_where_it_closed_makes_the_interdictor_hold`: the compiled doctrine closes along the line to a target abeam and above; a revision with `hold()` where that one has `speed(1000)`, named by its digest in the seal, leaves the same hull at rest with the same lock | The interdictor burned one fixed way whatever its doctrine asked, so the closing half fails |
| An escort that heaves to does not clear the runner beside her | The story `mw9_an_escort_that_heaves_to_is_stood_down_alone_though_she_fired_and_the_runner_is_not` (two chartered hulls, by commands), and the unit test `an_escort_that_heaves_to_is_stood_down_alone_though_she_had_fired` | One hull hove to cleared every party and marked the bubble cleared |
| After a stand-down a party that had fired is no longer hostile | The same story and unit test, and `arena/party.rs` `a_stand_down_ends_the_fight_for_the_two_it_names_and_only_those` | The marks were cleared and `fired` was left |
| A Dray can heave to against the interdictor, by a burn a mind computed from the view | The story `mw9_a_dray_whose_crew_has_no_link_heaves_to_against_the_interdictor_by_her_minds_own_burn`: her backup mind brakes her, she is never hit, she is stood down and takes up her passage | Her view said nothing of an interdictor; she coasted and was shot |
| Adding a ship the observer cannot detect leaves a mind's view the same bytes | `view.rs` `a_ship_the_observer_cannot_detect_leaves_her_view_the_same_bytes`, for three observers, with the unseen ship sorting first | Ids and designator letters were an index over every ship |
| No code outside a mind writes its memory | The contract test `space_mind_memory` | `sync_corvette_cleared_stage_in_memory` |

**Verified, and how.**

- **Mutation checks**, on Linux, each putting one removed behaviour back: the stand-down
  clearing every carrier; contacts numbered before the detection filter; the backup doctrine's
  burn given no direction; the picket's host writing the corvette's memory; a burn going one
  fixed way; the stand-down leaving `fired`. Each was caught by the test named for it above
  (the stand-down mutations by the unit test and by the story).
- **The whole of `tacsim`'s tests**, on Linux (conclav.dev), after the re-pin: 4,475 passed,
  31 skipped (the `#[ignore]`d ones).
- **The cross-platform corpus** (`m2_the_twin_corpus_hashes_the_same_on_every_host`, D-UM-28):
  the four re-pinned rows read the same on Linux and on Windows, byte for byte. The other two
  `#[ignore]`d steps of CI's `tacsim-minds` cell pass on Linux unchanged.
- **`tests/domain_world/`**, all 263 stories, among them. Three older stories were re-timed
  for the one think the hail and the raider's torch now take (`sfr6`, `sfr8`, `sfr20`), and
  one damage figure was re-pinned for the volley that no longer lands (`sfr15`).
- **`mush-contract-tests`**: 203 on Windows, 195 on Linux (eight are skipped there), with no
  budget re-blessed.
- `cargo clippy --all-targets -D warnings` on `tacsim` and `mush-contract-tests`, and
  `cargo doc -D warnings --document-private-items` on `tacsim`.
- **CI** run 43142475684 on `23b4ffcae`, the code as it stands, on main at `330a8b249`: every
  job green, the ten `tacsim` integration shards and `tacsim-minds` among them.

**The harness baseline, re-blessed by record** (doc 43 rule 8;
`crates/tacsim/tests/harness_baseline/arena.linux.json`, `mw9_deterministic_update`). The
dispatch-only harness job failed its delta on five fields of the deterministic block. A
deterministic-only update, as CF.0's and CF.6b's were; TS.21's timings and host stand.

| Field | Was | Is |
|---|---|---|
| `counters.ledger_bytes_streamed` | 1650150 | 1650916 |
| `minds.fill_authored.final_substep_hash` | 11296100550182435994 | 5028223047829299195 |
| `minds.stock_ci.final_substep_hash` | 10958828204349461749 | 2814354088987883261 |
| `minds.stock_authored.final_substep_hash` | 11620081165489262692 | 1771993770098865287 |
| `minds.ceiling_authored.final_substep_hash` | 13684314779734303388 | 1115581876497461230 |

- **Why the hashes moved.** Each folds the seals, and a seal's digest covers the ABI and the
  Standard source.
- **Why the byte count moved, and that nothing else did.** A ledger line writes the state
  hash as a decimal, and the hashes are different numbers with different lengths. Proved on
  conclav.dev, the blessing host, by dumping the arena lane's streamed ledger at main
  (`330a8b249`) and at this unit (`297e6161d`): 9,324 lines each, 9,323 hash values each, no
  line differs once `hash` and the `mind_sealed` lines' `source` are taken out, and the 766
  bytes are the digits of the hash values alone (180,132 to 180,898). The dump was a patch
  to the harness that was never committed. No count, outcome or stopping time moved: the
  other 101 fields of the deterministic block and every other field of the four minds
  blocks read equal.
- **Read on two hosts.** conclav.dev (by hand as `ghrunner`, nice 10, `HOSTNAME` unset) and
  GitHub's runner (dispatch run 43146168987) read the same five values. Against the updated
  file, on conclav.dev with the timing gate live: the deterministic block equal (106
  fields), the four minds blocks equal, p50 −0.5%, p99 +4.6% and the window p99 +6.1%, and
  the harness binary's 24 ordinary tests pass,
  `every_committed_baseline_agrees_on_the_deterministic_block` among them.

**The dispatch-only stories** (run 43146168987 on `297e6161d`; the pull-request CI does not
run them).

| Job | Result | Whose |
|---|---|---|
| 16b cut H TacSim harness | failed its delta | This unit's: the re-bless above |
| M8 tier 2, held minds sealed aboard | failed: `close_ic_mind` "is not currently offered" | Not this unit's. Main's dispatch at `bc52bf278` fails the same story; PR #554 (open) fixes it, and its branch's dispatch passes it |
| Release-profile build check (nightly) | failed: `deploy/instance/render.sh: INCOMPLETE … instance.env` | Not this unit's. Main's dispatch at `bc52bf278` fails with the same line; this unit touches nothing under `deploy/` |
| The M6 replay and author story, the M7 stories, the other M8 stories, the leak battery, the House doctrines' trials, the bot's round and win rates | passed, or still running when this was written | See the PR for the run on the head commit |

**Not verified.**

- **The `#[ignore]`d tests that no workflow runs** (the by-hand lanes: `minds_pool`'s gate
  lane in release, the cost lane's cohort, the field pulse at scale). Any of them that pins
  a hash that folds a seal has moved with the ABI and is not re-pinned here.
- Nothing was run against a live server or over SSH. The stories drive commands in the test
  rig.
- The think's cost. The raider's extra think a step is not measured; MW.11 measures.
- A sealed player mind on the backup seat: no verb seals one on a ship, so the seat's rules
  for it (`BurnRule`, `FireRule`) are pinned by the applier's unit tests only.
- An interdictor bubble with three or more carriers, and a picket bubble where the corvette
  is lost while a carrier is hailed (the stop then reads `none`; not pinned).

## MW.10

Not built.

## MW.11

Not built.

## MW.12

Not built.

## MW.13

Not built.

## MW.14a

The compiled NPC commander (doc 44 §7.1, §4 D, §4c rows 14 and 15; Q-44-20, Q-44-44; doc 27
FO-15, Q-D4, Q-F1). Built on Q-44-44's default: a host-side issuer, compiled and deterministic,
with no LLM and no change to the mind ABI.

**Built.**

- **The rule, in one file** (`crates/tacsim/src/detachment/commander.rs`). `View` is all a
  commander decides on: what its own frame holds (`Sight`: the live contacts its party's book
  holds Hostile), the state of its roster (`Strength`: hulls, and those lost in this field) and
  the detachment's work point. `decide` is the whole rule, first match wins: **withdraw** once
  the roster has lost a hull (`WITHDRAW_AT_LOSSES`, 1), **hold** while a Hostile is in sight,
  **advance** to the work point otherwise. `Posture` is what it may ask for. The types are plain
  data in and plain data out, with no handle on the world, the clock or the dice, which is the
  shape MW.14b's doctrine and a House's policy doctrine (Q-44-32) take.
- **The issuer** (`crates/tacsim/src/detachment/sortie/commander.rs`, called from the NPC walk
  in `sortie/npc.rs`). When a roster's field command names an NPC its leader and that NPC's unit
  stands in a running field, `step` builds the view, takes the posture and gives it as a roster
  order to the first hull that does not yet have it. It goes through a player leader's delivery
  and nothing beside it: submitted as the NPC, fenced by the roster's revision and term
  (`field_orders::Act::Issue`); Org's mission rights read for the NPC over each unit; a
  `mush_propagation` envelope to the recipient, which interference holds; the recipient's own
  executor and the kernel's own checks (`field/orders.rs`, `arena/detachment.rs`, both
  unchanged). The order book is the commander's only memory, so a restart derives the same next
  order. The one thing kept in RAM is scheduling: an order just submitted to a slot is not
  submitted again for `RESEND_AFTER_S` (60 wall seconds), and an attempt that submitted nothing
  is not repeated for `RETRY_AFTER_S` (5), so one refused at its final take or answered NEGATIVE
  is retried at that pace and the commander's own hull flies its order in the fires between.
  Wall time, because at the live rate of 24:1 an IC minute is under three fires.
- **What the commander sees** (`crates/tacsim/src/field/orders.rs`): `sight` reads the Hostile
  contacts off the last frame the NPC's own seat was sent; `lost` says a unit is lost in its
  field when its operator's own frame shows a wreck or the field's kernel has named it
  destroyed.
- **A lost hull stops leading at once** (`sortie/field.rs`, `reconcile`): the roster's command
  facts read `lost` beside the world's unit record, which is checked out and stale while a field
  runs. Before this a commander destroyed in a field stayed the roster's leader until the
  field's close, and the kernel refused every order it gave (`NoCommander`). The fact is the
  roster's, so it holds for a player leader too: a player whose hull is destroyed in a field
  stops leading, and the next eligible player or the NPC commander leads.
- **The successor** (`crates/mush-tactics/src/detachment/roster.rs`, `reconcile`, and
  `roster/checkpoint.rs`, `restore`): while the nominated commander's own unit is lost or
  disabled, the first other NPC in slot order that is healthy and authorized leads under a new
  term, and the commander leads again once its unit is not. A commander who is only absent
  (still aboard in ramp transit, out of its seat) or only lacks the command right is not passed
  over; a roster that nominated no commander never elects an NPC; an active player outranks
  both.
- **Yielding to a returning player** needed no code: the roster's own rule (doc 27 Q-D4, the
  return hysteresis) makes the player the leader, `step` finds the NPC is not, and what the NPC
  had in flight under its term is discarded by the poll as any deposed leader's is.
- `docs/help/detachments.md` says what an NPC leader does. Doc 27's Q-D4 carries the successor
  as an amendment.
- **`roster_rig`** grew what the stories needed: `fly` takes a roster of two hired NPCs
  (below); `grant_command` issues the mission-role grants `+sortie/command-grants` prints, each
  under the organiser's own command loan; `orders_of` reads the roster's order book.

**Tests.** In-source: the rule's three arms and their order, and the same view giving the same
posture (`detachment::commander`); each posture as the roster order a player types
(`detachment::sortie::commander`); the successor, its restore, and a player outranking it
(`mush_tactics::detachment::roster`, two tests). Stories in `domain_field`
(`crates/tacsim/tests/domain_field/field_mw14a.rs`), each passing alone on Windows:

- `mw14a_an_npc_led_detachment_advances_holds_and_withdraws` (108 s) and `…_at_512` (89 s). A
  captain hires Sana Okonkwo and Ilya Mendes, names Okonkwo the commander and launches both
  from aboard the carrier; no player is in the roster's units. An escort and an intruder stand
  on the ground and the escort's fire order promotes the site. With nobody typing an order:
  Okonkwo orders both hulls to the work point and both drive for it; staff jam the roster's
  comms and the intruder fires on the roster's carrier, so her HOLD is admitted and sent and
  does not arrive, and both hulls keep their ADVANCE; the jam is cleared and both hulls stand;
  the intruder destroys Okonkwo's rig, and Mendes leads under a new term, orders the withdrawal
  from his own unit and drives for the ramp.
- `mw14a_a_returning_player_takes_the_lead` (26 s). A captain leads from rig 1 with Okonkwo,
  the plan's commander, in rig 2. While the captain leads she gives no order. The captain's
  session ends: she leads and orders the advance, to her own hull only. The captain returns:
  after the hysteresis the lead is theirs, their `ORDER DT:… 2 HOLD` is delivered and obeyed,
  and she gives no order under their term.
- `mw14a_the_same_run_gives_the_same_orders_twice` (55 s). The script to the HOLD on two fresh
  worlds delivers the same orders, with the same sequences, from the same slot.

**Divergences and defaults.** None needs a refactor to undo; each is one function or one filter.

- **The work point is a default.** A plan names no objective, so the commander advances to the
  site cell `WORK_POINT_CELLS` (6) out from the flight's ramp, straight away from the pad. A real
  objective is a field on the plan, which is a plan-format change and was not made.
- **An NPC-led roster holds only when it is fired on.** Only a roster's current leader may mark
  a party Hostile (`field/parties.rs`, `current_leader`), and an NPC marks nobody, so its book
  holds a party Hostile once that party's fire arrives on the roster or on a friend. The story's
  intruder fires on the roster's carrier for that.
- **"The next NPC in the roster's order"** is the first other NPC by slot number, not the next
  after the commander's slot. Its contract scope is not checked: Mendes, hired as a subordinate
  operator, succeeds Okonkwo. That amends doc 27 Q-F1's "explicitly assigned" for the one case
  of a commander who cannot command, as Q-44-44 ruled.
- **Succession is on the commander's unit, lost or disabled**, and nothing else. A commander
  who is absent for another reason is not succeeded and the roster has no NPC leader, as before;
  otherwise the successor would lead for the seconds the commander spends on the ramp at every
  launch. The lead comes back when the commander's unit is whole again.
- **One order a slot, never `ALL`.** An `ALL` is refused whole when any slot is unavailable, and
  a principal acts once a fire, so a posture reaches `n` hulls over `n` world seconds.
- **Only inside a running field.** On open ground an NPC leader gives nothing, and the surface
  executor goes on running whatever was delivered.
- **The link loss in the returning-player story is the session's facts set by hand**
  (`set_connected`, `set_live_conns`), as `mush-server`'s leadership story sets them. What the
  field does with an absent pilot's hull is MW.7's and is not exercised.
- **The jam is staff's** (`@prop/jam <place>=4` at the crew's place). No player-held jammer
  reaches the IC transport.
- **The stories run at 1:1**, the field rigs' rate, as every field story does. The one IC window
  in them is the roster's three-IC-second return hysteresis, waited out in full; at 24:1 it is
  an eighth of a second.
- **A Dray-P berths two, and the acceptance needs three people aboard.** The organiser and two
  hired NPCs fit only because a person in a rig's cab holds no berth: the organiser boards,
  takes rig 1's cab, the crew board, the plan is drafted and activated, slot 2's NPC is seated,
  the organiser steps out (`+tv/1 leave`) and slot 1's NPC is seated, all at Ceres before the
  hull sails. Every step is a player's command, and `roster_rig::fly` pins the sequence. **For
  the owner:** no help page says so, and if a cab should count against the berths then no
  shipped hull can fly a two-NPC detachment with its organiser aboard.
- **A lessee's mission-role grant needs its parent.** `+sortie/command-grants` prints a draft
  with `"parent": null`, which Org refuses from a lessee; the rig adds the lessee's own command
  loan. The help page already says a lender supplies its parent, and the printed line does not.

**Not built.**

- MW.14b, the Rhai commander.
- **The organiser cannot read what their NPC commander ordered.** `+sortie/orders` shows the
  reader's own outbox and an NPC's orders are in nobody's but its own, so the stories read the
  roster's order book as typed state.
- Any posture but the three: the commander gives no ENGAGE, SCREEN, SUPPORT or MINE, holds no
  fire, and marks no party.
- Damping. A Hostile that drifts in and out of the commander's sight flips the posture between
  HOLD and ADVANCE as often as once a world second.
- A commander's orders on open ground, and a plan objective.

**Not pinned by a story.** A restart with an NPC leading. The commander taking the lead back
from its successor (the roster's unit test has it). The ADVANCE returning once the Hostile is out
of sight. The withdrawing rig boarding its carrier (the story ends with it driving for the ramp).
A successor refused for want of the command right (unit test only). A posture answered
NEGATIVE and sent again a minute later, and an order refused at its final take and retried: the
resend guard has no test of its own. Whether a destroyed hull keeps its field mark until the
field closes was not checked beyond the acceptance story, where the successor's WITHDRAW stood
for the half minute the story watched; if the mark were cleared earlier the roster would read no
loss and the posture would go back to HOLD or ADVANCE.

**Reviewed.** A read-only review of the engine diff by a second agent found six points, four
fixed in the PR's last commit: the resend guard was in IC seconds, which is no pacing at 24:1; a
posture answered NEGATIVE counted as standing and was never sent again; the work point's clamp
panicked on a site that reads no side; and an attempt that never reached the queue was recorded
as sent. The fifth is the missing damping above. The sixth was two doc comments.

## MW.14b

Not built.

## MW.15

The story doc 36 owed for SCREEN and SUPPORT flown as delivered orders (doc 44 §7.1; doc 36 cut
GC.0's "Not built"; Q-36-42). No engine code changed: the cut is a test rig and a story.

**Built.**

- **`roster_rig`** (`crates/tacsim/tests/support/field/roster_rig.rs`), the rig that flies a
  detachment's roster into a field in tacsim's own world harness. `fly` is `mush-server`'s
  `pair_field_fixture` on `driving_rig`'s composed world, every step the production command:
  the hire at the Slack Line (`+sortie/crew`, `+sortie/hire`), a leased prospector a slot with
  its operational loans (`+tv/lease go`, `+tv/<n>/permissions go`), the crew brought aboard
  (`+sortie/board`), the voyage to the site, the sub-loan to each NPC
  (`@unitgrant/issue … operate_crew parent=<loan>`), the leader's own boarding
  (`+tv/<n> board`, `+focus`), the plan (`+sortie/draft`, `+sortie/accept`, `@station/handoff`,
  `+sortie/ready`, `+sortie/activate`), each NPC's seat (`+sortie/seat`), the launch
  (`+sortie/launch`) and the refresh until the field command names a leader.
- Its parts are what MW.14a's story needs next: `Plan` takes any billets (a `Leader`, a `Hired`
  crew member by name) and a `commander` by name; `commit` lands whatever a line queued on the
  composed path whoever owns the action (tacsim's, Org's unit grants, Sim's helm hand-off, which
  `driving_rig::drain` drops); `apply_due` lands a due scheduled action on the composed scheduled
  path (Sim's `control_handoff_apply`, the lander doc 36 said such a story needs); `beat` is one
  world second of both; `fence` and `reply` read the order prefill and a sent order's status off
  `+sortie/orders`, as a player's client does.
- **The story** (`crates/tacsim/tests/domain_field/field_mw15.rs`), at 256 on Ahuna Flank and at
  512 on Urvara Reach. A captain leads a mining roster from rig 1 with Ilya Mendes hired into rig
  2. A second captain's armed hull escorts and a third party's armed hull stands off it; the
  escort's `+field/fire` promotes the site and three parties are admitted. Then, by the leader's
  own `ORDER DT:… ` lines:
  1. `2 SCREEN 90`: the order is delivered, stands on Mendes's own frame, the rig turns onto 90
     degrees and the WILCO comes back. The leader drives four cells off and the screen stays on
     its cell and its bearing;
  2. `2 SUPPORT 1`: the rig leaves the cell it screened from, closes to its station on the
     leader's rig, and answers WILCO;
  3. `2 SCREEN 270`: the rig turns onto the new bearing, so the bearing it holds is the order's;
  4. `2 SUPPORT 2` (its own slot): NEGATIVE, and the screen goes on standing.

**Tests.** `field_mw15::mw15_a_rosters_screen_and_support_are_delivered_and_obeyed` and
`…_at_512` in `domain_field`. Local, Windows: 85 s and 81 s, each passing alone. GC.0's two
in-source gates are untouched.

**Divergences.**

- **One NPC flies both orders, in turn.** A Dray-P's bay holds two rigs, so a roster with a
  player leader has one hired NPC (`fly` refuses a third billet). The row's "the screening unit"
  and "the supporting unit" are the same rig under two successive orders, with the screen's hold
  checked while the leader moves away and the supporter's close checked after it.
- **"Keeps to its charge" is the doctrine's station, with the motor's tolerance.** The supporter
  ends within `SUPPORT_STATION_CELLS` (5) of its own plot of the charge plus the kernel motor's
  arrival tolerance; the story allows two cells for that.
- **The escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on` and
  `seat`), not the content's raider on its doctrine, so nobody fires after the opening order and
  the story does not hang on the field's dice. Three parties stand in the field; only the roster
  is under test.
- **The rigs are stood on their cells by a staff hand** after the launch, as every field story
  places its units.

**Not built.** A SUPPORT whose charge is another NPC's rig (it needs a third bay), and SCREEN or
SUPPORT under fire with a Hostile in the screen's arc: the story is about delivery and obedience,
not the stand the doctrine takes. A restart with an order standing.

**Not pinned by a story.** The jammed and the damaged carrier relay holding these two orders back
(FR.5's stories pin that for the orders they fly). A SUPPORT refused for a charge in another party
or outside the field, which GC.0's in-source gate covers and this story covers only for the
recipient's own slot.

## MW.16

Surveyed as [doc 45](45-the-absent-player.md): every way a player can be absent while something of theirs is exposed, what the game does and what an attacker can do today, checked in the tree.
Twenty-five owner questions, Q-45-1 to Q-45-25, open at their defaults; no units of work until they are ruled.
PR #533.

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

Doc 23's cut S.4c: lease and organisation authority in the title fence, in the Roll and at the
dispatch (doc 44 §7.1, Q-44-39; second review, §4c row 2). The design note is doc 23 §18
(D-23-24), written first.

**Built.**

- **One rule, in the Roll.** `mush_roll::commerce::ic_claims::hull_authority(world, hull, actor)`
  (`crates/mush-roll/src/commerce/ic_claims/hull.rs`) answers by which of three arms a character
  may open a claim on a hull, or why not. The title must be live. Then: the character's own
  unscoped **title**; a **lease**, read from the economy alone (the title carries its holder's
  `Lease` claim, and that contract is Active, lease-shaped, issued by the holder, pledges the
  hull and names the character a party); or an **office** (the hull is an organisation's and
  the character holds a live `roll.holder.org` grant on the title's scope, asked as a read).
- **The Roll's reader captures it.** `TitleFence` (`ic_claims/reader.rs`) keeps the arm with the
  title's revision and re-reads both at the fresh check, so a lease that ended or a grant taken
  back between the read and the commit is stale. The reader's definition says
  `"hull":"current_title_lease_or_office"`.
- **The dispatch calls the same function.** `untitled_refusal` (`surface/dispatch.rs`) refuses at
  `go` exactly what the Open would refuse for want of authority, and names the hull and its
  title's holder: "{mark} is {owner}'s hull; a licensed mind flies a hull titled to you, leased
  to you, or held by an organisation you act for." The dispatch's own copy of the rule
  (`titled_to`) is deleted. The bot's pinned phrase is still inside the sentence.
- **An organisation's owner licence opens a claim** (doc 23 Q-23-19's path, which a House's
  licence on any hull needs). The reader's fence over an owner licence's charter accepted only
  the dispatcher's personal title; it now accepts the arm the owner projection proved, a House's
  charter by the acting character's office. The claim history's audit (`ic_claims/audit.rs`)
  accepted only the charter's historical owner as dispatcher; for a House birth it accepts any
  dispatcher, as the paid arm already does for an organisation's licence.
- **The kernel admits an owner licence's term on a claim.** Found on the CI dispatch: a House's
  own licence passed the reader and all four owners and was refused at the last step,
  "IC captured term bounds". `mush-core`'s audit of a claim row
  (`action_state/terminal_capacity/ic_claim.rs`) capped every claim's captured term at 90 IC
  days; a House's owner licence runs 365 (17g C0 raised the Roll's owner bound, and nothing
  could open an owner licence's claim that long until now). The bound now follows the claim's
  kind: 365 days for a claim dispatched under an owner licence, 90 for a paid seat. The two
  bounds are constants in the kernel (`mush_core::licence_term`) and the Roll's
  `MAX_TERM_SECONDS` and `MAX_OWNER_TERM_SECONDS` are those values.
- **The reader's owner arm says why it refuses**, at debug on the `ic_claim` target, as its paid
  arm did; doc 17h's tier-2 and zero-PC stories print that target.

**Tests.**

- `mush-roll`, six tests in `ic_claims::hull` (`s4c_*`): each arm, the order they are tried in,
  the refusal naming the holder, a title that is not live admitting nobody, a claim that is not
  the titled lessor's, a grant on another scope.
- `mush-core`, `the_captured_term_is_bounded_by_the_claims_kind`: an owner licence's claim with
  a 365-day term parses and a 366-day one does not; a paid seat's with 90 days parses and 91
  does not.
- `tacsim`, in source: `s4b1_a_licensed_mind_needs_the_hull_titled_where_claims_open`
  (`surface/dispatch/tests.rs`), rewritten: nothing is read without the claim path.
- `tacsim`, fast, on the production composition:
  `mw22_the_hull_rule_reads_a_real_lease_a_house_title_and_a_personal_title`
  (`domain_field::m8_world`) asks the rule of the M8 world's own rig as `+tv/lease go` left it
  (the lessee admitted by the lease, a stranger refused naming Occator), then titled to a House
  (refused naming the House; admitted by the office once granted), then titled to the pilot.
- The acceptance story, `#[ignore]`d, on the CI dispatch:
  `s4b2_a_licensed_rig_opens_its_claim_in_the_bay_and_closes_it_home`, rewritten. Through
  registered commands, with a non-GOD pilot: (1) a chartered pilot launches her paid Pin on her
  leased rig, the claim opening in the bay and the open line carrying the claim's seal; (2) the
  one seat refuses the second leased rig; (3) with the Pin's resale accepted while that claim
  is open, the second rig is refused by the pending sentence; (4) the recover lands the Close;
  (5) on a rig titled to a House she holds no office with, she is refused by the sentence,
  naming the rig; (6) on a rig titled to the Coprates House she flies the House's own owner
  licence, the House the holder and she the acting principal, the server restarts with that
  claim open, and it closes at home.
- Doc 17h's stories with the claim path composed, on the CI dispatch:
  `m8g_story_tier2_held_minds_seal_aboard`,
  `m8g_story_zero_pc_procurement_buys_the_best_two_under_the_cap` and
  `m8g_story_the_combined_world_rates_files_lists_and_sells`.

RESULTS_PLACEHOLDER

**Divergences.**

- **A change in `mush-core`, outside the row's files.** The term bound above is the kernel's. It
  is one bound in one audit, with its own test, but it is the engine's and not the Roll's or
  the dispatch's. Without it a House's own licence cannot open a claim on any hull, and doc
  17h's tier-2 story and the leak battery fail with the claim path composed.
- **The lease is read from the economy's contract, not from `TvLease`.** Doc 23 Draft 0.10
  sketched a `LeaseFence { lease, lessor, lessee, term_ends_ic }` captured from TacSim's rig
  lease record. The Roll cannot read a TacSim record, and the economy already holds the lease as
  a contract and a claim on the title, so the rule reads those. Doc 23 §18.2 records it.
- **The claim row does not record the arm.** Draft 0.10 has the lease "audited like the title's
  revision". The row keeps the title's revision, as before, and nothing else new: the row's wire
  is mirrored field for field in `mush-core`'s terminal-capacity protocol, and M8c S2 already
  set the precedent that the row records the dispatcher and not the grant.
- **The fence is asked as a read.** `roll.holder.org` is amount-bearing; flying a hull moves no
  money, so a grant with a spending limit admits (M8f's P2a ruling). An organisation's licence
  itself is still asked the way M8c built it.
- **A House is "an organization" in the refusal.** The sentence names the title's holder with
  the port's own helper (`port::claims::holder_name`), which names a character and a lane
  organisation (Occator) and calls any other organisation "an organization". So a pilot refused
  on a House's hull reads "OWP-TV-01 is an organization's hull; ...": the rig is named, the
  House is not. Naming it means deciding which organisations a player may be told the name of.

**Defaults that would be a significant change to undo.**

- **Recording the arm on the claim.** If an inspection or a dispute ever has to say on what
  authority a hull was flown, the claim row, the admission digest and `mush-core`'s mirror of
  the row all change, and every stored claim with them.
- **A forfeit rig lease still admits.** The Roll reads the economy's contract, which stays
  Active after Occator captures the deposit; the launch verb does not refuse a forfeit rig
  either. Refusing a licensed mind there means a TacSim fact in the Roll's rule, or a second
  rule at the dispatch that the Open does not share.

**Not built.** Hull cover still sells only over a hull titled to its dispatcher
(`coverage/reader.rs`); no production launch composes it. The Trust's charter of a ship has the
lease arm's shape but no ship opens a claim (D-23-12). No verb lets a House name which officers
may fly which hull: the grant is the organisation-wide one its licences use.

**Not pinned by a story.**

- A lease that ends, or a grant taken back, **while the claim is open**: the design says the
  claim stays open until the epoch closes; no story ends a lease mid-claim. No in-game verb
  takes a `roll.holder.org` grant back.
- A rig repossessed where it lies on a site with its claim open: not traced (doc 23 §18.4).
- The reader's stale fence (authority lost between the read and the commit) end to end; the
  unit tests pin the rule, not the race.
- A character-held owner licence flown on a leased rig, and an organisation's paid Pin on an
  organisation's hull: the story flies a personal paid Pin on a lease and a House owner licence
  on a House hull; the zero-PC story flies a House's paid Pin on a leased rig.

## MW.23

Not built.

## MW.24

SSH `mind push`, `mind pull` and `mind check` without a presence (doc 44 §7.1; §4c row 23;
Q-44-35 to Q-44-37, whose hand-back at reconnect is why this had to land first, §7.4).

**Built.** A **bare connection**: one bound to a character so that an invoke on it runs as them
and its reply reaches it, and nothing else. It is carried end to end as its own pair of
messages, not as a flag on `ConnUp` (see Divergences).

- **The engine** (`crates/mush-core/src/engine.rs`). `Inbound::BareUp { conn, actor }` and
  `Inbound::BareDown { conn, actor }`. `bare_up` binds the connection in the world
  (`World::bind_connection`, the one thing `handle_invoke` checks) and records it in the engine's
  connection map with `ConnInfo::bare`. It does not touch `conn_count`, `set_live_conns`,
  `set_connected` or `session_start`, announces nothing, and fires no hook and no world event;
  `bare_down` undoes exactly that. So `World::is_connected`, `has_live_conns`, `connected()`,
  `Engine::connections_of` and `WHO` cannot see one. `Engine::actor_of` does answer for it,
  which is what the gateway's readiness probe asks. A `BareDown` for a connection that is not
  bare is ignored, and a `BareUp` on a live connection's id is ignored, so a stray frame cannot
  take a player's real connection down or over. `conn_down` on a bare connection is silent
  (a link that ends takes everything it owned down as `Lost`), and `session_gone` leaves a bare
  connection bound, so an exec that outlives its player's grace still gets its reply.
- **The wire** (`crates/mush-ipc`). `WireInbound::BareUp` and `BareDown`, mirrored field for
  field. Two new `ToServer` arms, so **`IPC_VERSION` 13 → 14**, the first bump inside doc 33's
  window: the golden set `tests/golden/v14/` is recorded (34 `ToServer`, 43 `ToGateway` frames;
  the two new samples are last among the `Inbound` ones, so every older frame keeps its number)
  and the v13 set still decodes. Nothing is gated toward the gateway (no `ToGateway` frame
  changed). **Deploy order is the window's: the server first.** A 14 server serves a 13 gateway,
  which never sends either frame and attaches its exec in full as before; a 14 gateway refuses
  a 13 server. 12 leaves the window, so a gateway that sends no `Hello` is now refused.
- **The server** (`crates/mush-server`). The two frames are classed with the lifecycle
  messages in `world_task/mode.rs` (`ConnectionHooks`, held in the lifecycle slot behind a
  pending commit, allowed in an outage countdown), so they keep `ConnUp`'s ordering against a
  commit and against the invoke behind them. `LinkRegistry::claim_bare` (`links.rs`): the link
  owns the connection for routing and a duplicate id is refused as for any other, but the claim
  does not clear a `SessionGone` the link owes for that actor, and a link that ends with a bare
  connection up does not reap its actor on that account.
- **The session manager** (`crates/mush-session/src/manager.rs`). `attach_bare`,
  `detach_bare`, `bare_conns`, `character_of_conn`. A bare connection lives in its own map:
  no session is made, joined or revived, the link-dead epoch is not read or written, the
  link-dead observer is not called, and it receives nothing addressed to the character, a
  session or everyone. `send_to_conn`, `conn_handle` and the flood bucket work on it by id.
- **The gateway** (`crates/mush-gateway`). `GameBridge::enter_bare`: `attach_bare`, a
  `BareUp`, then the same exact readiness probe a login makes (`ServiceReq::ConnectionReady`);
  a refusal sends a `BareDown` and undoes the attach. `GameBridge::disconnected` ends a bare
  connection with a `BareDown` and nothing else: no `ConnDown`, no reaper. `SshBridge::connect_exec`
  makes the same ownership check as `connect_character` and then `enter_bare`; the exec is
  still watched by a gateway drain (doc 33 §4.6). After a server restart `resync_boundary`
  sends a `BareUp` for each bare connection behind the roster and ahead of the barrier, so the
  invoke the gateway replays from input custody finds its binding and an exec caught by a
  restart completes, as it did when it was an ordinary connection. `ReAnnounce` itself is
  unchanged, and a bare connection is never in it.
- **The SSH transport** (`crates/mush-net-ssh`). `SshHooks::connect_exec`, called by
  `start_mind_exec` in place of `connect_character`. Who may run the exec is untouched: verified
  public key, an owned character.
- **The doorway** (`crates/mush-sshdoorway/src/server.rs`). The pump writes an injected line
  only into a channel whose client asked for a shell (`PumpCtx::shell`, set by `shell_request`
  before it relays, and at once for the menu's channel). Until then the lines wait in the
  queue; in an exec or a subsystem they are never written. The session is still closed at the
  drain's deadline.

**Tests.** All run on Windows and Linux unless marked.

- `mush-core`: `a_bare_connection_is_never_presence` and
  `a_bare_frame_never_touches_a_live_connection` (`tests/module_api.rs`; the toy module now
  records the `revived` and `linkdead` hooks as well): offline, link-dead, grace expiring under
  a bare connection, a `Lost` teardown, and the six hook and event lists compared before and
  after. `engine::typed_invoke_tests::an_invoke_runs_over_a_bare_connection`.
- `mush-session` (`tests/session.rs`): `a_bare_connection_has_no_session_and_leaves_none`,
  `a_bare_connection_does_not_touch_a_linkdead_window` (four minutes in; the expiry instant is
  compared), `a_bare_connection_beside_a_live_session`.
- `mush-ipc`: the round trip and conversion cases gain both frames; the golden suite; the
  version pins moved to 14. `window::tests::a_frame_newer_than_the_link_is_withheld` is now
  written against the window instead of the numbers 12 and 13.
- `mush-server`: `links::tests::a_bare_claim_routes_and_owes_no_reaping`;
  `admit_gateway_pins_the_window_and_the_marker` now pins that a gateway with no `Hello` is
  refused. `tests/split_boundary/ipc_window.rs` is rewritten for the window as it now stands:
  a gateway one step behind is served in full, one at the server's version is served, one with
  no `Hello` and one a step ahead are refused.
- `mush-gateway`: `ssh_bridge::exec_tests` (four, against a link that records every frame):
  an offline exec sends `BareUp`, the invoke and `BareDown` and never `ConnUp`, `ConnDown` or
  `SessionGone`; an exec four minutes into a link loss leaves the expiry instant where it was;
  a server that does not bind it refuses and undoes it; a character the account does not own
  is refused before anything is sent. `resync_orders_full_roster_before_readiness_barrier`
  gains the bare connection's `BareUp` between the roster and the barrier.
- `mushd`: `mw24_a_mind_exec_is_never_presence` (`tests/world_journeys/ssh_e2e.rs`), over real
  SSH on the all-in-one server with a second player watching the room: an offline player's
  `mind pull` answers as them, nobody is told anything and `WHO` has no row; a connected
  player's own exec works; an exec halfway through a real link loss is announced as nothing,
  and the character is disconnected when the original grace ends, less than one grace after
  the exec.
- `mush-sshdoorway`: `a_drain_warns_a_shell_and_writes_nothing_into_an_exec`
  (`tests/e2e.rs`): a spliced shell reads the warning while a `mind pull` through the same
  doorway gets its one result line and no other byte.
- The existing SSH mind stories are unchanged and are the "still works" half:
  `mush-net-ssh` `tests/loopback.rs` (`mind_exec_*`) and `mush-bot`
  `arena_mind_author_story.rs`.

Run on the Windows box: the `mush-core`, `mush-session` and `mush-ipc` tests above; the
doorway's `tests/e2e.rs` (34 passed; with the pump's condition taken out the new test fails on
the warning in the exec's output, so it tests the fix); `mush-net-ssh` `mind_exec` (3); the
gateway's `exec_tests` and the resync test (5); the `mushd` story (with the old full attach put
back it fails on "Nick has connected."); `mush-contract-tests` (35 + 80 + 76); clippy with
warnings denied and rustdoc on the eight changed crates. Left to the Linux CI: everything in
`mush-server`, and the gateway's `tests/split_halves/*` and
`tests/split_boundary/unit_signals.rs` (`cfg(unix)`, so the `Hello` added to
`server_panic_relaunch.rs` was not even type-checked here). CI ran all of them green on the
first full run but one: `mush-server`'s
`ic_slice_loop::ic12_part_two_the_queue_runs_the_return` ("a sale is never negative"), a
trade story that builds no bare connection. It passed when its cell was run again on the
same commit, so it is a flake of that story's own (it ran 93 s on a loaded runner), owed a
look by doc 18's owner and not touched here.

**Divergences.**

- **Two messages, not a flag on `ConnUp`.** The row says "a flag from the SSH hook through the
  gateway bridge and the IPC message to `Engine::conn_up` and `conn_down`". A field on
  `Inbound::ConnUp` would have edited about 110 sites that build one, in fifty files, most of
  them other chains' tests, for a connection that shares none of `ConnUp`'s behaviour. As its
  own variant every exhaustive match had to choose what a bare connection is, which is where
  the server's classes and the link registry's rule came from. **Costly to undo** only in the
  sense that it is now wire vocabulary (version 14).
- **A bare connection takes no slot of the split gateway's lifecycle ledger**
  (`LifecycleCustody`). That ledger counts sessions and the `SessionGone` each may owe, and
  reserving in it converts a link-dead player's owed `SessionGone` into the new connection,
  which is the bug this unit removes. A bare connection is bounded by the SSH accept gates and
  by public-key authentication instead.
- **`SshHooks::connect_exec` has a default** that attaches in full, so the test doubles of four
  crates keep compiling and keep their old behaviour. The gateway is the one production
  implementer and overrides it.
- **A subsystem channel through the doorway gets no banner either.** The rule is "only a
  shell", which is wider than "not an exec" by that one case.
- **Bare frames are refused by a world mode that refuses connections** (booting, recovering,
  integrity-frozen): the exec is then refused at the readiness probe with "Character
  attachment was refused.", as it was before.
- **An exec no longer holds its character connected, so it no longer shields them.** The
  workshop cancels a character's authoring jobs when they go link-dead or disconnect
  (`WorkshopCommands::linkdead`). While the exec was a live connection, a player whose shell
  dropped during their own `mind check` was not link-dead until the exec ended; now they are
  at once, and that check is cancelled with their other jobs. In the other direction, the end
  of an offline player's exec used to fire that hook and now fires nothing: a job the client
  abandoned runs to its own bound and its result is dropped by the connection lease
  (`structured.rs`), which is what already happened for a player connected elsewhere.

**Not built.**

- A story on the split pair (gateway and server as two processes) for the exec, and one for an
  exec caught by a server restart. The resync order is pinned by a unit test; the replayed
  invoke reaching its binding on a fresh server is not run anywhere.
- A story that a ship or a field unit held by a backup mind stays held through an exec. It is
  true by construction (the space seat and the arena bridge act on the `connected`, `revived`
  and `linkdead` hooks and on `has_live_conns`, and none of those moves), and the engine test
  pins exactly those; no tacsim story flies it.
- The exec's refusal still reads "Character attachment was refused." whatever the reason (a
  restart gap, a busy world).
- A second shell channel opened after an exec on the same doorway connection gets no drain
  warning (the inject queue belongs to the first channel). The connection is still closed at
  the deadline.

**Not pinned by a story.** `BareUp` and `BareDown` behind a pending commit (the lifecycle
slot); a duplicate bare id across two gateway generations; a gateway drain's deadline closing
an exec in flight; a link's end with a bare connection up, end to end (the registry half and
the engine half are each pinned).

## MW.17b

Not built.
