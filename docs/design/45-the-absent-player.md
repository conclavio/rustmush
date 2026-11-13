# 45. The absent player — asynchronous attack and defence, surveyed

Draft 0.3, 2026-11-11, docs only. Surveyed against the doc 44 branch (`claude/mind-work-survey-doc44`,
PR #530, Draft 0.5), whose base is `origin/main` at `0d8f30211`. Anchors are file and function
names; line numbers drift. Every code fact below was read in the tree for this survey; where a fact
is carried from doc 44 without a fresh read, the row says so. Draft 0.1, the same day, was the
survey. **Draft 0.2 records the owner's ruling of 2026-11-11: every default, Q-45-1 to Q-45-25, is
accepted as written.** **Draft 0.3, the same day, cuts the units of work from the rulings (§7):**
eleven units, `AP.1` to `AP.11`, and what each waits on in doc 44.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) and
[doc 44](44-minds-wired-through-the-game.md) used: what exists, what is owed, and owner questions,
each with a proposed default, an alternative and a reason. Its plan is cut after the rulings, as
doc 44 §7 was, and is §7 here. "Provisional" marks a number to be tuned
later under the 2026-10-24 rule. **(expensive to reverse)** marks an answer that would take a
significant refactor of the codebase to undo, never stored data or a live box.

## 1. Why this document exists

On 2026-11-04 the owner asked for asynchronous attack and defence to be designed "via standing
orders and stances", and called the 28 IC day operation notice (`OPERATION_NOTICE_MIN`,
`crates/mush-campaign/src/record.rs`) a stopgap. On 2026-11-11 he ruled doc 44's Q-44-22: there is
one concept. Every unit always has a mind, the player's sealed one or else a house default; presence
decides who holds the controls; link loss, logging off with units in a field, and a defender who is
not there when an attack comes are the same hand-over at different delays. The link-loss case is
ruled in full (doc 35d Q-35d-2; doc 44 Q-44-16 to Q-44-19 and Q-44-35 to Q-44-38). This document
asks what the rest of the concept needs: what a player may leave standing, what the unit does with
it when an attack comes, what an attacker may do meanwhile, and what the 28 IC days become.

It is doc 44's category E and unit MW.16. Doc 44's second review added one constraint this survey
takes up directly: a mind that evacuates never defends a site, and the link-loss ruling sends a unit
home.

## 2. Sources, and what is already ruled

