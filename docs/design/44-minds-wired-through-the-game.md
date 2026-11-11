# 44. Minds, wired through the game — the remaining work, surveyed

Draft 0.2, 2026-11-11, docs only. Surveyed against `origin/main` at `8c8ebc184` (the space train
through SF.R31 and SF.R26b merged) by four read-only lanes, one each for player unit minds, ground
combat, space flight, and the non-combat actors; the lead checked each lane's main claims in the
tree. Anchors are file and function names; line numbers drift. Draft 0.1, the same day, was the
survey. **Draft 0.2 records the owner's rulings of 2026-11-11 on every question:** Q-44-16 goes to
its alternative, Q-44-29 is amended, Q-44-6 and Q-44-32 are accepted with a clarification each,
and every other default is accepted as written. §4a lists what the amendments change, and §7 cuts
the units of work from the rulings.

This is a survey in the form [doc 38](38-after-the-ic-phase-next-systems.md) used: it asks the
owner questions, each with a proposed default, an alternative and a reason. "Provisional" marks a
number to be tuned later under the 2026-10-24 rule. **(expensive to reverse)** marks an answer that
would take a significant refactor of the codebase to undo, never stored data or a live box.

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

## 4a. Consequences of the rulings

- **The LLM is part of play (Q-44-6).** Play was never meant to run without it; `[llm].enabled =
  false` exists so tests and CI spend no tokens. So on a play box a route that does not answer is a
  fault to find at bring-up, not a mode the box may run in. §8's check is a gate for that reason.
  *(The second sentence is the lead's reading of the clarification.)*
- **No stopgap in the field (Q-44-16).** Until the field's backup mind lands, a linkdead player's
  unit in an IC field still passes to the compiled doctrine and fires on hostile parties. The
  field's backup mind (MW.7) is the fix and moves to the front of the field chain's design work.
- **The storyteller role stays where it is (Q-44-29).** Nothing is deleted and nothing is
  retargeted. `StAdjudicate`, `StInterpret` and the two routes stay in the tree and the template
  with no caller. Doc 03 says so (MW.4); there is no code unit.
- **House policy as a doctrine is deferred, not refused (Q-44-32).** The institutions stay compiled
  for now. The owner expects house policy to become a Rhai doctrine at some point, so nothing built
  in this programme should make that harder: MW.14's commander doctrine and MW.10's doctrine
  lookup are written so a house doctrine can use the same store path later.
- **One concept (Q-44-22).** Every unit always has a mind and presence decides who holds the
  controls. MW.7, MW.8 and doc 45 are written on that: the backup mind is the house default mind,
  not a link-loss feature, and doc 45 extends the same hand-over to the absent player.

## 5. Owner survey

**Ruled 2026-11-11.** The owner chose the alternative for Q-44-16 (no stopgap; wait for the
backup mind), amended Q-44-29 (nothing deleted, nothing retargeted), and accepted Q-44-6 and
Q-44-32 with a clarification each. Every other default is accepted as written. Each of those four
carries its decision first, with the proposed text kept under it for the record.

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
- **Ruled 2026-11-11: the default, with a clarification.** Play was never intended to run without
  the LLM. Turning it off exists so that tests and CI do not burn tokens.
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
- **Ruled 2026-11-11: the alternative.** No stopgap. The backup minds are being built now, and the
  field waits for its own.
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
- **Ruled 2026-11-11, amended.** Delete nothing, and make no effort to retarget it now. It stays
  disconnected for the time being.
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
- **Ruled 2026-11-11: the default, as a deferral and not a decision.** House policy as a Rhai
  doctrine is likely wanted at some point.
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
| **This document, §7** | A, D, G, I | the units below; H has none (Q-44-32 deferred) |
| **Docs 35 and 27** | C | the ruling's rules; the field cut beside the space one |
| **Doc 45** | E | a survey in this form |
| **Doc 35d §9's successor** | F | rows after SF.R31 |
| **Docs 17g and 17h** | B | the defects and C4 appended to their lists |

## 7. Units of work (Draft 0.2)

Twenty units, `MW.1` to `MW.20`, cut from the rulings. "Waits on" is a unit that must be merged
first. Every acceptance is through registered commands with a non-GOD actor, and each story that
waits out a real-time window does so at 24:1.

### 7.1 The units

| Unit | Category | What it is | Mostly in | Waits on | Done when |
|---|---|---|---|---|---|
| **MW.1** | A | The launch switch and the bring-up script (Q-44-4, Q-44-5), with a procurement policy for each House (Q-44-8) and the raid director on (Q-44-7) in the dev instances | `deploy/instance/` (`render.sh`, `instance.env.example`, `mushd.toml.tmpl`, a new bring-up script), `deploy/dev1`, `deploy/dev2`, `deploy/README.md` | — | a fresh store on a throwaway Redis goes boot, bring-up, boot and reports the House chain and the player market composed; rendering with the switch off gives today's file byte for byte |
| **MW.2** | B | The two defects ruled to be fixed before the go-live (Q-44-14, Q-44-15): a withdrawn listing leaves the counter; a renewal that cannot land stops, and `+minds/renew` says why in the ruled words | `crates/mush-market` (`offers.rs`), the renewal service under `crates/tacsim/src/minds/` | — | a story withdraws a listing and the counter no longer offers it; a refused renewal is tried once and the holder reads the reason |
| **MW.3** | A | The go-live on dev1 and dev2: §8, run with the owner | the boxes | MW.1, MW.2 | §8's eight steps recorded for each box |
| **MW.4** | docs | The rulings written into their own documents: D-UM-56, Q-UM-12, Q-UM-14, Q-UM-18 (doc 17); paid Follow deferred, the renewal wording (17g); the heirloom and the House term (17h); the raid director on the dev boxes (doc 36); doc 03's "No code exists yet" replaced by what is live and by the storyteller role's state | `docs/design/` | — | each question reads "ruled 2026-11-11" where it was asked |
| **MW.5** | B | Doc 17g's C4: 17f §9's stories 1, 2, 3, 5, 6 and 7 on the production composition, each with a restart | `crates/tacsim/tests/`, `crates/tacsim/src/minds/` | MW.1, MW.2 | the six stories pass and "C4 as built" is written in 17g |
| **MW.6** | B | 17g's follow-ups: the recall notice drain that aborts on one bad holding; Registry pacing of 3 to 7 s a lease | `crates/tacsim/src/minds/registry*` | MW.2 | one bad holding no longer stops the drain; a measured lease time in the cut note |
| **MW.7** | C | The field's backup mind: the house default doctrine for a unit whose player is gone. It goes to its party's home and boards; with no carrier on the site it holds at the site's edge on its side (Q-44-17). It never opens fire first. A stricter order the player left stays; an order to advance or attack lapses (Q-44-18). A reconnect takes the controls back at once. The simulator is unchanged (Q-44-19) | `crates/tacsim/src/arena_bridge.rs` (the release for an IC task), `crates/tacsim/src/field/`, the doctrine library | the design section MW.8's session writes for the field | a player's field unit, five minutes after its link drops with a hostile in view, has not fired, is aboard its carrier, and answers its player's first command on reconnect |
| **MW.8** | C | The space backup mind. **In flight** (the session started 2026-11-11) | `crates/tacsim/src/space/` | — | as that session's brief has it |
| **MW.9** | F | One applier from intents to bubble commands, used by every space actor, movement included; the two trace-string hooks removed; the interdictor's stand-down per carrier, on state (Q-44-24) | `crates/tacsim/src/space/picket.rs`, `tick.rs`, the three doctrine scripts | MW.8 | a doctrine edited to hold where it used to close makes its actor hold; an escort that heaves to does not clear the runner beside her; a Dray can heave to against the interdictor |
| **MW.10** | F | A doctrine named by the pack's cast with its digest; an edit to the store's copy takes effect at the next bubble; staff replace one with the workshop's verbs (Q-44-25) | `crates/tacsim/src/minds/content.rs`, the cast in the pack, the seal sites in `space/` | MW.9 | a store-edited doctrine changes what the next picket does, in a story |
| **MW.11** | F | The think's cost measured, then thinks taken off the world thread: the snapshot of step N, intents at N+1, one think a second an actor (Q-44-28) | `space/tick.rs` (the step), `space/picket.rs` (the think host) | MW.9 | the measured cost before and after in the cut note; a replay of a recorded bubble is identical |
| **MW.12** | F | Drones as mind-driven sorties: a view of the rock and the hold, intents that move and work, the yield from what the drone did, the sortie record closed (Q-44-27) | `space/mining.rs`, the drone think in `space/picket.rs`, `drone_mining.rhai` | MW.9 | a drone whose mind is edited to stop short brings back less; the excursion book holds no drone after it lands |
| **MW.13** | F | A player's own mind sealed on a ship: helm and sensors, the class from the hull's core (Q-44-26) | `space/` seal sites, `crates/tacsim/src/minds/sealed.rs`, the registry | MW.8, MW.9, MW.10 | a player seals a saved mind on her ship; with her link down it flies her to her filed port in place of the house default |
| **MW.14** | D | The NPC commander: a Rhai commander doctrine, read from the store, issuing the roster orders a player leader can (Q-44-20) | `crates/tacsim/src/detachment/`, `field/orders.rs`, the doctrine library | — | a detachment launched with no player aboard advances, holds and withdraws on its commander's orders, the same way on replay |
| **MW.15** | D | The field tests doc 36 still owes for SCREEN and SUPPORT | `crates/tacsim/tests/` | — | both orders pinned in a field |
| **MW.16** | E | Doc 45: the survey of asynchronous attack and defence, on one concept (Q-44-22, Q-44-23) | `docs/design/` | — | the survey open for rulings |
| **MW.17** | G | A staff verb that shows what an NPC knows, remembers and was last asked (Q-44-30) | `crates/mush-npc` | — | staff read it for a station NPC; a player is refused |
| **MW.18** | G | The ambient timer scheduled: major NPCs, a player in the room, at most once in 30 real minutes each, under the budgets (Q-44-31) | `crates/mush-npc` | MW.17 | a major NPC speaks unprompted once in the window and not again; with no player in the room nothing is spent |
| **MW.19** | I | The bot's minds lane on the nightly at 24:1 (Q-44-33): write, file, list, buy, fly in the simulator and on an IC sortie | `crates/mush-bot`, the nightly workflow | MW.1, MW.5 | green three nights running |
| **MW.20** | I | Minds measured at size (Q-44-34): 100 units with every seat on a mind, in the simulator and in a field; eight bubbles in space | the fill harness, a space harness | MW.7, MW.11 | the figures in the cut note, with what misses |

Not units: the storyteller role (Q-44-29), the institutions (Q-44-32), paid Follow (Q-44-11),
sub-leaders and the squad layer in IC fields (Q-44-21). Each is deferred by its ruling.

### 7.2 What may start now

MW.1, MW.2, MW.4, MW.14, MW.15, MW.16 and MW.17 wait on nothing, and MW.8 is running. That is
seven units across five chains (§7.3): the deploy, market, NPC and docs chains each start a
session today, and the field chain takes MW.15 and then MW.14 in one.

### 7.3 One writer a file

| Chain | Units, in order | The shared files |
|---|---|---|
| **Deploy** | MW.1 → MW.3 | `deploy/` |
| **Market** | MW.2 → MW.6 → MW.5 | `crates/tacsim/src/minds/` and `crates/mush-market` |
| **Space** | MW.8 → MW.9 → MW.10 → MW.11 → MW.12 → MW.13 | `space/picket.rs` and `space/tick.rs`; every unit of the chain edits both |
| **Field** | MW.15 → MW.14 → MW.7 | `field/orders.rs`, `detachment/`; MW.7 also holds `arena_bridge.rs` alone |
| **NPC** | MW.17 → MW.18 | `crates/mush-npc/src/module.rs` |
| **Docs** | MW.4, MW.16 | no shared file |
| **Acceptance** | MW.19, MW.20 | last; they read what the others built |

The space chain is one writer for its whole length. MW.8's session writes a mapping from intents
to commands for the backup mind; MW.9 makes that mapping the one applier and moves the three older
ones onto it, so MW.9 does not start until MW.8 has merged.

### 7.4 The edges between chains

- **MW.7 waits on a design, not on code:** the field section MW.8's session writes into doc 27.
  If that section is late, MW.7's first act is to write it from Q-44-17 and Q-44-18.
- **MW.13 needs the launch** for a filed licence to be sealed; with MW.1 unmerged it can still be
  built and tested on a saved draft.
- **MW.19 needs MW.1's bring-up** to stand a launch up in the bot's rig, and MW.5's stories to
  have found what a story can find first.
- **MW.3 is the owner's.** It also wants what is open against main merged first.
- **Doc 45 (MW.16) may change MW.7 and MW.8** where its rulings widen the hand-over to an absent
  player. Both are built for link loss alone; neither should assume link loss is the only door.

### 7.5 Waves

- **Wave 0, now:** MW.1, MW.2, MW.4, MW.14, MW.15, MW.16, MW.17; MW.8 already running.
- **Wave 1:** MW.3 (after MW.1 and MW.2), MW.6, MW.7 (after the field section), MW.9 (after
  MW.8), MW.18.
- **Wave 2:** MW.5, MW.10, then MW.11 and MW.12.
- **Wave 3:** MW.13, MW.19, MW.20.

## 8. The dev-box list (dev1 and dev2)

Nothing here is done without the owner; a redeploy wipes the store.

1. **Before the redeploy.** Merge what is open against main. MW.2 fixes the withdrawn-listing
   and renewal defects first (Q-44-15 as ruled).
2. **The launch.** MW.1: the bring-up script; the `MINDS_LAUNCH` switch in `instance.env`,
   `instance.env.example` and `render.sh`; the template's `[tacsim.minds]` block and its class
   tables kept in step.
3. **Wipe and redeploy both boxes** at main. Boot without the launch; run the bring-up; boot with
   it. The boot must report the House chain and the player market composed.
4. **Grants.** The Registry's operators hold `roll.registry.recall` and `roll.registry.revoke`
   (README step 9).
5. **Procurement** (Q-44-8): each House has a policy at the template's defaults. **The raid
   director** (Q-44-7): on.
6. **The LLM** (Q-44-6): keys present; an NPC answers; a fill squad deliberates; the budgets read
   back. Play does not run without it, so a route that does not answer holds the box back.
7. **A smoke by hand, on w11term, at each box:** `@mind/new`, `@mind/save`, `@mind/prove`;
   `SIM MIND` and a match; `@mind/publish`; `+minds` and a purchase; `+tv/<n> launch mind=filed:N`;
   a station NPC asked a question.
8. **Record** what each box runs in the deploy notes: the commit, the switches, the launch
   document's identities.
