# 45. The absent player — cut notes

One section a unit of [doc 45](45-the-absent-player.md) §7. A unit's session replaces its own
"Not built." line with what it built, under these headings: **Built**, **Tests**,
**Divergences**, **Not built**, **Not pinned by a story**. Each session edits only its own
section, so two units' notes never touch the same lines.

## AP.1

**Built.**

- **Who is told of a field** (`crates/tacsim/src/field/notice.rs`, new). A *stake* is a unit of
  the field and the players behind it: whoever sat in it at admission, the operator of a rig out
  on an excursion, a carrier's commander (`port::coop::helm_holder`), and its party's principals
  (the leader and, for a roster, the organizer). Players only. A player with nothing on the
  site holds no stake and is told nothing.
- **The opening.** The host's snapshot branch (`field/mod.rs`, `on_snapshot`) reads the stakes
  at the field's first snapshot, which the arena takes on the opening pulse. Each stakeholder is
  told `FIELD_OPENED` once: queued at trip level (`port_tick::notify_budget`) when they are not
  on watch, said when they are and the site's own line did not reach them (a commander
  elsewhere). A player the site's line reached hears it once, as before. A unit that joins the
  running field brings its players in the same way at the next snapshot.
- **The close** (`field/close.rs`, `on_ended`). Each stakeholder is told `FIELD_SETTLED` by the
  same rule, then what became of each of their units, one line a fate (`AFTER_FIELD`): `lost`,
  `captured, aboard another's carrier`, `taken in tow`, `down on the site, a casualty`,
  `holding on the site`, `home`, `gone from the site`. A player the close left stranded on the
  site, and who was not there to hear it, is queued the stranded line too.
- **Mail.** A stakeholder with a unit lost, down, captured or in tow, or stranded, is sent one
  letter listing every unit they had in the field, through the salvage claim's IC mail hook
  (`surface/claim.rs`, new `letter`, which `tell` now uses), under the site's clerk. A
  character taken captive when their casualty is hauled into a captor's bay
  (`field/crew.rs`, `captured_loaded`) is queued the capture's line when not on watch and sent
  it by the same mail (`notice::captured`). A salvage claim already went by IC mail.
- **The notice budget.** One field costs a player at most one opening line, one settling line,
  one line a fate (seven) and one stranded line of the reconnect queue's 32, however many units
  they have there: a fate's line names at most four units (`MAX_NAMES_A_LINE`) and counts the
  rest. One letter a player a field, listing at most 24 units (`MAX_LETTER_UNITS`).