**Sources.** Doc 44 §4 C, §4 E, §4a, §4c and its questions Q-44-16 to Q-44-23 and Q-44-35 to
Q-44-48; doc 35d Q-35d-2; doc 27 (Q-H5, Q-D4, FO-6, FO-15); doc 40 §4.2 (FR.1's stances); docs 32
and 36 (how a field opens, who is admitted, the raid director); docs 38, 41 and 42 (retrieval,
salvage, capture); doc 18 (the sailing plan, "Offline execution"); doc 20 (the fill plan). The code
under `crates/tacsim/src/{field,arena,surface,space,port,detachment,match_}`, `crates/mush-campaign`,
`crates/mush-sim/src/seat.rs` and `crates/mush-faction/src/stance.rs`.

**Ruled, and not asked again here:**

- **One concept** (Q-44-22). The backup mind is the house default mind, not a link-loss feature.
- **Link loss** (Q-35d-2, Q-H5 as amended): after five minutes the unit's mind takes it home; it
  may move, pay from the unit's own stores, stop for customs and defend itself; it never opens fire
  first, mines or trades; a reconnect takes the controls back at once.
- **Home** (Q-44-17, with Q-44-42 open): the party's carrier ramp; with no carrier on the site the
  unit holds where it stands.
- **Orders left** (Q-44-18): the stricter rule wins; a HOLD FIRE stays, an order to advance or
  attack lapses.
- **Who keeps a unit crewed, when the window starts, what a hand-back cancels, what "defends itself"
  means** (Q-44-35 to Q-44-38). Q-44-38: a mind flying for an absent player may fire only at a unit
  that has fired on this unit since the field or bubble opened; the engine refuses any other fire
  intent, whoever's script asked, and the same gate refuses mining and trading.
- **The simulator** (Q-44-19): unchanged; a dropped pilot's unit goes to its sealed mind and fights.
- **Open, not ruled, and used here only as defaults:** Q-44-39 to Q-44-48, in particular Q-44-42 (no
  edge), Q-44-43 (conscious and at liberty), Q-44-45 (a hull on an offline plan keeps her plan) and
  Q-44-46 (a clean QUIT hands over at once).
- **Standing rules.** Any wait on a player's response is at least 28 IC days. An IC field is
  multi-party (miners, an escort, a raider) and is never scoped down to two sides. Tuning numbers
  are provisional. There are no continuous games yet; boxes are wiped at every step, so no question
  here turns on stored data.

## 3. The picture today

Every way a player can be absent while something of theirs is exposed, what the game does, and
what an attacker can do to it now.

| # | The player is absent and… | What the game does today | What an attacker can do today |
|---|---|---|---|
| 1 | their link drops while they pilot a unit in a field | For `[game].linkdead_grace_s` (300 s) the seat is `Linkdead`: the unit "keeps its continuous orders, fires nothing". When the grace ends the seat is `Released` and the unit goes to its sealed mind, which fires on hostile parties (`arena_bridge.rs`, `SeatFacts::linkdead`, `disconnected`). The pilot's condition slots are cleared at the hand-over (`arena/mind.rs`, `set_controller`). Per doc 44 §4c item 11, a pilot back after the grace cannot retake an IC unit | Fight the unit as it is: for five minutes it does not shoot back; after that it fights as doctrine does, against the 2026-11-11 ruling until MW.7 lands (Q-44-16) |
| 2 | they QUIT cleanly while piloting a unit in a field | The same `disconnected` hook, at once: "A clean `QUIT` reaches this hook too and is a drop like the grace running out" | As row 1 without the five minutes |
| 3 | they log off with a unit standing on open ground on a site (no field yet) | Nothing acts for the unit: `SeatFacts` only touches a seat `ArenaRegistry` holds. A rig with no live plan and nothing in flight for `IDLE_PARK_IC_SECS` (24 IC hours) is recovered by the surface tick as its operator, or reported parked where it stands if its charge cannot pay the way home (`surface/tick.rs`, module head, "idle") | Open a field on the site at any moment with `+field/mark` or `+field/fire` (`field/commands.rs`). Admission takes **every unit on the ground** (`field/admit.rs`, `admit`): a unit with no seated pilot enters as `Controller::Mind` under the ROM seal (`custody_unit`); a unit whose seat still names a character enters as `Controller::Pilot`, because `parties::seated` reads the seat assignment and asks nothing about the connection. *Since AP.5:* admission asks the connection too (`parties::at_controls`), and such a unit is its mind's, as an absent player's, from the turn the field takes it in |
| 4 | a rig of theirs works a site unattended on a fill plan | The plan steps the existing acts as its author for up to `PLAN_BOUND_IC_SECS` (288 IC hours) (`surface/plan.rs`). When the site is promoted every start waits: "the plan pauses, and resumes when the flag clears" (`surface/tick.rs`). A rig destroyed in the field has its plan cleared (`field/provider.rs`, the destroyed rig's record) | As row 3. The raid director lands on a site when enough flights and enough ore are on the ground (`surface/raid.rs`, module head, "presence", "ore value"); it reads nothing about whether their operators are connected |
| 5 | a carrier of theirs sits on a site's pad with its crew logged off | At admission the carrier is a unit like any other (`field/carrier.rs`), with no pilot, so `Controller::Mind` (`parties::carrier_entrant` sets `pilot: None`). It is "shootable from the first pulse"; a destroyed hull "wrecks it, strands its crew and wrecks the rigs in its bay" | Shoot the carrier, and with it everything in its bay |
| 6 | their ship flies an offline sailing plan | The order arm of `tacsim.port` runs each due step as the plan's author (`port/executor.rs`; doc 18 "Offline execution"). A journey is interdicted when it crosses an interdicted leg into a domain with a live interception (`space/tick.rs`, `interdicted_domain_for_journey`); nothing in that test reads whether anyone aboard is connected. Per doc 44 §4c item 24 her plan is otherwise left alone when a link drops | Interdict her, as any carrier; inside the bubble she is crewed by whoever is aboard |
| 7 | their ship is in flight and they QUIT, or their link drops | Link loss: 300 s (`SPACE_LINKDEAD_GRACE_WALL_S`), then one planned, debited return passage to the filed port, else drift (`space/tick.rs`, `expire_linkdead_grace_for_hull`). A clean QUIT goes to `handle_crew_disconnected` with no linkdead entry: the drive is cut and no return is queued | Find a ship adrift and take her as a derelict |
| 8 | they lead a detachment and leave it | IC roster facts read each player member's connection (`detachment/sortie/field.rs`: `Connected`, `Linkdead`, `Disconnected`), and leadership falls to the next active player in slot order (doc 27 FO-6). An order with no IC deadline is a standing order (`field/orders.rs`, `arena_deadline` returns `None`) and NPC units keep flying it. A plan's `npc_commander` is a roster field nothing decides for (doc 44 §4 D) | Fight NPC units that hold their last orders; nobody re-orders them |
| 9 | they hold a block, a claim, a site with none of their units on it | Nothing fights on a site with nobody on the ground: admission refuses "nobody is on the ground". A block claim's contest window is `CONTEST_WINDOW_IC_SECS` (30 IC days) (`surface/tenure.rs`). A salvor's ask on owned ground is answered by the landholder or becomes consent after 28 IC days of silence (`surface/claim.rs`, `SALVAGE_HELP`). A chartered Dray with no departure for a week is recalled (`port/recall.rs`) | Contest the claim, or ask to salvage off the block and wait out the silence; nothing physical |
| 10 | a unit of theirs falls and lies on a site | The casualty or wreck stays on the site record. A non-friend may tow and load it (doc 36 GC.5); the custodian's salvage claim runs a `RealUtc` contest window of `CLAIM_WINDOW_IC_DAYS` (48 IC days), with notice by IC mail to the title holder (`surface/claim.rs`) | Tow it, file the claim and wait 48 IC days for title |
| 11 | their character is aboard a unit that is captured | Crew location is derived from where the character physically is (`field/crew.rs`, `CrewLocation::Captured`); nothing in `field/crew.rs` reads a connection | Take the character captive whether or not the player is there |
| 12 | a declared operation names them as an eligible defender | The operation's window opens at least 28 IC days after the declaration (`commands.rs`, `op_declare`; `record.rs`, `OperationTerms::check`). The notice is a visibility lens: the defender sees the window through `@op/show` and `@op/list` (`render.rs`); nothing is pushed to them | Nothing different: see §4 E |

## 4. The categories

### A. Presence: who is absent, and when the mind holds the controls

**What exists.** Three facts of presence reach the arena: linkdead onset, revival inside the grace,
and the release (`arena_bridge.rs`, `SeatFacts`). They are hooks on a seat the arena registry holds,
so they act only on a unit already in a field. On open ground nothing hands a unit over, and at a
field's admission the controller is decided once, from the seat alone (`field/admit.rs`,
`custody_unit`: a non-NPC seated character makes the unit `Controller::Pilot`; anything else,
including an empty seat, makes it `Controller::Mind`). So a player who logged off while still named
in a rig's seat is admitted as its pilot with nobody at the controls, holding fire as every IC
seat opens (`arena/party.rs`, `FieldHost::weapons_hold`), and no hook ever releases them, because their
disconnect fired before the field existed. `mush-sim` has an older precedent for the house default:
each station may name a declared fallback controller, which its `Disconnected` hook hands the
station to (`mush-sim/src/seat.rs`, `SeatDisconnectHook`; `@station/fallback`, staff only); the
field does not read it.

*Since AP.5 (2026-11-12)* the paragraph above is history for a seated unit: admission asks the
connection as well as the seat, and a unit whose seated player is not at its controls goes through
the same hand-over a lost link uses (`field/admit.rs`, `seat_absent`; `field/absent.rs`,
`hand_over`). *Since AP.7 (2026-11-12)* a unit nobody sits in (a rig worked from its carrier or
on a fill plan, a carrier on its pad) is admitted as a bare `Controller::Mind` as before and is an
absent player's unit from the same turn (`field/absent.rs`, `unattended`): its player is the rig's
launch operator or the carrier's commander, connected or not.

**What is owed.** One rule for "absent" that covers a unit at admission as well as a unit already
in a field, using Q-44-35's and Q-44-43's definition of who keeps a unit crewed.

**Questions.** Q-45-1, Q-45-2.

### B. What a player may leave standing

**What exists.** Nothing is a standing order for a player's own unit. What comes closest:

- **Condition slots** (`arena/conditions.rs`): native fire, veto and lock rules up to the core's
  slots, evaluated by the arena. They "belong to the hull's principal" and are cleared "on every
  controller transition", so they do not survive the hand-over (`arena/mind.rs`, `set_controller`).
- **The cockpit's master switch** `FIRE HOLD|FREE`, and in the simulator `ORDER <squad|ALL>
  <objective>` with `HOLD FIRE`, which stands at most `HUMAN_ORDER_TTL_MS` (ten minutes,
  `match_/cockpit/orders.rs`); `RELEASE` hands the squad back to its minds.
- **Roster orders** in an IC field (`field/orders.rs`): HOLD, ADVANCE, WITHDRAW, SCREEN, SUPPORT and
  ENGAGE, each with an optional HOLD FIRE and an optional IC deadline. They are executed only for
  NPC units; a player's unit is told its orders and flies them by hand.
- **Organisation stances** (`mush-faction/src/stance.rs`; doc 40 FR.1): Allied, Neutral, Hostile,
  declared by an organisation with `+faction/stance`. They are relations between organisations,
  not unit postures, read once at admission and at a join (`field/parties.rs`, `opening_stances`,
  `admit_stances`). A Hostile stance gives the declaring party a standing Hostile mark on the other,
  which the doctrine reads as a target (`arena/party.rs`, `Relations::relation`;
  `arena/stock_mind.rs`, `holds_a_hostile`).
- **Plans that run offline**: the fill plan (`surface/plan.rs`) and the sailing plan
  (`port/executor.rs`). Each is the player's own act filed before they left, run as its author.
- **The proxy seat** (`mush-campaign/src/seat.rs`): an operation may offer a station to another
  player, with a declared fallback. It is a consent grant on `mush-sim` stations and nothing in a
  TacSim field reads it.

**What is owed.** The thing the owner asked for: a standing order a player sets for a unit they
are not at the controls of, which the unit's mind obeys when an attack comes.

**Questions.** Q-45-3 to Q-45-6.

### C. Defend in place, or go home

**What exists.** Home is the party's carrier ramp (`field/home.rs`, `homes`); a WITHDRAW drives
there and the host asks the unit out once it stands beside the ramp. With no reachable ramp the
unit holds where it stands, "firing only when cornered" (`field/home.rs`, module head). The link-loss
ruling sends a unit home. A unit sent home never defends the site it left, so under the ruling as it
stands an absent player's site has no defence at all once five minutes have passed.

**What is owed.** A choice, per unit, between going home and staying, and what a unit that stays
may do.

**Questions.** Q-45-7 to Q-45-11.

### D. The limits, when the player chose to be absent

**What exists.** Q-44-38's gate is ruled and not built (MW.7). Q-44-45 lets an offline plan's own
buying and selling stand as the player's act. Neither distinguishes a player who lost their link
from one who logged off on purpose after leaving orders.

**What is owed.** Whether a player who chose absence and left orders may leave more than the link
loss floor allows, and how orders left for NPC units behave when their leader is gone.

**Questions.** Q-45-12, Q-45-13.

### E. The attacker: what may start, what must wait, and the 28 IC days

**What exists.** A field opens by either door at any moment: a detachment leader's Hostile mark on
a party seen on the presence picture, or a fire order at a presence row (`field/mod.rs`, "The
lifecycle"). The raid director opens fields by itself on its schedule (`surface/raid.rs`). Nothing in
`crates/tacsim/src/{field,surface,space,port,detachment}` reads `mush-campaign`; the crate's only
mentions in TacSim are size constants (`pack.rs`). So a declared operation, its notice and its
window gate no fight: an attacker may fight an absent player today with no operation at all. What an
operation does do is a paper record: a window at least `OPERATION_NOTICE_MIN` (28 IC days) after the
declaration and at most `OPERATION_NOTICE_MAX` (84 IC days); an objective evaluated against durable
state by the campaign tick (`mush-campaign/src/tick.rs`: the window elapsed, a muster, or a canon
record); a bond held on the attacker's own account; and stakes, among them a front or control change
(`record.rs`, `Consequence`). The presence picture an attacker sees carries a bearing, a range band
and, close in, a hull class; "no identity, no party" (`surface/presence.rs`). Nothing an attacker
reads says whether a unit is flown by a player or a mind, except the simulator's own board line.

**What is owed.** What an attacker may start against an absent player, what they must wait for,
and what the 28 IC days become once standing orders exist.

**Questions.** Q-45-14 to Q-45-16.

### F. Learning of it, and coming back

**What exists.** A field's opening and settling are told to everyone on the site and to the
operators of rigs out there (`field/mod.rs`, `tell_site`), with `sys_to_actor`: an offline operator
is not queued the line. The surface and port ticks queue trip-level lines for an offline player
(`port_tick.rs`, `notify`, `notify_budget`, `NoticeLevel::Trip`), and a salvage claim goes by IC mail
(`surface/claim.rs`). A field's close tells only the site (`field/close.rs`). An operation's notice
is pull-only (row 12). In the simulator a returning pilot is told "their own tank is flying on its
mind and `BOARD` takes it back" (`arena_bridge.rs`, `SeatFacts::connected`). In the field a returning
pilot cannot take an IC unit back (doc 44 §4c item 11; MW.7 owes the re-seat).

**What is owed.** A notice that reaches an absent defender, a way back to the controls from where
the unit is, and a way to change a standing order while away.

**Questions.** Q-45-17 to Q-45-19.

### G. Losses, capture and salvage when one side was a mind

**What exists.** Standing moves at demotion by rounds that reached another party
(`field/standing.rs`): the shooter party's principals lose standing with the victims' factions,
bounded per pair a field. Friendly fire offers the shooter party's leader a void for
`VOID_WINDOW_IC_S` (600 IC seconds, 25 real seconds at 24:1) or until their next command; "a
reconnect seeds the stamp afresh". Capture (`field/crew.rs`), salvage claims (`surface/claim.rs`,
48 IC days) and retrieval (`surface/retrieval.rs`, `RETRIEVAL_WINDOW_IC_DAYS`, 28) read no
connection.

**What is owed.** Whether anything reads differently when the side that fired, fell or was taken
was flown by a mind for a player who was not there.

**Questions.** Q-45-20 to Q-45-22.

### H. One concept across the field, space and the simulator

**What exists.** The field and the simulator share the arena kernel and its controllers. Space has
its own bubble, its own linkdead state (`SpaceBubbleBook::linkdead_hulls`) and its own minds (doc 44
§4 F); MW.8 is building its backup mind. The organisation stance table feeds the field only:
nothing in `crates/tacsim/src/space/` reads it, though doc 40 §4.6 names that as the seam's next
step.

**What is owed.** One record of what a player left standing, read by every host that can hold their
unit.

**Questions.** Q-45-23 to Q-45-25.

## 4a. What this survey looked for and did not find

- **Any link from an operation to a fight.** `OPERATION_NOTICE_MIN` gates the window of an
  operation record and nothing else; no field, bubble or raid consults an operation.
- **Any standing order a player's own unit obeys.** Condition slots are the nearest thing, and they
  are cleared at the hand-over.
- **A push of an operation's notice** to its eligible defenders.
- **The stance table in space.**
- **Whether a character's seat assignment on a rig survives their disconnect on open ground.**
  `parties::seated` reads `SpatialState`'s seat assignments; `SeatDisconnectHook` releases the
  account's focus and fires the station's fallback, and whether the assignment row goes with it was
  not settled here. Q-45-2 is asked so the answer does not depend on it. *Settled by AP.5:* the
  assignment survives a quit and a lost link on open ground (`field_ap5.rs`, story 1), which is
  why admission has to ask the connection.
- **Doc 27a's `resolve_link_loss_posture`**, which doc 44 also records as absent.

## 5. Owner survey

**Ruled 2026-11-11: all 25 defaults accepted as written** ("accepted as default across the
board"). No question was amended.

### 5.1 Presence (A)

**Q-45-1 — When a unit is "absent".**
- **Proposed default:** a unit is flown for an absent player whenever no character of Q-44-35 (as
  amended by Q-44-43, conscious and at liberty) is connected and at its controls. The hand-over has
  three delays: five minutes after the last such character's link drops (Q-44-36), at once on a
  clean QUIT (Q-44-46), and from the start for a unit that is admitted with nobody at its controls
  (an unattended rig, a carrier on its pad, a unit left on open ground).
- **Alternative:** absence counts only from a disconnect while seated; a unit with no pilot at
  admission is an NPC unit and flies the doctrine without the absent-player limits.
- **Why:** Q-44-22 names "a defender who is not there when an attack comes" as the same hand-over.
  Under the alternative an unattended rig would open fire first on behalf of a player asleep at the
  time, which is exactly what Q-44-38 forbids for the five-minute case.

**Q-45-2 — A character named in a seat who is not connected at admission.**
- **Proposed default:** admission asks the connection, not only the seat. Such a unit is admitted
  under its mind as an absent player's unit, and the character, if they reconnect, takes the
  controls back as after any hand-over (Q-44-37 and MW.7's re-seat).
- **Alternative:** unseat every character at a disconnect on open ground, so the seat is empty by
  the time a field opens.
- **Why:** today such a unit is admitted as `Controller::Pilot` with nobody at the controls, and
  nothing ever releases it. The default fixes it where the decision is made; the alternative
  changes a `mush-sim` rule every seat in the game shares.

### 5.2 What may be left standing (B)

**Q-45-3 — For what a player may leave a standing order.**
- **Proposed default:** a unit and a detachment. A unit carries a posture (Q-45-4). A detachment
  carries its standing roster orders and its commander (Q-45-12). A site, a block or a holding
  carries none of its own: it is defended by the units on it and by nothing else.
- **Alternative:** a holding also carries a standing defence, an emplacement or a picket of its
  own that fights when nobody is there.
- **Why:** nothing fights on a site with nobody on the ground today, and a holding that fights by
  itself is a new kind of object with its own custody, damage and cost. Holdings already have their
  legal defences, and those run on windows at or above 28 IC days (§3 rows 9 and 10).

**Q-45-4 — Posture, conditions or mind. (expensive to reverse)**
- **Proposed default:** a **posture** chosen from a short list and set per unit, read by whichever
  mind holds the controls, the house default or the player's sealed one, as one more field in the
  mind's view. The list to start with (provisional): `RETURN` (the link-loss ruling: go home),
  `GUARD` (hold where it stands and defend, Q-45-7 and Q-45-8) and `HOLD` (stay, hold fire, return
  fire only under Q-44-38). The default posture of every unit is `RETURN`. No new rule language.
- **Alternative A:** the standing order is a set of conditions, the condition slots of
  `arena/conditions.rs` kept across the hand-over and extended to movement.
- **Alternative B:** the standing order is the unit's mind alone; a player who wants something
  other than the house default writes and seals their own.
- **Why:** Q-44-22 makes the mind the one thing that acts, so the standing order should be what the
  mind is told, not a second actor beside it. Conditions are fire rules written for a present pilot
  and fire is what Q-44-38 restricts. A sealed mind needs a licensed core and the claim path, which
  no live server composes (doc 44 §4c item 1), so most players could leave nothing. A posture works
  for every player at once. It is expensive to reverse because every mind's view and every doctrine
  would read it.

**Q-45-5 — The player's condition slots at the hand-over.**
- **Proposed default:** cleared, as `set_controller` does today; the posture is what stands.
- **Alternative:** they survive into the mind, filtered by Q-44-38.
- **Why:** they are the principal's own fire decisions and assume the principal is watching; a
  filtered copy would be a third set of rules for the same shot.

**Q-45-6 — An organisation's Hostile stance and an absent player's unit.**
- **Proposed default:** the stance still sets the opening relation, so everyone reads the party as
  it was declared; it gives a mind flying for an absent player no leave to fire first. Q-44-38's
  gate applies unchanged.
- **Alternative:** a Hostile stance is a standing order the organisation chose, and licenses first
  fire at the declared party.
- **Why:** today a declared Hostile stance opens a standing Hostile mark, and the doctrine fires at
  any Hostile, so a rig of an absent member would fire first on a party nobody in the field marked.
  The ruling says the unit never opens fire first; a declaration made weeks ago by someone else is
  not the player answering for it.

### 5.3 Defend in place or go home (C)

**Q-45-7 — Home or hold.**
- **Proposed default:** the posture decides. `RETURN` goes home, as the link-loss ruling does.
  `GUARD` and `HOLD` stay where the unit stands and are never evacuated by the hand-over. A link loss
  with no posture set is `RETURN`.
- **Alternative:** an absent player's unit always goes home; asynchronous defence is the house's
  (Q-38-6, Q-38-18) and the claims' business, not the unit's.
- **Why:** a mind that evacuates never defends a site, so without a posture that stays there is no
  asynchronous defence at all, which is what the owner asked for.

**Q-45-8 — What a unit on GUARD may do.**
- **Proposed default:** hold its ground, turn and move within a short leash of where it was set
  (provisional: ten cells), and fire at a unit that has fired on this unit **or on another unit of
  the same player** since the field opened. Never first, never at a party only marked Hostile, and
  never mining or trading.
- **Alternative:** Q-44-38 unchanged: only at a unit that has fired on this one.
- **Why:** a guard that may answer only for itself watches the rig beside it die. The widening is
  bounded to the player's own units, so nobody else's quarrel is taken up by a mind on their behalf,
  and the floor "never opens fire first" still holds for the unit and for the player.

**Q-45-9 — When GUARD ends.**
- **Proposed default:** when the player returns, when the field demotes (the unit stays where it
  stands on open ground, under its posture for the next field), or when the unit can no longer
  defend itself (no live fire channel, or immobile), which turns it to `RETURN`.
- **Alternative:** GUARD lapses after a fixed time (provisional: 28 IC days) and turns to `RETURN`.
- **Why:** a field runs on wall time and lasts minutes; a timer measured in IC days would never
  fire inside one. The lapse that matters is the unit's own state.

**Q-45-10 — A rig on a fill plan when a field opens around it.**
- **Proposed default:** the plan pauses as today. The rig flies its posture. An unarmed rig cannot
  guard, so its useful postures are `RETURN` and `HOLD`; its default is `RETURN`, which ends the plan
  with a trip-level note once the rig is aboard. A rig that held resumes its plan when the site
  demotes.
- **Alternative:** today's rule alone: the plan pauses and the rig stands where it was, flown by
  the ROM doctrine.
- **Why:** an unattended rig on a raided site is the commonest absent case the game has (the raid
  director's trigger is flights and ore on the ground), and today's rule leaves it as a target with a
  mind that may fire first if it carries anything to fire.

**Q-45-11 — A carrier with nobody aboard.**
- **Proposed default:** a carrier on its pad is home, so `RETURN` means stay. Its mind never lifts
  by itself (a lift leaves its rigs behind, and doc 27 Q-H10 allows that only live, on the
  leave-behind confirmation). It returns fire under Q-44-38 and takes its rigs aboard as they come
  up the ramp.
- **Alternative:** its mind lifts once every unit of its party is aboard or down.
- **Why:** a wrecked carrier wrecks the rigs in its bay (`field/carrier.rs`), so a carrier that
  leaves early strands the rigs coming home to it, and one that leaves late is shot on the pad. The
  default keeps the one rule the owner made about lifting.

### 5.4 The limits when the absence was chosen (D)

**Q-45-12 — What a player who chose to be absent may authorise.**
- **Proposed default:** nothing beyond Q-44-38 and Q-45-8. Choosing to be absent and leaving a
  posture changes what the mind does, never the floor: it never fires first, mines or trades. A plan
  filed before leaving (a fill plan, a sailing plan) is the player's own act, as Q-44-45 has it.
- **Alternative:** a player may leave an engagement order (for example `ENGAGE` a named party or
  organisation), which a mind carries out while they are away.
- **Why:** under the alternative an attacker meets a script fighting for nobody, and a defender
  can go offline to attack; the asymmetry the 28 days were patching would run the other way.

**Q-45-13 — NPC units of a detachment whose leader is gone.**
- **Proposed default:** NPC crews are people in the world (doc 27 FO-15), so their units are not
  "flown for an absent player" and Q-44-38 does not bind them. A standing order with no deadline
  stays in force. An `ENGAGE` or `ADVANCE` the absent leader gave lapses when the leader's absence
  passes the grace and no other leader takes over (FO-6's fall-through, then an `npc_commander`
  under MW.14a); the units then hold and defend under their doctrine.
- **Alternative:** NPC units of an absent player's detachment fall under the same floor as the
  player's own units.
- **Why:** hired crew who stop firing back because their employer logged off would be the only
  people in the world bound by a rule about players. But an attack the player started should not
  run on without them, which is Q-44-18's reason applied to orders rather than to the mind.

### 5.5 The attacker (E)

**Q-45-14 — What an attacker may start against an absent player.**
- **Proposed default:** anything, at any time, as today. A field opens by a mark or a fire order or
  by the raid director, whoever is connected. Absence is protected by the defender's posture and by
  Q-44-38's floor, not by a wait.
- **Alternative:** a field may not open on a site where every party but the attacker's is absent;
  an attacker must declare an operation and wait its notice.
- **Why:** the field is multi-party, and a gate on who is connected is gamed by logging off when
  a raider is seen. The owner called the notice a stopgap for the lack of standing orders; once
  postures exist, the wait is what they replace.

**Q-45-15 — What the 28 IC days become.**
- **Proposed default:** `OPERATION_NOTICE_MIN` stays on operations, and an operation is narrowed
  to what a fight cannot settle: a stake that changes what a player or organisation holds (a front
  or control change, `Consequence`), which a fight's outcome feeds but does not decide by itself.
  Fights need no operation. The notice is pushed to the eligible defenders as a queued notice and
  IC mail (Q-45-17).
- **Alternative A:** retire the operation notice once postures land; operations keep a window of
  any length the attacker sets.
- **Alternative B:** wire operations into fields: a field that can change holdings opens only inside
  an active operation's window.
- **Why:** today the notice gates nothing a player meets (§4a). Keeping it for holdings honours the
  standing rule that a wait on a player's response is at least 28 IC days, without making every
  skirmish wait four weeks. Alternative B would join `mush-campaign` to the field host, an edge the
  crate was built not to have (`mush-campaign/src/lib.rs`: its edge set is `mush-core`,
  `mush-economy`, `mush-canon` and `mush-org`).

**Q-45-16 — Whether an attacker can tell a unit is flown by a mind.**
- **Proposed default:** no. Nothing an attacker reads in the field or in space says who holds a
  unit's controls; the presence picture and the contact rows stay as they are.
- **Alternative:** a close contact shows "flown by its mind", as the simulator's board does.
- **Why:** a visible marker makes an absent player's units the first chosen, and invites attacks
  timed to logouts. Behaviour will give some of it away; the game need not.

### 5.6 Learning of it and coming back (F)

**Q-45-17 — How an absent defender learns.**
- **Proposed default:** trip-level queued notices through `port_tick::notify` to every player with
  a unit or a carrier on the site: that a field opened there, and at its close what became of each of
  their units (home, holding, lost, captured, taken in tow). IC mail as well for a loss, a capture
  or a claim, so the record outlives a login. An operation's notice is pushed the same way.
- **Alternative:** IC mail only.
- **Why:** today `tell_site` says the field's opening to connected characters only, and a player
  who was away learns of a lost rig by finding it gone.

**Q-45-18 — What a returning player can do.**
- **Proposed default:** take the controls back at once, wherever the unit is (Q-44-37). If the
  field still runs they are re-seated in it (MW.7). A unit already home stays home; a unit on
  `GUARD` keeps its posture until they change it.
- **Alternative:** a returning player rejoins only at the carrier, after the unit is home.
- **Why:** the ruling's "takes the controls back at once" already covers this; the default only
  says it holds for a chosen absence too.

**Q-45-19 — Setting or changing a posture while away.**
- **Proposed default:** a posture is set the way an order is given: from the unit's seat, from
  aboard its carrier, or over the comms relay (doc 15) as an order to the unit, which jamming can
  stop. Never from OOC.
- **Alternative:** `+tv/<n> posture` works from anywhere, as `+salvage/authorize` does.
- **Why:** a posture changes what a unit does in a fight; a change that crosses the map with no
  transport would be the one order the relay never carries.

### 5.7 Losses, capture and salvage (G)

**Q-45-20 — Standing, and the friendly-fire void, for an absent leader.**
- **Proposed default:** standing moves exactly as for any fire (`field/standing.rs`). An absent
  leader's void offer waits: its window runs from their reconnect, at most 28 IC days from the
  demotion.
- **Alternative:** unchanged: 600 IC seconds or the leader's next command.
- **Why:** 600 IC seconds is 25 real seconds, so an absent leader always loses the offer before
  they could answer, and the standing rule says a wait on a player is at least 28 IC days.

**Q-45-21 — Capturing an absent player's character.**
- **Proposed default:** as for a present one (doc 42's rulings). The character is where they are,
  and capture reads no connection. They are told on return (Q-45-17), and every clock that waits on
  them is at least 28 IC days.
- **Alternative:** a character whose player is absent cannot be taken; their unit's loss sends them
  to port by extraction.
- **Why:** a character who cannot be taken while their player sleeps is an exploit (log off in a
  doomed hull); the protection an absent player is owed is time to answer, which the clocks give.

**Q-45-22 — Salvage and retrieval of an absent player's casualty.**
- **Proposed default:** unchanged. The claim's 48 IC day contest window and its IC mail already
  exceed the 28 IC day rule.
- **Alternative:** every window that names an absent title holder runs from their next login.
- **Why:** a window that never starts while a player stays away lets a player hold a wreck by
  never logging in.

### 5.8 One concept across the hosts (H)

**Q-45-23 — One posture record for the field and space. (expensive to reverse)**
- **Proposed default:** the posture lives on the unit, not in a host, and the field's and space's
  minds read the same words. In space `RETURN` is the filed-port return of Q-35d-2, `HOLD` holds
  station and complies with customs, and `GUARD` holds station and defends under Q-45-8. A ship on an
  offline plan keeps her plan (Q-44-45); if a bubble opens around her she flies her posture inside
  it and resumes her plan once she leaves it.
- **Alternative:** space keeps the backup mind only, with no postures.
- **Why:** Q-44-22 is one concept, and a ship and a rig belonging to one player should obey the same
  word. Expensive to reverse because two hosts and their minds would read the record.

**Q-45-24 — Stances in space.**
- **Proposed default:** not in this design; the stance table stays a field input, and doc 40 §4.6's
  seam carries it to space when that host is built. Q-45-6's limit then applies there too.
- **Alternative:** feed stances to bubbles now, with the postures.
- **Why:** nothing in space reads stances today, and doing both at once doubles what the first
  posture work must test.

**Q-45-25 — The simulator.**
- **Proposed default:** the postures are dealt in the simulator so a player can practise one, and
  a dropped pilot's unit still goes to its sealed mind and fights (Q-44-19).
- **Alternative:** postures are IC only.
- **Why:** a player should be able to see what `GUARD` does before trusting a rig to it, and the
  simulator is where minds are already flown for practice.

**Q-45-26 — A leader sets a roster member's posture.** (Raised by unit AP.4, which deferred it
for the owner. **Ruled 2026-11-12: yes**, "that sounds like a reasonable thing for a leader to
be able to do".)
- **Ruled:** a detachment's leader may set the posture of another player's unit on the roster,
  as a roster order: `ORDER DT:<id>:<rev>:<term> <slot> POSTURE <word>`. It rides the delivery
  every roster order rides: the leader's authority, the comms relay, a jam able to stop it, never
  from an OOC room (Q-45-19).
- **Defaults taken with the ruling** (the owner ruled the principle; unit AP.4a's cut note has
  them): the unit's player is told who set it and to what, queued if they are away; the player's
  own later set wins; a hired crew's unit takes none (Q-45-13); the plan's NPC commander does not
  issue one.
- **Why it was a question:** a posture changes what a unit does when its player is away, and
  until this ruling only that player could set it.

## 6. Out of scope, and where it lives

- **The backup minds themselves**: MW.7 (the field) and MW.8 (space) in doc 44 §7; this survey
  only says what they read.
- **The NPC commander**: MW.14a and MW.14b (doc 44 Q-44-20, Q-44-44). Q-45-13 assumes it.
- **Player minds on ships**: doc 44 Q-44-26, MW.13.
- **Crime and enforcement** for an attack on an absent player: the evidence log only (doc 36
  Q-36-30); a crime system is not designed.
- **Housing and instanced holdings**: not in the IC field; no document covers attacking them.
- **Tuning**: the leash, the posture list and every window are provisional.
- **Operations as a whole**: `mush-campaign` is surveyed here only for its notice; its formations,
  musters and bonds are doc 10b's.

## 7. Units of work (Draft 0.3)

Eleven units, `AP.1` to `AP.11`, cut from the rulings. "Waits on" is a unit that must be merged
first; `MW.n` is a unit of [doc 44](44-minds-wired-through-the-game.md) §7. Every acceptance is
through registered commands with a non-GOD actor, in a field with more than two parties where it
is a field. The rule doc 44's reviews set holds here: **a unit that wires a posture is done only
when changing the posture changes what the unit does**, and a wall-clock window is never waited
out in a story (doc 44 §7, "Time in a story").

### 7.1 The units

| Unit | Rulings | What it is | Mostly in | Waits on | Done when |
|---|---|---|---|---|---|
| **AP.1** | Q-45-17, Q-45-15, Q-45-21 | **An absent defender is told.** A field's opening on a site is queued, at trip level, to every player with a unit or a carrier there, connected or not; its close queues what became of each of their units (home, holding, lost, captured, taken in tow). A loss, a capture and a claim also go by IC mail. An operation's notice is pushed to its eligible defenders the same way, by the campaign's own tick, with no new edge from `mush-campaign` to the field | `crates/tacsim/src/field/mod.rs` (`tell_site`), `field/close.rs`, `crates/tacsim/src/port_tick.rs` (`notify`), `crates/mush-campaign/src/` (the notice) | — | a player who was offline through a field reads, at login, that it opened and what became of each unit, and has mail for a loss; an eligible defender of a declared operation has the notice queued and by mail without typing `@op/show`; a player with nothing on the site is told nothing |
| **AP.2** | Q-45-20 | **The void waits for an absent leader.** A friendly-fire void offered to a leader who is not connected runs from their reconnect, at most 28 IC days from the field's demotion; for a connected leader it is unchanged | `crates/tacsim/src/field/standing.rs` (`VOID_WINDOW_IC_S`) | — | a leader offline at the demotion is offered the void on login and may take it; 28 IC days later it is gone; a connected leader still has 600 IC seconds or their next command |
| **AP.3** | Q-45-3, Q-45-4, Q-45-23 | **The posture record. (expensive to reverse)** Every unit carries one posture, `RETURN`, `GUARD` or `HOLD`, `RETURN` by default. It lives on the unit, not in a host, survives a restart and a change of host, and is one more field in the view every mind reads, the house default's and a sealed mind's alike. Adding it to the view moves the mind API's version (`MIND_ABI`, `api=`), so filed minds are conformed again by the Registry's existing path | the unit object (as built: one attribute on it, `crates/tacsim/src/posture/`; the `mush-tactics` record is closed while a host holds the unit), `crates/tacsim/src/arena/mind.rs` (`MindView`), `crates/tacsim/src/minds/api.rs` and `mod.rs` (`API_VERSION`, and `RETAINED_ABIS`, which the Registry's existing path needed before it could conform a mind filed under API 6 to 8), the doctrine library | MW.7 | a unit's posture reads back after a restart and after a field promotes and demotes around it; a mind's view carries it in a field; a script that reads it is conformed, and one sealed under the old version is conformed again, not refused for ever |
| **AP.4** | Q-45-19 | **Setting a posture.** As an order is given: from the unit's seat, from aboard its carrier, or over the comms relay as an order to the unit, which jamming can stop. Never from an OOC room. The unit's status and its carrier's roster show it | `crates/tacsim/src/port/tv.rs` (the `+tv` verbs; as built `+tv/<n> posture`), `crates/tacsim/src/match_/cockpit/` (the seat's verb; as built `MIND POSTURE`), `crates/tacsim/src/field/orders.rs` (the relay; as built an order to the player's own rig over the roster's transport, not an entry in the roster's order book), `crates/tacsim/src/posture/order.rs`, `docs/help/` | AP.3 | each of the three ways sets it and the unit reports it; with the relay jammed the order does not arrive and the posture is unchanged; from an OOC room it is refused |
| **AP.4a** | Q-45-26, Q-45-19, Q-45-13 | **A leader sets a roster member's posture.** `ORDER DT:<id>:<rev>:<term> <slot> POSTURE <word>`, for a player's slot only. The order rides the roster's order book and transport like any other; when it is delivered the host writes the word to the unit's own record, tells the unit's player who set it and to what (queued if they are away) and answers WILCO for the unit, and an answered order is never applied again, so the player's own later set wins. **Built** ([cut note](45-cut-notes.md#ap4a)): the roster's command reaches a unit only while its player holds its seat, the five minutes after a lost link included; once the unit has passed to its mind the order is refused by the roster's own authority rule | `crates/tacsim/src/detachment/sortie/` (`field_orders.rs`, `field_commands.rs`, `npc.rs`), `crates/tacsim/src/field/orders.rs`, `crates/tacsim/src/posture/order.rs`, `docs/help/` | AP.4, AP.6, AP.8 | a leader orders a roster member's rig to `GUARD` and it is flown as AP.6 has it; the same order is stopped by jamming; a player who is not the leader is refused; the owner is told; the owner setting it back wins |
| **AP.5** | Q-45-1, Q-45-2 | **Absent at admission.** Admission asks the connection as well as the seat, in Q-44-35's sense as Q-44-43 amended it. A unit whose seated character is not connected, or is incapacitated or captive, is admitted under its mind as an absent player's unit. The three delays are one rule: five minutes after the last such character's link drops, at once on a clean QUIT, and from the start at admission. **Built** ([cut note](45-cut-notes.md#ap5)): the rule is the player's clock, so a character whose link dropped less than five minutes before the field opened keeps the seat, link-dead, for what is left of them; `custody_unit` still checks the seat out and the hand-over is made in the same turn through MW.7's door | `crates/tacsim/src/field/admit.rs` (`seat_absent`, `custody_unit`), `field/parties.rs` (`at_controls`, `seated`) | MW.7 | a player logged off in a rig's seat is not admitted as its pilot: the rig moves under its mind and does not fire first; the same player, reconnecting while the field runs, is re-seated and their first command wins; a connected pilot is admitted as before |
| **AP.6** | Q-45-7, Q-45-8, Q-45-9, Q-45-5, Q-45-6, Q-45-12, Q-45-16 | **The field's mind flies the posture.** **Built** ([cut note](45-cut-notes.md#ap6)): the posture lays the standing order (a hold that turns to face what the unit may answer, by orders the doctrine already obeys, so no doctrine source and no mind API moved); the leash bounds the widening for a mind that moves; a guard that cannot defend itself is flown as `RETURN` with the player's record kept; the seat's `FIRE HOLD` no longer binds the mind (a default for the owner). `RETURN` is MW.7's way home. `HOLD` stays, holds fire, and returns fire only at a unit that has fired on this one. `GUARD` stays within a leash of where it was set (provisional: ten cells) and may also answer fire on another unit of the same player; it ends when the player returns, when the field demotes, or when the unit can no longer defend itself, which turns it to `RETURN`. The engine's absent-player gate (MW.7's) is widened by exactly that one case and by nothing a player leaves: never first, never at a party that is only marked or declared Hostile, never mining or trading. The pilot's condition slots are still cleared at the hand-over. Nothing an attacker reads changes | `crates/tacsim/src/arena/` (the gate MW.7 built, `absent.rs`; as built also `withdraw.rs`, `mind.rs`, `intents.rs`; `stock_mind.rs` is not touched), `crates/tacsim/src/field/absent.rs`, `field/posture.rs` | MW.7, AP.3 | in a field of three parties, a rig on `GUARD` returns fire on the unit that hit its owner's other rig and holds fire on a stranger's quarrel beside it; the same rig set to `HOLD` does not answer for its neighbour; set to `RETURN` it goes home; a party declared Hostile by stance is not fired on first under any posture; a `GUARD` unit that loses its last fire channel goes home; the presence picture an attacker reads is byte for byte what it was |
| **AP.7** | Q-45-10, Q-45-11 | **Rigs on a plan, and carriers.** **Built** ([cut note](45-cut-notes.md#ap7)): every unit nobody sits in that is some player's is an absent player's unit from the turn a field takes it in, connected operator or not (a default for the owner); the plan is parked by the start of the recover that follows the rig's exit at the ramp and ended by that recover's finish, whoever asked for it; a carrier's arena is told `HOLD` and the host never asks a carrier out; an unarmed rig set to `GUARD` is flown as `HOLD`; the host asks neither half of a tow out. A rig on a fill plan flies its posture when a field opens around it: on `RETURN`, the default for an unarmed rig, it goes home and its plan ends with a trip-level note once it is aboard; on `HOLD` it stays and its plan resumes when the site demotes. A carrier on its pad is home: `RETURN` means stay; its mind never lifts, returns fire under the gate, and takes its rigs aboard as they reach the ramp | `crates/tacsim/src/surface/plan.rs`, `crates/tacsim/src/field/carrier.rs` (as built also `field/absent.rs`, `field/posture.rs`, `field/admit.rs`, `arena_task.rs`, `arena/absent.rs`, `arena/withdraw.rs`, `surface/act/`, `port/tv.rs`, `port/yard.rs`, the bot's sentence table; `surface/tick.rs` is not touched and `field/exit.rs` by one argument) | AP.6 | an unattended rig on a raided site does not fire first, goes home under `RETURN`, and its operator reads the plan's end at login; the same rig on `HOLD` is still there at demotion and its plan runs on; a carrier with nobody aboard is still on its pad with its returned rigs in its bay |
| **AP.8** | Q-45-13 | **A detachment whose leader is gone.** NPC crews are not flown for an absent player and the gate does not bind them. A standing order with no deadline stays. An `ENGAGE` or an `ADVANCE` the absent leader gave lapses when their absence passes the grace, whoever leads next (the next active player, then the plan's NPC commander); the unit holds its fire where it stands and answers once it is hit, until a leader orders it otherwise. **Built** (cut notes): the lapse is on the field's own clock (MW.7's record), is written into the roster's order history, and the roster's succession reads the same presence fact | `crates/tacsim/src/field/orders.rs`, `crates/tacsim/src/detachment/sortie/` (`field.rs`, `field_orders.rs`, `prepare.rs`, `npc.rs`, `tick.rs`); `detachment/live.rs` is the practice host and is not touched | MW.14a | NPC units under an absent leader's `ENGAGE` stop pressing after the grace and still return fire; with an NPC commander in the plan the commander's orders replace the lapsed one; a HOLD with no deadline is still held |
| **AP.9** | Q-45-23, Q-45-24 | **Space reads the same posture.** `RETURN` is the filed-port return (built, MW.8). `HOLD` holds station and complies with customs. `GUARD` holds station and defends under AP.6's rule. A ship on an offline plan keeps her plan; if a bubble opens around her she flies her posture inside it and resumes the plan when she leaves. Stances stay out of space | `crates/tacsim/src/space/` (the backup mind and its view) | MW.9, AP.3, AP.6 | a ship and a rig of one player, both set to `HOLD`, both stay and both hold fire; the ship set to `GUARD` returns fire on the hull that hit her and not on one that hit a stranger; a ship on a plan, interdicted, complies under `HOLD` and is on her plan again after the bubble |
| **AP.10** | Q-45-25 | **Postures in the simulator.** A player may set a posture on a simulator unit and watch its mind fly it. A dropped pilot's unit still goes to its sealed mind and fights (Q-44-19) | `crates/tacsim/src/match_/` (the lobby and the cockpit) | AP.3, AP.4, AP.6 | in a practice match a unit set to `GUARD` and left behaves as AP.6 has it; a pilot who drops mid-match is flown by their sealed mind as today |
| **AP.11** | Q-45-18, Q-45-22, Q-45-14 | **The closing story** (§7's list, as one). One player leaves a rig on `GUARD` and a second rig on `RETURN` on a site, a ship on `HOLD`, and logs off in the first rig's seat. A raid lands with a third party on the ground. The player reconnects while the field runs | `crates/tacsim/tests/` | AP.1, AP.5, AP.6, AP.7, AP.9 | no unit of theirs fired first; the `RETURN` rig is aboard its carrier and the `GUARD` rig answered only for its owner's units; the player was not admitted as a pilot while away, is re-seated on return and their first command wins; at login they read what happened to each unit; the ship and the rigs obeyed the same words; a casualty's salvage window is the 48 IC days it was |

**Not units, and where each lives:**

- **Fights need no operation, and an attacker may start one at any time** (Q-45-14, Q-45-15): this
  is what the tree does, and no code changes. The operation's notice is AP.1's push.
- **Capture and salvage of an absent player's units** (Q-45-21, Q-45-22): unchanged by ruling;
  AP.1 tells the player and AP.11 pins the window.
- **Stances in space** (Q-45-24): deferred to doc 40 §4.6's seam.
- **A standing defence for a holding with no unit on it** (Q-45-3's alternative): not ruled for.
- **The backup minds, the commander, a player's own mind on a ship**: doc 44's MW.7, MW.8,
  MW.14a, MW.14b and MW.13.
- **Crime and enforcement** for an attack on an absent player, and **housing**: §6.

### 7.2 What may start now

AP.1 and AP.2 wait on nothing. Everything else waits on doc 44: AP.3 and AP.5 on MW.7 (the field
backup mind, which builds the gate and the way back into a seat), AP.8 on MW.14a, AP.9 on MW.9.

### 7.3 One writer a file

| Chain | Units, in order | The shared files |
|---|---|---|
| **Notices** | AP.1, AP.2 | `field/mod.rs`, `field/close.rs`, `field/standing.rs`; no file shared with the others |
| **Posture** | AP.3 → AP.4 → AP.6 → AP.4a → AP.7 → AP.10 | the unit's record, `MindView`, the arena's gate and stock mind; the same files MW.7 holds, so this chain is the field chain's next link |
| **Admission** | AP.5 | `field/admit.rs`, `field/parties.rs`; after MW.7, beside the posture chain |
| **Detachment** | AP.8 | `field/orders.rs`, `detachment/sortie/` (the IC host; `detachment/live.rs` is the practice host, which Q-44-19 leaves alone); after MW.14a in doc 44's field chain |
| **Space** | AP.9 | `space/`; after MW.9 in doc 44's space chain |
| **Acceptance** | AP.11 | last |

AP.3 moves the mind API's version. Anything that seals or rates a mind while it is in flight
(doc 44's MW.13, MW.14b) should land on one side of it or the other, not across it.

### 7.4 Waves

- **Wave 0, now:** AP.1, AP.2.
- **Wave 1, after MW.7:** AP.3, then AP.4; AP.5 beside them.
- **Wave 2:** AP.6, then AP.7; AP.8 once MW.14a is merged.
- **Wave 3:** AP.9 once MW.9 is merged; AP.10.
- **Wave 4:** AP.11.
