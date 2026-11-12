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

Not built.

## AP.3

Not built.

## AP.4

Not built.

## AP.5

Not built.

## AP.6

Not built.

## AP.7

Not built.

## AP.8

Not built.

## AP.9

Not built.

## AP.10

Not built.

## AP.11

Not built.