- **The operation's notice** (`crates/mush-campaign/src/notice.rs`, new; `tick.rs`). Once an
  operation's terms are frozen the campaign tick tells each eligible defender the terms name as
  an object, once, and marks the runtime (`OperationRuntime::noticed`) in the same batch. The
  text is the notice lens and nothing else (`render`'s). It leaves through a composition seam,
  `CampaignModule::with_defender_notice`, which `crates/mush-composition/src/campaign_notice.rs`
  fills with the reconnect queue and `mush_mail::post_notice`. `mush-campaign`'s edge set is
  unchanged, and nothing in it reads a field.
- **`port_tick::on_watch`**: "connected, over a live transport", the test `notify` and
  `notify_budget` already made, named so the field asks the same question.
- `docs/help/field.md` ("While you were away") and `@op`'s help line.

**Tests.**

- `crates/tacsim/tests/domain_field/field_ap1.rs`, at 256 and at 512: three parties on a site.
  Away puts a carrier down, drives a rig out and logs off; Pilot opens a field with a fire order
  at the rig and fights it down; Third sits in a hull on the same ground; Nobody has nothing
  there and is offline. Away reads at login, under the reconnect heading and in order, the
  opening, the settling, the rig lost and the carrier home, each once, and has one letter naming
  the loss. Nobody has nothing queued and no letter. Pilot and Third heard the opening and the
  settling once each and their own hull's fate, and were sent no letter.
- `crates/mush-campaign/tests/ap1.rs`: nothing is pushed while an operation is only declared;
  an activation three IC days after the declaration stamps the notice at the activation with
  the whole 28 IC days ahead; each named defender is told once, in the terms' order, and a
  bystander and the issuer are not; ticks and a restart tell nobody again; the letter carries
  the window and never the plan. A covert operation tells nobody. With no seam wired the line
  is still said.
- Unit tests: `field::notice::tests` (a fate's line stays one row of the queue with forty
  units; a letter's rows and when one is owed) and `mush_campaign::notice::tests` (the letter
  is the notice lens).

**Divergences.**

- **The opening is told from the first snapshot, not from admission.** `field/admit.rs` is
  another unit's file, so nothing was changed there. The arena snapshots on its opening pulse,
  so the delay is one pulse. A field that ends before its first snapshot tells its
  stakeholders of the close only.
- **Seven fates, not five.** The ruling's list (home, holding, lost, captured, taken in tow)
  has no word for a casualty nobody has towed, nor for a unit that is no longer on the site and
  aboard nothing. They read `down on the site, a casualty` and `gone from the site`.
- **A connected stakeholder hears the fate lines too.** They are new lines, said once. The
  opening and the settling are still heard once.
- **The operation's notice is pushed at activation, and the notice period now runs from
  activation.** At `@op/declare` an operation names no defenders; they are added by
  `@op/terms` and can change until the terms freeze. So the push is made when they freeze. The
  notice stamp used to be the declaration's: an issuer who declared, waited and then activated
  gave a defender less than 28 IC days, or none. `@op/activate` now moves the stamp to the
  activation and the window by the same amount, derives the hash again, and tells the issuer
  ("Notice runs from now: …"). The lead the issuer set is unchanged. This is a default I took
  under the 28 IC day rule; it is one function (`commands::notice_from_now`) and cheap to undo.
- **A covert operation pushes nothing**, because `@op/show` shows its defenders nothing
  (`render::lens_for`). The push never says more than the lens.
- **Files outside the row.** `crates/tacsim/src/surface/claim.rs` (the letter, so the field
  uses the existing mail path), `crates/tacsim/src/field/crew.rs` (one call at the capture),
  `crates/mush-composition/src/` (the seam's function, where both crates are in reach).
- **Who a letter is from.** The field's letter is under the site's clerk where the site has
  one, else whoever the code runs as, which in the close and in the campaign tick is the
  system's own character. Neither names a player.

**Not built.**

- **Durable stakes.** They are RAM (`FieldBook::stakes`). A restart mid-field forgets the units
  that had already left at a ramp: the resumed field reads its stakes again from the units
  still in it, says no second opening, and the close reports those units only.
- **A casualty or wreck an earlier field left.** It enters a later field in a party nobody
  leads, and its title holder is not read, so its owner is not told of the later field.
- **The rigs in a wrecked carrier's bay** are reported only through the carrier's own `lost`
  line. An operator of one who is not the carrier's commander is not told.
- **Organizations as defenders.** `@op/terms …/defender=` names objects only, and the push
  tells objects only.
- **An operation's end.** Only the notice is pushed, not the resolution or an abandonment.
- **Other site lines.** A raid's landing, a carrier's arrival and a tow's lines are still said
  to whoever is there and not queued (`tell_site`'s other callers).

**Not pinned by a story.**

- The fates `captured`, `taken in tow`, `down` and `gone`, the stranded line, and the capture's
  queue and letter: the story has one loss and one unit at home. The line and letter formats
  are unit-tested.
- A unit joining a running field and bringing its player, and a restart mid-field.
- A roster's leader or organizer as a stakeholder: the story's stakes are an operator and a
  commander.
- The letters reaching a `mush-mail` inbox, and the operation's line reaching the reconnect
  queue. Both stories read the mail seam through a recorder: TacSim's test world composes no
  mail module, and `mush-campaign` cannot reach the queue. The two functions that fill the
  seams (`mush-server`'s `install_claim_mail` call, `mush_composition::campaign_notice`) are
  compiled and run by no test.

## AP.2

**Built.**

- **The window waits** (`crates/tacsim/src/field/standing.rs`, `at_demotion`). For a player
  leader who is not on watch when the field demotes (not connected, or linkdead), the grant's
  void window is written to end `VOID_ABSENT_IC_S` (28 IC days) after the demotion, where a
  connected leader's ends `VOID_WINDOW_IC_S` (600 IC seconds) after it. Nothing else about the
  grant differs, and standing moves exactly as before.
- **The offer runs from the return** (`LeaderReturns`, a `Connected` hook, on a connection and
  on a revival inside the linkdead grace). A leader with offers that waited is told
  `VOID_WAITING` ("While you were away your party's fire on a friend cost standing: …") and the
  IC second is stamped. `+field/void` takes a waiting offer until the leader's next command or
  600 IC seconds after that stamp, whichever is first, and never past the 28 IC days.
- **How a waiting offer is told from an ordinary one.** By what is left of it: an ordinary
  offer never has more than 600 IC seconds to run, so a window with more is waiting on its
  leader. No field was added to `mush-faction`'s receipt.
- **Unchanged:** a connected leader (the same window, the same lines, `field_fr2.rs`'s stories
  untouched); an NPC leader, who is never waited for; a director raid's own void.
- The 28 IC days are pinned by the player-wait floor guard
  (`crates/mush-contract-tests/tests/engine_contracts/player_wait_floors.rs`, 11).
- `docs/help/field.md`, under *Standing*.

**Tests.**

- `crates/tacsim/tests/domain_field/field_ap2.rs`, three parties on a site (Pilot and Other,
  Friends by a faction; Third, nobody's friend, seated and holding fire). Pilot's fire reaches
  Other, both stand down, and Pilot logs off before the field settles. At the demotion the
  change is Pilot's, bounded, naming Pilot as its voider, with 28 IC days on its window. Then:
  - at 256 and at 512, well past ten IC minutes: Pilot logs in, is offered the void and takes it;
  - at 512: Pilot logs in, lets ten IC minutes pass and is refused; logs in again a day later,
    is offered it again and takes it;
  - at 256: the clock is carried past the 28 IC days; Pilot logs in, is offered nothing, is
    refused, and the change stands.
- Unit tests in `field::standing::tests`: the waiting window's arithmetic beside an ordinary
  offer's, and that only a player who is not on watch is waited for (a linkdead player is; an
  NPC is not).

**Divergences.**

- **A lapsed offer is offered again at the next return.** The ruling says the window runs from
  the leader's reconnect, at most 28 IC days from the demotion. A leader who logs in, types
  something else or lets ten IC minutes pass, and later logs in again inside the 28 IC days is
  offered it again: the stamp is the latest return. It was the simplest reading of "from their
  reconnect", and it is one line to change (`LeaderReturns::returned`).
- **Linkdead at the demotion counts as absent.** A leader inside the linkdead grace hears
  nothing said to them, so they are waited for, and a revival offers the void.
- **The last ten IC minutes of the 28 IC days.** There a waiting offer is an ordinary one: open
  until its end or the leader's next command, with no line at login.
- **Stacked on AP.1.** The branch is cut from AP.1's, for `port_tick::on_watch` and so the two
  units' test-root lines do not conflict.

**Not built.**

- **Durable return stamps.** They are RAM (`standing::Returned`). A restart ends every session,
  so the next connection stamps again; nothing is lost that a player could notice.
- **The standing change itself is not queued for an absent principal.** `STANDING_CHANGED` is
  still said only to whoever is connected. An absent leader learns of it through the offer's
  line; an absent organizer who is not the leader reads it in `+standing`.
- **An offer for anyone but the party's leader.** As before, only the leader may void.

**Not pinned by a story.**

- A leader who is linkdead at the demotion and revives (the unit test covers the predicate, not
  the hook's `revived` arm).
- An offer that waited across a server restart.
- Several waiting changes at once (the line's count).

## AP.3

The posture record (doc 45 §7.1; Q-45-3, Q-45-4, Q-45-23). **Expensive to reverse**: every
mind's view carries the field, so the mind API moved (`api=8` to `api=9`) and the frame with it
(9 to 10). PR #565. Paths are under `crates/tacsim/src/` unless given.

**Built.**

- **The word** (`posture.rs`, new). `Posture` is `Return`, `Guard` or `Hold`, `Return` by
  default. A mind reads it as `return`, `guard` or `hold`; a player reads `RETURN`, `GUARD`,
  `HOLD`.
- **The record, on the unit** (`posture/record.rs`, new). One attribute on the unit's own
  object, `TACSIM.POSTURE`, owned by `GOD`: `{"posture": "guard"}`. A unit on `RETURN` has no
  attribute, so every unit that existed before reads `RETURN` and no stored format moved.
  `record::of(world, unit)` is the one reader and `record::set(ctx, unit, posture)` the one
  writer. An attribute `GOD` does not own is not the record, so a player cannot forge one on a
  hull they own; removing it sets the default they could always have.
- **The arena's copy** (`arena/posture.rs`, new; `arena/state.rs`, `arena/hash.rs`).
  `UnitState::posture`, written only by `Arena::set_posture`. It rides the snapshot and folds
  into the state hash **only when it is not `RETURN`** (hash item 1, sub-item 26), as MW.7's
  `absent` does, so `SNAPSHOT_FORMAT` stands at 19 and every match that never sets a posture
  hashes as it did. Nothing in the arena acts on it: the absent player's gate, the stock mind
  and the orders are untouched.
- **The frame, the keyframe and the view.** `Frame::posture` (`arena/step/frames.rs`), the
  keyframe's top-level `posture` (`match_/cockpit/frames.rs`, `FRAME_VERSION` 10), and
  `MindView::posture` (`arena/mind.rs`), built from the frame like every other view field. A
  changed posture is a changed frame (`Frame::same_as`), so a seat is sent it at once.
- **The getter and the ABI** (`minds/api.rs`, `minds/mod.rs`). `View.posture`, with its row
  in the parity table (151 rows), 286 names pinned, `API_VERSION` 9,
  `MIND_ABI = rhai=1.26.0;api=9;stock=3`. A getter and not a kernel, because the doc's words
  are "one more field in the view" and doc 17 D-UM-4's parity then gives the person in the
  seat the same word on the keyframe, which AP.4's status needs.
- **The field host** (`field/posture.rs`, new). The host copies each unit's record to its
  arena: every unit once when a task first holds it, then whenever its record differs from
  what the host last told, checked once a world second from the host's pass and at once when
  the record is written; and before a resumed arena's task starts (`field/boot.rs`), where
  the record decides over the snapshot's older word. The message is `ArenaMsg::Posture`
  (`arena_task.rs`), latest-wins a unit, applied at a pulse's head after that head's joins;
  a word for a unit whose join has not landed waits in the task for it. A task that is not a
  field's ignores it. A demotion changes nothing: the arena's copy goes with the arena.
- **Space** (`space/view.rs`, `space/backup.rs`). `space_mind_view` sets the view's posture
  from `Situation::postures`, which the backup seat fills from the hull's record; a hull
  outside every bubble (`tier1_view`) reads its record too. So a player's ship flown by the
  backup seat already reads the word a rig's mind does, and AP.9 does not move the ABI again.
  An NPC actor's hull (the corvette, the raider, the interdictor, a drone) has no row and
  reads `return`. The space doctrines read nothing of it yet.
- **The retained ABIs** (`minds/mod.rs` `RETAINED_ABIS`, `minds/metadata.rs`). See
  "Conformance" below.
- Doc 17 §3 and §24, and doc 45's AP.3 row, read as built.

**Where the posture lives, and what was rejected.**

| | Where | Verdict |
|---|---|---|
| Chosen | An attribute on the unit's object | Outside custody: a field, a bubble and a practice match check the unit out and in around it and never touch it. Present for every kind of unit (a rig, a carrier, a ship, a roster unit, a simulator hull all are objects). The precedent is the raider's standing withdrawal (`field/raiders.rs` `RAIDER_ATTR`), kept on the hull "so a restart keeps asking". No format version moves |
| Rejected | The unit's `mush-tactics` record (`TacticalUnit`, as a capability, like `tacsim.space.subsystems`) | The record is closed exactly when a posture is set. A deployed unit refuses every rewrite but its check-in (`mush-tactics/src/module.rs`, `encode_unit_document`'s admission sites), and a check-in that voids restores the snapshot taken at check-out (`custody::check_in(.., None)`), so a posture set from a seat in a field would be refused, or lost when the field is voided. It would also bump the unit's `revision` for a fact that is no part of the machine, and teach a rules-neutral crate a game word |
| Rejected | The host's own record (`FieldUnit` on the live record, the bubble's ship) | Q-45-23 rules it out: it would not survive the host |
| Considered, not taken | A typed record family of its own, written by an authoritative action | It is the lane new player acts use (`RigRecord`), and the object's attribute rides the older object-persistence lane (`CommandCompatibility::LegacyPostImage`: `@field/raiders` is classed so for the same attribute write). It needs a provider, an offer, a definition and a `go`/`Queued.` round trip for one word, and the relay's delivery (AP.4) would need a second action. **Cheap to change later**: nothing reads the attribute but `record::of` and nothing writes it but `record::set`, so moving the record is those two functions and a one-time read of the old attribute |

**The ABI move, and what was re-pinned.** One commit moves it (`b6d3fb7e5`; the re-pin is `ed5593464`, the baseline `644088223`). Then:

| Pinned set | Where | What moved | Why nothing else did |
|---|---|---|---|
| The nine default seal identities | `match_/seal.rs` `m5rules_default_seal_identities_are_pinned` | 9 digests | The same test pins the API 4 projection of the same nine seals (ABI, source and stock provenance put back), and that half still passes: no custody moved |
| The source identities | `minds/content.rs` `STANDARD_SHA256` (`42c694ed…` to `2bea2391…`), the five other stock doctrines' headers, the acceptance mind (`field_pilot`), three House envelopes and their manifests | the digest of each file whose header or envelope names the API | Each `.rhai` differs from main in the header's `api:` word and each envelope in its `mind_abi` string, and nothing else |
| Arena state hashes | `tests/domain_arena/` `minds.rs` (`m1a5_the_production_hash_does_not_move`, 3), `motor.rs` (1), `reflex.rs` (2) | 6 values | They fold the seals; the same runs' ledger lines are the next row |
| The `hash` of pinned ledger lines | `motor.rs` (34 lines), `reflex.rs` (50 lines) | 84 | A script over the diff: every changed line of the four files differs from main in a hash token alone (the `mind_sealed` lines also in their `source` prefix). No kind, unit, time, cell or count on any line moved |
| The cross-platform mind corpus (D-UM-28) | `minds_rhai.rs` `M2_CROSS_PLATFORM_PINS`, an `#[ignore]`d test CI's `tacsim-minds` cell runs | 4 rows, 12 values | The stopping times (180,250, 177,500, 170,000, 900,000), the probe's 3,600 thinks and the 369098751 bitmask are as they were. Linux (conclav.dev) and Windows (this box) print the same four rows |
| MW.5's Registry trial (doc 17g C4 item 4) | `minds/registry/trials/physical_tests.rs` `C4_TRIAL_PINS`; merged as #558 while this unit was in flight | 4 rows, 12 values | Every case still ends `Elimination { winner: Some(PartyId(66)) }/Loss` at 58,500 ms. The filed result of each case was dumped at main (`0bad68f0b`) and here: 31 fields each, and three differ, the end's state hash, the end's ledger digest and the coverage's witness (a rolling digest that folds each think's sealed program). The meter's figures and the think counts are equal. Linux and Windows read the same rows |
| MW.10's space cast (doc 44 unit MW.10; merged as #562 while this unit was in flight) | `content/space/picket_cast.json`, three `doctrine_sha256` | 3 digests | Each is the SHA-256 of a shipped doctrine file, and those three files differ from main in their header's `api:` word alone. MW.10's cut note names this as the one pin that moves with the bytes; its test `the_packs_cast_names_each_shipped_doctrine_by_its_digest` holds it |
| The harness baseline | `tests/harness_baseline/arena.linux.json`, `ap3_deterministic_update` | 5 fields (below) | The ledger dump (below) |

58 values came from one Linux run of the whole of `tacsim`'s tests by the script MW.9 left
(`mw9-rebless-pins.py`): 7 failed comparisons, 58 pinned values, the same seven tests MW.9
re-pinned. The 12 corpus values and the 12 trial values were read from each test's own print.

**The harness baseline, re-blessed by record** (doc 43 rule 8). A deterministic-only update;
TS.21's timings and host stand.

| Field | Was | Is |
|---|---|---|
| `counters.ledger_bytes_streamed` | 1650916 | 1651236 |
| `minds.fill_authored.final_substep_hash` | 5028223047829299195 | 16090300274155741832 |
| `minds.stock_ci.final_substep_hash` | 2814354088987883261 | 8502901592418194973 |
| `minds.stock_authored.final_substep_hash` | 1771993770098865287 | 3144024366270822576 |
| `minds.ceiling_authored.final_substep_hash` | 1115581876497461230 | 477923012376592334 |

- **Why the byte count moved, and that nothing else did.** A ledger line writes the state hash
  as a decimal, and the hashes are different numbers of different lengths. Proved on
  conclav.dev by dumping the arena lane's streamed ledger at main (`668425fab`) and at this
  unit: 9,323 lines each, 9,323 hash values each, 300 sealed-source prefixes each, **no line
  differs once `hash` and the `mind_sealed` lines' `source` are taken out**, and the 320
  bytes are the digits of the hash values alone (180,898 to 181,218). The dump was a patch to
  the harness that was never committed (`Counts::absorb` appending each chunk to a file).
- **Against the updated file**, on conclav.dev by hand as `ghrunner` (nice 10, `HOSTNAME`
  unset) with the timing gate live, `scripts/harness-weekly.sh` whole: the deterministic
  block equal (106 fields), the four minds blocks equal (102, 30, 30 and 34 fields), p50
  −4.2%, p99 +0.9% and the window p99 −1.2%.
- Every unit of the harness is on the default posture, which the snapshot and the hash spend
  no byte on, so the snapshot's sizes did not move either.

**Conformance: what "the Registry's existing path" is, and what it did when MW.9 moved the
ABI.**

- **The path.** The Registry's manifest digest covers the mind API's own source files
  (`minds/registry/manifest.rs` `implementation`: `api.rs`, `mod.rs`, the kernels, the fees),
  so any change to the API is a new compatibility. The automatic lane
  (`minds/bulk/registry_service/reconformance.rs`) scans the filed revisions, and for each one
  whose certificate is under an older compatibility prepares it as a re-conformance subject
  (`registry/subject.rs` `PreparedSubject::prepare_reconformance`) and rates it again on the
  engine as it stands; a subject that does not measure up is recalled with notice (doc 17
  §16). Separately, every boot audits every saved revision as an archive (`store.rs`
  `audit_mind_revisions`, through `authoring::RetainedRevisionRules`).
- **What it did at MW.9, and before.** Both doors ask
  `minds/metadata.rs` `validate_retained_registry` which ABI the revision's envelope names,
  and that function knew two: `api=4;stock=2` and `api=5;stock=2`. The ABI has since been 6,
  7 and 8, and nobody added a row. So when MW.9 moved `api=7` to `api=8`, a revision saved
  under API 7 was (a) **quarantined at the next boot** ("unsupported retained Registry ABI",
  its library reference disabled), and (b) **never conformed again**: `prepare_reconformance`
  failed at every scan, and a preparation failure is by design neither a rejection nor a
  recall, so the lane tried for ever. The same for API 6 at CF.0. Nothing was lost only
  because no server holds a filed mind yet.
- **What AP.3 built.** `RETAINED_ABIS` (`minds/mod.rs`) lists every ABI since the Registry
  opened, API 4 to API 8, and `validate_retained_registry` admits exactly those. A test
  (`every_abi_since_the_registry_opened_is_retained`) fails if `API_VERSION` is raised and the
  ABI it replaces is not added, so the next move cannot repeat this. Nothing else about
  retention moved: a retained revision is no fresh save, its header's `api` must be the one
  its envelope names, and it runs again only if its unchanged source compiles and thinks on
  the current sandbox.

**Tests.**

In source:

- `posture::tests`, `posture::record::tests`: the word, the default, the record's shape; an
  attribute `GOD` does not own, or one that is not the record's shape, is no record.
- `arena::hash::tests::ap3_the_hash_folds_a_posture_only_when_it_is_not_return`,
  `arena::posture::tests` (the snapshot carries it only off the default and round-trips; the
  frame and the view carry it, and a changed posture is a changed frame),
  `arena::snapshot` (`af1_a_unit_is_carried_whole_by_its_derive`, the field listed).
- `field::posture::tests::ap3_a_resumed_arena_takes_each_units_posture_from_its_record`: the
  record decides both ways.
- `minds`: `every_abi_since_the_registry_opened_is_retained` (the table's own rule),
  `authoring` (`every_abi_since_the_registry_opened_is_retained_and_none_is_a_fresh_save`,
  `a_script_that_reads_the_posture_saves_under_the_current_api`), the getter's read and its
  parity row (`api`), the 286 pinned names.
- `minds::registry::tests`:
  `ap3_a_mind_filed_under_any_abi_since_the_registry_opened_is_conformed_again` (for each of
  API 4 to 8: not a fresh rating subject, a re-conformance subject, no byte of it moves, and
  its unchanged source thinks on the current sandbox) and
  `ap3_a_script_that_reads_the_posture_is_rated_and_reads_the_units_word` (rated under API 9;
  its think answers `return`, `guard` or `hold` as the arena holds it).
- `store::tests::ap3_a_source_saved_under_every_retained_abi_survives_the_cold_audit`: saved
  through the real Roll and source union, then a cold boot: not quarantined, byte for byte.

Arena (`tests/domain_arena/minds_rhai.rs`):
`ap3_a_sealed_script_reads_its_posture_and_a_changed_posture_changes_what_it_does`. A sealed
script that goes home on `return` and stays on any other word: on the default it reads
`return` and drives home; set to `HOLD` it reads `hold` and never leaves its cell; set back
mid-run it goes. The parity test reads `View.posture` against the keyframe with the other
150 rows.

Task (`arena_task`): `ap3_a_posture_told_before_its_units_join_is_applied_when_it_lands`.
The word and the join at one head; the word a head early, kept, replaced by a later word and
applied when the join lands; a practice match's task drops it and keeps none.

Space, in source: `space::view` (a ship's view carries her own posture by her id in the
bubble, and `return` without a row), `space::backup` (a hull outside every bubble reads the
posture her seat hands the view).

Story (`tests/domain_field/field_ap3.rs`, a field of three parties, about 85 s on
conclav.dev): `ap3_a_units_posture_is_its_own_through_a_field_a_restart_and_a_demotion`. The
ward is set to `GUARD` on the open surface; the site promotes around it and its frame reads
`guard` while the raider's reads `return`; set to `HOLD` in the running field its frame
reads `hold`; the story waits until the field's saved snapshot carries `HOLD` for the ward,
sets the record back to `RETURN` and restarts at once, and the resumed field reads `return`;
set to `GUARD` in the resumed field it reads `guard`; the field demotes on its idle bound
and the record still says `GUARD`.

Each acceptance clause failed on main: the story and the arena test cannot be compiled there
(no record, no field), and the conformance tests fail there for API 6 and API 7 (main's list
stops at 5; with the ABI at 9 they fail for API 8 too without the table).

**What doc 45 had wrong, or left out.**

- **"Filed minds are conformed again by the Registry's existing path."** The path exists and
  could not do it: see "Conformance". The row's acceptance ("one sealed under the old version
  is conformed again, not refused for ever") was failing on main for API 6 and API 7 before
  this unit touched anything.
- **"The unit's record in `crates/mush-tactics`"** cannot hold it: see the table above.
- **`API_VERSION` is not the only version.** A field a mind reads is first a field of the
  frame and the keyframe (doc 17 §3, D-UM-4), so `FRAME_VERSION` moved too (9 to 10) and the
  three shared cockpit fixtures under `web/src/lib/fixtures/cockpit/` with it.
- **The word "posture" was taken four times**: a pose's opaque driver blob (`Pose.posture`),
  the mind card's section (`Stance`), a space hull's emission posture (`EmconPosture`,
  `space(v).emcon`) and the detachment commander's decision (`detachment::commander::Posture`).
  None is this. `crate::posture::Posture` is the only one a player sets on a unit, and
  `View.posture` collides with no registered name.
- **A resume across the ABI move** (doc 17 §13, unchanged): a practice match running at the
  restart is voided by name, and an IC field keeps its sortie and drops each scripted mind to
  the drive ROM. A deploy of this unit over a running field does that once.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **The ABI moved to `rhai=1.26.0;api=9;stock=3`**, once, and the frame to 10 | **Expensive**, as ruled (Q-45-4): every sealed mind, compile key and resume check carries it |
| 2 | The record is an attribute on the unit's object | Low: two functions (above) |
| 3 | A getter, `v.posture`, answering a word, and the keyframe's `posture` key | Medium: it is API and frame. A kernel would have left the frame alone and given the seat nothing to show |
| 4 | The host tells its arena by a message: every unit once when a task first holds it, then on a change, once a second and at once on a write; a unit off the default is told again every ten seconds. The task keeps a word for a unit whose join has not landed | Low. A unit that leaves at a ramp and joins again while its row stood in the live record starts on the default and is corrected within ten seconds |
| 5 | The arena's copy is hashed and carried only off the default | Low; it is what kept every practice hash and `SNAPSHOT_FORMAT` |
| 6 | Every ABI since API 4 is retained, API 6 and 7 among them though no revision is known to carry them | Low: a row each |
| 7 | An NPC hull in a bubble (the corvette, the raider, the interdictor, a drone) reads `return` | Low: `Situation::postures` takes a row for any ship |

**Not built, and where each lives.**

- **Setting a posture** (the seat, the carrier, the relay; the status and the roster):
  **AP.4**. Until then only `record::set` writes one, and no verb calls it.
- **What a mind does with the word.** The field's mind on `GUARD` and `HOLD`, and the gate's
  one widening: **AP.6** (`Arena::lay_go_home` lays `Withdraw` whatever the posture is, as
  MW.7 built it). A rig on a plan and a carrier: **AP.7**. Space: **AP.9**; the view already
  carries the word. No shipped doctrine reads `v.posture`.
- **The simulator**: **AP.10**. `ArenaMsg::Posture` is ignored by a task that is not a
  field's, and a practice match's posture would have to ride the input tape to replay.
- **The clients** draw nothing for the keyframe's `posture`: **AP.4**, with the status.
- **A typed record family** for the record: not taken (the table above); no unit.

**Not pinned by a story.**

- The object's attribute through a real store round trip. The story's restart carries the
  world as the rig's reboot does (the objects in RAM, the typed records and module documents
  from the store); that an object's attributes persist is `mush-core`'s own contract and is
  not re-proved here.
- The automatic re-conformance lane end to end over a revision filed under API 8
  (`reconformance_lifecycle_integration_tests` drives it over a compatibility change with a
  current revision). What is pinned is each door the lane goes through, for every retained
  ABI: the cold audit, the preparation, and a think of the unchanged source.
- A unit that leaves at a ramp and joins again inside one host pass (default 4 above).
- A ship's record read inside a bubble by a story. The view's field is pinned in source for
  a ship in a bubble and for one outside every bubble; the one line that reads the hull's
  record into the bubble's situation (`space/backup.rs` `fly_in_bubble`) is not. AP.9's
  stories fly it.

**Reviewed.** A second agent read the diff adversarially (read-only). It found nothing in
the determinism, the resume ordering, the record's ownership rule, the retained-ABI table or
the re-pins (it checked that the changed pin lines differ in hash, digest and `source` tokens
alone). It found three gaps, all fixed in `3436eacca`:

- **A word that beat its unit's join was dropped.** A unit launching into a running field is
  in the live record before its join lands; the host told the task its posture, the task
  applied postures before joins and dropped the word, and the host believed it had told. The
  unit read `return` until the ten-second refresh. The task now applies postures after the
  head's joins and keeps a word for a unit it does not hold; the host tells every unit once
  when a task first holds it. Pinned at the task.
- **The story's restart step assumed the snapshot held the older word.** It now reads the
  field's saved snapshot and waits until it carries `HOLD` for the ward before the restart.
- **Nothing tested the space view's posture.** Two in-source tests.

It also noted that the per-unit hash folds carry no terminator, so two crafted states could
in principle fold the same bytes (an absent unit on `GUARD` against a unit with 258 entries
in `fired_on_by`). That is the folds' standing shape, not this unit's; the posture's tag byte
is its own. Not changed.

**Verified, and how.**

- **Mutation checks**, on Linux (conclav.dev), two builds of a throwaway clone, each
  mutation caught by the test named for it:

  | Mutation | What failed |
  |---|---|
  | The host never tells its arena | the story, at "the field reads the posture the unit came in with" |
  | The getter answers `return` whatever the unit holds | the arena script test, the Registry's posture test, the getter's read |
  | API 8 is not retained | the table's test, the audit test, the save test, the re-conformance test |
  | The hash does not fold a posture | the hash test, the snapshot test |
  | A resumed arena keeps the snapshot's word | the story, at "the resumed field reads the record, not the snapshot's older word: Some(Hold)" |
  | A word for a unit the arena does not hold is dropped | the task's join-order test |

- **The whole of `tacsim`'s tests**, on Linux (conclav.dev): on the first tree 4,516 run,
  the only failures the eight pin tests re-pinned here and two tests of this unit since
  fixed; on the pushed tree `4d0d78c3e`, 4,526 run and 4,526 passed; again on
  the head with main at `d2be044c5` merged in (MW.5's trial rows and MW.10's cast re-pinned on top), 4,538 run and 4,538 passed.
- **The cross-platform corpus** and **MW.5's Registry trial rows**: read on Linux
  (conclav.dev) and on Windows (this box), the same rows byte for byte. The other two
  `#[ignore]`d steps of CI's `tacsim-minds` cell pass on Linux unchanged.
- **The harness**, whole, by hand on conclav.dev against the updated baseline, timing gate
  live (above).
- **`mush-contract-tests`**: 195 on Linux, and on Windows (203 with the eight Linux skips),
  no budget re-blessed.
- `cargo clippy --all-targets -D warnings` and `cargo doc -D warnings
  --document-private-items` on `tacsim`, on Linux and Windows.
- **CI**: the pull request's CI and a full dispatch (`gh workflow run ci.yml`) on the head commit; the runs and what each dispatch-only job answered are in the pull request's body, since a note cannot name the run of its own commit.

**Not verified.**

- **The `#[ignore]`d tests that no workflow runs** (the by-hand lanes: `minds_pool`'s gate
  lane in release, the cost lane's cohort, the field pulse at scale). Any of them that pins
  a hash that folds a seal has moved with the ABI and is not re-pinned here, as MW.9 left it.
- Nothing was run against a live server or over SSH, and nothing was deployed.
- The cost of the host's pass at a field of a hundred units (one attribute read a unit a
  second, and a clone of the told map). Not measured; the reviewer judged it acceptable.

## AP.4

Setting a posture (doc 45 §7.1; Q-45-19). PR #568, stacked on AP.3 (#565) and built on
AP.8's branch (#557), which is merged into it. Paths are under `crates/tacsim/src/` unless
given.

**Built.**

- **The order's rules, once** (`posture/order.rs`, new): the three ways, the OOC rule, the
  lines. Every way ends in the record's one writer (`posture::record::set`, AP.3).
- **From the unit's seat**: `MIND POSTURE [RETURN|GUARD|HOLD]`
  (`match_/cockpit/notices.rs` `mind_posture`). A form of the seat's `MIND` verb, not a new
  verb: the posture is what the unit's mind is told. With no word it reads. With one it sets
  the posture of the hull the actor is flying, in an IC field only, and only from the seat
  that holds the helm (the check a helm intent takes, `arena_bridge::authorise_seat`).
- **From aboard its carrier, or the rig's own cab**: `+tv[/<n>] posture [return|guard|hold]`
  (`port/tv.rs`, a first word, as `tow` is). An operator who is with the rig
  (`RigAt::operator_present`: aboard the carrier, or at the rig's own helm) sets it at once.
- **Over the comms relay**: the same line from the rig's own operator (its lessee or title
  holder) when he is neither; a hired crew's operator sets it from aboard the rig only. The order goes to
  the rig through `field::orders::posture_order` (`field/orders.rs`): it needs a
  `mush_propagation` delivery context between the sender and the rig (two ends on one site),
  and it is stopped by interference declared where the rig stands, as a roster's orders
  are, and inside a field by its party's carrier relay reading jammed (`carrier_relay`, the
  rule `relay_passes` holds a roster's orders to). It arrives now or not at all: an order
  that does not arrive changes nothing and is not kept.
- **Never from an OOC room** (`posture::order::ooc`): Limbo, a room of the OOC area's pack,
  or anything inside one. A seat in the simulator is refused the set (its pod is in the OOC
  area, and no field holds its unit). A read answers anywhere.
- **What shows it.** `STATUS` in the seat and the carrier's roster (`+tv`) each carry one
  line, `posture GUARD`, for a unit that holds a posture other than `RETURN`. `MIND POSTURE`
  and `+tv/<n> posture` read it always. The cockpit keyframe has carried `posture` since AP.3.
- **Help**: `docs/help/field.md` has a new section, "Postures"; `cockpit.md` and `mining.md`
  list the two forms; `+tv`'s and `MIND`'s own help lines name them.
- `crates/mush-contract-tests/legacy-handlers.toml`: the `mind` and `+tv` notes are refreshed.
  No new verb, switch or family.

**Tests.**

- In source (`posture::order`): Limbo and what stands in it are OOC and a plain room and a
  cab in it are not; the words and the lines; every line inside 120 columns.
- `tests/domain_arena/cockpit.rs` (a practice match): `MIND POSTURE` reads in a pod and
  `MIND POSTURE GUARD` is refused with the OOC line, and nothing changes.
- Stories (`tests/domain_field/field_ap4.rs`), each typed by a player who is not staff:
  - `ap4_a_posture_is_set_from_aboard_the_carrier_and_never_from_an_ooc_room`. An operator
    with two rigs, aboard her carrier. She reads and sets each rig's; the roster shows the
    line under rig 1 and none under a rig on `RETURN`; a second set says nothing changed; a
    word that is no posture is refused. From an OOC room every set is refused and nothing
    changes; the read answers.
  - `ap4_a_posture_is_set_from_the_units_seat_in_a_field` (a field of three parties). The
    ward's pilot reads and sets the posture from the cockpit; the record at once, the hull's
    own frame within a pulse, `STATUS`; a bad word is refused; the raider's pilot sets his
    own hull's and not the ward's.
  - `ap4_from_a_rigs_cab_its_own_posture_is_set_at_once_and_another_rigs_over_the_relay`
    (`escort_lease`'s ground, the lessee with a second rig). He boarded his armed rig in the
    bay with `+tv/1 board` and drove it out, and sits in its cab on open ground, off his
    carrier. His own rig's posture is set at once, with interference declared where it
    stands. His order to the rig in the carrier's bay goes over the relay and arrives; with
    interference declared where that rig stands the order does not arrive and the rig holds
    what it held; with it lifted the same order arrives.

**Divergences from the row, and what doc 45 had wrong.**

- **The row's "the relay" is not the roster's order book.** `field/orders.rs` executes a
  detachment's delivered orders for hired NPC crews. A posture is not one of those: an order
  in the book is a standing objective that replaces the slot's last one, and a posture is a
  word beside the objective; and hired NPC crews are not flown for an absent player at all
  (AP.8), so nothing would read it there. What Q-45-19 rules out is a posture set from
  anywhere with no transport, so the relay built here is for the unit a posture is for, a
  player's own rig: the order rides the same transport and meets the same jamming, in the
  same file, without entering the book. `ORDER DT:... <slot> POSTURE <word>` is not built
  (below).
- **A posture verb cannot be a new bare verb.** `posture` would share the prefix `pos` with
  core's `pose`, and the registry resolves a bare verb by prefix. It is a form of `MIND` in
  the seat and a first word of `+tv` aboard.
- **A set is not a submission.** The record is an attribute on the unit's object (AP.3), so
  a set is one object write and prints no `Queued.`; `@field/raiders` writes the raider's
  standing withdrawal the same way. `+tv` and the cockpit verbs are classed as ingress for
  authoritative actions; `+tv/quiet` already writes a document from inside `+tv` without
  one. The notes in `legacy-handlers.toml` say so.
- **GUARD and HOLD are set and shown, and not yet flown.** AP.6 makes the field's mind act on
  them. Until then the house mind takes the unit home whatever it holds, and every set of
  `GUARD` or `HOLD` says so in its answer, as does the help. A filed mind reads the word.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | The seat's form is `MIND POSTURE`, the carrier's `+tv/<n> posture` | Low: words |
| 2 | An order over the relay is not queued: it arrives at once or does not arrive. A roster's order waits out a jam; a posture does not | Low: one function. Queueing it would need a record of orders in flight, which the roster's book is |
| 3 | The relay reaches a rig on the sender's own site only (one place, one tactical window or one domain, `PropagationContext::build`'s rule). From another site or a port the answer is "you have no relay to where it is" | Medium: an order across sites would ride the interplanetary relay (doc 15), with its delays and queues |
| 4 | A damaged carrier relay, which delays a roster's order, lets a posture through; a jammed one stops it | Low |
| 5 | A unit on `RETURN` shows no posture line in `STATUS` or the roster; the explicit read always answers | Low. It keeps every existing transcript of `+tv` and `STATUS` as it was |
| 6 | A read is allowed from an OOC room; only a set is an order | Low |
| 7 | The simulator's seat is refused, by the OOC rule | Low: AP.10 opens it |
| 8 | Who may set it: the seat that holds the helm, for `MIND POSTURE`; for `+tv`, the rig's lessee or title holder at the helm hull (at once when with the rig, over the relay otherwise), and a hired crew's operator from aboard the rig only | Low |
| 9 | A room no atlas record places in the OOC area is in the world (a hull's interior has none). A staff-built room with no record is therefore not an OOC room; an order from one finds no relay and no rig to be aboard | Low: one function, `posture::order::ooc` |

**Not built, and where each lives.**

- **A posture as a roster order** (`ORDER DT:<id>:<rev>:<term> <slot> POSTURE <word>`), for a
  leader to set the posture of an absent member's unit. It needs a decision this unit should
  not take alone: whether a leader may set another player's unit's standing word, and a
  place in the order book that is not the slot's objective. **Deferred, for the owner**;
  AP.8's chain is where it would be built.
- **A carrier's own posture, and a ship's.** `+tv` addresses rigs. A carrier in a field is
  set from its seat by `MIND POSTURE` like any hull; on its pad with no field there is no
  verb for it. **AP.7** (the carrier's mind) for the carrier, **AP.9** for a ship in space.
- **What the house mind does with `GUARD` and `HOLD`**: **AP.6**. The line
  `POSTURE_NOT_FLOWN_YET` and the help's "What is flown today" paragraph go when it lands.
- **The simulator**: **AP.10**.
- **The clients** (the web panel and the TUI) draw nothing for the keyframe's `posture`; the
  line client's `STATUS` and `+tv` show it. No unit names it; it is a renderer's change.

**Not pinned by a story.**

- The carrier relay's jam inside a running field (the part's condition is read off the
  carrier's world record; doc 40 FR.5's stories jam it for roster orders, none for a
  posture). The declared-interference jam is pinned.
- The relay inside a running field, and "no relay to where it is" from another site.
- `+tv/<n> posture` by a hired crew's operator, aboard and refused off the rig.
- `MIND POSTURE` refused to a seat that does not hold the helm (a second berth, a seat its
  mind has taken over): the check is the one every helm intent takes and is pinned there.
- A captive's read passing the held-character gate, and their set refused.

**Reviewed.** A second agent read the unit adversarially (read-only). It found no defect in
the authorization of `+tv` (only the actor's live leases and titled rigs on their helm hull
can be selected), the held-character gate, the direction of the relay (interference is read
at the unit's end; a carrier under way or another site gives no path; nothing is written on
a jam), the parsing, or the persistence (a posture command stages no action, so the object's
write goes out with the step's dirty objects and survives a restart in the production
composition). It found four gaps, all fixed:

- **The relay story stood the operator in a position the game cannot produce** (inside a rig
  by a hand move, which the presence rule reads as nowhere). The relay is now exercised from
  a rig's cab the lessee boarded with his own commands, to his other rig.
- **`MIND POSTURE` asked no helm.** Any focused station of the hull could set it. It now
  takes the helm intent's own check.
- **A hired crew's operator could relay a posture to the owner's rig** from off it. A crew
  now sets it from aboard the rig only.
- **The help said "your carrier's relay"**; the rule reads the carrier relay of the rig's
  party, as a roster's orders do. The help says so.

Smaller: the OOC walk now fails closed past eight containers; a posture's read passes the
held-character gate, as the status does.

**Verified, and how.**

- **Mutation checks**, on Linux (conclav.dev), three builds of a throwaway clone:

  | Mutation | What failed |
  |---|---|
  | No room is OOC, and a seat no field holds may set a posture | the in-source OOC test; the practice match's seat ("Red Tank 1 now holds GUARD." where the OOC line was due). Either guard alone still refuses a pod, so the two were removed together |
  | The roster shows no posture line | story 1, at "the roster shows rig 1's posture, once" |
  | Every set goes over the relay, an operator with the rig or not | story 1 (aboard the carrier the answer became "reaches Rig 1 over the relay") and story 3 (in the cab, with interference declared, "does not reach Rig 1") |
  | `STATUS` shows no posture line | story 2, at "the unit's status shows it" |
  | Jamming does not stop an order over the relay | story 3, at the jammed order ("reaches Rig 2 over the relay" where "does not reach Rig 2" was due) |

- **The whole of `tacsim`'s tests**, on Linux (conclav.dev), on this unit's tree (AP.3, AP.8's
  branch and main at `d2be044c5` under it): 4,551 run and 4,551 passed.
- **The bot's job** (`cargo nextest run -P ci -p mush-bot`, the two long binaries left out as
  CI leaves them): 763 passed. It holds the cockpit's lines under 120 columns, which caught
  this unit's first `MIND` usage line.
- **`mush-contract-tests`** on Linux and on Windows (the help lint, the command allowlist,
  the census, the panic budget, the restriction lints, the fiat budget), no budget re-blessed.
- `cargo clippy --all-targets -D warnings` and `cargo doc -D warnings
  --document-private-items` on `tacsim`, on Linux.
- **CI**: the pull request's CI and a full dispatch on the head commit; the runs are in the
  pull request's body.

**Not verified.**

- Nothing was run against a live server or over SSH, and nothing was deployed. The lines a
  player reads were read in the test rig's line mode, not in the TUI or the web client.
- The story's OOC step with the OOC rule alone removed (the mutation that removed it also
  removed the roster's line, which fails the same story earlier); the in-source test and the
  practice match's seat hold the rule.

## AP.5

Absent at admission (doc 45 §7.1; Q-45-1, Q-45-2; doc 44's Q-44-35, Q-44-36, Q-44-37, Q-44-43,
Q-44-46). Built on MW.7 (doc 44's cut note, "the field's presence record and the one door").

**Built.**

- **Admission asks the connection as well as the seat** (`crates/tacsim/src/field/parties.rs`,
  `at_controls`). For a unit's seated character it answers one of three things: the seat is
  theirs (`Pilot`); it is nobody's absence (`Mind`: a hired NPC sits there, or nobody does);
  or the player is not at the controls (`Away`), with why: not fit, link lost at a known
  second, or gone. The test is the one MW.7's hand-over and its way back already make, in the
  same order: `absent::fit` (conscious and at liberty, Q-44-43), then `port_tick::on_watch`
  (connected, over a live transport).
- **The seats nobody is at pass to their minds** (`field/admit.rs`, `seat_absent`). Called
  once the live record is written and the task is open, in the same world turn as the
  admission. For each seated player who is not at the controls it calls MW.7's door and
  nothing else:
  - gone, or a quit: `absent::hand_over(.., Why::AwayAtAdmission)`, at once;
  - incapacitated or held: `absent::hand_over(.., Why::Incapacitated)`, at once, connected or
    not;
  - link lost less than five minutes ago: `absent::admitted_link_lost`, which opens MW.7's
    one clock at the second the link was lost and sets the seat `Linkdead`. MW.7's tick
    hands the unit over when five minutes from the loss have passed.
- **No second gate, and no change in the arena.** The hand-over writes the same `Away`
  record on the unit's row, sends the same `ArenaMsg::Absent` and the same `Released`, and the
  kernel's rule (`arena/absent.rs`, `permits`), its go-home order and its narrowed view bind
  the unit exactly as they bind one whose pilot quit mid-field. Nothing under `arena/`,
  `MindView`, the mind ABI or the stock mind was touched.
- **The way back and a restart are MW.7's**, unchanged: a player who connects while the field
  runs is bound and seated by `absent::take_back`; a restart keeps the record
  (`absent::at_resume`), seats nobody who is not connected, and the tick tells the resumed
  task again.
- **When a link was lost** (`field/admit.rs`, `LinksLost`, `LinkWatch`, `link_lost_at`). The
  world keeps no such second, so the field host now does, in RAM: a hook stamps the wall
  second of a lost link, a return clears it, and a session that ends on a lost link keeps its
  stamp until the five minutes are over (the field's five minutes are its own, not the
  server's session grace: MW.7's `FB.1`). A clean `QUIT` leaves no stamp.
- **A join asks too** (`field/carrier.rs`, `on_joined`, two calls of `seat_absent`): a hull
  put back into a running field after its carrier was lost at the ramp, and a launch. A join
  is not a field opening, so such a unit is handed over for the reason its player was away
  when it left (`absent::owed_why`), else as after a quit, and its player reads MW.7's line,
  not the admission's. MW.7's own re-hand-over of a hull put back in (`retry_recovers`) now
  waits for the kernel's answer to the join (`carrier::rejoining_unit`): offered earlier, the
  host's word named a unit the arena did not hold yet and was dropped, while the release
  behind it landed after the join and left the hull a mind under no rule. A row that already
  says its mind has the unit is told to the kernel again (`absent::tell_again`).
- **The kernel's word is retried until all of it is taken** (`absent::tell_arena`). It used
  to count the unit as told once the flag was offered, whether or not every channel's
  release followed; a release the task would not take left a seat nobody held for the rest of
  the field. It now offers again at the next pass.
- **What the absent player is told**, through the reconnect queue AP.1 and MW.7 already use
  (`port_tick::notify_budget`, trip level), and no new channel: `absent::ADMITTED_AWAY`, "A
  field opened around #N while you were away: its mind has the controls, makes for its
  carrier's ramp and fires only on a unit that fires on it." It is queued at admission, so
  it reads ahead of AP.1's opening line, which is queued at the field's first snapshot. The
  leaders the unit was with hear MW.7's `GONE_HOME` with the clause "its pilot was away when
  the field opened". AP.1's close lines and letter are unchanged.
- `docs/help/field.md`, under "If you lose your link, or quit"; doc 45's AP.5 row, §3 row 3,
  §4 A and §4a.

**The questions the brief set, and the answers taken.**

- **Where admission learned "connected" before.** Nowhere. `custody_unit` chose the
  controller from `Entrant::pilot`, which is `parties::seated`, which reads the simulation's
  seat assignment. The assignment outlives a quit and a lost link on open ground (story 1
  asserts it), which settles doc 45 §4a's open point.
- **What "connected" can tell.** `on_watch` is `World::is_connected` and
  `World::has_live_conns`. A live connection passes, a `DARK` wizard's included: `DARK`
  hides a character from a room and from `WHO` and does not stop them typing. A link-dead
  session is still `is_connected` and has no live connection, so it fails. A bare SSH `mind
  push` connection (MW.24) sets neither, so it is never a presence. `mush-npc`'s
  `present_for_ambient` (MW.18) is narrower on purpose and stays different: it asks who an
  NPC may speak to unprompted (not `DARK`, typed within ten minutes), which is not who may
  fly a unit. An idle pilot is still a pilot.
- **A multi-crew hull.** Q-44-35 says "in the field it is the seated pilot", and the field's
  row carries one pilot: `seated` names the first occupied station by station id. That
  character is the only one who counts. A second station's holder neither keeps the mind off
  nor is handed the controls, whichever of the two is connected. No unit the field admits
  with a seat has more than one station today (a rig's `pilot`, a line hull's `cockpit`, a
  raider's crew station); a carrier's bridge is not a cockpit and is admitted with no pilot
  (`carrier_entrant`). A hull with two working stations in a field needs the row to carry
  more than one name, which is not built: **for whichever unit first ships such a hull.**
- **Inside the five minutes at admission: the pilot's seat, not the mind.** The reading that
  makes the three delays one rule is that the clock is the player's and never the field's:
  the unit passes to its mind five minutes after the link dropped, at once after a quit, and
  a field opening changes neither. So a character whose link dropped two minutes before the
  field opened is admitted in their own seat, link-dead (the unit fires nothing), with three
  minutes left; one whose link dropped six minutes before, or who quit, or whose session
  ended long ago, has no time left and the unit is its mind's "from the start". The
  alternative, the mind at once for anybody not on watch, would give a player whose link
  blinked ten seconds before a raid less than one who blinked ten seconds after.
- **The gate.** One gate: see "no second gate" above.

**Tests.**

- In-source: `field::parties::tests::ap5_admission_asks_the_connection_as_well_as_the_seat`
  (each answer of `at_controls`, a `DARK` character and a character with no connection among
  them) and `field::admit::tests::ap5_a_lost_links_stamp_is_kept_while_anything_reads_it`.
- Stories, `crates/tacsim/tests/domain_field/field_ap5.rs`, each a field of three parties or
  more:
  1. `ap5_a_player_logged_off_in_a_rigs_seat_is_flown_home_by_its_mind`. A miner's
     prospector, a leased escort whose pilot quit in its cab, an intruder whose fire order
     opens the field. The seat still names the pilot. At admission the rig is its mind's
     (`Why::AwayAtAdmission`, handed, nobody bound); it drives to its carrier's ramp, the
     kernel lets it out, the host puts it aboard, and it has fired nothing. The pilot reads
     the admission's line at login under the reconnect heading.
  2. `ap5_a_mark_left_by_a_player_who_then_quit_is_not_fired_on`. The ward's pilot marks the
     raider's flight Hostile, which promotes the site, and quits before the tick admits
     anyone; a bare connection is then bound to them. The ward is its mind's from admission
     and fires nothing for thirty seconds with the marked raider eight cells off, the rally time cut to four seconds. The pilot
     reconnects, is seated, finds the raider still Hostile on their first frame, and their
     `DRIVE` is the hull's goal with no order standing.
  3. `ap5_a_connected_pilot_is_admitted_as_before`. All three pilots are bound with no
     record, one of them `DARK`, and a `DRIVE` steers.
  4. `ap5_a_link_lost_before_the_field_opens_keeps_what_is_left_of_its_five_minutes`. The
     link is lost a hundred seconds before the promotion. The record is `LinkLost` since the
     loss, not handed, the seat still bound; ten seconds short of five minutes from the loss
     the mind has nothing, and five seconds past it the mind has the unit.
  5. `ap5_a_pilot_who_is_down_or_held_is_admitted_under_the_mind`. Two connected players,
     one incapacitated in the hull and one held, are both admitted under their minds; a
     `DRIVE` steers nothing and a reconnect is refused the controls.
  6. `ap5_a_restart_before_the_player_returns_keeps_the_mind_and_still_seats_them`.

**Verified, and how** (Linux, conclav.dev, the PR's tree over main `9d8fc96b7`).

- The six stories pass, and beside them MW.7's six and AP.1's and AP.2's (eighteen at once).
  The whole `domain_field` binary passes: 262 run, 262 passed, 11 ignored as on main. No
  older story changed: none opens a field over a seated player who is not connected.
- Locally (Windows): `cargo clippy -p tacsim --all-targets -- -D warnings`, `cargo doc -p
  tacsim --document-private-items` under `-D warnings`, and `cargo test -p
  mush-contract-tests` (nothing re-blessed).
- Five mutations in a second throwaway clone, each run against the six stories. The stories
  cannot be compiled on main (they read `Why::AwayAtAdmission`), so the first row stands in
  for "fails on main today".

| Mutation | What failed |
|---|---|
| Admission asks the seat alone (`seat_absent` is not called: main's behaviour) | five of six; only the connected pilot's story passes |
| The record is written and the seat released, but the kernel is never told the unit is an absent player's (a mind under no rule, Q-45-1's alternative) | story 2: the ward put fourteen rounds into the raider its pilot had marked |
| The five minutes run from the field's opening, not from the loss | story 4 |
| A link-dead pilot is handed over at once, like one who is gone | story 4 |
| Admission asks the connection and not whether the pilot is fit | story 5 |

The second row is why story 2 cuts the rally time to four seconds and waits thirty: a hull
released to a mind under no rule falls back first and fights after
`[tacsim.arena.reflex].rally_ms` (twenty seconds as shipped), so a twelve-second stretch
passed with or without the rule.

**Reviewed.** A read-only review of the diff by a second agent (Opus). Fixed in the PR:

- a hull put back into a field after its carrier was lost was handed over by MW.7's
  `retry_recovers` before the kernel had answered the join, so the flag was dropped and the
  release landed on a hull the rule did not bind; the hand-over now waits for the answer;
- the link stamp was kept from the first drop, so a return no hook reported (a reconnect
  that races the session's end) made the next drop look old; each link-dead onset is now a
  fresh stamp;
- a join handed over with the admission's reason and line, though no field had opened;
- `tell_arena` counted a unit as told when the flag was taken and a channel's release was
  not; it now offers again;
- the note said in two places that a restart ends every session, which a hot restart does
  not; the wording is corrected under "not built";
- story 5's `DRIVE` assertion claimed more than it checks, and is reworded.

It found the ordering inside `admit`, the kernel interleaving at admission (a pulse that
lands between the flag and the release leaves an inert seat), the hook facts the stamps rely
on, the order of `at_controls`, and the raid director's NPC crews sound.

**Divergences from the row.**

- **`custody_unit` still checks the seat out.** The row names `custody_unit` as the place.
  It still returns `Controller::Pilot` and the pilot's seat for a seated player who is away,
  and `seat_absent` hands the unit over in the same turn. A unit checked out as a bare
  `Controller::Mind` would be a mind under no rule until the host's word reached the kernel
  (the first pulse is due at once, the word is applied at a pulse's head, and the seat's own
  hold on its fire binds a pilot's hull only: `arena/conditions.rs`); a seat nobody types at
  does nothing at all. Checking the seat out is also what makes the link-dead case the same
  code path.
- **"Not connected" is three cases, not one** (above): gone, link-dead inside five minutes,
  link-dead past them.
- **Files outside the row.** `field/absent.rs` (MW.7's: one `Why`, one line, and two small
  functions, `admitted_link_lost` and `tell_again`), `field/carrier.rs` (two calls),
  `field/mod.rs` (the hook's registration), and `tests/domain_field/field_mw7.rs` (its
  helpers made `pub(crate)` so these stories share them; no behaviour changed).

**What doc 45 had wrong, or left out.**

- **§4 A: "no hook ever releases them".** True, and worse than it reads: such a unit was a
  seat with nobody in it for the whole field, so it also never went home and its player
  could not be told apart from one at the controls.
- **§4a's open point is settled**: the seat assignment survives a disconnect on open ground.
- **The row's "is admitted under its mind" hides the seat's fire hold.** Every seat in a
  field opens at `FIRE HOLD` (Q-CF-41), and MW.7's default is that a hold on the seat binds
  the mind flying for an absent player ("the hold the pilot left"). A player who was away at
  admission never had the switch in front of them, so under that default **a unit admitted
  for an absent player answers nobody, even under fire**. It goes home; it does not defend
  itself. This is where AP.5 stops at a seam: the read is in the arena's gate
  (`Arena::absent_denies_fire`, `Arena::answerable`), which is AP.6's. What it needs is one
  of two things: the gate stops reading the seat's own switch for an absent player's unit
  (doc 45's `HOLD` posture already "returns fire only at a unit that has fired on this
  one", so the posture, not the switch, would be the player's word), or the admission
  hand-over clears the switch. **For the owner, with MW.7's flagged default, and for AP.6.**
  The help page says what happens today.

**Defaults taken.** None needs a refactor to undo.

- **A session link-dead with no stamp** (a drop this boot never saw) is read as lost now and
  gets five minutes, MW.7's rule for a pilot found away at a restart.
- **A stamp outlives the server's session grace.** A server whose `[game].linkdead_grace_s`
  is shorter than five minutes ends the session early; the field still waits the five
  minutes from the loss, as MW.7 does for a unit already in a field. One whose grace is
  longer holds the session past five minutes; the field does not wait past them.
- **A held character in a seat reads `Why::Incapacitated`**, MW.7's word for "incapacitated
  or held", and those the unit was with are told "its pilot is down".
- **An NPC in the seat is unchanged** (`AtControls::Mind`): hired crews are not flown for an
  absent player (Q-45-13; AP.8).
- **A fire order whose pilot left before the admission lapses.** `+field/fire` opens the
  field and owes one shot "through the pilot's own seat" once the shooter's sensors hold the
  target (`field/mod.rs`, `on_frames`). A pilot who typed it and quit before the tick admitted
  anyone has no seat by then, so the shot never leaves. Before this cut the seat was still
  bound, and nothing on that path (`arena_bridge::authorise_seat`) reads a connection. It is
  Q-44-18's "an order to attack lapses", and the field still opens. Read in the code, not
  pinned by a story.
- **The admission's line is queued before AP.1's opening line**, because the hand-over is
  made at admission and AP.1 tells at the first snapshot.

**Not built, and where each lives.**

- **A unit nobody sits in**: a rig worked from its carrier, a rig on a fill plan, a carrier
  on its pad. It is still admitted as a bare `Controller::Mind` with no record, the rule does
  not bind it, and nothing takes it home; an armed rig left with an empty cab will fire on a
  party a stance declares Hostile. Q-45-1 counts these as absent "from the start". The door
  needs a row with a pilot (`absent::rows`), and such a unit's player is its operator or its
  commander, not a seat. **AP.7**, which waits on AP.6.
- **The posture** a player may leave and what the mind does with it: **AP.3, AP.4, AP.6.**
- **The fire hold of a unit admitted while its player was away**: above. **AP.6.**
- **Space and the simulator**: **AP.9, AP.10.** Nothing here acts on a unit no field hosts.
- **Durable link stamps.** RAM. After a restart a player who is not connected is handed
  over at once by a field that opens then, and a session the boot finds link-dead with no
  stamp gets five minutes from that moment (the default above). A field that was already
  running at the restart is MW.7's rule, unchanged: a seated pilot who is not connected gets
  five minutes from the boot, whatever took them away.
- **The party's leader.** A party of one is still led by its seated character, absent or not
  (`parties::entrants`), so nobody can mark for it while they are away. The detachment's
  rule for an absent leader is **AP.8**.

**Not pinned by a story.**

- A join: a hull put back in after its carrier was lost at the ramp with its pilot away, a
  launch whose pilot dropped, `tell_again`, and the wait for the join's answer. Read in the
  code by this session and by the reviewer; no story loses a carrier at the ramp with its
  pilot away.
- A release the task would not take, offered again (`tell_arena`).
- A link-dead pilot who comes back inside what is left of the five minutes after admission
  (the path is MW.7's `on_present`, which story 2 runs for a handed unit, not for an open
  clock).
- A session held link-dead past five minutes by a server with a longer grace (the in-source
  test covers the stamp, not the admission).
- A roster unit of a DT flight whose pilot is away at admission (the stories fly a rig and
  fixture hulls).
- The line the leaders hear (`GONE_HOME`) at an admission.

## AP.6

Not built.

## AP.7

Not built.

## AP.8

A detachment whose leader is gone (doc 45 §7.1; Q-45-13; doc 44 Q-44-18, Q-44-38, Q-44-43,
Q-44-44, Q-44-46). PR #557. Built on MW.7's presence record and MW.14a's commander.

**Built.**

- **The rule, in one place** (`crates/tacsim/src/field/orders.rs`). `presses` says which orders
  carry an attack forward: an `ENGAGE` and an `ADVANCE`, and no other. `lapses` is the rule: a
  delivered order lapses when it presses, a player gave it, and the field has that player gone
  from the unit they gave it from (`gone_from_field`: MW.7's record on that unit reads
  `handed`, or the unit is out at its ramp with its recover still owed). So the clock is
  MW.7's and no other: five wall minutes after a lost link, at once on a clean quit
  (Q-44-46), at once for a pilot who is knocked out (Q-44-43). An NPC leader's order never
  lapses this way.
- **The roster's succession reads the same record** (`presence`, read by
  `crates/tacsim/src/detachment/sortie/field.rs`, `reconcile`). While a running field holds a
  player's unit, the field's record says whether they are here, waited for or gone, and the
  session's facts are read only where the field keeps no record. Before this a player whose
  unit had passed to its mind went on leading for as long as the server kept their session.
- **The lapse is written once, into the roster's order history**
  (`detachment/sortie/field_orders.rs`: `Lapse`, `FieldOrders::lapsed`, `lapsing`, `lapse_due`;
  `detachment/sortie/prepare.rs`). Every write that saves the roster records the lapses that
  are due in the same write: an order's poll, an issue, a reply and a refresh. So no write
  makes somebody else the leader and leaves the absent leader's attack running. The flight
  tick's poll runs when a lapse is due and nothing else has (`needs_poll`). The record
  survives a restart and the leader's return, and goes only when another order is delivered to
  the slot. Only an order a hired NPC is carrying out on its current field flight lapses: a
  player recipient's order is advice, an order from an earlier flight binds nothing, and an
  order the recipient answered NEGATIVE was never flown.
- **Two reserved sequences** (`crates/mush-tactics/src/detachment/orders.rs`,
  `OrderBook::reserve_sequence`). The field's kernel fences a hull's orders by sequence, so
  the stand a unit takes in place of a lapsed order needs numbers above every order issued
  before the lapse and below every order issued after it. The history takes two from the
  order book when it records the lapse. Before an issue or a reply, so a new leader's order is
  above them; after a poll's deliveries, so an order the poll replaces is not lapsed first.
- **What the unit does** (`field/orders.rs`, `stand`, called from the NPC walk in
  `detachment/sortie/npc.rs`). First it holds: a `HOLD` under the first number. It stops where
  it is, takes no target and fires nothing, whoever its leader marked Hostile. Then it
  answers: once its own frame shows a hit on its own hull that it did not show when the hold
  was taken, a `SCREEN` under the second number, facing the nearest contact its own frame
  holds Hostile. It stays on its cell and fires on a Hostile in the screen's arc and its
  reach, until a leader orders it otherwise. A `HOLD FIRE` on the lapsed order is kept through
  both (Q-44-18), so such a unit never answers. The stand is the crew's own act: it waits on
  no relay, is answered to nobody, and is issued by the unit's own operator from its own
  unit. The host decides on the unit's own frame only. An offer the kernel did not take is
  made again after the window any order has to stand.
- **A successor's order replaces the stand.** The compiled commander treats a lapsed order as
  not standing (`detachment/sortie/commander.rs`, `owes`), so its posture is sent even when it
  reads the same as the order that lapsed; a player who leads next types an order as usual.
- **The leader is told** (`detachment/sortie/runtime.rs`, `tell_lapses`, at the submission of
  the write that records the lapse). The leader who gave the order reads `ORDER_LAPSED` on the
  reconnect queue at trip level (`port_tick::notify_budget`, the queue AP.1's notices use);
  the roster's leader after the write, when that is another player, hears
  `ORDER_LAPSED_LEAD`. `+sortie/orders` marks a lapsed order in the issuer's list
  with a sentence after its line (`ORDER_LAPSED_STATUS`). The line itself is unchanged: the
  bot parses it for the reply's word (`mush-bot`, `order_replies`), and its contract test now
  carries a lapsed order.
- `docs/help/detachments.md` and `docs/help/field.md` say all of it. Doc 45's AP.8 row and
  §7.3 name the files as built.

**Settled from the code** (the four questions the unit was given).

- **A leader who is connected but out of the field** (aboard the carrier, in another room) is
  not absent, and their order stands. They stop leading at once, because the roster's leader
  must control a unit on its field flight, and that was already so. They can come back and
  lead, and they chose to step away with the order standing.
- **A clean QUIT lapses the order at once.** One rule: the order lapses when its leader's
  unit passes to its mind, and a quit hands the unit over at once (Q-44-46).
- **An order mid-execution** stops where it is. A unit that was closing stops on the cell it
  has reached and does not finish the approach; rounds already in the air land.
- **A returning leader finds the order still lapsed.** They are told at login, the order
  report says so, and the unit holds until they give the order again, which is then an order
  like any other. They lead again once they hold their unit's console.

**Tests.** In-source: the presence rule's arms and what presses (`field::orders`); a reserved
sequence between two orders, across a checkpoint (`mush_tactics::detachment::orders`); the
order of succession, a leader who is waited for, then the next player, then the commander
(`mush_tactics::detachment::roster`). Stories in `domain_field`
(`crates/tacsim/tests/domain_field/field_ap8.rs`), each a field of three parties flown with
`roster_rig::fly`:

- `ap8_an_absent_leaders_engage_lapses_and_the_crew_still_answers_a_hit`. A captain leads a
  combat roster with Rook Adeyemi in an armed rig, marks the intruder Hostile and orders
  `2 ENGAGE`; Adeyemi fires on the intruder. The link is lost and the server goes on keeping
  the session. Half a minute short of five minutes the captain still leads and the order
  stands. At five minutes the rig passes to its mind, the lead passes with the session still
  there, the order lapses and the tank holds: no round in fourteen seconds, on the same cell.
  The third party's hull, which nobody has fired on, fires on it; after a round hits, the
  tank fires again, from a screen, on the same cell. The captain reads the lapse at login
  and in `+sortie/orders`, and it is still lapsed after their return.
- `ap8_the_npc_commander_replaces_a_lapsed_advance`. The captain orders Okonkwo, the plan's
  commander, to a cell of their choosing and quits. She leads, and her own ADVANCE to the
  work point is delivered and stands; its sequence is three past the captain's, which is the
  lapse's two numbers taken first.
- `ap8_a_hold_the_leader_left_is_still_held`. `2 HOLD`, a quit, four hundred seconds: the
  same order, the same order row on the rig, no lapse, nobody told.
- `ap8_a_restart_keeps_the_clock_and_the_lapse`. The link is lost, the server restarts a
  hundred seconds in, the order still stands; it lapses five minutes after the loss with
  less than five minutes since the boot. A second restart finds it lapsed and the rig
  holding, and offers the old order to nothing.
- `ap8_a_lapsed_order_stays_lapsed_until_its_leader_gives_it_again`. A quit, the lapse, the
  captain back while their rig is still in the field. They are told, take the focus and the
  helm back, lead again, find the order lapsed and the rig holding, give the order again,
  and it is delivered and obeyed.

**Verified, and how** (Linux, conclav.dev, at the PR's head). The five stories pass, run
twice, five at once and two at once. The whole `domain_field` binary passes (261 run, 11
ignored), as do the tacsim library's tests (2,238) and `mush-tactics`' (73). Clippy is clean on
every target of both crates, `cargo doc` is clean under `-D warnings`, `cargo fmt --check`
passes on both, and `mush-contract-tests` passes with nothing re-blessed. Five mutations, each
run against the five stories:

| Mutation | What failed |
|---|---|
| Nothing lapses (`lapses` is false) | stories 1, 2, 4 and 5; the HOLD story passes |
| The stand never answers a hit | story 1 only, at "the tank answers once it is hit" |
| Every order presses, so a HOLD lapses too | the HOLD story only |
| The roster's succession reads the session, as on main | story 1 only, at "the lead passed on the field's clock" |
| The stand is never offered: the lapse is a record and nothing more | stories 1, 4 and 5; the commander's order still replaces it in story 2 |

The stories cannot be compiled on main (they read `Lapse` and the lapse's lines), so the first
row stands in for "fails on main today".

**What doc 45 had wrong, or left out.**

- **`detachment/live.rs` is the practice host.** The IC roster is hosted by
  `detachment/sortie/` (`field.rs`, `field_orders.rs`, `prepare.rs`, `npc.rs`, `tick.rs`).
  The practice host is untouched (Q-44-19).
- **"Hold and defend under their doctrine" is not an order the doctrine has.** A `HOLD` parks
  the unit and fires nothing, even under fire. With no order the doctrine closes on every
  party its book holds Hostile, which is the attack running on. A `SCREEN` fires in an arc of
  sixty degrees either side. Nothing stands and answers all round. Hence the two halves.
- **Leadership did not follow MW.7's clock.** The roster read the session, so the lead
  passed when the server's own grace ended, whatever the field had done with the unit.
- **The session's end gives up the console.** Sim's disconnect hook releases a departing
  player's focus and billet when their session ends, so the lead can pass before the field's
  five minutes when the server's grace is shorter. The order's lapse still waits for the
  field's clock. With the default server grace the two are the same five minutes. Seen in a
  run of story 1 that ended the session a minute in (the lead passed at once, the order stood
  until five minutes); the story as merged keeps the session, to pin the field's clock.
- **A write that saved the roster did not have to be the poll.** An NPC's WILCO is a message
  too, and it saved the new leader seconds before the flight tick's poll ran; the first
  build recorded the lapse only in the poll and the commander issued over an order that had
  not lapsed yet. Hence every write.
- **No hired NPC can be seated in an armed rig.** `+sortie/seat` hands the helm over through
  Sim's hand-off, which takes a console of exactly one channel (`mush_sim::handoff`,
  `station_channel`), and the escort tank's one console carries the helm, the sensors and
  the fire channel. Rook Adeyemi, hired under a combat contract, can therefore only fly a
  prospector, which carries nothing that fires. **For the owner; not fixed here.**
- **The field rig cannot restart a roster's world as it stands.** `driving_rig` keeps every
  composed module write as typed action state, and a roster's journey lands comsys's channel
  documents and the transport's envelopes, which are their modules' own. The restart story
  hands those to the boot as module data. A rig fix would be `driving_rig::keep`'s.

**Defaults taken.** None needs a refactor to undo unless it says so.

- **The order lapses whoever leads next.** Q-45-13 reads "lapses ... and no other leader
  takes over". Taken as: the absent leader's attack ends at the grace, and a successor's
  orders stand in its place. The alternative, an ENGAGE that runs on under a new leader until
  they change it, is one condition in `lapsing`.
- **The clock is the field's only.** A leader who was never at the controls of a unit in a
  running field (they recovered to the carrier and then quit, or no field was running) has no
  record, and their orders stand. Reading the session there would lapse every such order at
  each restart, when every session ends for a moment. AP.5, merged while this unit ran, gives
  a record to a leader who is away in their seat when a field opens
  (`Why::AwayAtAdmission`), and by the same rule their order lapses at admission. A leader
  who is in no seat when they go is still without one: AP.7's.
- **It answers a hit, not a near miss.** The unit's own frame carries its last hit and
  nothing of a round that missed, and the host reads nothing else. MW.7's rule for a player's
  unit counts a miss; that record is the kernel's.
- **It answers in an arc, and not only the unit that hit it.** The screen faces the nearest
  Hostile the unit's own frame holds when it is first hit, and fires on any Hostile in the
  arc and in reach, a party its leader marked among them. A hit from behind is answered only
  if a Hostile stands in front.
- **Only hired NPC recipients lapse.** A player who was sent an ENGAGE still reads it.
- **Told at submission, not at commit.** A write that is refused at its final take is made
  again and tells nothing twice. A restart between the line and the commit tells it once
  more. A leader who reconnects in the same instant is told of a lapse that then is not
  written.
- **The numbers are provisional**: none of this unit's own. The five minutes are MW.7's.

**Not built, and where each lives.**

- **A stand that answers all round, and only the unit that fired on it.** It needs an
  objective the doctrine does not have and the arena's record of who fired on whom, which is
  MW.7's gate. `stand` names the seam: swap the second half's objective. **AP.6**, which
  builds what a `HOLD` posture needs.
- **A lapse for a leader with no field record** (above): a leader in no seat. **AP.7**.
- **A second player in the roster's units taking the lead before the commander, as a field
  story.** A Dray-P bays two rigs, so a roster of two players has no room for the
  commander's. The order is pinned on the roster (`mush_tactics`). A story needs a carrier
  with three bays. **AP.11**, or whichever unit ships such a hull.
- **An NPC crew in an armed rig by production commands** (the gap above). The story refits
  the hired prospector by a staff hand.
- **The practice simulator** (Q-44-19), **space** (AP.9), **a Rhai commander** (MW.14b).
- **Open ground.** A delivered order is lapsed only for a unit on its field flight whose
  leader the field has gone, which needs a running field. An `ADVANCE` on open ground is one
  drive and an `ENGAGE` there is a mark, and neither is touched.

**Not pinned by a story.** A lapse recorded by a refresh or by an issue (the stories reach it
through a poll and a reply). The stand offered again after the kernel did not take it. A
`HOLD FIRE` kept through a lapse. A leader knocked out while connected. The next player
leader's line (`ORDER_LAPSED_LEAD`). A unit hit while holding across a restart (the boot takes
the hit it finds as known, so only a later one is answered). A lapse while the clock is
frozen (the flight tick does not run; it is written at the thaw). The lead passing back to a
player whose unit went home while the server still keeps their session. A leader who is away
when the field opens (AP.5's record, which this rule reads; AP.5's own stories fly no roster).

**Reviewed.** A read-only review of the first commit by a second agent found one bug and
eleven risks and nits. Fixed in the PR's second commit: a lapse was not tied to the
recipient's current flight, so an order from an earlier flight could freeze a relaunched
unit; the lead and the lapse were not one write; the stand was never offered again; the
notice was computed apart from the write; the presence record was read without its pilot;
the session was read where the field has no record, which would lapse orders at a restart;
NEGATIVE-answered orders and player recipients lapsed; the stand's journal line named the
absent leader. Left as recorded above: the hit across a restart, the frozen clock, the lead
passing back.

## AP.9

Not built.

## AP.10

Not built.

## AP.11

Not built.
