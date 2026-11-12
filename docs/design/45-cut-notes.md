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

Not built.

## AP.4

Not built.

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
  (`ORDER_LAPSED_STATUS`) and in the recipient's own (`ORDER_LAPSED_OWN`).
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
