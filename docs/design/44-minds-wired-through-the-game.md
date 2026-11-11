# 44. Minds, wired through the game — the remaining work, surveyed

Draft 0.1, 2026-11-11, docs only. Surveyed against `origin/main` at `8c8ebc184` (the space train
through SF.R31 and SF.R26b merged) by four read-only lanes, one each for player unit minds, ground
combat, space flight, and the non-combat actors; the lead checked each lane's main claims in the
tree. Anchors are file and function names; line numbers drift.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) used: it asks the
owner questions, each with a proposed default, an alternative and a reason. "Provisional" marks a
number to be tuned later under the 2026-10-24 rule. **(expensive to reverse)** marks an answer that
would take a significant refactor of the codebase to undo, never stored data or a live box. The
units of work are cut from the rulings, in §7.

A **mind** here is whatever holds a unit's or an actor's controls when a player does not: a Rhai
script run by `mush-mind`, the compiled Standard Doctrine, an LLM persona, or a compiled rule on a
tick. "Wired" means a player meets it in play through a registered command, on a production boot.

## 1. Why this document exists

On 2026-11-11 the owner ruled that a unit whose crew have lost their link passes to a backup mind
(doc 35d Q-35d-2, amending doc 27's Q-H5), then asked what remains to have minds wired through
every part of the game. The answer is spread across docs 17 to 17h, 23, 26, 27, 35 to 35d, 36, 38
and 41 and across four crates, and several of the notes that summarise it had gone stale. This
document is the one list, the questions that list raises, and the dev-box work that turns on what
is already built.

It is not a bug list. Defects the lanes found are named in §4 so a cut can carry them; they raise
no question.

## 2. Sources, and what is already in hand

- Docs 17, 17b, 17d, 17f, 17g, 17h (unit minds, commerce, procurement); doc 23 (IC sorties); docs
  26, 27, 27a, 32, 36, 40, 42 (the field); docs 35, 35d and `35-cut-notes.md` (space); docs 38 and
  41 (houses and retrieval); doc 03 (LLM integration, which still says "No code exists yet").
- The code under `crates/tacsim/src/{minds,arena,match_,field,detachment,space,port,procurement}`,
  `crates/mush-npc`, `crates/mush-llm`, `crates/mush-mind`, and `deploy/`.

**In hand, and not re-surveyed here:**

- The space half of the backup mind (Q-35d-2): a session started 2026-11-11 is writing its design
  section and first cut. Its field half is only a plan there; §4 C asks what that plan needs.
- SF.R26c (the customs ruling as one submitted action), PR #528.
- Everything else `35-cut-notes.md` lists as not built for space that is not a mind (the store
  budget, the damage table, real cluster sites, key tapes).

## 3. The picture today

| Where | Who holds the controls when a player does not | State |
|---|---|---|
| Simulator arena, every AI seat | compiled Standard Doctrine, one think a second (`PRODUCTION_FILL_ENGINE = Native`) | wired |
| Simulator arena, squads | an LLM persona per fill squad issuing orders with a time to live; doctrine when no order comes (`deliberation/squad.rs`) | wired where an LLM provider answers |
| Simulator arena, a dropped pilot | steers and holds fire through the grace, then the unit's sealed mind; `BOARD` takes it back | wired |
| A player's own Rhai mind, written and proved | `@mind/new|paste|line|check|save|card|set|rule|prove`, SSH `mind push|pull|check` | wired |
| A player's own mind in the simulator | `SIM MIND`; a saved draft flies; with humans on both sides only Standard Doctrine flies (Q-UM-15, OQ-5) | wired |
| Registry, filing, the market, licences, House procurement and attestation | built and story-tested; composed only when `[tacsim.minds].launch_path` is set | **off on every box** |
| A licensed mind on an IC sortie or in an IC field | `+tv/<n> launch mind=filed:N`; the claim seal at admission | built; needs the launch |
| IC field, hired NPCs and raiders | compiled doctrine under roster orders (HOLD, ADVANCE, WITHDRAW, MINE) | wired; the raid director is off by default |
| IC field, a linkdead player's unit | after 300 s, the compiled doctrine, which fires on hostile parties | **wired, against the 2026-11-11 ruling** |
| IC field, an NPC detachment commander | nothing; `npc_commander` is a roster field | not built |
| A player absent from a fight they are party to | nothing beyond orders left on a timer; the 28 IC day operation notice | not designed |
| Space, the customs corvette, the raider, the interdictor | a Rhai doctrine each, with Rust deciding heat, uncloaking, heave-to, the raider's attack and the stand-down | partly real |
| Space, collector drones | a Rhai mind that thinks and is ignored; the yield is a timer | built, unwired |
| Space, a linkdead player ship | one planned passage when the grace ends | wired as a timer; the mind is in hand (§2) |
| Space, a player's own mind on a ship | nothing; every seal is engine-owned | not built, not designed |
| Station cast, passengers, hired crew, quest givers | LLM personas through `mush-npc`, acting only through typed actions | wired |
| Clerks, executors, the yard, the recovery house, procurement | compiled arms on the `tacsim.port` tick | wired; not authorable |
| The storyteller or adjudicator LLM role | `StAdjudicate`, `StInterpret` and two routes with no caller | dead |
| Staff inspecting or driving an NPC | nothing | not built |

## 4. The categories

### A. Turning on what is built

**What exists.** The whole of doc 17f and 17g's commerce (`@mind/publish`, `+minds/*`, the typed
screen), doc 23's licensed sortie, the field's claim seal and doc 17h's procurement are composed by
`mush-server`'s boot only when `[tacsim.minds].launch_path` names a launch document.
`deploy/instance/mushd.toml.tmpl` ships the key commented, because the document names
organisations, accounts, rooms and characters that only a booted world can create, and a named
identity that does not exist refuses the boot. `deploy/README.md` ("Named launch and the player
market") gives the bring-up as nine hand steps and leaves "how the path becomes live per instance"
to the owner: `instance.env` cannot say "unset" today. On 2026-11-11 the owner confirmed dev1 runs
with the key commented. `deploy/dev1/instance.env` and `deploy/dev2/instance.env` both set
`LLM_ENABLED=true`; whether each box's keys are in place and its routes answer was not checked.
`[tacsim.field].raid_director` defaults to false, and no procurement policy ships.

**What is owed.** A bring-up that survives a wipe (the boxes are wiped at every step), the go-live
itself on dev1 and dev2, a check that the LLM routes answer, and a decision on the two switches
beside them. §8 is the list.

**Questions.** Q-44-3 to Q-44-8.

### B. The player unit-minds programme

**What exists.** Writing, proving and flying a mind in the simulator is wired. Behind the launch:
filing and the Roll, listings, personal and charter licences, forks, resale, recall, renewals,
procurement, attestation, the contraband loop and testimony (M8a to M8h, S.4b-T).

**What is owed.** Doc 17g's C4 acceptance (17f §9's stories on the production composition, each
with a restart) was never built. Open defects: a withdrawn listing stays on the counter
(`mush-market`'s `current_policy` reads no Registry status); a renewal draft that can never land is
retried about every 24 IC seconds for ever, and the holder is told nothing useful; the recall
notice drain aborts on one bad holding; Registry pacing of 3 to 7 s a lease; six copies of "a sale"
with no shared lifecycle. Unruled: D-UM-56 (private filing and the owner's licence) is still
"proposed, pending the owner"; Q-UM-12, Q-UM-14 and Q-UM-18 stand on defaults nobody ruled; doc
17g §5.3's market numbers and §5.4's paid Follow; doc 17h's heirloom exemption and House term.
After M8, doc 17 §22 lists the per-channel lease and second seat, the first non-combat station
mind, and multi-second fire cycles; none is planned as cuts.

**Questions.** Q-44-9 to Q-44-15.

### C. Link loss: the backup mind

**What exists.** The ruling (doc 35d Q-35d-2): five minutes, in the field and in space; a house
default mind every unit carries, with a mind the player sealed taking its place; it may move, pay
from the unit's own stores, stop for customs and defend itself; it never opens fire first, mines or
trades; a reconnect takes the controls back at once. In space the grace is five minutes and the
return makes for the filed port (PR #529), still as one planned passage. In the field, pilots are
bound into the same `ArenaRegistry` as the simulator, so the simulator's hook
(`arena_bridge.rs`, `disconnected`) releases a player's unit to the compiled doctrine after the
grace, and that doctrine fires on hostile parties. Doc 27a's `resolve_link_loss_posture` is not in
the tree. A party's home already exists as its carrier's ramp (`ArenaDefinition::homes`, the
raiders' WITHDRAW uses it).

**What is owed.** The field half: a mind that does not fire first and takes the unit home. What
"home" is, and how the backup mind meets orders the player left.

**Questions.** Q-44-16 to Q-44-19.

### D. NPCs that lead in the field

**What exists.** Hired NPCs and raiders fly the compiled doctrine under roster orders. The raid
director (`surface/raid.rs`) is built and ruled (Q-36-41) and is off by default. A detachment's
plan can name an `npc_commander` (doc 26 D-26-1, doc 27 FO-15), and nothing decides for one. The
simulator's LLM squad layer is not dealt in IC fields.

**What is owed.** A decision layer for an NPC commander, so a detachment with no player aboard is
led. SCREEN and SUPPORT have no field test (doc 36). Sub-leaders stay deferred (Q-36-43).

**Questions.** Q-44-20, Q-44-21.

### E. The absent player: standing orders and stances

**What exists.** A simulator `ORDER` with a time to live, HOLD FIRE and RELEASE; condition slots
(`arena/conditions.rs`); roster orders in the field; organisation-to-organisation stances (doc 40
FR.1), which are not unit postures. The 28 IC day operation notice (`OPERATION_NOTICE_MIN`) is the
stopgap the owner named on 2026-11-04, when he asked for asynchronous attack and defence to be
designed. No design exists.

**What is owed.** The design: what a player may leave standing for the units and holdings they
are not at the controls of, and what an attacker may do to them meanwhile. Category C is the
smallest case of it.

**Questions.** Q-44-22, Q-44-23.

### F. Minds in space

**What exists.** Three Rhai doctrines (`kirkwood_picket`, `belt_raider`, `interdictor_patrol`) and
a drone script, run by `run_space_rhai_think_with_seal` (`space/picket.rs`). Each actor has its own
hand-written mapping from intents to bubble commands. Rust still decides: heat blowdown;
uncloaking; heave-to and the perimeter; the raider's attack, on the trace string
`RAIDER_PRESS_ATTACK`; and the interdictor's stand-down, on the trace string
`INTERDICTOR_STAND_DOWN_CLEARED`, which clears relations for every carrier party at once. Speed
intents are ignored or fixed. SF.R29 made the source lookup read the store first, but every seal is
built with the compiled digest and boot installs the compiled sources, so an edited doctrine
changes nothing. The drone's think keeps only its memory and trace; its launch stages a
`RigRecord` that nothing closes. Every think is `MindPool::shared(1).submit().wait()` on the world
thread, each time a bubble steps; the cost has never been measured. Seals carry `principal: None`,
no mounts and the helm and sensor channels only. Free flight and corridor journeys take no intents
at all.

**What is owed.** One applier from intents to commands that every space actor uses; doctrines that
steer by intents alone; a stand-down per carrier; a doctrine named by the pack with its digest
(D-35-17); drones as mind-driven sorties that close (doc 35d §9, SF.R30's row); thinks off the
world thread (SF.R29's row). Not raised in any document: whether a player may seal a mind onto a
ship.

**Questions.** Q-44-24 to Q-44-28.

### G. NPC characters and the LLM roles

**What exists.** `mush-npc` in the production boot: personas, tiered knowledge, lore pools,
memory with summaries, relationships, experience, the informant path, and typed actions (say,
pose, move, look, remember, use an ability). Live LLM roles: world generation, ambience, content
assist, NPC dialogue at three tiers, NPC decision, NPC summarise, and the simulator's squad
deliberation. The storyteller role is two enum variants and two routes in the deploy template with
no caller; the code that used them went with the loadable rulesets. `Stimulus::AmbientTimer` is
modelled and never scheduled. Embedding recall and derived geography are seams with nothing behind
them. No staff verb inspects or drives an NPC.

**What is owed.** A decision on the dead role; whether NPCs act unprompted; whether staff get a
window into an NPC.

**Questions.** Q-44-29 to Q-44-31.

### H. The world's institutions

**What exists.** Port clerks, the order executor, the Occator yard dispatcher, the Tycho Mutual
recovery house with its retrieval contracts, and House procurement are compiled arms that re-derive
their next act from the world each fire. A designer authors their numbers and their cast in JSON,
never their behaviour.

**What is owed.** Nothing is owed. No document proposes making them minds; the question is whether
one should.

**Questions.** Q-44-32.

### I. Acceptance and scale

**What exists.** Stories per cut through production commands. The fill harness at 300 units came
back `held=no` three times, the owner accepted the timing misses as non-blocking, and the fill is
native for that reason (doc 17d). No bot plays a mind's whole life, and no bot plays space.

**What is owed.** What "fully wired" is tested by, and at what size.

**Questions.** Q-44-33, Q-44-34.

## 5. Owner survey

### 5.1 Order and shape

**Q-44-1 — What leads.**
- **Proposed default:** three things at once, because they do not touch each other: the dev-box
  bring-up (A); the field's link loss (C); and the space applier (F's first cut), which the space
  backup mind already needs. E's survey is written beside them. B's defects and C4, D, G and I
  follow.
- **Alternative:** the market first, everything else after players can buy a mind.
- **Why:** A is operations and costs no design; C has a ruling and a unit that opens fire while its
  player is away; F's applier is a dependency of work already running.

**Q-44-2 — One document or several.**
- **Proposed default:** this document carries A, D, G, H and I as cut lists. C stays in docs 35 and
  27, where the ruling is. E gets its own survey, doc 45. F is written as rows after SF.R31 in doc
  35d §9's form. B's items go back onto doc 17g's and 17h's lists.
- **Alternative:** one doc 44 plan with lanes.
- **Why:** E is a design nobody has started; F and B already have owning documents with their own
  cut notes.

### 5.2 Turning on what is built (A)

**Q-44-3 — Where the launch goes live.**
- **Proposed default:** dev1 and dev2 both.
- **Alternative:** dev1 only, with dev2 left without commerce as the plain box.
- **Why:** there are no players and both boxes are wiped at every step; a second box without the
  market only hides what the first one finds.

**Q-44-4 — How the launch survives a wipe.**
- **Proposed default:** a script, run by each box's wipe-and-redeploy step, that boots once without the launch,
  creates the identities the README's steps 2 to 7 name, writes the instance's `launch.json` from
  the ids the world printed, then renders `launch_path` live and boots again. `instance.env` gains
  a `MINDS_LAUNCH` switch that `render.sh` turns into the commented or the live line, so an empty
  value is never rendered.
- **Alternative:** a per-instance overlay file kept by hand on each box.
- **Why:** a nine-step hand procedure repeated at every wipe will be skipped, and the market will
  stay off.

**Q-44-5 — Who the launch names on a dev box.**
- **Proposed default:** the three Houses and the Republic as the README's example has them; the
  Registry's operators and the clerk are staff characters the script creates; no player accounts.
  Fees, the levy, the venue share, the royalty cap and the offer windows stay at doc 17g §5.3's
  written numbers (provisional).
- **Alternative:** a smaller launch, the House chain alone (format 1), with the player market
  later.
- **Why:** format 2 is what was built and story-tested, and the numbers can only be tuned by
  using them.

**Q-44-6 — The LLM on the dev boxes.**
- **Proposed default:** both boxes keep `LLM_ENABLED=true`. The bring-up checks that the keys are
  present and that one route of each kind answers (an NPC replies, a fill squad deliberates), and
  records the result. Budgets stay at the template's.
- **Alternative:** the LLM on one box only, to compare play with and without it.
- **Why:** the simulator's squad layer and every NPC voice fall back silently when a route does not
  answer, so "enabled" in the file proves nothing.

**Q-44-7 — The raid director.**
- **Proposed default:** on, on both dev boxes, at doc 36's defaults.
- **Alternative:** off; raiders stay staff-run with `@field/raiders`.
- **Why:** with no staff watching a dev box, off means a field never meets a raider.

**Q-44-8 — A procurement policy on the dev boxes.**
- **Proposed default:** the launch gives each House a policy at `[tacsim.procurement]`'s defaults
  (200,000 minor credits a month, a 40 percent cap), so a listed mind has a buyer.
- **Alternative:** none until a player has listed a mind.
- **Why:** procurement runs only where a policy is configured, and none ships; without one the
  loop from doc 17h never turns.

### 5.3 The unit-minds programme (B)

**Q-44-9 — D-UM-56, private filing and the owner's licence.**
- **Proposed default:** accept it as written on 2026-10-04: filing and listing are separate acts,
  and filing issues the charter owner a free, non-transferable owner's licence.
- **Alternative:** filing is always a listing.
- **Why:** it is built that way and has stood for five weeks marked "pending".

**Q-44-10 — Q-UM-12, Q-UM-14 and Q-UM-18.**
- **Proposed default:** rule all three at their written defaults: the ROM on an unarmed hull runs
  Standard Doctrine with its fire half absent; no `orgs=off` term on a listing; a pending transfer
  settles at the last admitted sortie's close, or at the earlier of the term's end and 3 IC days.
- **Alternative:** rule them one by one.
- **Why:** the code relies on each default.

**Q-44-11 — Paid Follow licences (doc 17g §5.4).**
- **Proposed default:** defer.
- **Alternative:** build it with C4.
- **Why:** nothing else waits on it.

**Q-44-12 — The heirloom and the Class III entitlement (doc 17h).**
- **Proposed default:** no exemption; a heirloom in the back room is under the entitlement like
  any other.
- **Alternative:** exempt it.
- **Why:** it is what is built, and an exemption is a way round the class rule.

**Q-44-13 — The House term (doc 17h).**
- **Proposed default:** keep 365 IC days (provisional).
- **Alternative:** 90.
- **Why:** at 24:1 a year is fifteen real days, which is already short.

**Q-44-14 — The renewal that cannot land.**
- **Proposed default:** the service stops at the first lasting refusal, and `+minds/renew` answers
  "This licence cannot be renewed: <the Registry's reason>. Nothing further will be tried."
- **Alternative:** keep retrying, at a longer interval.
- **Why:** the wording was left for the owner to approve; the retry is a defect either way.

**Q-44-15 — C4 and the go-live.**
- **Proposed default:** C4's stories are a cut of this programme and do not gate the dev boxes. The
  withdrawn-listing and renewal defects are fixed before the bring-up runs.
- **Alternative:** C4 first, then the bring-up.
- **Why:** there is nobody on a dev box to harm, and a live market finds what stories do not.

### 5.4 Link loss (C)

**Q-44-16 — A stopgap in the field.**
- **Proposed default:** now, ahead of the backup mind: a unit released because its player's link
  is down holds fire and returns fire only at a unit that has fired on it.
- **Alternative:** wait for the backup mind.
- **Why:** today it opens fire on any hostile party five minutes after its player's machine
  restarts. The change is one rule at the release.

**Q-44-17 — What "base" is for a field unit.**
- **Proposed default:** the party's home, its carrier's ramp. The unit goes there and boards. With
  no carrier on the site it goes to the site's edge on its party's side and holds.
- **Alternative:** it leaves the field for the nearest friendly port.
- **Why:** the home already exists and the raiders' WITHDRAW already steers by it; a unit cannot
  cross open ground to a port without a carrier.

**Q-44-18 — Orders the player left.**
- **Proposed default:** the stricter rule wins. A HOLD FIRE the player left stays. An order to
  advance or attack lapses when the backup mind takes over.
- **Alternative:** the player's standing orders always win.
- **Why:** the ruling's "never opens fire first" is a floor; an order cannot lower it for a player
  who is not there to answer for it.

**Q-44-19 — The simulator.**
- **Proposed default:** unchanged: a dropped pilot's unit goes to its sealed mind and fights.
- **Alternative:** the simulator follows the IC rule.
- **Why:** the simulator is out of character and a match is short; a unit that walks home loses its
  side the match.

### 5.5 NPCs that lead (D)

**Q-44-20 — Who decides for an NPC commander.**
- **Proposed default:** a Rhai commander doctrine, read from the store like the other house
  doctrines, issuing the roster orders a player leader can issue. An LLM persona may be laid over
  it later, as in the simulator.
- **Alternative:** deal the simulator's LLM squad layer in IC fields and let it lead.
- **Why:** an IC field has money and custody at stake and must run the same way on replay and when
  a provider is down; a doctrine is deterministic and an author can read it.

**Q-44-21 — Sub-leaders and the squad layer in IC fields.**
- **Proposed default:** both stay deferred (Q-36-43 as ruled).
- **Alternative:** plan them with Q-44-20.
- **Why:** no field yet has enough hulls a side to need them.

### 5.6 The absent player (E)

**Q-44-22 — One concept or two. (expensive to reverse)**
- **Proposed default:** one. Every unit always has a mind: the player's sealed one, else the house
  default. Presence decides who holds the controls. Link loss, a player who logs off with units in
  a field, and a defender who is not there when an attack comes are the same hand-over at
  different delays.
- **Alternative:** the backup mind is for link loss only, and absent defence is a separate system
  of stances.
- **Why:** two systems would each need their own rules for fire, movement, payment and return, and
  would disagree at the edges.

**Q-44-23 — Doc 45.**
- **Proposed default:** the asynchronous attack and defence survey is written next, as doc 45, on
  Q-44-22's answer: what may be left standing, for which units and holdings, what an attacker may
  do during the notice, and what the 28 IC days become.
- **Alternative:** fold it into this document.
- **Why:** it is a design with its own questions, and the owner asked for it on 2026-11-04.

### 5.7 Space (F)

**Q-44-24 — One applier, and doctrines that steer.**
- **Proposed default:** yes: one function turns a mind's intents into bubble commands for every
  actor, movement included; the trace-string hooks are removed; the stand-down is per carrier, on
  state.
- **Alternative:** leave the three hand-written mappings and fix only the stand-down.
- **Why:** the backup mind would be a fourth copy, and an edited doctrine cannot change what Rust
  decides.

**Q-44-25 — Where a doctrine comes from.**
- **Proposed default:** the pack's cast names each actor's doctrine by digest (D-35-17). An edit to
  the store's copy takes effect when the next bubble opens. Staff replace one with the workshop's
  existing verbs.
- **Alternative:** compiled sources only, as today in effect.
- **Why:** SF.R29 built the lookup and nothing uses it.

**Q-44-26 — A player's own mind on a ship.**
- **Proposed default:** yes, after the backup mind, through the same seal: helm and sensors first,
  so a mind can fly a passage, hold station and comply at customs; the fire channels wait for the
  mounts' redesign. The class comes from the hull's core as on the ground.
- **Alternative:** no player minds in space at launch; the house default only.
- **Why:** the ruling already lets a sealed mind take the backup's place, which needs the seal
  path; stopping at helm and sensors keeps the first cut small.

**Q-44-27 — Drones.**
- **Proposed default:** the house drone mind first, as SF.R30's row has it: a real view of the
  rock and the hold, intents that move and work, a yield from what the drone did, and a sortie
  record that closes. A player-authored mining mind is doc 17's first non-combat station and comes
  after.
- **Alternative:** leave drones as timers until the mining station is designed.
- **Why:** a mind that thinks and is ignored is the one thing in the tree that looks wired and is
  not.

**Q-44-28 — Where space minds think. (expensive to reverse)**
- **Proposed default:** as the arena does: a think on the snapshot of step N lands its intents at
  step N+1, on the pool's workers, at one think a second for each actor. Measure the present cost
  first and put the figure in the cut note.
- **Alternative:** keep the blocking think and cap the actors a bubble may hold.
- **Why:** a blocking think for every actor on every step is paid by every player on the server.

### 5.8 NPC characters and the LLM (G)

**Q-44-29 — The storyteller role.**
- **Proposed default:** delete the two variants and the two routes. The idea stays a note.
- **Alternative:** retarget it to this game now.
- **Why:** it is configuration for code that no longer exists.

**Q-44-30 — A staff window into an NPC.**
- **Proposed default:** a small first cut: a staff verb that shows what an NPC knows, remembers and
  was last asked. Possession and act-as wait.
- **Alternative:** nothing yet.
- **Why:** with the LLM live on the dev boxes, somebody will need to see why an NPC said what it
  said.

**Q-44-31 — NPCs that act unprompted.**
- **Proposed default:** schedule the ambient timer for major NPCs only, when a player is in the
  room, at most once every 30 real minutes each (provisional), under the existing budgets.
- **Alternative:** off.
- **Why:** it is modelled and costs a call; the cap keeps the bill bounded.

### 5.9 The world's institutions (H)

**Q-44-32 — Authorable behaviour for ports, houses and procurement.**
- **Proposed default:** no. They stay compiled rules with authored numbers.
- **Alternative:** house policy as a Rhai doctrine.
- **Why:** each has one behaviour and one instance; a second house that must behave differently
  would be the reason to revisit it.

### 5.10 Acceptance and scale (I)

**Q-44-33 — What "wired" is tested by.**
- **Proposed default:** a story for each cut, as now, plus one bot lane on the nightly at 24:1 once
  the launch is live in a rig: the bot writes a mind, files and lists it, buys a licence, flies it
  in the simulator and on an IC sortie.
- **Alternative:** stories only.
- **Why:** the market's path crosses five modules and a restart, and no single story walks it.

**Q-44-34 — The size minds are measured at.**
- **Proposed default:** 100 units, the combat-scale target, for the field and the simulator; eight
  bubbles for space. The 300-unit misses stay accepted.
- **Alternative:** hold out for 300.
- **Why:** 100 is the stated target and nothing has been measured there with every seat on a mind.

## 6. Which category gets which document

| Document | Categories | Shape |
|---|---|---|
| **This document, §7** | A, D, G, H, I | cut lists, written from the rulings |
| **Docs 35 and 27** | C | the ruling's rules; the field cut beside the space one |
| **Doc 45** | E | a survey in this form |
| **Doc 35d §9's successor** | F | rows after SF.R31 |
| **Docs 17g and 17h** | B | the defects and C4 appended to their lists |

## 7. Units of work

Written after the rulings. Each unit will name its files, what it waits on, and its acceptance,
in the form of doc 38 §9: what may start at once, what shares a file with what, and which question
gates which unit.

## 8. The dev-box list (dev1 and dev2)

Nothing here is done without the owner; a redeploy wipes the store.

1. **Before the redeploy.** Merge what is open against main. Fix the withdrawn-listing and
   renewal defects if Q-44-15 is ruled so.
2. **The launch.** Write the bring-up script (Q-44-4); add the `MINDS_LAUNCH` switch to
   `instance.env`, `instance.env.example` and `render.sh`; keep the template's `[tacsim.minds]`
   block and its class tables in step.
3. **Wipe and redeploy both boxes** at main. Boot without the launch; run the bring-up; boot with
   it. The boot must report the House chain and the player market composed.
4. **Grants.** The Registry's operators hold `roll.registry.recall` and `roll.registry.revoke`
   (README step 9).
5. **Procurement** (Q-44-8) and **the raid director** (Q-44-7), as ruled.
6. **The LLM** (Q-44-6): keys present; an NPC answers; a fill squad deliberates; the budgets read
   back.
7. **A smoke by hand, on w11term, at each box:** `@mind/new`, `@mind/save`, `@mind/prove`;
   `SIM MIND` and a match; `@mind/publish`; `+minds` and a purchase; `+tv/<n> launch mind=filed:N`;
   a station NPC asked a question.
8. **Record** what each box runs in the deploy notes: the commit, the switches, the launch
   document's identities.
