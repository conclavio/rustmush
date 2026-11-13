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
  AP.8's chain is where it would be built. (Ruled yes on 2026-11-12 and built as AP.4a, below.)
- **A carrier's own posture, and a ship's.** `+tv` addresses rigs. A carrier in a field is
  set from its seat by `MIND POSTURE` like any hull; on its pad with no field there is no
  verb for it. **AP.7** (the carrier's mind) for the carrier, **AP.9** for a ship in space.
- **What the house mind does with `GUARD` and `HOLD`**: **AP.6**. The line
  `POSTURE_NOT_FLOWN_YET` and the help's "What is flown today" paragraph go when it lands.
  (They went with AP.6: see its note.)
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

## AP.4a

A leader sets a roster member's posture (doc 45 §5 Q-45-26, the owner's ruling of 2026-11-12
on the one thing AP.4 deferred; Q-45-19, Q-45-13). Stacked on AP.6. Paths are under
`crates/tacsim/src/` unless given.

**Built.**

- **The order** (`detachment/sortie/field_orders.rs`, `FieldObjective::Posture`;
  `detachment/sortie/field_commands.rs`). `ORDER DT:<id>:<rev>:<term> <slot> POSTURE
  RETURN|GUARD|HOLD`. It is issued through the same act every roster order is
  (`Act::Issue`), so the leader's authority, the command rights over the unit, the order
  book, the transport and its jams are the ones a `HOLD` or an `ENGAGE` meets. Three
  refusals of its own at the issue: from an OOC room (`posture::order::ooc`, AP.4's rule);
  for a slot a hired crew flies (`POSTURE_ORDER_CREW`: a crew is not flown for an absent
  player, Q-45-13); and for anything but one numbered slot and one posture word
  (`ALL` is not taken).
- **Carrying it out** (`detachment/sortie/npc.rs`, `posture_order`, called from the flight
  tick's walk for a unit a player flies). A player's unit takes its orders as words its
  player reads; a posture is the exception, because it is the unit's own standing word.
  Once the order book has the order delivered to the unit's slot on its current flight, and
  inside a field once its party's carrier relay does not read jammed
  (`field::orders::relay_jammed`, the rule a roster's kernel orders are held to), the host
  writes the word to the unit's record (`posture::record::set`, the one writer), tells the
  unit's player, and answers WILCO as the unit's player. The answer is the durable mark: an
  answered order is not applied again. Between the write and the answer landing, this
  boot's walk remembers the sequence it wrote, so the write and its notice are made once.
  The host waits while the player has an act of their own in flight
  (`ctx.authoritative_pending`), as MW.7's recover for an absent pilot does. The write and
  the answer are two passes of the walk: the record is an object write on the older
  persistence lane and the answer is a prepared action, and the world task does not take
  both in one step (main's #574 met that refusal in a story's rig).
- **Only the unit answers it** (`field_commands.rs`, `field_response`, `posture_at`). The
  player's own `WILCO` or `NEGATIVE` to a `POSTURE` order is refused
  (`POSTURE_ORDER_ANSWERED`), and their `+sortie/orders` shows the order with no reply
  prefill. So an answer on the leader's report always means the word was written, and a
  NEGATIVE is not a veto: the player sets the posture back instead.
- **The player is told** (`posture::order::POSTURE_ORDERED`, through
  `port_tick::notify_budget` at trip level, the queue AP.1 uses): "{leader}, leading your
  detachment, ordered #N to GUARD: it holds that word until you set another." Said at once
  to a connected player, queued for one who is away, and only when the order changed the
  word the unit held. A leader ordering their own unit is not told.
- **What flies it** is AP.6's, untouched: the record is the unit's, the field's host copies
  it to the arena within a second, and a mind flying the unit lays the posture's order.
- `docs/help/detachments.md` and `docs/help/field.md` say it. Doc 45 has Q-45-26 and the
  AP.4a row.
- **The roster test rig flies a second player** (`tests/support/field/roster_rig.rs`,
  `Billet::Member`): a guest lent the slot's rig under the organiser's own operational loan,
  who boards its cab, takes its console and accepts the plan by their own commands. No story
  flew two players on one roster before.

**What the code said about the ruling.**

- **A leader cannot order a unit whose player has already gone.** The roster's command
  rights are read per unit at every issue, and a unit that has passed to its mind (a quit,
  or five minutes after a lost link) is no longer covered: the order book answers "command
  authority does not cover the target unit". AP.4's note imagined the order "for a leader to
  set the posture of an absent member's unit". As the roster is built, the leader sets it
  while the member still holds the seat, which includes the five minutes after their link
  drops, and the word is then flown when the member is gone. The story pins both halves.
  Widening the roster's command to an absent member's unit is a change to the detachment's
  authority rule, not to this order. **For the owner.**
- **AP.4's objection to the order book** was that a book order replaces the slot's standing
  objective. It does: a `POSTURE` to a player's slot replaces whatever the leader last sent
  that slot. A player's slot holds only advice (nothing executes a player's order), so what
  is lost is a line in the leader's own report.

**Defaults taken** (the owner ruled the principle). None needs a refactor to undo.

| | Default | Cost to reverse |
|---|---|---|
| 1 | The unit's player is told who set it and to what, at trip level, queued when away | Low |
| 2 | The player's own later set wins: an answered order is never applied again. A new order from the leader is a new order and is applied | Low |
| 3 | A hired crew's slot is refused, and `ALL` is not taken | Low |
| 4 | The plan's NPC commander (MW.14a) does not issue it: nothing in `commander.rs` names it | Low |
| 5 | The unit answers WILCO for itself, as its player, whether or not that player is connected | Low |
| 6 | The order needs no field: on open ground it is applied when delivered | Low |
| 7 | A leader may order their own slot (it is a player's unit); they are not sent the notice | Low |
| 8 | The sentences are in `posture/order.rs`, which the bot's census does not read, so they have no row in the bot's table | Low: rows, if the census is widened to that module |
| 9 | Only the unit answers a `POSTURE`; its player's `WILCO` or `NEGATIVE` is refused | Low: one check in `field_response` |
| 10 | An expiry on the order limits how late it may arrive; a posture that was set stays set | Low |
| 11 | A damaged carrier relay, which delays a kernel order, lets a posture through; a jammed one holds it (AP.4's default 4, the same function now) | Low |

**Tests.** Stories (`tests/domain_field/field_ap4a.rs`), on AP.8's scaffold with the roster
flown by `roster_rig::fly`:

1. `ap4a_a_leader_orders_a_members_rig_to_guard_and_its_owner_can_set_it_back`: a field of
   three parties (the roster, an escort, an intruder); the roster is two players. The member
   typing the order is refused ("the issuer is not this detachment's current leader"). With
   the roster's comms jammed (`@prop/jam`, as MW.14a's story jams a roster) the captain's
   order is issued and not delivered, and the rig holds `RETURN`. The member's link drops;
   the jam lifts; the order arrives inside the five minutes, the record reads `GUARD`, and
   the leader's report shows WILCO. Five minutes after the loss the rig is its mind's: it
   fires on nobody, the escort's player fires on it and it answers, and it is still in the
   field. The captain's second order is refused by the roster's authority rule. The member
   logs in, reads `POSTURE_ORDERED`, takes the seat, sets `RETURN`, and thirty seconds later
   it is still `RETURN`.
2. `ap4a_a_hired_crews_slot_takes_no_posture_order`: the order to a hired crew's slot is
   refused with `POSTURE_ORDER_CREW` and nothing is issued; a word that is no posture and
   `ALL` are refused.

**Not pinned by a story.** The OOC refusal at the issue (the rule is AP.4's function, pinned
there). The carrier relay's jam inside a field holding a posture (`relay_jammed`; the
declared-interference jam is pinned). A restart between the write and the answer: the order
is carried out once more, and if the player had set the posture back in that window their
set is overwritten and they are told again. The durable mark across a restart (the answer):
step 7 would also pass on this boot's own memory of the write. An expiry on the order. A
leader ordering their own slot. The host waiting on the player's own act in flight.

**Reviewed.** A second agent (Opus) read this branch's diff over AP.6's adversarially,
read-only, with the ruling and not this note's conclusions. It found the authority sound
(the book's own leader, revision, term and command-right checks; a poll throws out an order
from an old term, a former leader or another flight; the host requires the slot's standing
order on the current flight and site), every other reader of `FieldObjective` sound (the
kernel's objectives refuse it, an NPC that somehow received one answers NEGATIVE, it never
lapses, the commander never issues it), and no scheduling change. Its findings:

- **A player who answered the order first stopped it being carried out** (confirmed by
  reading: their `WILCO` or `NEGATIVE` recorded an acknowledgement, and the host skipped an
  acknowledged order). A NEGATIVE was a silent veto. Fixed: only the unit answers a
  `POSTURE`.
- **"The owner's later set wins" has a window**: between the write and the answer landing
  only this boot's memory guards it, so a restart there re-applies the order. Kept, and the
  help no longer says "not applied twice"; recorded above.
- **The host now submits as a live player**, and a submission by the same principal in the
  same slice from outside the tick can be voided as a fan-out. Mitigated by waiting on the
  player's pending act; the same-slice case is not traced and stands as a risk. The host
  already submits as a player for MW.7's recover.
- **The jam step could have passed on the transport's own pace.** Fixed: the story holds the
  jam forty-five seconds and requires the same order to arrive within thirty once it lifts.
- The WILCO check read the whole report: it now reads slot 2's own reply line. Story 2's
  refusals now assert their sentences. The notice is sent only for a change. The new
  sentence is in the 120-column test. `relay_jammed` is now the one copy AP.4's relay order
  uses too.
- Left as recorded: the player-only rule is checked where the order is parsed and not again
  where it is admitted (a controller change bumps the revision and the book refuses the
  stale recipient; an NPC that received one would answer NEGATIVE); the notice names the
  order's issuer as leading, which is true when it was issued.

**Verified, and how** (Linux, conclav.dev, a throwaway clone at main `63dbcb3f6` with AP.6's
and this unit's diff applied).

- The two stories, with every AP.3, AP.4, AP.5, AP.6, AP.8, MW.7, MW.14a and MW.15 test and
  story (65: every story that flies `roster_rig`, which this unit changed), passed.
- **Mutation checks**, four, each built and run against the two stories, each undone by
  applying its own diff backwards over a WIP commit (taken before the second reader's
  fixes; the checked lines stand):

  | Mutation | What failed |
  |---|---|
  | The host never carries the order out | story 1, at "the leader's order sets the member's rig to GUARD" |
  | An answered order is applied again at every pass | story 1, at "the owner's own later set wins": `GUARD` where `RETURN` was due |
  | The unit's player is not told | story 1, at "the owner reads who set it and to what" |
  | A hired crew's slot takes the order | story 2: `Queued.` where the refusal was due |

  "Stopped by jamming" and "a player who is not the leader is refused" are the transport's
  and the order book's own rules, which this unit did not write: the story exercises them
  and no mutation of this unit's code removes them.
- CI's M7 memory-size clippy step, `cargo clippy -p tacsim -p mush-bot --all-targets -- -D
  warnings`, `cargo doc -p tacsim --document-private-items` under `-D warnings`, `cargo fmt
  --check`, and the bot's sentence census.
- **CI**: the pull request's CI and a full dispatch on the head commit; the runs are in the
  pull request's body.

**Not verified.**

- The whole `tacsim` suite was not run on conclav.dev for this unit (AP.6's run of 4,562
  was on the tree below it); CI's shards run it on the pushed head.
- `mush-contract-tests` was run on Windows for AP.6's tree, not again for this one; the help
  lint's input changed (two help pages). CI runs it.
- Nothing was run against a live server or over SSH, and nothing was deployed.

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
  The help page says what happens today. (AP.6 took the first: the gate no longer reads the
  switch. See its note, "the two seams".)

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

The field's mind flies the posture (doc 45 §7.1; Q-45-5 to Q-45-9, Q-45-12, Q-45-16). Built on
MW.7's gate and door, AP.3's record and AP.4's verbs. Paths are under `crates/tacsim/src/`
unless given.

**Built.**

- **The gate is still one function, widened by one case** (`arena/absent.rs`, `permits`).
  `Facts` gains two fields: `guards` (the unit stands on `GUARD`, inside its leash, able to
  defend itself) and `fired_on_kin` (the units whose fire has arrived on another unit of the
  same player). A shot is let through at a unit that has fired on this one, as before, or,
  from a unit that stands guard, at one in the second set. Nothing else moved: a hold still
  refuses every shot, mining and trading are still refused, and a party that is only marked,
  held or declared Hostile is still a first shot. No mind's script decides any of it: the
  house default and a sealed mind come through the same door (`Arena::resolve_fire_by`).
- **The posture decides the standing order** (`Arena::lay_standing`, where MW.7 laid the
  go-home order). `RETURN` lays `Withdraw`, as MW.7 built it. `HOLD` and `GUARD` lay a `Hold`:
  the unit parks. At each think of such a unit (`Arena::stand_posture`, called from
  `Arena::collect_think` ahead of the view) the order is turned to a `Screen` that faces the
  nearest contact the unit may answer and its party holds hostile, kept while that contact is
  inside the screen's arc, and turned back to `Hold` when there is none. So the house mind
  stands, turns and returns fire by two orders the doctrine already obeys. **No doctrine
  source moved**: `STOCK_VERSION`, `MIND_ABI` and every seal are as they were.
- **Where it is stood** (`UnitState::stand_at`). The cell the unit stands on at the first
  think of a mind flying `HOLD` or `GUARD` for its absent player. At that think a Standard
  Doctrine memory is told to stay (`arena/mind.rs`, `seed_stay`), once. A memory that was
  driving home is set back to its patrol: the doctrine has no edge out of `Withdraw` for a
  new order, so without it a unit told to stay would first finish its drive to the ramp. And
  the doctrine's one retreat is marked spent (`rallied`), its own switch for "stay and
  fight": without it a unit hurt past the withdraw threshold breaks for home under any
  order. A `Released` that hands over a unit left on `HOLD` or `GUARD` no longer seeds
  `Withdraw` (`arena/intents.rs`).
- **The leash** (`GUARD_LEASH_CELLS`, ten, one constant). A guard answers for its player's
  other units only within ten cells, Chebyshev, of where it was stood. The house mind stays
  on its cell while it has a round to fire, so for it the leash does not bite. A mind that
  moves (a player's sealed one) keeps the widening inside the leash and answers for itself
  alone past it.
- **Whose unit it is** (`UnitState::player`; `field/posture.rs`, `player_of`). The arena
  knows no players, so the host's posture word now carries one (`ArenaMsg::Posture`): the
  player seated in the unit; for a unit nobody sits in, the commander of a carrier, else the
  operator who launched the rig. A unit a hired crew sits in has none (Q-45-13). A unit of
  the same player that fired on its own is never the one answered.
- **When `GUARD` ends** (Q-45-9; `arena/absent.rs`, `flown`, `can_defend`). When the player
  returns, the flag is cleared and nothing of the posture is flown (`Arena::set_absent`).
  When the field demotes, the arena goes and the record stands. When the unit can no longer
  defend itself (a wreck, its motion disabled, or no fire channel with a round left) it is
  flown as `RETURN`: its frame and its mind's view read `return`, the order is the go-home
  one, and the widening is gone. The player's record is not rewritten, so a unit that can
  defend itself again guards again.
- **The ramp** (`arena/withdraw.rs`, `WithdrawRefusal::Standing`). MW.7's host asks every
  absent player's unit out at its ramp once a second and the kernel decides. It now also
  refuses a unit its mind stands on `HOLD` or `GUARD`, so a guard set beside its ramp is not
  taken aboard, and lets a guard go once it is flown as `RETURN`. The host does not ask for a
  unit on `HOLD` at all (`field/absent.rs`, `tick`). A casualty under tow is its tug's.
- **The seat's fire hold no longer binds the mind** ("the seams", below).
- **What the player and the leaders are told** (`field/absent.rs`). A unit left on `GUARD` or
  `HOLD` does not "make for its carrier's ramp", so four lines say what it does:
  `MIND_HAS_IT_ON`, `TAKEN_ON`, `ADMITTED_ON` (the pilot) and `STAYS_ON` (the leaders MW.7's
  `GONE_HOME` goes to). Each names the word the unit is flown on and promises nothing the
  host cannot know (the host holds no live rounds: a guard that cannot defend itself goes
  home). Each has its row in the bot's sentence table (`crates/mush-bot/src/mine/minds.rs`).
- **AP.4's note is gone**: `POSTURE_NOT_FLOWN_YET` and its line after every `GUARD` and
  `HOLD` set, and the help's "What is flown today". `docs/help/field.md` says what each
  posture does, when `GUARD` ends, and that the seat's switch is the seat's.
- **Nothing an attacker reads changed** (Q-45-16). No contact row, notice or presence row
  carries the flag, the posture, the cell stood on or the player.

**The two seams earlier units left, and how the posture and the seat's fire hold relate.**

MW.7 took as a default, flagged for the owner, that the seat's master switch (`FIRE HOLD`,
where every IC seat opens) binds the mind: "a seat left at hold answers nobody even under
fire". AP.5 found what that meant for a player who was away when a field opened: nobody was
there to touch the switch, so the unit answers nobody.

Doc 45 does not name the switch. What it does rule:

- Q-45-4: the posture is what a player leaves standing, and the strictest of the three,
  `HOLD`, is "stay, hold fire, return fire only under Q-44-38". No posture answers nobody.
- Q-45-5: the pilot's condition slots are cleared at the hand-over, because they "are the
  principal's own fire decisions and assume the principal is watching"; "the posture is what
  stands".
- Q-45-7 to Q-45-9: the posture, and nothing else, decides whether the unit stays and what it
  may answer.

**The reading built** is the one closest to those: the seat's switch is another of the
pilot's own fire decisions for a pilot who is watching. It binds the seat and never the mind
that flies for an absent player. A unit whose seat was left at hold, or never touched, still
returns fire on a unit that fires on it, under every posture. A standing order's `HOLD FIRE`
(a leader's, kept through the hand-over by Q-44-18) still holds, and a unit under one
answers nobody. In the code it is the two clauses MW.7's note named: `weapons_hold` is no
longer read by `Arena::absent_denies_fire` or `Arena::answerable`. **A default, for the
owner.** The other reading (the switch binds unless a posture other than `RETURN` is set)
would make `HOLD`, the posture that "holds fire", fire more than `RETURN`.

**Tests.**

In source:

- `arena::absent::tests::ap6_a_guard_answers_for_its_players_other_units_and_for_nobody_else`:
  the rule's arms with the widening.
- `arena::weapons::tests`, in a field of four parties (red, blue, a second unit of red's
  player in a party of its own, a stranger):
  - `ap6_a_guard_answers_for_its_players_other_unit_and_holds_fire_on_a_strangers_quarrel`:
    blue is held hostile by red's party throughout. On `GUARD` red's mind is refused its shot
    at blue by every source no seat typed; still refused after blue fires on the stranger;
    allowed after blue fires on the second unit and red's mind has stood, with the order
    turned to a screen and the view showing blue. On `HOLD` and `RETURN` the same fire is not
    answered and each still answers for itself. A unit of the same player is never answered;
    a unit whose player is not named answers for nobody but itself.
  - `ap6_hold_and_guard_stay_where_their_mind_took_them`: the order laid, the cell stood,
    the ramp's refusal, the player's return, a posture set to `RETURN` mid-flight.
  - `ap6_a_guard_past_its_leash_answers_for_itself_alone`: the leash at ten cells and at
    eleven.
  - `ap6_a_guard_that_cannot_defend_itself_goes_home`: a guard with no round left and one
    with its motion disabled (flown, framed, ordered and let go as `RETURN`, the record still
    `GUARD`), a standing order's hold kept.
  - `ap6_what_another_unit_reads_of_it_is_byte_for_byte_what_it_was`: blue's whole frame and
    its mind's whole view, compared as text, for red on each posture, flown by its pilot and
    by its mind.
  - `ap6_a_mind_driving_home_is_stopped_when_its_posture_stays`: the seed, once, and only
    over a Standard Doctrine seal.
  - MW.7's `mw7_an_absent_players_mind_fires_only_on_a_unit_that_fired_on_it` now asserts
    that the seat's switch does not bind the mind.
- `arena::hash::tests::ap6_the_hash_folds_the_stood_cell_and_the_player_only_when_set`,
  `arena::snapshot` (`af1_a_unit_is_carried_whole_by_its_derive`, the two fields listed),
  `arena_task` (`ap3_a_posture_told_before_its_units_join_is_applied_when_it_lands`, the
  player carried with the word and dropped by a practice match).

Stories (`tests/domain_field/field_ap6.rs`), each typed by players who are not staff. The
ground of the first four is `escort_lease`'s with one more rig (`stand_up_out`): a lessee in
the cab of his armed rig, his own prospector launched and standing three cells off with
nobody in it, a miner's prospector beside them, an intruder's armed hull ten cells east, and
the three carriers: four parties among the units the stories name. The lessee sets the
posture from his cab on open ground, marks the intruder's flight Hostile, which opens the
field with no round fired, and quits. His seat is at weapons hold throughout.

1. `ap6_a_guard_answers_for_its_owners_other_rig_and_holds_fire_on_a_strangers_quarrel`. The
   presence picture the miner reads is the same before and after the posture is set. On
   `GUARD` the rig fires nothing for twelve seconds with the marked intruder in range, and
   the intruder's own row of it is what it was when its pilot flew it. The intruder's fire
   lands on the miner's rig: nothing. It lands on the lessee's prospector: the rig fires on
   the intruder, after that first arrival, and on nobody else. The intruder never fired on
   the guard itself. The rig is still where it stood.
2. `ap6_the_same_rig_on_hold_does_not_answer_for_its_neighbour`. The same fire on the
   prospector is not answered. Fire on the rig itself is. It is not taken home, and its
   player reads `TAKEN_ON` at login.
3. `ap6_the_same_rig_on_return_goes_home`. It answers for nobody, is let out at its
   carrier's ramp and put aboard.
4. `ap6_a_guard_that_loses_its_last_fire_channel_goes_home`. One fire channel with two
   rounds (a staff fixture on the open surface). The guard answers with both, is then let out
   at the ramp and put aboard, and the record still says `GUARD`.
5. `ap6_a_party_declared_hostile_by_stance_is_not_fired_on_first_under_any_posture`. Three
   parties of one hull each. The ward's co-op declared the raider's Hostile with
   `+faction/stance` before the field opened, and nobody marks anybody: the live record's
   `stance_hostile` and the ward's own frame hold the raider Hostile. For `GUARD`, `HOLD` and
   `RETURN` in turn the pilot sets the word from the seat and quits; twenty seconds, with the
   doctrine's rally time cut to four, and the ward has fired nothing; the pilot returns.

MW.7's third story asserted the rule this unit reverses and is rewritten in place
(`field_mw7.rs`, `ap6_a_seat_left_at_fire_hold_does_not_bind_the_mind`): the seat left at
hold, the unit on `RETURN`, and it answers the raider once the raider's fire arrives.

**Divergences from the row, and what doc 45 had wrong or left out.**

- **"Stays within a leash" is two things.** The house mind does not move: a parked `Screen`
  is the only order the doctrine has that stands and answers, and it stands on one cell. The
  leash is built as the bound of the widening, which is what it has to be for a mind that
  does move. The engine does not drive a unit back inside it (MW.7's default for a sealed
  mind that ignores its order, kept).
- **"Which turns it to `RETURN`" is not a write to the record.** The kernel is the only
  holder of the unit's live rounds and damage while a field runs, and a host cannot read
  another player's unit's frame once its seat is released. So the kernel flies the unit as
  `RETURN` and the record keeps the player's word.
- **The doctrine cannot be told to stop going home.** A hull released to a mind is seeded
  `Withdraw`, and the Standard Doctrine leaves `Withdraw` only by its own rally. Hence the
  seed above. The alternative was to change the doctrine's source, which moves
  `STOCK_VERSION` and with it `MIND_ABI`, every seal and every pinned hash, for a rule that
  belongs to the host of the posture and not to the doctrine.
- **The doctrine retreats under any order.** A unit hurt past the doctrine's withdraw
  threshold, or out of rounds, falls back to its party's home under a `Hold` or a `Screen`
  as under any order (16b cut Q.7). The first build left both edges standing, and the
  second reader traced what the hurt one did to a guard: it drove to the ramp with rounds
  left, was refused there, was past its leash, and never came back. The hurt edge is now
  spent by the seed. The out-of-rounds edge reads no switch and still fires: on `GUARD` that
  is "can no longer defend itself", and the unit is taken aboard; on `HOLD` it stands beside
  the ramp and is not. The help says so.
- **"Never mining or trading"**: unchanged from MW.7. No intent in the arena does either.
- **The row's "Mostly in" names `stock_mind.rs` and `field/home.rs`.** Neither is touched.
  The files are `arena/absent.rs`, `arena/withdraw.rs`, `arena/mind.rs`, `arena/intents.rs`,
  `arena/posture.rs`, `arena/state.rs`, `arena/hash.rs`, `arena/snapshot.rs`,
  `arena/step/frames.rs`, `arena_task.rs`, `field/absent.rs`, `field/posture.rs` and
  `posture/order.rs`.

**The mind API did not move.** `MindView::posture` and the keyframe's `posture` are AP.3's.
One thing about them changed in meaning and not in shape: for a unit a mind flies for an
absent player on a `GUARD` it can no longer defend, they read `return`. No seat is sent
that frame: the flag is only set while no pilot holds the seat. The record, the roster and
`MIND POSTURE` go on saying `GUARD`, which the help states.

**Defaults taken.** None needs a refactor to undo unless it says so.

| | Default | Cost to reverse |
|---|---|---|
| 1 | **The seat's `FIRE HOLD` does not bind the mind**; the posture is the standing word. Reverses MW.7's flagged default | Low: two clauses (`absent_denies_fire`, `answerable`) |
| 2 | **Whose unit**: the seated player; else a carrier's commander; else the operator who launched the rig. A hired crew's unit is nobody's for this rule, and a unit another player sits in is that player's | Low: one function (`field::posture::player_of`) |
| 3 | The posture is stood where the unit stands when its mind first thinks on it, not where the word was typed | Low |
| 4 | The house mind does not move inside the leash; past the leash a moving mind loses the widening and is not driven back | Medium: a manoeuvring guard needs an order the doctrine does not have |
| 5 | A guard that cannot defend itself is flown as `RETURN` and its record is kept. "Cannot defend itself" is a wreck, motion disabled, or no fire channel with a round left; a hull authored with no speed still guards | Low |
| 6 | An unarmed rig set to `GUARD` is flown as `RETURN` at once (Q-45-10's "cannot guard") | Low |
| 7 | The ramp is refused to anybody's ask while a mind stands the unit there; `RETURN` brings it in | Low |
| 8 | The screen faces the nearest unit the mind may answer. One it may answer that stands behind it, outside the arc, is answered only when it becomes the nearest or the nearer one is lost | Medium: the same missing order as 4 |
| 9 | `GUARD_LEASH_CELLS` is ten | A constant |
| 10 | The player is not told separately when a guard turns to `RETURN`; AP.1's close line says where the unit ended | Low: the host would need the kernel to tell it |
| 11 | A guard answers for its player's units that are still in the field. One that has gone aboard takes its fire record out with it, and the unit that fired on it is no longer answered for | Low: keep the record by player when a unit leaves |
| 12 | A unit left on `HOLD` or `GUARD` under the house mind does not take the doctrine's retreat when hurt | Low: one line in `seed_stay` |
| 13 | "The same player" (default 2) and the party's friendships (by owner or organisation, `field/parties.rs`) are two rules. Where they differ (a rig one player owns and another sits in), a guard may be let through the gate at a unit its own party does not hold hostile; the house mind then does not face it. Never a leak, sometimes a guard that does nothing | Medium: one ownership rule for both |

**Not built, and where each lives.**

- **A rig nobody sits in, on a fill plan or worked from its carrier, and a carrier.** Such a
  unit is still admitted as a bare mind with no record and no flag: the rule does not bind it
  and no posture is flown for it. It is "another unit of the same player" for a guard
  (stories 1 and 4 fire on exactly such a rig), and is not itself a guard. **AP.7.**
- **AP.8's stand** (`field/orders.rs`, `stand`) still answers in an arc by the host's hand.
  The kernel's turning screen here is what its seam note asked for, and swapping it in is a
  change to the detachment chain's file. **AP.8's chain; not done here.**
- **Space** (AP.9) and **the simulator** (AP.10). `ArenaMsg::Posture` is still ignored by a
  task that is not a field's, and nothing sets `absent` in a practice match.
- **A manoeuvring guard** (defaults 4 and 8).
- **A notice when a guard turns to `RETURN`** (default 10).
- **A posture as a roster order**: the owner ruled yes on 2026-11-12; it is unit AP.4a,
  stacked on this one.

**Not pinned by a story.**

- **The gate alone, for `GUARD` and `HOLD`, in a story.** Three things keep such a unit from
  firing first: the order (a `Hold` fires nothing), the view (nothing shown hostile) and the
  gate. Story 5 fails when the view and the gate are both removed (mutation 2) and would
  pass on the order alone if only the gate were. The gate by itself is pinned where a shot is
  asked for directly: the in-source tests, and MW.7's sealed-script test in `domain_arena`.
- A unit hurt past the doctrine's threshold staying on its cell (the seed is pinned in
  source; no story wounds a guard that far and waits).
- The leash with a mind that moves (in source only: no rig flies a sealed mind that drives).
- A guard whose motion is disabled, and an unarmed rig set to `GUARD` (in source).
- A posture changed over the relay while the mind flies the unit, and the stop of a unit
  that was driving home (in source).
- The lines `MIND_HAS_IT_ON`, `ADMITTED_ON` and `STAYS_ON` (story 2 reads `TAKEN_ON`).
- A restart while a mind stands a posture: `stand_at` and `player` ride the snapshot, and
  the host tells the player again within a second; no story restarts over one.
- A guard answering for its player's carrier.

**Pinned values moved.** No state hash, no ledger line, no seal identity and no harness
baseline field: every unit of a practice match and of the harness has no absent flag, so
`stand_posture`, the seed, the `Standing` refusal and the two new fields are never reached or
written, and the hash folds no byte for them. What moved, each because this unit changed the
rule it asserts and for no other reason:

| Assertion | Was | Is | Why |
|---|---|---|---|
| `arena::weapons` `mw7_an_absent_players_mind_fires_only_on_a_unit_that_fired_on_it`, the seat's gate | a mind's shot refused with the seat at hold | the shot fires | the seat's switch no longer binds the mind ("the two seams") |
| `field_mw7.rs` story 3 | the unit fires nothing under three arrivals | the unit answers the raider after its first arrival, and nobody else | the same; renamed `ap6_a_seat_left_at_fire_hold_does_not_bind_the_mind` |
| `field_ap4.rs`, six answers to a `GUARD` or `HOLD` set, and `posture::order` `the_words_and_the_lines` | the line and AP.4's note | the line alone, compared whole | the note is removed |
| `arena::snapshot` `every_unit_field` | 51 names | 53 | the two new fields, listed and decided |

**Reviewed.** A second agent (Opus) read the diff adversarially, read-only, with the row and
the rulings and not this note's conclusions. It found the gate sound: one function, one new
arm after the hold; the widening cannot reach `HOLD`, `RETURN`, a guard past its leash or one
that cannot defend itself; marks and stances are never read; nothing a practice match can
reach changed; the snapshot and the hash carry the new fields only when set. Its findings:

- **A hurt guard left its post for good** (confirmed by reading the doctrine's withdraw
  edges). Fixed: the seed spends the doctrine's one retreat.
- **The hand-over lines read the record, not the posture flown**: a guard that could not
  defend itself was said to "stay on GUARD" and then went home. Fixed: the lines name the
  word and promise nothing more, and the help says the roster goes on showing the word.
- **A guard stops answering for a unit that has gone aboard.** Kept, and recorded as
  default 11.
- **Story 5 does not exercise the gate alone for `GUARD` and `HOLD`.** True; recorded under
  "not pinned by a story" with what does pin it.
- **The presence picture was read only before the hand-over.** Fixed: story 1 reads it
  again, as the miner, with the mind standing the guard.
- **A unit off the ground's window never got a stood cell**, so the seed would have run at
  every think. Fixed.
- New prose said "weapons hold". Reworded to the verb, `FIRE HOLD`.
- Left as recorded: the two ownership rules (default 13); a posture changed from `RETURN`
  to `HOLD` in the second its unit is let out at the ramp (the word is applied after that
  head's withdrawals); a host's repeated word turning a standing screen back to a hold for
  one think.

**Verified, and how** (Linux, conclav.dev, a throwaway clone at main `63dbcb3f6` with this
unit's diff applied).

- **Mutation checks.** Ten mutations, each built and run against the unit's in-source tests,
  its five stories and MW.7's (17 tests), each undone by applying its own diff backwards over
  a WIP commit. Every one fails the test named for it:

  | Mutation | What failed |
  |---|---|
  | `GUARD` answers for a stranger (every unit's fire record, whoever its player) | the kernel's guard test ("fire on a stranger is not a guard's to answer"); story 1 |
  | A mind may fire first at a party its own holds hostile (the rule lets every shot through and the view is not narrowed) | 12 of 17: story 5 (the declared stance), stories 1 to 3, MW.7's three fire stories, the kernel tests |
  | `HOLD` answers for a neighbour (any posture that stays stands guard) | the kernel's guard test; story 2 |
  | The posture is not flown: every absent player's unit is laid the way home (main's behaviour) | stories 1, 2 and 4; four kernel tests |
  | A guard that cannot defend itself still stands guard | story 4; the kernel's stay test |
  | The seat's hold binds the mind again (MW.7's default) | stories 1, 2 and 4; the rewritten MW.7 story 3; MW.7's kernel test |
  | No leash | the kernel's stay test, at eleven cells |
  | The ramp lets a standing unit go | the kernel's stay test |
  | The host never says whose a unit is | stories 1 and 4 |
  | A mind driving home is not stopped | the kernel's seed test |

  The stories cannot be compiled on main (they read `TAKEN_ON`), so the fourth row stands in
  for "fails on main today".
- **The whole of `tacsim`'s tests**: 4,562 run, 4,562 passed, on the tree before the second
  reader's fixes. After them: the unit's tests with every MW.7, AP.3, AP.4, AP.5 and AP.8
  test and story (55), passed.
- `cargo clippy -p tacsim -p mush-bot --all-targets -- -D warnings`, `cargo doc -p tacsim
  --document-private-items` under `-D warnings`, `cargo fmt --check` on both crates, and the
  bot's sentence census (`cargo test -p mush-bot --lib mine::minds`, 21).
- **`mush-contract-tests`** on Windows (the help lint, the census, the panic budget, the
  restriction lints, the fiat budget), nothing re-blessed; run before the second reader's
  fixes, which touched no lint's input but the help page. CI runs it again.
- **CI**: the pull request's CI and a full dispatch on the head commit; the runs are in the
  pull request's body. The first push failed one step, the lint job's M7 memory-size limits
  (`clippy::large_stack_frames`): one in-source test held 75,878 bytes of arenas on its
  frame, over the step's 64 KiB. It is three tests now (the stay, the leash, the guard that
  cannot defend itself), and the step was run by hand on conclav.dev before the second push.
  The mutation table above was taken before the split: where it says "the kernel's stay
  test", the leash and the spent guard are now the second and third of those three.

**Not verified.**

- Nothing was run against a live server or over SSH, and nothing was deployed. The lines a
  player reads were read in the test rig's line mode.
- The whole `tacsim` suite was not run again after the second reader's fixes on this box;
  CI's shards run it on the pushed head.
- The cost of `stand_posture` at a field of a hundred units. For a unit that is not an
  absent player's it is one field read a think; for one that stands a posture it builds the
  unit's frame once more a think.

## AP.7

Rigs on a plan, and carriers (doc 45 §7.1; Q-45-1, Q-45-10, Q-45-11; doc 27 Q-H10). Built on
MW.7's door, AP.5's admission and AP.6's posture. Paths are under `crates/tacsim/src/` unless
given.

**Built.**

- **A unit nobody sits in is an absent player's unit from the turn a field takes it in**
  (`field/absent.rs`, `unattended`; Q-45-1). In a field the player who counts is the seated
  pilot (Q-44-35), and a rig worked from its carrier, a rig on a fill plan and a carrier on
  its pad have none. Each row of the live record with no pilot, whose unit is some player's
  (`field/posture.rs`, `player_of`: the operator who launched the rig, the carrier's
  commander), gets MW.7's presence record with a reason of its own (`Why::Unattended`),
  written handed, and the kernel is told. There is no seat to release, no five minutes to
  wait and nobody to take the unit back, so the rest of MW.7's pass applies unchanged: the
  kernel's rule (`arena/absent.rs`, `permits`), the standing order of the unit's posture, the
  narrowed view, the ask at the ramp once a second, the recover the host runs and tries
  again. A unit that is nobody's (a raider's fixture carrier, a fixture hull, a hull a hired
  crew launched) is left as it was. It is called at admission in the turn the task opens
  (`field/admit.rs`), at every join's answer (`field/carrier.rs`, `on_joined`) and once a
  second by the pass for whatever neither reached (a resumed field).
- **The gate is still one function.** Nothing was added to `arena/absent.rs`'s rule. A rig
  nobody sits in and a carrier come through `Arena::resolve_fire_by` and `permits` exactly as
  a seated absent pilot's unit does.
- **An absent word waits for its unit's join** (`arena_task.rs`, `apply_field_acts`). A word
  that sets the flag on a unit the arena does not hold yet used to be dropped; it is now kept
  and applied after that head's joins, as a posture word is, so a hull joining with nobody
  at its controls is bound from the pulse that takes it in. A word that clears the flag never
  waits.
- **A rig flies its posture** (Q-45-10), by AP.6's kernel and MW.7's host with nothing new
  between them. On `RETURN` its mind drives it to its carrier's ramp, the kernel lets it out,
  and the host runs its recover as its operator into the bay slot it launched from. On `HOLD`
  it stands, and is never asked out.
- **The plan ends when the rig comes aboard** (`surface/plan.rs`, `PlanEnd::Field`;
  `surface/act/providers.rs`). The start of a recover whose rig left a running field at its
  carrier's ramp (the site record names the exit: `SiteRecord::exited`) parks the record's
  live plan in the same write. The finish that puts the rig in the bay ends a plan so parked,
  with one `Ended` note, and counts the trip if the hopper came home loaded. Only the plan
  of whoever runs the recover is parked: a former lessee's plan still on the record is not
  theirs to end. That recover is also the one hand act a live plan does not hold its rig
  from (`surface/act/commands.rs`, `left_a_field`, and the same test in the start's prepare):
  without the exemption the host's recover of a planned rig was refused `PLAN_LIVE` at every
  try, and the rig stood beside the ramp. The surface tick
  tells the note as it tells every plan's end: said to a connected author, queued at trip
  level for one who is not (`surface/tick.rs`, `tell_plan`; `port/tv.rs`, `END_FIELD`). A
  recover that does not land leaves the plan parking, and once the site demotes the plan
  brings the rig home by itself and ends from the bay, by the interpreter's existing parking
  arm. A plan already parking keeps its own reason.
- **A rig that holds resumes its plan** by the rule that was already there: a promoted site's
  starts wait and go on when the flag clears (`surface/tick.rs`, Q-CF-30). Nothing parks the
  plan of a rig that stays, so nothing was needed but the posture being flown.
- **A carrier is home** (Q-45-11; `field/posture.rs`, `flown_by`). The host tells the arena
  of a carrier whose record says `RETURN` the word `HOLD`: its mind stands where it is, turns
  to and answers only a unit that has fired on it, by the orders `HOLD` already lays. The
  record, and what a player reads, still says `RETURN`. **No mind lifts a carrier**: the
  host's pass skips a carrier outright (`field/absent.rs`, `tick`), a carrier has no recover
  for it to ask with, and the kernel's lift (`Arena::withdraw_to` with no ramp) is reached
  only from `field/exit.rs`'s `Then::Lift`, the commander's live `+ship/lift` with its
  leave-behind confirmation, and the raid director's own carrier. When the commander does
  lift, the host's retry list does not take the carrier (`on_out`): the departure is filed
  once, as they typed it, and its refusal is told to them. A wrecked carrier still wrecks the
  rigs in its bay (`field/carrier.rs`, unchanged). Its rigs come aboard by their own
  recovers.
- **The host puts a unit nobody sits in aboard only when the host asked it out** (`on_out`).
  Its record stands for the whole field while its player may be connected: an exit that
  player typed is theirs, told to them, and never tried again behind them.
- **A unit with nothing to fire cannot guard, and stays** (`arena/absent.rs`, `flown`): a
  unit that carries no fire channel at all and is set to `GUARD` is flown as `HOLD`. One that
  had something to fire and has spent it still goes home, as AP.6 built it.
- **The host asks neither half of a tow out** (`field/absent.rs`, `in_tow`): found by this
  unit's first whole run (below). And the kernel judges a casualty in tow by its tug's
  posture as well as its pose (`arena/withdraw.rs`): a tug standing on `HOLD` or `GUARD` keeps
  the pair, so a load typed by its player never lets the casualty out and keeps the tug.
- **A refused join takes its waiting word back** (`field/absent.rs`, `join_refused`).
- **What the player is told** (`field/absent.rs`): `UNATTENDED` and `UNATTENDED_ON`, to the
  rig's operator at admission, through the reconnect queue at trip level; MW.7's `GONE_HOME`
  and AP.6's `STAYS_ON` to the leaders the rig was with, with the clause "nobody is in its
  seat". Nothing is said of a carrier: it stays where it is. Both lines have their rows in
  the bot's sentence table (`crates/mush-bot/src/mine/minds.rs`).
- `docs/help/field.md`: "A rig nobody sits in" and "Your carrier" under *If you lose your
  link, or quit*; the unarmed guard under *Postures*.

**The mind API did not move.** `MIND_ABI` is still `rhai=1.26.0;api=9;stock=3`; no doctrine
source, no `MindView` field and no seal changed. One meaning moved: for a unit with no fire
channel flown for an absent player on `GUARD`, `MindView::posture` and the frame read `hold`
where they read `return`. A carrier's read `hold` while a mind flies it.

**Tests.**

In source:

- `arena::weapons::tests::ap7_a_unit_with_nothing_to_fire_set_to_guard_stays_as_on_hold`:
  flown, framed, ordered and kept at the ramp as `HOLD`, the word kept; a lift with no ramp
  is never refused; the same unit with a spent magazine still goes home.
- `arena_task::tests::ap7_an_absent_word_told_before_its_units_join_is_applied_when_it_lands`:
  the word and the join at one head, the word a head early, a clearing word, a practice match.
- `field::absent::tests::ap7_a_unit_nobody_sits_in_is_recorded_as_its_minds_from_the_start`,
  `field::posture::tests::ap7_only_a_carriers_return_is_told_as_hold`,
  `surface::plan::tests::ap7_a_plan_ended_by_a_field_is_stored_and_worded`.

Stories (`tests/domain_field/field_ap7.rs`), each a field of four parties, every command
typed by a player who is not staff. The ground is one operator's whole outfit left to
itself: his carrier on the pad, his leased armed rig launched and stood off with nobody in
its cab, and his prospector on a pocket it has sounded, with a fill plan he filed by `+tv/2
plan <pocket> go`. He logs off aboard his carrier. A second miner's prospector stands beside
them on `HOLD`, and an intruder's armed hull sits ten cells east, whose co-op the operator's
co-op declared Hostile before the field. The second miner's mark opens the field.

1. `ap7_an_unattended_rig_on_a_raided_site_goes_home_and_its_operator_reads_the_plans_end`.
   Both rigs, the second miner's and the carrier carry the record from admission. The
   intruder's fire lands on the second miner's rig. The armed rig fires nothing. Both rigs
   are let out at the ramp and put aboard. The plan is `Ended { why: Field }`. At login the
   operator reads `UNATTENDED` for each rig and `PLAN_ENDED` with `END_FIELD`, and nothing
   about the carrier.
2. `ap7_the_same_rig_on_hold_is_still_there_at_demotion_and_its_plan_runs_on`. The plan
   starts nothing while the field runs. At demotion the prospector is on its cell, on the
   ground, the plan live at step 0. The rig's next act is started by the surface tick with
   `by: Plan { no: 1 }`, its author still away. He reads `UNATTENDED_ON` at login and no
   plan's end.
3. `ap7_a_carrier_with_nobody_aboard_stays_on_its_pad_and_takes_its_rigs_aboard`. The carrier
   carries the record. The armed rig comes home and is in its bay. The field's own snapshot
   holds the carrier as an absent player's unit on `HOLD`, its record still `RETURN`. A
   minute and a half on, the prospector set to `GUARD` has not been let out (story 1's goes
   home from the same cell in less), is plotted on its cell by the intruder, and the snapshot
   holds its word as `GUARD`; and the carrier is in the field, at its pad pose, with no exit
   on the record. The carrier's last three facts hold on main too, where nothing ever asked a
   carrier out: what this unit adds for a carrier, and what the story pins, is the record,
   the kernel's flag and word, and the rig in its bay.

**Found by the first whole run of `domain_field`, and fixed here.**

- **The host asked a towed casualty out from behind its tug.** The kernel judges a casualty
  in tow by its tug, so the pass's ask for the casualty alone, made once the tug stood beside
  the ramp, took it out of the field and left the tug's load with nothing to load (FR.5's two
  ramp stories failed at "the casualty's stand"). The pass now asks neither a casualty in tow
  nor the tug that tows it. This was reachable before for a seated pilot's unit and is fixed
  for both.

**Found by hosted CI on the first push, and fixed here.**

- **The raid director put a hull that had just boarded back into the field with no order**
  (`surface/raid.rs`, `rejoin_refused`). A raider's hull let out at its ramp goes aboard in
  the batch that lets it go, which spends its WITHDRAW mark, and the site's record names its
  exit only when that write lands. In between the director's "land the hulls the kernel
  refused" check read the hull as refused, landed it again and joined it. Its WITHDRAW's one
  sequence was spent, so the kernel refused the order a second time and the hull fought until
  the field ended: in `field_director`'s barge story it destroyed the miner's carrier, guard
  and a rig, and never went home. The window is on main. The check now also counts a hull
  whose exit is in flight (`field::exit::leaving`, the test the launches already use).
  Evidence, the story run pinned to one core on conclav.dev: main, six of six pass; this unit
  without the guard, between four of six and four of four fail, each with the kill list
  hosted CI printed; with the guard, eight of eight pass. The kernel's log of a failing run
  shows the hull's last think under its order at 14.8 s, no think at 15.8 s, and a fresh
  mind row with no order at 16.8 s. Why this unit makes a slow runner fall into the window
  more often was not established: it adds work to the host's passes and tells the kernel of
  three more units, and switching either off made the failures stop.
- **Two stories the default run ignores** (`m8_world`, D-23-18 at 256 and at 512; they run
  only in the dispatch) had a rig nobody sits in that was to stand: the last rows of the
  table below.

**Pinned outcomes moved, each because of this unit and for no other reason.** The first whole
run failed ten older stories; `domain_field` was otherwise green (270 of 281). All ten had a
rig nobody sits in that the story expected to stand where it was put, and under this unit
such a rig goes home on `RETURN`. No assertion was weakened. Two were the tow defect above
and pass with the fix alone. In the others the rig's operator now says what the story needs
by their own command (`field_host_rig::left_on_hold`: `+tv/<n> posture hold` from aboard the
carrier, the answer compared whole), or takes the rig's console.

| Story | What failed | Why | What changed |
|---|---|---|---|
| `field_fr5` `fr5_ahuna_`/`fr5_urvara_a_coupled_pair_is_withdrawn_at_the_ramp_and_loaded` | "the casualty's stand" | the pass took the casualty out from behind its tug | the tow guard; the stories are untouched |
| `field_gc4` `lane3_a_casualty_keeps_its_row_when_its_tug_falls_irrecoverable`, `gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift` | "Nothing rig 2 could tow lies within a cell of it" | the escort, to lie a casualty beside the wingman, had moved off under its mind | its captain leaves rig 1 on `HOLD` |
| `field_gc4` `lane3_a_loaded_combat_casualty_never_darkens_the_lift` | the tow start: `tow_rig_busy` | the tug stood beside the ramp, was let out at the field's opening and was in its recover | its captain leaves rig 1 on `HOLD` |
| `field_gc3` `gc3_an_irrecoverable_hull_leaves_remains_and_closes_destroyed` | remains at (116, 124), not (114, 124) | the prospector drove two cells toward its ramp before the field settled | its operator leaves rig 1 on `HOLD` |
| `field_rs1` `rs1_ahuna_`/`rs1_urvara_the_tender_works_through_fields_and_the_tow_counts_her_laden` | "the jam is cleared" | the escort the tender works on left its side, and the stranger's pair left theirs | the captain leaves rig 1 on `HOLD`, the stranger rigs 1 and 2 |
| `crew_exit` `cf5_a_rig_recovers_out_of_a_running_field_and_a_stranded_pilot_is_extracted` | the preview read "Rig 1 is climbing the stern ramp" | the prospector on the ramp was let out by its mind before its operator typed the recover | the operator holds the prospector's console (`seat`), so the recover is theirs, as the gate has it |
| `m8_world` `d23_18_a_licensed_mind_carries_its_memory_between_fields_at_256`/`_at_512` (dispatch-only), and the ground they share with S.4b-3's two | "the second promotion: You see no flight there." | rig 1, flown by its licensed mind with nobody in it, drove off toward home in the first field, so the raiders' second mark found nothing where it had stood | its pilot leaves rig 1 on `HOLD` (`licensed_field_story`); the four pass |
| `field_ap6` story 1, and the ground of stories 1 to 4 | "a guard returns fire on the unit that hit its owner's other rig": no fire arrived on the lessee's prospector | both prospectors, nobody in either, had gone home | the lessee leaves his on `HOLD` from aboard the carrier (`escort_lease::stand_up_out`), the miner hers (`field_ap6::ground`) |

No state hash, ledger line, seal identity or harness baseline field moved: a practice match
and the harness set no absent flag, so nothing here is reached there.

**Divergences from the row, and what doc 45 had wrong or left out.**

- **"A carrier on its pad is home: `RETURN` means stay" is built as `HOLD` told to the
  kernel.** The alternative, the go-home order on a hull that cannot move, leaves the house
  mind plotting a drive it cannot make and firing only in its cornered stand.
- **The row says "a carrier with nobody aboard".** The rule built does not read who is
  aboard: a carrier is nobody's to fly in a field, so it is its mind's whoever is aboard and
  connected (default 1 below).
- **Row 4 of §3 said a destroyed rig's plan is cleared and nothing else touches a plan.** A
  plan now also ends when its rig leaves a field by the ramp.
- **Q-45-10 says an unarmed rig "cannot guard" and does not say what `GUARD` on one does.**
  AP.6 flew it as `RETURN`; this unit flies it as `HOLD` (default 3).
- **Files outside the row**: `arena_task.rs` (the waiting word), `arena/absent.rs` (the
  unarmed guard), `arena/withdraw.rs` (the tug's posture), `field/absent.rs`,
  `field/posture.rs`, `field/admit.rs` (one call), `surface/act/providers.rs`, `fixed.rs`
  and `commands.rs` (the park, the end, the exemption, the end's sentence), `port/tv.rs`
  (one sentence), `port/yard.rs` (two matches), `crates/mush-bot/src/mine/minds.rs` (two
  rows). Of the row's own files, `surface/tick.rs` is not touched (its hold and its tell
  already did what the unit needs) and `field/exit.rs` only passes `on_out` one more fact.

**Defaults taken.** None needs a refactor to undo unless it says so.

| | Default | Cost to reverse |
|---|---|---|
| 1 | **Every unit nobody sits in is its mind's in a field, whether or not its player is connected** (Q-45-1 as written: in a field it is the seated pilot). A connected operator's unseated rig goes home on `RETURN` like an absent one's, and never fires first. The other reading, absent only while the operator is offline, needs the operator's presence followed through the field (a grace, a way back with no seat to take) | Medium: a presence rule for a player who holds no seat |
| 2 | **A plan ends when its rig leaves a field by the ramp, whoever asked**: its mind on `RETURN`, or its operator's own `+tv/<n> recover` out of the field | Low: one clause at the recover's start |
| 3 | **An unarmed rig set to `GUARD` is flown as `HOLD`**: it stays and answers nobody. This replaces AP.6's default 6 (flown as `RETURN` at once). "Unarmed" is no fire channel on the unit; a unit that has spent its rounds still goes home | Low: one arm of `flown` |
| 4 | **A carrier's arena is told `HOLD`** whatever its record says, and a carrier is never asked out. No verb sets a carrier's posture today | Low: `field/posture.rs`, `flown_by` |
| 5 | **The carrier's commander is told nothing** when its mind takes it; AP.1's opening line already names the field | Low |
| 6 | **The operator's own recover, and their load of a pair whose tug stands, are refused for a rig standing on `HOLD` or `GUARD`** (AP.6's default 7, unchanged): setting `RETURN` brings it in. On `RETURN` the mind is at the ramp before the operator can ask, so an operator's typed recover of a rig nobody sits in has no story any more; CF.5's story now recovers from the rig's console. The kernel's sentence, and MW.7's `LEFT_AT_RAMP`, say "while its player is away" and "while you were away", which is loose for a connected operator | Low: let a typed ask through the kernel's check; reword two lines |
| 7 | **Neither half of a tow is asked out.** An absent player's tug in tow stands beside the ramp with its casualty until its player loads it | Low |
| 8 | A house rig's plan ended by a field is filed again by its yard like one that ran out of time (`port/yard.rs`). No house rig is flown home today: its dispatcher is no player | Low |
| 9 | The hopper's ore is unloaded by SM.8's queued unload once the field settles, as after any recover on a promoted site | None: existing rule |

**Not built, and where each lives.**

- **A launch into a running field with nobody in it.** If such a hull is some player's, the
  join's answer hands it to its mind, and on `RETURN` it is asked straight back out at the
  ramp it stands beside; its operator hears only the exit's line. FR.3's launches are a
  roster's flights, normally crewed; whether one can go down with an empty cab and a player
  for its operator was not established here. Read in the code, and not built for.
- **The admission's line promises "makes for its carrier's ramp" for every rig on `RETURN`**,
  including one that cannot move or is in tow, which stays.
- **A notice when the plan parks.** Only its end is told. A recover that never lands (the
  carrier lifted, the operator ashore) leaves a live plan and MW.7's `LEFT_AT_RAMP` line.
- **A rig whose operator is not aboard its carrier.** The recover runs as the operator and
  is refused while they are ashore, as their own would be; MW.7's thirty tries then leave the
  rig beside the ramp.
- **A manoeuvring carrier, or a posture verb for one.** A carrier has no verb that sets its
  posture; one set to `GUARD` by a future verb would answer for its commander's rigs if it
  carried anything to fire.
- **A player who boards a rig mid-field.** The live row's pilot is read at admission; a rig
  boarded later stays its mind's until the field closes.
- **Space** (AP.9), **the simulator** (AP.10), **the closing story** (AP.11).

**Not pinned by a story.**

- A carrier returning fire: no shipped carrier carries a fire channel. The gate binding a
  unit with nobody in it is pinned by story 1's armed rig and in source.
- The commander's live lift of a carrier its mind holds (the lift's own stories run with the
  record on the carrier and pass; none asserts the host's retry list stays empty), the tug's
  posture judging its casualty, a refused join's word taken back, and the park's author
  test: read in the code after the second reader's findings, no test of their own.
- The join paths (`on_joined`'s three arms) and a resumed field: read in the code; the
  waiting word is pinned in source.
- The fallback of a parked plan whose recover did not land (it brings the rig home after the
  demotion).
- The leaders' lines for a rig nobody sits in, and `UNATTENDED` said to a connected operator.
- A tug in tow that is an absent player's standing at the ramp (the guard is pinned by FR.5's
  two stories for the casualty).

**Verified, and how** (Linux, conclav.dev, a throwaway clone at main `1a1c721de` with this
unit's diff applied; Windows for the lints).

- **Mutation checks.** Fourteen mutations, each a patch over the unit's diff made in a scratch
  worktree and undone there by applying its own diff backwards, each built and run against
  the unit's three stories (FR.5's pair for the tow guard; the in-source tests where named).
  Every one fails the test named for it. The first ten ran before the second reader's fixes
  and the last four after them; the lines the first ten mutate did not change in between.

  | Mutation | What failed |
  |---|---|
  | The gate lets every shot through and the view is not narrowed (the rig may fire first) | all three stories; story 1 at "an unattended rig does not fire first": the armed rig fired 79 rounds at the declared intruder |
  | A unit nobody sits in is not handed over (main's behaviour) | all three stories, at the record |
  | A rig on `HOLD` goes home (every absent player's unit is flown as `RETURN`) | story 2, "a rig on HOLD is still on its cell at demotion"; six of AP.6's kernel tests |
  | The recover out of a field does not park the plan | story 1, "the plan ends once its rig is aboard": the plan is still `Live` |
  | The plan does not resume (a held start is held for ever) | story 2, "after the demotion the rig's next act is its plan's": none |
  | The host asks an absent player's carrier to lift (two forms: at once, and once a rig of hers is aboard) | story 3, "the carrier is still in the field, on its pad"; story 1, whose prospector then has no ramp |
  | An unarmed rig set to `GUARD` goes home (AP.6's default 6) | story 3, "it stays where it stood"; the kernel test |
  | The host asks a towed casualty out | FR.5's two ramp stories, "the casualty's stand" |
  | An absent word does not wait for a join | the arena task's test |
  | A live plan holds its rig from the exit's recover | story 1: the prospector stands beside the ramp, out of the field, never aboard |
  | A carrier's arena is told its record's `RETURN` | story 3: the snapshot holds the carrier on `RETURN` |
  | A carrier is not handed to its mind | stories 1 and 3, at the carrier's record |

  The stories cannot be compiled on main (they read `Why::Unattended`), so the second row
  stands in for "fails on main today". With the plan's hold in place a rig on a live plan
  could not be recovered out of a field at all, on main as here.
- **The stories and the in-source tests on the final code**: the unit's three stories with
  every story of AP.5, AP.6, MW.7, FR.5, GC.3, GC.4, RS.1, the carrier, boarding, crew-exit
  and escort-lease files (57 run, 57 passed), and 121 in-source tests (`ap7_`, `ap6_`,
  `mw7_`, `ap5_`, `ap3_`, the withdraw tests, the plan's). The whole `domain_field` binary
  was run once before the fixes that followed it (281 run, 270 passed, the ten stories of
  the table above and this unit's first story failing) and again on the final code; the
  second figure is in the pull request.
- Locally (Windows): `cargo fmt --check` and `cargo clippy --all-targets -D warnings` on
  `tacsim` and `mush-bot`, CI's M7 memory-size clippy pass, `cargo doc
  --document-private-items` under `-D warnings`, the bot's sentence census, and `cargo test
  -p mush-contract-tests` (nothing re-blessed).
- **CI**: the pull request's CI and a full dispatch on the head commit; the runs are in the
  pull request's body.

**Not verified.**

- Nothing was run against a live server or over SSH, and nothing was deployed. The lines a
  player reads were read in the test rig's line mode.
- **The stories the default run ignores were not run before the first push**, which is how
  the D-23-18 pair reached hosted CI failing. Of the field suite's twelve, the four
  licensed-mind stories were then run by hand on the fixed code; the other eight
  (`c4_filed_mind_field`, `m8_leaks`' three, `m8g`'s three, S.4b-2) passed in the dispatch.
- The whole `tacsim` suite was not run by hand: one attempt on conclav.dev was ended by a
  signal that was not this session's. CI's shards run it on the pushed head.
- The cost of the pass at a field of a hundred units. For a unit that is nobody's it is one
  read of whose the unit is a second.

**Reviewed.** A second agent (Opus) read the diff adversarially, read-only, with the row and
the rulings and not this note's conclusions. It found the gate still one function, the mind
API unmoved, a practice match unaffected, the restart sound, the bookkeeping of what the
kernel was told sound, and no assertion weakened in the older stories. Its findings, and
what was done:

- **The commander's own live lift put the carrier on the host's retry list**, which would
  have filed the departure again as the commander every ten seconds and swallowed its
  refusal. Fixed: `on_out` takes a unit nobody sits in only for the host's own quiet ask, and
  never a carrier.
- **A tug nobody sits in, standing on `HOLD` or `GUARD`, split its pair** when its player
  loaded it in a field: the casualty was let out on the tug's pose and the tug kept. Fixed
  in the kernel: the casualty is judged by its tug's posture too.
- **The park ignored the plan's author.** Fixed.
- **A stale waiting word could outlive a refused join** and bind a later join of the same
  hull. Fixed: the refusal takes the word back. Two comments the waiting word made false are
  corrected.
- **Story 3's carrier checks would pass on main, and the carrier's `HOLD` word was not
  tested.** The story now reads the field's snapshot for the carrier's flag and word, and
  says in its head and here which of its facts are new.
- **Story 3's `GUARD` check was read too early to tell a rig that stays from one still on its
  way** (this session's own mutation run found the same). It now waits a minute and a half.
- **The help said an operator's own recover out of a field ends the plan**, which for a rig
  nobody sits in happens only by beating the mind to the ramp. Reworded; the lost story is
  default 6.
- **The pass walked every unit that is nobody's twice a second.** It now asks whose the unit
  is once and hands over only a player's.
- A seated rejoin could be handed over for an empty seat's reason. Fixed in `owed_why`.
- Left as recorded: story 1's "does not fire first" rests on the mutation run for its
  control (with the gate off the same rig fires 79 rounds in the same story); a launch into
  a field with nobody in it; the two loose sentences; AP.8's readers of the retry list by
  unit (whether a roster unit can be a player's unit nobody sits in was not established).

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

Space reads the same posture (doc 45 §7.1; Q-45-23, Q-45-24, with Q-45-7, Q-45-8, Q-45-12
and doc 44's Q-44-38). It is also the follow-up doc 44's MW.9 note left as "return fire in
space". **The mind ABI did not move**: it is `rhai=1.26.0;api=9;stock=3`. Paths are under
`crates/tacsim/src/` unless given.

**What a ship could do with a posture before this unit.** Read it, and nothing else. AP.3
put the word in a space mind's view (`v.posture`, off the hull's own record), no verb set it
on a ship, the house default did not read it, and the backup seat dropped every lock and
every shot whatever had been fired at her (`Seat::ABSENT`, `FireRule::Never`). A bubble kept
no record of which ship had fired on which: `Relations::fired` is by party.

**Built.**

- **One record, the same words, and a verb** (`posture/order.rs` `ship`, `port/ship.rs`).
  A ship's posture is the attribute every unit's is (`posture::record`, AP.3). No second
  record was grown. `+ship/posture` reads it anywhere, and to a held character too;
  `+ship/posture return|guard|hold` sets it at once for whoever commands her (`helm_of`),
  from aboard her, through the record's one writer. It is refused from an OOC room by AP.4's
  sentence, and off her by "A posture is an order to a ship in the world. Set The <name>'s
  from aboard her." `+ship` shows it when it is not the default. Help: `docs/help/ships.md`,
  "Her posture", and a line in `docs/help/field.md`, "Postures".
- **The bubble records whose fire has arrived on whom, by ship**
  (`space/bubble.rs`, `LocalSpaceBubble::fired_on`, `fired_on_by`). Written where a shot's
  flight ends on its target, hit or miss, as the arena writes at an impact: a beam's shot; a
  slug at the end of its flight, struck or dodged; a torpedo that strikes, or that her side
  stops short of her (a defensive beam, a point-defence burst, a decoy). It is part of the
  bubble's document, absent while nobody has fired, so a restart reads it back, and it is
  folded into the bubble's state hash when it is not empty. A stand-down does not clear it.
- **One gate** (`space/applier.rs`). `FireRule` has a third variant, `Answer`, and the seat
  hands the applier `Answers`: the ships whose fire has arrived on her, whether she stands
  guard, and the ships whose fire has arrived on another unit of her player. `Answers::permits`
  calls `arena::absent::permits`, the ground's rule, with those facts and nothing else; no
  second rule was written. The applier asks it at its two doors. A `lock` on a ship she may
  not answer is dropped, because a bubble's fire commands take their target from the lock.
  A shot at a lock she may not answer is dropped whoever set that lock (a crew's lock left
  standing is not a target). The seat has no act that mines or trades.
- **The seat reads the posture** (`space/backup.rs`, module head "the posture her player
  left on her"; `standing`, `answers_of`, `flown`, `complying_with`, `player_of`,
  `stand_guard`).
  - `RETURN` is what it was: the filed-port return outside a bubble; inside one she heaves
    to when stopped and never locks or fires (`FireRule::Never`).
  - `HOLD` and `GUARD` outside a bubble: she holds where she is. The seat reads a passage
    asked for as a hold, so no source can take her home against the word, and says once
    "[SPACE] Backup mind: holding station where she is; her posture is HOLD."
  - `HOLD` inside a bubble: the same helm as `RETURN` (the one burn let through is the one
    that heaves her to; hove to under a customs hail she is submitted to inspection), and
    the house default may answer a ship whose fire has arrived on her.
  - `GUARD` inside a bubble: as `HOLD`, and the house default may also answer a ship whose
    fire has arrived on another hull of her player in the bubble (AP.6's one widening).
  - The first time she answers, her crew are told "[SPACE] Backup mind: returning fire."
    Her own shots go with the step's events, so the black box has them.
- **Who asks to fire: the house default** (`content/minds/linkdead_return.rhai`). On the
  ground a posture lays a standing order the stock mind already obeys. A bubble has no
  orders, and the house default never asked to fire, so with the gate alone nothing would
  have answered. The doctrine now reads `v.posture`, which AP.3 gave it: on `HOLD` and
  `GUARD` inside a bubble it locks the nearest contact its view shows hostile and fires what
  is ready, then flies its helm as before; outside one it asks to hold. It is a source
  change and not an API change: no getter, intent or kernel moved, and the doctrine's digest
  is pinned nowhere.
- **What the mind is shown** (`space/view.rs`, `Situation::answerable`). A hull flown on
  `HOLD` or `GUARD` is shown as hostile only the contacts she may answer; every other
  contact her party holds hostile reads neutral, as on the ground since MW.7. Narrowed,
  never widened, and the seat's doors do not depend on it. On `RETURN` the view is the bytes
  it was.
- **`arena::absent::flown_as`** (`arena/absent.rs`): the rule that reads a `GUARD` over what
  a unit carries, lifted out of `flown` as a pure function so the bubble reads it too.
  `flown` calls it; nothing the arena does changed.
- **A carrier on her pad is told `HOLD` whatever her record says** (`field/posture.rs`,
  `flown_by`). Until this unit no verb could set a carrier's posture, so only `RETURN` was
  mapped. A carrier is a ship, and `+ship/posture` can now leave `GUARD` on her record.

**What was settled from the code.**

- **Holding station, for a hull under way when the bubble opens.** She is on a passage, and
  a passage is her player's own act (Q-44-45): the seat never thinks for a hull on a
  corridor, under any posture, so she keeps it. Inside the bubble her drive is cut and she
  keeps the way she had; the seat lets through the burn that heaves her to and no other.
  She does not brake to rest of her own accord: a Dray's drive gives under 1 m/s² and she
  enters a picket's bubble 25 m/s off it, and a hull that stopped for nobody would never
  leave. When the bubble lets her go she is on her passage again, at the leg she left.
- **`HOLD` and customs.** While a picket's or an interdictor's stop on her stands, and
  after it has cleared her, the ship that stopped her is not one she answers, whatever
  arrived from it (`complying_with`). A hull that ran is past that. Without this rule a
  `GUARD` hull would take up the fight with an interdictor that fired on her player's other
  hull, be stood down when she hove to, and take it up again from the ship-by-ship record,
  which a stand-down does not clear.
- **A hull that cannot defend herself.** Read as on the ground: on `GUARD` with nothing to
  fire she is flown as `HOLD`; on `GUARD` with her drive section destroyed, or with no beam
  and no round left, as `RETURN`. Her record is kept. Outside a bubble the seat reads the
  record alone, where `HOLD` and `GUARD` do the same thing.
- **Presence** is MW.8's rule and was not touched.
- **Each carrier is a party of its own in a bubble, and carriers are marked Friendly to each
  other when they join.** So a `GUARD` hull's party does not hold hostile a ship that fired
  on her player's other hull, and the bubble would refuse the shot the gate lets through.
  When she stands guard the seat marks that ship's party hostile in her own party's book
  and no other (`stand_guard`). The mark is made on the bubble, not through the tape, as a
  clearance and a stand-down are.
- **No leash in space.** A guard that may not burn cannot leave where it stands.

**Defaults taken.**

| | Default | Cost to reverse |
|---|---|---|
| 1 | **A mind sealed aboard keeps the helm and the sensors under every posture, and nothing answers fire for her.** By ruling a ship's seal carries no fire channel (Q-44-26), and the seat lands one think, the first source's. So a player who seals a mind of their own on a ship gives up the return fire `HOLD` and `GUARD` give the house default. The other reading is that the house default's think is landed beside the sealed mind's and answers under the gate | **Medium.** `space/thinks.rs` answers one think a hull: it would have to hand back the house default's as well, and the seat would have to rule whose lock stands, the sealed mind's sensor lock or the answering lock. No record moves |
| 2 | **On `RETURN` she still never fires**, in a bubble, though Q-44-38 would let her answer a ship that fired on her. It is doc 35 §11 `B-4` as it was built, and it keeps the three words different: `RETURN` runs home, `HOLD` and `GUARD` stay and answer | Low: one arm in `backup.rs` (`answers_of` answers `None` on `RETURN`) and one test in the doctrine |
| 3 | **She never answers the ship she has hove to for**, while its stop stands or after it cleared her | Low: one filter (`complying_with`) |
| 4 | **Holding station inside a bubble is a cut drive, not a stop.** She keeps her drift | Low: `BurnRule` for a hull with no stop on her |
| 5 | **What "arrived" means.** A torpedo her side stops short of her counts; one that burns out and is lost does not | Low: one call in `bubble.rs` |
| 6 | **Whose a ship is**: whoever commands her by charter or title (`port::coop::helm_holder`), if a player. A cast hull and a hull a non-player commands are nobody's | Low |
| 7 | **A ship's posture is set from aboard her and by no relay.** A rig's goes over the field's comms relay; nothing carries an order to a ship | Medium: a relay for ships is a new transport |
| 8 | **Only her commander sets it** (the helm `+ship` reads), not a crew member or a passenger aboard | Low |
| 9 | **A carrier on her pad is told `HOLD` whatever her record says.** A carrier that stood guard over her player's rigs from the pad is not built | Low to map `GUARD` through; what a guarding carrier does on the ground is AP.7's question |
| 10 | **The record is folded into the bubble's state hash** when it is not empty. No bubble hash is pinned by value anywhere; the replay tests compare a bubble with its own replay | Low |
| 11 | **A hull left adrift on `HOLD` or `GUARD` stays until her player comes back.** Nothing sends her home when her stores run low | Low: a floor in `land_adrift` |

**The acceptance, and what holds each.** The stories are in
`crates/tacsim/tests/domain_field/space_ap9.rs`, on the fitted excursions rig (the shipped
lane: a passage from Ceres to Stickney crosses the Kirkwood picket, whose bubble holds the
customs corvette and the belt raider). Every act is a registered command typed by a player
who is not staff. **No staff fiat is used**, and the stories write no module state.

| The row asks | Held by |
|---|---|
| A ship and a rig of one player, both set to `HOLD`, both stay and both hold fire | **The record half**: `ap9_one_player_sets_a_ship_and_a_rig_to_hold_by_the_same_word_on_the_same_record`: one player on a pad sets the Dray-P by `+ship/posture hold` and her rig by `+tv/1 posture hold`, and `posture::record::of` reads `HOLD` for both, across a restart. **The rig's flying** is AP.6's and AP.7's, asserted through that record and not flown again here: `field_ap6::ap6_the_same_rig_on_hold_does_not_answer_for_its_neighbour`, `field_ap7::ap7_the_same_rig_on_hold_is_still_there_at_demotion_and_its_plan_runs_on`. **The ship's**: she stays (`ap9_a_ship_left_at_a_cluster_holds_station_on_hold_and_guard_and_makes_for_port_on_return`) and she holds fire (the picket story below: before the raider's fire arrives she has locked and fired on nobody, though her party holds the raider hostile from the bubble's opening) |
| The ship set to `GUARD` returns fire on the hull that hit her and not on one that hit a stranger | `ap9_at_the_picket_a_ship_answers_the_raider_that_fired_on_her_by_her_posture` (her arm on `GUARD`: her fire arrives on the raider and on nobody else, with the corvette and a second pilot's ship beside her) and `ap9_a_ship_left_on_hold_or_guard_does_not_answer_for_a_stranger` (the second pilot runs the picket, the raider opens on them, and she locks nothing and fires on nobody). On the bubble's own step, with no world: `space::tick::thinks_tests::ap9_the_house_default_answers_the_raider_that_fired_on_her_by_her_posture` |
| A ship on a plan, interdicted, complies under `HOLD` and is on her plan again after the bubble | The first story's `HOLD` arm: she hove to by her mind's own burn and customs cleared her before the raider lit, she never ran, and once the picket lets her go she is under way on her filed passage, and still is after a restart |
| (§7) Changing the posture changes what the ship does | The same ship, hit by the same hull, answers on `GUARD` and on `HOLD` and not on `RETURN` (the first story, and the tick-level test); the same ship at the same cluster holds station on `HOLD` and `GUARD` and makes for port on `RETURN` |
| (the brief) The contact picture, the presence picture, and a hull she cannot detect | `space::view::tests::ap9_a_hull_that_answers_is_shown_hostile_only_what_she_may_answer`: a ship her sensors do not hold, added to the bubble and to the set she may answer, leaves her view the same bytes; MW.9's own pin is untouched and passes. Nothing an attacker reads moved: no feed, panel, contact row or presence row carries the posture, the record of arrivals or what she may answer (`holotank_feed`, `cockpit_feed` and `surface/presence.rs` are not touched by this unit) |

What separates `GUARD` from `HOLD` in space, an answer for another hull of the same player,
is pinned at the seat and not by a story: `space::backup::tests::ap9_what_a_hull_may_answer_is_read_off_the_bubbles_record_by_her_posture`,
`ap9_standing_guard_marks_only_the_party_that_fired_on_her_players_unit` and
`space::applier::tests::ap9_a_guard_answers_for_her_players_other_unit_and_a_hold_does_not`.
No route in play puts two hulls one player commands into one bubble: a player commands the
hull `+ship` reads, and a second hull of theirs has no crew to lose a link.

**For AP.11.** `space_ap9::left_on_posture_to_the_picket(rig, pilot, hull, posture,
not_before)` has the ship's commander set the posture from aboard her, files her passage
for Stickney, drops their link, jumps the grace and brings her into the picket's bubble
with the corvette and the raider (and any pilot the story sailed first, whose first leg
ends at `not_before`). `space_ap9::ship_at_the_picket(rig, hull)` reads what she did: her
lock, the ships her fire has arrived on (`answered`), the ships whose fire has arrived on
her (`fired_on_by`), what customs made of her, her thrust, her mind's last word and the
seat's last act. `space_ap9::until_ship` steps a second at a time until a reading holds.
They take `excursions::Rig`, which the combined M8 world carries as `world.rig`.

**Not built, and for whom.**

- **Return fire for a hull with a mind sealed aboard** (default 1). The owner's call.
- **A relay that carries a posture to a ship** (default 7).
- **A carrier that guards from her pad** (default 9).
- **Stances in space** (Q-45-24): not in this design.
- **A story for `GUARD`'s widening**, and for the two readings of a hull that cannot defend
  herself: unit tests only, for the reason above and because no story disarms a ship.
- **Nothing an attacker reads says she is flown by a mind**, as on the ground (Q-45-16).
- **A leash**: none in space.

**Mutation checks.** Each was applied by an edit over a WIP commit, in batches of mutations
that fail different tests, the `ap9_` unit tests or the four stories run, and the edit
undone by the reverse edit. All were caught.

| | Mutation | Fails |
|---|---|---|
| U1 | The first door lets a seat that answers lock any ship | `space::applier` `ap9_a_seat_that_answers_locks_and_fires_only_on_a_ship_that_fired_on_her` |
| U2 | The second door trusts the lock that stands | `ap9_a_shot_at_a_lock_she_may_not_answer_is_dropped_and_return_never_fires` |
| U3 | `RETURN` is given something to answer | `space::backup` `ap9_what_a_hull_may_answer_is_read_off_the_bubbles_record_by_her_posture` |
| U4 | She answers the ship she complies with | The same |
| U5 | `HOLD` is given `GUARD`'s widening | The same |
| U6 | Her player's own unit may be answered | The same |
| U7 | A beam's shot is not recorded | `ap9_the_bubble_records_whose_fire_arrived_on_whom_and_a_snapshot_keeps_it` |
| U8 | A slug's arrival is not recorded | `ap9_a_slug_and_a_torpedo_are_on_her_record_when_their_flight_ends` (its slug half) |
| U9 | A torpedo is recorded neither when it strikes nor when it is stopped | The same (its torpedo half) |
| U10 | The view is not narrowed | `space::view` `ap9_a_hull_that_answers_is_shown_hostile_only_what_she_may_answer` |
| U11 | Standing guard marks both parties' books | `ap9_standing_guard_marks_only_the_party_that_fired_on_her_players_unit` |
| U12 | Outside a bubble the seat does not hold her to her posture | `ap9_a_passage_asked_for_on_hold_or_guard_is_a_hold` |
| U13 | A mind sealed aboard is given the answering seat | `ap9_a_mind_sealed_aboard_is_never_given_the_answering_seat` |
| U14 | An unarmed `GUARD` is not flown as `HOLD` | `ap9_a_guard_that_cannot_defend_herself_is_flown_as_on_the_ground` |
| U15 | A `GUARD` that cannot defend herself is not flown as `RETURN` | The same |
| U16 | The house default does not ask to answer | `space::tick::thinks_tests::ap9_the_house_default_answers_the_raider_that_fired_on_her_by_her_posture` |
| S1 | The verb sets a posture from off the ship | The record story |
| S2 | The house default does not ask to hold station (the seat still holds her) | The cluster story: her mind's word is not `BACKUP_HOLD_STATION` |
| S3 | The seat never answers | The picket story: on `GUARD` her fire arrives on nobody |
| S4 | The seat lets her answer any ship her party holds hostile, with U10 | The stranger story: she locks and fires on the raider |

U1 with U10 also fails the tick-level test, where she locks the raider before it has fired.
S4 alone changes nothing a story can see: the doors and the narrowed view each stop the house
default by themselves, which is why the doors are pinned at the applier with a mind that asks
anyway.

**Not pinned by a test.** That a carrier's arena is told `HOLD` when her record says
`GUARD` (`field/posture.rs`; the function is `pub(crate)` and no story opens a field round a
carrier set to `GUARD`). That `+ship/posture` with no word is answered to a held character.
The two lines her crew are told.

**Persisted records.** `LocalSpaceBubble` gains `fired_on`, absent on the wire when empty: a
bubble written before reads as it did. `BackupAct` gains two words, `holding_station` and
`returning_fire`; a server older than this unit would not read a record that holds one. No
definition, descriptor, manifest or seal moved, and no pin was re-pinned. The house
default's source changed, so its digest did: a store that names an operator's own house
default by digest (`tacsim:doctrine:linkdead_return`) still flies that one, which does not
read the posture, and the seat's rules hold it all the same.

**Found, and not fixed.**

- **No carrier can fire on another carrier in a bubble, by any command** (by reading, not
  run). A carrier that joins a bubble is marked Friendly with every carrier in it, both ways
  (`space/tick.rs`, the join), a mark stands ahead of `Relations::fired`, and no `+tac` or
  `+helm` switch marks a party. So `+tac/beam` at another player's ship is always refused as
  "Cannot fire on a friendly contact." The only ships a carrier can fire on are the raider,
  an interdictor, and the corvette once she has run. It is why a stranger's ship cannot be
  the hull that fires in these stories.
- **A hull left adrift on `HOLD` has nothing that brings her home** when her stores run low
  (default 11).

**Verified, and how.** On Windows, at this branch's head: the thirteen `ap9_` unit tests
with the space suite's `mw9_`, `mw11_` and `mw13_` tests beside them; the four stories (about
twenty seconds together, no wall-clock window waited out: the link-loss grace is jumped and
a bubble is stepped a second at a time, as it flies); `cargo clippy -p tacsim --all-targets`
with warnings denied; the lint job's memory-size pass; rustdoc with warnings denied and
private items; `cargo test -p mush-contract-tests`; `cargo test -p mush-bot`. The
memory-size pass failed once: one more field on the bubble took an older test's frame
(`mw11_a_step_whose_thinks_are_out_is_held_and_the_thread_goes_on`) from under 64 KiB to
66,213 bytes, and its saved copy of the run is now kept on the heap. **Not run on this
machine**: the rest of the tacsim suite, `domain_world`'s space stories among them
(`voyage.rs`, which flies `RETURN` at the picket and at a cluster); they are the branch's CI
run's. conclav.dev was not used: a gate run held it.

## AP.10

Postures in the simulator (doc 45 §7.1; Q-45-25, with doc 44's Q-44-19). The last link of
the posture chain, built on AP.3's word, AP.4's seat form and AP.6's kernel. Paths are under
`crates/tacsim/src/` unless given.

**What a simulator unit could do with a posture before this unit.** Read it, and nothing
else. `MIND POSTURE` in a pod answered "holds RETURN" off the hull's own record, which no
verb could write for a hull in the centre; a set was refused as from an OOC room (AP.4's
default 7). The keyframe carried `posture: return`. A match's task dropped both of the host's
words (`ArenaMsg::Posture`, `ArenaMsg::Absent`), so no hull in a match was ever flown for an
absent player, and the kernel kept no record there of who had fired on whom.

**Built.**

- **Three forms of the seat's `MIND` verb, in a pod** (`match_/practice.rs`, new;
  `match_/cockpit/notices.rs`).
  - `MIND POSTURE [RETURN|GUARD|HOLD]` reads the hull's **practice posture**, or sets it. Set
    only by the seat that holds the helm, as in a field. The word is kept on the running
    match's mirror and the arena is told nothing until the pilot lets go. A set while the
    mind flies the hull is told at once, so a pilot can change the word and watch what
    changes.
  - `MIND AWAY` lets go of the controls. The pilot stays in the pod, seated on the board,
    bound in the registry and on the arena's frame roster, so the cockpit goes on reading.
    The hull is handed to its sealed mind as an absent player's.
  - `MIND BACK` takes the controls again where the hull stands.
  In an IC field `MIND AWAY` and `MIND BACK` are refused in one line; there a unit's mind
  takes it when its player leaves the controls. `MIND POSTURE` in a field is AP.4's,
  untouched.
- **The practice reaches the kernel by the field host's own two words**, and by nothing new
  (`arena_task.rs`, `Task::apply_practice_words`). At `MIND AWAY` the simulator sends
  `ArenaMsg::Posture` (the word, and whose the hull is) and `ArenaMsg::Absent`; a task with
  no field terms now applies them at the pulse's head by the calls a field's does
  (`Arena::set_player`, `Arena::set_posture`, `Arena::set_absent`). The hand-over itself is
  the seat's `Released` on each channel **naming no pilot** (`arena_bridge::send_control`):
  the kernel hands the hull to its mind and lays the posture's standing order
  (`arena/intents.rs`, `apply_control`'s existing arm), and because the control names nobody
  the arena drops no pilot from its frame roster, the registry keeps the binding and the
  board keeps the slot seated.
- **One gate.** Nothing was added to `arena/absent.rs`'s rule: `permits` is untouched, and a
  practising hull comes to it through `Arena::resolve_fire_by` as every absent player's unit
  does, because the practice sets the same flag. One line beside the rule moved:
  `Arena::note_fired_on` kept its fire record only in a field whose end is its host's, so in
  a match the rule would have had nothing to read and a practising hull could never answer.
  It now also records on a hull **a pilot has named a player for** (the practising hull and
  the pilot's other units, from the pulse the word lands). A match nobody practises in
  records nothing, carries no byte of it and hashes as it did.
- **The standing orders are AP.6's** (`Arena::lay_standing`, `Arena::stand_posture`), reached
  with no change: `GUARD` and `HOLD` park the hull and turn it to face what it may answer;
  `RETURN` lays the go-home order.
- **A practice ends on the seat's own control** (`arena/absent.rs`, `Arena::end_practice`;
  `arena/intents.rs`, `apply_control`). See "How a practice is told from a drop".
- **The input tape** (`arena/input_tape.rs`, `minds/resimulation.rs`). AP.3's note said a
  practice posture "would have to ride the input tape to replay". It does: two event kinds,
  `TapeEvent::Posture` and `TapeEvent::Absent`, recorded by the task at the head it applies
  each word and only when the word moves the arena, ahead of that pulse's own event; the
  replay applies them at the same boundary. The hand-over's control, and the control that
  ends a practice, were already on the tape with their pulse.
- **The seat gives no order while its mind has the controls** (`arena_bridge.rs`, `hands_on`;
  `IntentRefusal::Away`). The arena takes a seat's `HEADING` on a hull its mind flies, so
  without this a watching pilot's order would land beside the mind's. Every door a seat's
  order goes through (`forward_intent`, `forward_order`, `forward_release`) refuses with
  "Refused: the seat gives no order while the mind flies the hull; MIND BACK takes the
  controls."
- **What the watching pilot reads.** The cockpit keyframe, sent as before: the hull's pose,
  speed, rounds and contacts, `posture`, and `order`, which is the standing order the mind
  is under (a pilot's own hull carried none; a hull its mind flies does). `STATUS` adds
  "its mind has the controls; MIND BACK takes them again" under the posture line. Bare
  `MIND` replaces "you hold helm, sensors and fire; the mind is dormant while you fly" with
  "the mind holds every one and flies the hull on its posture". `VIEW`, `CONTACTS`,
  `WEAPONSTATUS` and `WATCH` read as they did.
- **The debrief** (`match_/report.rs`, `HandOver::LetGo`; `match_/lobby.rs`,
  `practice_span`). `SIM REPORT`'s `Your mind` block says when the mind flew the tank and
  counts the mind's shots apart from the pilot's, by spans the world opens and closes. A
  practice opens one at `MIND AWAY` and closes it where the practice ends: "It flew your
  tank from 0:42 to 1:10, while you had let go of the controls." Without it the rounds the
  mind fired on `GUARD` would have been reported as the pilot's own.
- **Help**: `docs/help/simulator.md` has a new section, "Practising a posture";
  `cockpit.md` lists the three forms; `field.md`'s "Never from an OOC room" points at the
  practice. `MIND`'s own help line and usage line name the forms (the usage line was reworded
  to stay inside 120 columns).
- **The bot's sentence table** (`crates/mush-bot/src/mine/minds.rs`): a seventh family,
  `practice`, eleven rows, with its census over `match_/practice.rs`.
- `crates/mush-contract-tests/legacy-handlers.toml`: the `mind` note is refreshed. No new
  verb, switch or family.

**How a practice is told from a drop.** By who said it, and the arena decides it on the
control.

- **A practice is an act typed in the seat** by a pilot who is connected and stays in the
  pod: `MIND AWAY`. Its hand-over reaches the arena as a `Released` that names no pilot, with
  the absent word ahead of it.
- **A drop is a fact about the pilot's seat that the world tells the arena**: the link lost,
  the grace run out, a `QUIT`, an `EXIT`, a boot's resume, and a pilot coming back. Every one
  is a control that **names the pilot**.
- **In a match, a control that names the practising pilot ends the practice before it
  applies** (`Arena::end_practice`, called at the head of `apply_control`). The rule is
  lifted, the arena's copy of the posture goes back to `RETURN`, and the hull is handed from
  its mind to the pilot exactly as a `Live` hands one over. The control then does what it
  always did, to a hull its pilot holds. So:
  - a `Released` hands the hull to its sealed mind seeded as every release seeds it
    (`Withdraw`, the doctrine's one retreat unspent, the seat's slots cleared, the motor let
    go, no standing order, no rule). What else the mind remembers of the thinks it made
    while it flew the practice it keeps, as a mind's memory is kept across any reclaim and
    drop;
  - a `Linkdead` leaves a link-dead pilot's hull: for the grace it stands still and fires
    nothing;
  - a `Live` is the pilot back: `MIND BACK` sends exactly that.
- **The hull is stopped when a practice ends**, as a `Released` stops one: the speed its mind
  last ordered is nobody's order once the mind is off the controls, so a hull handed back by
  `MIND BACK`, or held through a lost link's grace, does not drive on by it.
- **Never in an IC field.** There a `Released` naming the pilot is the host handing an
  absent player's unit to its mind (MW.7), with the flag the host set standing. The kernel
  tells the two by the arena's end condition (`EndCondition::Host` is the field's), as it
  already does for the fire record and the sortie's close.
- **Whichever seat the control names.** A hull's controller is the hull's and not a seat's:
  any pilot's `Live` takes a hull from its mind and any pilot's `Released` gives it one, so
  a word about a second seat of a practising hull moves who flies it, as it always did, and
  the practice does not outlive that. A control naming nobody (the practice's own
  hand-over) ends nothing. No pod of the centre has a second seat today.

**Why it is on the control and not a message of its own** (the first build's mistake, found
by the second reader). The first build ended a practice from the world: a `Live` on every
channel, a word clearing the flag, then the caller's own control. The arena task keeps one
control a channel a pulse and the latest stands (`arena_task.rs`, `buffer_control`), so a
`Live` sent ahead of a `Released` in one world step was collapsed away and the `Released`
met a hull its mind already held: `set_controller` does nothing for a hull already under
that controller, so the dropped hull kept the practice's memory (told to stay, its retreat
spent) and was not seeded for home. It fought, so every story passed. Ending the practice
where the control is applied makes the result the same however the task buffers, and it
needs no message that could be refused or arrive apart.

A restart: the snapshot holds the flag and the player. The resume's own release of every
seat is a `Released` naming the pilot, which ends the practice in the arena; the resumed
match's mirror is rebuilt from the decoded arena so that the world's half runs too and
closes the debrief's span, which the restored fold still held open.

**The simulator is out of character.** The practice posture is kept on the running match's
mirror (`MatchMirror::postures`, `MatchMirror::away`) and, while the pilot is away, in that
match's arena, and nowhere else. `crate::posture::record` is neither read nor written by any
of the three forms, and `posture::order::set_present` is not called. The story writes
`GUARD` into the practising hull's own record by hand before the match: the pod reads
`RETURN`, and the record is still `GUARD` after the practice has set `GUARD` and `HOLD`. A
second story shows the next match opens on `RETURN` for a hull the last one left on `HOLD`,
with no attribute on the hull.

**The mind API did not move.** `MIND_ABI` is `rhai=1.26.0;api=9;stock=3`; no `MindView`
field, no getter, no doctrine source, no seal. `FRAME_VERSION` is 10 and `SNAPSHOT_FORMAT`
19. `TAPE_FORMAT` stays 2 and `REPLAY_ENGINE` stays `tacsim-engine-cf0-v1`: a tape recorded
before this unit holds neither new event and no practising hull, so it replays byte for
byte; and one that holds the new events cannot be read by an older engine at all (an unknown
event kind is refused at decode), so no tape can replay to a different answer under the same
stamp. One meaning widened, as AP.6 and AP.7 widened it: the keyframe's `posture` and
`order` are now also sent to a seated pilot whose mind is flying a posture, where AP.6 could
say "no seat is sent that frame". For a `GUARD` the hull can no longer defend, that frame
reads `return` while `MIND POSTURE` and `STATUS` go on saying `GUARD`, as the help says of a
field. A sealed mind that reads `v.posture` reads the practice word only while the practice
runs: a posture set and never left, and a hull after a drop, read `return` as they did.

**Which matches take a posture.** Any match with a pilot in a pod: a skirmish, a hold match
and a range. A range's targets never fire, so a hull left there on `GUARD` or `HOLD` stands
and fires nothing, and on `RETURN` drives home. A match of machines only (a staff
`@arena/start`, a proving run, the harness) has no seat to set one from and takes none. An
IC field is AP.4's and AP.6's and is not touched.

**Tests.**

In source:

- `arena::weapons::tests`:
  - `ap10_a_practising_hull_in_a_match_is_held_to_the_one_rule`: a match records no fire and
    binds nobody until a pilot is named; the words that name a player start the record on
    those hulls and on no other; a posture alone binds nothing; with the pilot's hands off,
    the hull is refused the first shot by every source no seat typed, refused after fire on a
    stranger, let through after fire on the pilot's other hull; `HOLD` does not answer for
    the neighbour and does for itself.
  - `ap10_a_drop_ends_a_practice_and_leaves_the_hull_as_any_dropped_pilots`: a hull whose
    pilot practised `GUARD` (the mind stood, told to stay, its helm left at speed) and one
    whose pilot never let go, each given the same `Released`: the same controller, flag,
    posture, order, speed, doctrine state and, in this fixture where the mind stood and
    never thought, the same memory bytes (the release's seed overwrites what standing the
    posture wrote). A `Live` and a `Linkdead` leave the pilot's hull, stopped. A control
    naming another seat ends the practice too; one naming nobody does not.
  - `ap10_a_fields_release_does_not_end_what_its_host_said`: the same control in a field
    leaves the host's flag, the posture and its order.
- `arena_task::tests`:
  - `ap10_a_practice_match_takes_the_posture_and_the_absent_word`: the two words applied by
    a match's task, posture first; a word that says what stands moves nothing; a word that
    clears the flag is dropped; then the pilot's `Live` and `Released` are put through the
    task's own buffer, which keeps the `Released` alone, and that one control ends the
    practice and seeds the hull for home.
  - `ap10_a_practice_word_is_recorded_when_it_moves_the_arena_and_only_then`: each word that
    moves the arena is one tape event, in order; nothing else records.
- `minds::resimulation::tests::ap10_a_recorded_practice_replays_through_its_two_words`: a
  recorded match with a practice replays to its recorded hash with the hull still absent on
  `HOLD`; the same match with nobody practising ends on another hash.
- `match_::practice::tests`: the lines, and every line inside 120 columns with a hull's name
  of twenty characters.
- `match_::cockpit::tests` (`MIND`'s help and usage as reworded, the usage inside 120) and
  `match_::report::tests` (the span line for a practice, open and closed, and its width).

Stories (`tests/domain_arena/cockpit.rs`, over the production composition; every command
is typed by a pilot who is not staff, and the set-up stands the hulls with the staff
placement verb the arena's other stories use; five of the six run under paused time):

1. `ap10_a_hull_left_on_guard_in_a_practice_match_answers_only_for_its_pilots_units`. Red:
   the lead in the hull that practises, the side's one machine (the lead's other unit), and
   a second pilot's hull (a stranger's). Blue: a pilot six cells off, and two more cold at
   their muster so blue fields no machine. The hull's own record is written `GUARD` by hand
   first and the pod reads `RETURN`. `MIND POSTURE GUARD`, `MIND AWAY`: the pilot is still in
   the pod and seated on the board; the cockpit shows `guard`, the `Hold` the mind stands
   under and blue's hull held hostile; `STATUS` and bare `MIND` say who has the controls;
   the debrief's block opens its span; a typed `HEADING` is refused. Ten seconds: nothing
   fired. Blue's fire lands on the second pilot's hull: nothing, eight seconds more. Blue's
   fire lands on the machine: the hull fires on blue's hull, after that first arrival and on
   nobody else; blue never fired on the guard. `MIND POSTURE HOLD` from the seat while the
   mind flies it: no shot after the word, while blue's fire goes on landing on the machine;
   then blue fires on the hull and it answers. It never left its cell. `MIND BACK`: the
   span closes, and the pilot's `HEADING` is taken. The record still reads `GUARD` as
   written by hand, and every other hull's reads `RETURN`.
2. `ap10_a_pilot_who_drops_while_practising_is_flown_by_their_sealed_mind_and_fights`. `GUARD`
   and `MIND AWAY`; ten seconds, nothing fired. The session ends: the board reads `flown by
   its mind`, the practice is over, the registry holds no seat, and the hull fires on blue's
   hull, which never fired a round. The match's saved snapshot then holds the hull as any
   dropped pilot's: its mind's, not absent, on no posture. The reconnect's line is what it
   was, and `BOARD` takes the tank back.
3. `ap10_a_posture_never_left_changes_nothing_about_a_drop_and_exit_ends_a_practice`. Twice:
   `HOLD` set and the pilot at the controls when the session ends; `HOLD` set, `MIND AWAY`,
   six quiet seconds, then `EXIT` (final, and `BOARD` is refused). Both snapshots hold the
   hull its mind's, not absent, on `RETURN`; both hulls fire on a hull that never fired.
4. `ap10_return_takes_a_practising_hull_home_and_a_match_leaves_no_posture_behind`. No
   posture is set: `MIND AWAY` on the default. The cockpit shows the go-home order; the hull
   drives from fourteen cells out to its side's entry zone, stops at its edge and stays,
   having fired nothing. `MIND BACK`; `HOLD` is set; the hull's record is `RETURN` and it
   carries no attribute. The match is called off and the next one's pod reads `RETURN`.
5. `ap10_a_restart_ends_a_practice_and_the_resumed_hull_fights`. `GUARD` and `MIND AWAY`; the
   saved snapshot holds the hull absent on `GUARD`; the server half restarts; the resumed
   match's own snapshot holds it its mind's, not absent, on `RETURN`; the pod reads `GUARD`
   back, the hull fights, and the debrief's fold holds two spans: the practice's, closed
   where the restart released the seat, and the drop's, open.
6. `ap10_a_link_lost_while_practising_leaves_a_link_dead_pilots_hull`. `HOLD` and `MIND
   AWAY`; the link is lost: the snapshot holds the hull the pilot's, not absent. The pilot
   revives inside the grace, reads the revival's own line, finds the hull standing still,
   and their `HEADING` is taken with no `MIND BACK`. The hull fired nothing. `MIND AWAY`
   starts a new practice on the same word.

Each acceptance clause fails without the unit: on main `MIND POSTURE GUARD` in a pod is
refused and `MIND AWAY` is the usage line, so every story stops at its first line. The
mutation table below says what each rule's removal fails.

**Divergences from the row, and what doc 45 had wrong or left out.**

- **"Set a posture and left" needed a way to leave that is not a drop.** The simulator had
  two ways out of a cockpit, `EXIT` and a drop, and Q-44-19 rules both: the hull fights. A
  third was needed, and it had to keep the pilot watching, which neither does (`EXIT` puts
  the pilot in the team room with a cleared panel; nothing there shows a hull). Hence `MIND
  AWAY`: the row's "watch its mind fly it" is the pilot's own cockpit, kept.
- **`GUARD` answers for "another unit of the same player", and a pilot in the simulator has
  one.** See default 1.
- **The kernel kept no fire record in a match.** Not in the row; without it `GUARD` and
  `HOLD` in a match would have answered nobody, ever.
- **The row's "Mostly in" is `match_/`.** The practice is there (`match_/practice.rs`, the
  cockpit, the debrief's span). Outside it: `arena_bridge.rs` (the mirror, the refusal, the
  door), `arena_task.rs` (the words applied), `arena/input_tape.rs` and
  `minds/resimulation.rs` (the tape), `arena/absent.rs` (the fire record and the practice's
  end), `arena/intents.rs` (one call), comments in `arena/state.rs`, `arena/hash.rs` and
  `posture/order.rs`. `minds/sealed.rs`, `space/` and `port/` are not touched.

**Defaults taken.** None needs a refactor to undo unless it says so.

| | Default | Cost to reverse |
|---|---|---|
| 1 | **A side's lead owns the side's machines, for `GUARD`.** The fill is fielded by the lead (`SIM AI`, `SIM AI MIND`; the lead is already its warrant principal, `lobby::lead_character_of`), so a lead's guard answers for it; to every other pilot the fill is a stranger's, as another player's rigs are in the world. A second pilot's hull is never anybody else's. The other readings: nobody has other units in a match (then `GUARD` reads as `HOLD` and cannot be practised), or every hull of the side is one's own (then `GUARD` in the simulator answers for more than it does in the world). **For the owner.** | Low: one function, `practice::other_units` |
| 2 | **The verbs are `MIND AWAY` and `MIND BACK`**, forms of `MIND` beside `MIND POSTURE` | Low: words |
| 3 | **`RETURN` goes to the side's entry zone** and stops at its edge, which is where a machine ordered to withdraw goes in a match; there is no carrier, and nothing takes the hull out of the match | None to keep: it is the kernel's existing home for a party with a zone. A different home needs a new order |
| 4 | **The fire record starts the first time the pilot lets go in the match**, not at the match's opening: a hull that fired on the tank before that is not held to it | Low: record for every hull of a match from the opening, at the price of every match's hash moving |
| 5 | **The pilot's other units are named at `MIND AWAY`** (and at a set while away). A side whose lead changes mid-match keeps the old naming until the new lead lets go | Low |
| 6 | **Any word about a seat of the hull ends the practice**, a link lost and regained inside the grace included: the pilot reads "You are back at your station" and types `MIND AWAY` again. During the grace the hull is a link-dead pilot's, standing still. The revival's line ("it held your orders and fired nothing") is the engine's for every revival and is said here of a hull that held still | Low: let `Linkdead` pass `end_practice` |
| 7 | **The seat is refused every order while away**; the pilot types `MIND BACK` first. The world's rule is "their first command wins"; here it would have meant every cockpit verb taking the controls as a side effect | Low: `hands_on` could take the controls instead of refusing |
| 8 | **`EXIT` while away ends the practice and the hull fights.** A pilot who wants to leave the pod and keep the hull on its posture cannot | Medium: a posture that outlives the seat is the world's rule brought into a match, against Q-44-19 |
| 9 | **The board says nothing of a practice**: the slot reads `seated`, to both sides, as nothing an attacker reads changes in the world (Q-45-16) | Low |
| 10 | **A practice posture is the match's.** It lasts the match and the next match opens on `RETURN`. A restart ends every practice; a pilot who was away reads their word back afterwards, one who had only set it reads `RETURN` (the word was on the mirror, which is RAM) | Low |
| 11 | **`MIND AWAY` takes no proxy guard.** A driver lent somebody else's crew member may let go and take back like any pilot: they stay at the station, which is the tactical discretion a seat is lent (doc 10 §19 T11), where `EXIT` is its abandonment. The first build guarded it; hosted CI's census of the guard's call sites (`mush-campaign` `e7c11`, doc 10b §3.11) counted the new site, and the guard was taken out as not owed rather than recorded | Low: one call, one row of that census |
| 12 | **The kernel tells a match from a field by the arena's end condition** when it ends a practice, as it does for the fire record | Low: a flag of its own on the arena, which would be one more hashed field |
| 13 | **A practice's end stops the hull** and leaves its heading, as a release does | Low: one line in `end_practice` |
| 14 | **What a practice leaves on the hull**: whose it is and the fire record kept since (they read nowhere without the flag, and serve the pilot's next practice in the match), and what the mind remembers of having flown | Low: clear them in `end_practice` |

**Not built, and where each lives.**

- **Watching from outside the pod.** A pilot who has climbed out, and anybody in the team
  room or the lobby, sees the board and nothing of a hull. Staff have `@arena/watch`.
- **The clients.** The web panel and the TUI draw nothing for the keyframe's `posture` (AP.4
  left it so) and nothing new for `order` on a watched hull; the line client's `STATUS`,
  `MIND` and `WATCH` say it. A renderer's change; no unit names it.
- **A pilot who is not the lead has no other unit**, so their `GUARD` behaves as `HOLD`. They
  see the half of the rule that says a stranger's quarrel is not theirs.
- **A practice in a detachment's practice roster** (`detachment/live.rs`). A pilot there may
  type the three forms; what a roster's command does to a hull its mind flies on a posture
  is AP.6's rule in the kernel (a command can only tighten the order) and was not exercised.
- **AP.11**, the closing story.

**Not pinned by a story.**

- A pilot who is not the lead (their guard holding fire while the fill is hit). Story 1 pins
  the stranger's quarrel on a second pilot's hull; the kernel test pins it on a hull with no
  player named.
- A lead that changes mid-match (default 5), a range, a hold match, an unarmed hull and a
  wreck (`MIND AWAY` is refused on a wreck, as every order is).
- The leash and a guard that runs out of rounds, in a match: AP.6's kernel tests hold them,
  and nothing here changes them.
- A second seat or a proxy driver in the practising hull (in source: a control naming
  another seat ends the practice; `MIND BACK` and `MIND AWAY` take the helm's check and no
  proxy guard), and `Busy` when the arena will not take a word.
- A control the arena's inbox will not take. The world's half runs ahead of the offer, so
  after an undelivered control the mirror says the practice is over while the arena still
  holds it, and `MIND BACK` answers that the pilot holds the controls. It is the existing
  failure of an undelivered control (the world's epoch is ahead of the arena's either way);
  the next control that names the pilot ends the practice in the arena.
- An order to the practising hull from another pilot. Read in the code: a pilot's hull is
  in no squad of the match's record, so no `ORDER` reaches it; if one did, AP.6's rule keeps
  its `HOLD FIRE` (Q-44-18).
- The live task's tape replayed end to end from a story. The task's recording and the
  replay's reading are each pinned in source.

**Pinned values moved.** No state hash, ledger line, seal identity, corpus row or harness
baseline field: a match nobody lets go in sends no word, records no fire and reaches none of
the new arms. What moved, each because this unit changed the rule it asserts:

| Assertion | Was | Is | Why |
|---|---|---|---|
| `tests/domain_arena/cockpit.rs` `m5_mind_profile_switches_...`, the pod's `MIND POSTURE GUARD` | refused, `POSTURE_OOC` | not asserted there; the `ap10_` stories set it | a pod sets a practice posture |
| `arena_task` `ap3_a_posture_told_...` and `ap7_an_absent_word_...`, their last block | a practice match drops every word | it takes a word for a unit it holds and keeps none for one it does not | the same |
| `match_::cockpit::tests` `m5_mind_is_the_seventeenth_selector_...` | the help and usage lines as AP.4 left them | the two forms named; `<rule or OFF>` is `<rule|OFF>` and "reads it" is "reads" in the usage line, to stay under 120 columns | two forms added |
| `arena::weapons` `mw7_an_absent_players_mind_...`, its last block | "a practice match keeps no record" | the same assertion, said of a hull nobody named a player for | the record's new second case |
| `match_::report::tests`, the span line's cases and its width test | two ways a mind comes to fly a tank | three | `HandOver::LetGo` |

**Reviewed.** A second agent (Opus) read the diff adversarially, read-only, with the row and
the rulings and not this note's conclusions, and a second pass read the fix that followed.
Found sound, both times: one gate (`permits` untouched, no second gate); no read or write of
`posture::record` and no call of `set_present`; a match nobody practises in sends no word,
records no fire and keeps its hash and tape; the tape's boundary (settle, the words, the
pulse) is the same live and in replay; IC fields are untouched (every field arena's end is
its host's); every cockpit order goes through a door `hands_on` guards; the mind API and
MW.13's files are not touched; no stale control can be the one that should end a practice;
the kernel ends a practice in every same-pulse buffering it traced (letting go and a drop,
letting go and taking back, a drop and a reclaim, a lost link and its release). Its findings:

- **A drop while practising was not a drop as today.** The first build ended a practice from
  the world; the task's buffer collapsed the hand-back under the release ("Why it is on the
  control", above). Confirmed by reading `buffer_control`, and reproduced as mutation 21.
  Fixed: `Arena::end_practice`. The stories only asserted "fires on a hull that never
  fired", which passed either way; they now read the arena's own snapshot, and the kernel
  test compares a practised hull with a never-practised one.
- **`MIND BACK` took no helm check**, and any pilot-named control for the hull ended a
  practice and reseated whoever it named. Fixed: both forms take the helm's check, and the
  second pass's finding below settled the second seat.
- **A second seat's `Live` left a practising hull its pilot's with the flag standing** (the
  second pass; the first fix ended a practice only for the pilot who let go). Fixed: a
  control naming any seat of the hull ends the practice, since any seat's control moves who
  flies the hull.
- **The posture stayed on the dropped hull**, where a mind a player wrote reads it
  (`v.posture`). Fixed: the arena is told the word only while the pilot is away, and the
  practice's end puts it back to `RETURN`.
- **After a restart the debrief put the drop inside the practice's span** (the second pass:
  the resumed mirror did not know who was practising, so the span was never closed). Fixed:
  the mirror is rebuilt from the decoded arena, and story 5 reads the two spans.
- **A hull handed back, or held through a lost link's grace, drove on by its mind's last
  order** (the second pass). Fixed: the practice's end stops it.
- **The refusal sentence had no row in the bot's table and contained another row's needle.**
  Fixed: it is a constant of `practice.rs` with a row and words of its own.
- **The help's new section sat inside "The match, and afterwards"**, so the paragraphs after
  it read as the practice's. Fixed: a section of its own.
- **"The same memory byte for byte" overclaimed** (the second pass). The seed a release
  writes is the same; what the mind remembers of thinks it made while flying is kept, as
  across any reclaim and drop. The note and the test say so now.
- Left as recorded: the fire record and whose the hull is run on after a practice (default
  14); a lead that changes keeps the old naming (default 5); a control the inbox refuses
  after the world's half has run ("not pinned by a story"); an `ORDER` cannot reach a
  pilot's hull (read in the code).

**Verified, and how.** Linux is conclav.dev, a throwaway clone at main `e4e2f6e8e` with this
unit's diff applied; Windows is this box.

- **Mutation checks.** Twenty-two mutations, each one text replacement in a scratch copy of
  the clone, built and run against the unit's in-source tests (`ap10_`) or its six stories or
  both, then undone by writing the original text back. Every one fails the test named for
  it:

  | Mutation | What failed |
  |---|---|
  | `MIND AWAY` does not tell the arena the hull is an absent player's (the gate never reaches it) | all six stories; story 2 at "practising GUARD, the hull does not fire first" |
  | A control naming a pilot does not end the practice in the arena | the drop-parity kernel test and the task's test; stories 2, 3, 5 and 6 (in story 2 the dropped hull never fires) |
  | A practice set also writes the hull's own posture record | stories 1 and 4, at the record |
  | The pod reads the hull's own posture record | story 1, at its first read |
  | The kernel keeps no fire record in a match (main's rule) | the kernel's rule test; story 1, "the guard answers for its pilot's machine" |
  | A lead's machines are not named as the lead's other units | story 1, the same line |
  | Every hull of the side is named as the pilot's other unit | story 1, "fire on another pilot's hull is not a guard's to answer" |
  | The seat's orders are taken while its mind has the controls | story 1, at the `HEADING` that should be refused |
  | The replay does not apply a recorded absent word | the replay test |
  | The task does not record the absent word on its tape | the task's recording test |
  | A match's task drops the practice words (main's rule) | both of the task's tests; all six stories |
  | A resumed match's mirror does not know who was practising | story 5, at the debrief's two spans |
  | `MIND AWAY`'s hand-over names the pilot (it is a drop) | all six stories |
  | The posture's word does not name the pilot | story 1 (the guard never answers) and story 5 |
  | `MIND AWAY` opens no span on the pilot's debrief | stories 1 and 5, at the debrief |
  | The world's mirror does not hear a practice end | all six stories ("a drop is not a practice", "a lost link is not a practice") |
  | A practice's end leaves the posture on the hull its mind then flies | the drop-parity kernel test and the task's test; stories 2, 3, 5 and 6, at the snapshot |
  | A practice's end leaves the hull driving on its mind's last order | the drop-parity kernel test |
  | The rule that ends a practice runs in an IC field too | the field's kernel test |
  | A posture set from a seat that holds the controls is told to the arena | story 3, at the dropped hull's snapshot |
  | A practice's end does not hand the hull back to its pilot before the control applies (the first build's defect) | the drop-parity kernel test and the task's test |
  | A control naming nobody ends a practice | the drop-parity kernel test; all six stories |

- **The unit's own tests on the final tree** (Linux): the six stories, and 82 in-source
  tests (`ap10_`, `ap3_`, `ap6_`, `ap7_`, `mw7_`, `m5_`).
- **The whole of `domain_arena` and of `tacsim`'s in-source tests** (Linux, `cargo nextest
  run -P ci`): 1,132 run and 1,132 passed; 2,314 run and 2,314 passed.
- **The `#[ignore]`d tests that fly a match or replay a tape** (Linux, on the final tree):
  the 24-mind 900 s tape replay (`m6_real_24_class_iii_...`), and the three steps of CI's
  `tacsim-minds` cell (the twin corpus's pinned rows, the twin's whole-match hashes, the
  pool's thirty). All pass with no pin moved. MW.5's filed-mind field story
  (`c4_filed_mind_field`) passes; it failed once when run beside three mutation builds
  (the escort was out of the field and the site record did not name its exit inside the
  story's 420 s), and passed alone before and after on the same code.
- **The bot's job** (`cargo nextest run -P ci -p mush-bot`, the two long binaries left out
  as CI leaves them; Linux): 780 passed, on the tree before the second pass's fixes, which
  touch nothing the bot reads. Its sentence census was run again on the final tree
  (Windows): 25 passed.
- **`mush-contract-tests`** on Linux and on Windows (the help lint, the command allowlist,
  the census, the panic budget, the restriction lints, the fiat budget): 199 passed each,
  nothing re-blessed.
- `cargo fmt --check`, `cargo clippy --all-targets -D warnings` on `tacsim` and `mush-bot`,
  CI's M7 memory-size clippy pass, and `cargo doc` under `-D warnings` with and without
  `--document-private-items`: Linux and Windows.
- **CI**: the branch's dispatch run and the pull request's CI on the head commit; the runs
  are in the pull request's body, since a note cannot name the run of its own commit. The
  branch's first dispatch failed one test, `mush-campaign`'s census of the proxy guard's
  call sites (default 11): no local run had covered that crate.

**Not verified.**

- Nothing was run against a live server or over SSH, and nothing was deployed. The lines a
  player reads were read in the test rig's line mode, not in the TUI or the web client.
- The dispatch-only field stories other than MW.5's (`m8_world`, `m8_leaks`, the licensed
  field pair) were not run by hand; the branch's dispatch runs them.
- No harness baseline field moves, so the harness was not run.
- The cost of `end_practice` is one comparison a control; it was not measured.

## AP.11

Not built.
