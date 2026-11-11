# 44. Minds, wired through the game — the remaining work, surveyed

Draft 0.4, 2026-11-11, docs only. Surveyed against `origin/main` at `8c8ebc184` (the space train
through SF.R31 and SF.R26b merged) by four read-only lanes, one each for player unit minds, ground
combat, space flight, and the non-combat actors; the lead checked each lane's main claims in the
tree. Anchors are file and function names; line numbers drift. Draft 0.1, the same day, was the
survey. **Draft 0.2 records the owner's rulings of 2026-11-11 on every question:** Q-44-16 goes to
its alternative, Q-44-29 is amended, Q-44-6 and Q-44-32 are accepted with a clarification each,
and every other default is accepted as written. §4a lists what the amendments change, and §7 cuts
the units of work from the rulings. **Draft 0.3, the same day, answers the first outside review**
(a comment on PR #530, read at `a7c9d2926`): its nine findings were each checked in the tree and
all nine hold. It keeps the 34 rulings. What it changes is what each unit is done by; §4b lists
the findings, §5.11 asks the four questions they raise, and §7 and §8 are rewritten to match.
**Draft 0.4, the same day: the owner accepted Q-44-35 to Q-44-38 at their defaults**, so every
question in this document is ruled.

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
- SF.R26c (the customs ruling as one submitted action), merged as PR #528.
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
| Staff inspecting or driving an NPC | `@agent/show`, `/lore`, `/relationship`, `/poke` and `/audit` (Builder) show a profile, its knowledge and its standing; nothing shows why an NPC decided what it did, and nothing drives one | partly built |

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

The runbook's real sequence is longer than "boot, bring-up, boot" (Draft 0.3, review finding 1):
the first boot with the launch live is **Initialize**; the operators take **seven** grants on the
Registry's review scope (recall, revoke, inspect, fine, claims, seize, forfeit); four initialize
actions run (`roll-initialize-commerce` and three for the core's stock, factory and refit); the
document's mode is set to `active` and the world restarts; and `@mind/launch` then names every
operator `ready`, or says which grant one lacks. A House buys nothing unless three things hold:
its cap allows a licence (`cap_for` is `floor(cap_permille × hulls / 1000)`, so at 400 per mille
a House needs three titled hulls for its first), its root-scope treasury holds the money, and a
rated line is listed. The zero-player story `m8g_story_zero_pc_procurement_…` already stands such
a world up with five hulls.

**What is owed.** A bring-up that survives a wipe (the boxes are wiped at every step) and walks
that whole sequence, the go-live itself on dev1 and dev2, a check that the LLM routes answer, and
a decision on the two switches beside them. §8 is the list.

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

Three more facts from the first review (findings 2 and 3). **A player's sealed mind is already
what flies** a licensed rig in a field: admission seals an open claim's own script
(`field/admit.rs`, `claim_seal`), so the limits of the ruling have to bind that script, not only
the house default. The IC seat's weapons hold does not: "Doctrine and NPC pilots are untouched by
the gate" (`arena/party.rs`, `weapons_hold`). And `Relation::Hostile` is a leader's mark or fire
on the viewer's party or a friend of it; neither says this unit was attacked. **Presence is
decided by anyone aboard.** In space `all_aboard_crew_for_hull` counts every player in a walkable
room, so a connected passenger with no bridge role keeps the hull "crewed" and the backup away,
and a passenger's reconnect clears the hull's linkdead state. With two crew, the first to drop
starts no window while the second is live; when the first one's own grace ends, the server's
disconnect finds nobody else live and ends the hull's window at once, however recently the second
dropped. In the field, a restart re-seats a player who is still sitting in the unit without asking
whether they are connected (`field/boot.rs`).

**What is owed.** The field half: a mind that does not fire first and takes the unit home. What
"home" is, and how the backup mind meets orders the player left. On both tracks: the limits
enforced where the engine accepts an action, whoever's script asked; a rule for who may keep or
take back the controls; when the window starts; and what a hand-back cancels.

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

Four more facts from the first review (findings 4 to 6). **The source store is addressed by
digest and immutable**: `check_mind_source` quarantines a digest whose bytes differ, so nobody can
"edit the store's copy"; a changed doctrine is a new revision under a new digest that something
must select. **The compact seal forgets who sealed it**: `compact_mind_seal` writes only the unit,
the sortie, the mind's name and the digest, and a reload rebuilds a Class I seal with no
principal. **A replay proves nothing about a mind**: `replay_from_snapshot` re-applies the commands
on the tape and never runs the think that chose them. And **the pool is sized by its first
caller**: `MindPool::shared(workers)` builds the process's one pool on the first call, and space
asks for one worker, so a server whose first think is in space runs every mind on one thread.
**A mind can see what its ship cannot**: the view numbers contacts by their index among all the
bubble's ships and filters undetected ones afterwards, so an unseen ship that sorts earlier changes
the id and the designator letter of a contact the mind does see, and both are readable from Rhai.

**What is owed.** One applier from intents to commands that every space actor uses; doctrines that
steer by intents alone; a stand-down per carrier; a doctrine named by the pack with its digest
(D-35-17) and a way to publish and select a new revision; drones as mind-driven sorties that close
(doc 35d §9, SF.R30's row); thinks off the world thread (SF.R29's row), on a pool the server's
configuration sizes; contact ids that are stable for each observer and say nothing of what is
unseen; a seal that keeps its class, principal and claim across a restart. Not raised in any
document: whether a player may seal a mind onto a ship.

**Questions.** Q-44-24 to Q-44-28.

### G. NPC characters and the LLM roles

**What exists.** `mush-npc` in the production boot: personas, tiered knowledge, lore pools,
memory with summaries, relationships, experience, the informant path, and typed actions (say,
pose, move, look, remember, use an ability). Live LLM roles: world generation, ambience, content
assist, NPC dialogue at three tiers, NPC decision, NPC summarise, and the simulator's squad
deliberation. The storyteller role is two enum variants and two routes in the deploy template with
no caller; the code that used them went with the loadable rulesets. `Stimulus::AmbientTimer` is
modelled and never scheduled. Embedding recall and derived geography are seams with nothing behind
them. Staff already have `@agent/show` (profile and runtime status), `/lore`, `/relationship`,
`/poke` and `/audit` at Builder (Draft 0.3, review finding 8; Draft 0.1 said there was no such
verb, which was wrong). What is missing is the last decision: what woke the NPC, what it was
allowed to know, and whether the line it spoke came from the model or from its profile's fallback.
An NPC whose provider fails answers with a fallback line and no error, so a reply proves nothing
about the LLM; the provider audit (`@llm/log`, `@llm/dashboard`) does. Prompts are audited by hash
and a short summary and never stored in full, and a decision view has to keep to that.

**What is owed.** A decision on the dead role; whether NPCs act unprompted; the last decision in
the staff view.

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

- **A changed doctrine is a new revision (Q-44-25, corrected in Draft 0.3).** The ruling stands:
  a changed doctrine takes effect at the next bubble. The mechanism the default named, "an edit to
  the store's copy", cannot exist, because the store is immutable by digest. Staff publish
  revision B and select it for future bubbles; a bubble that is open, and every replay, keep A.

## 4b. The first review (Draft 0.3)

A comment on PR #530, 2026-11-11, by an outside reviewer who read the document at `a7c9d2926` and
the tree, and ran nothing. Its thesis: the survey found the right problems, and several units
could pass their stated tests while the thing they are for still does not work. Each finding was
checked in the tree. All nine hold; none was a misreading.

| # | Finding | Checked at | What changes |
|---|---|---|---|
| 1 | The dev-box list stops one boot early, checks two of seven grants, and a House with a policy still buys nothing without hulls, money and a rated listing | `deploy/README.md` steps 7 to 12; `procurement/rank.rs` `cap_for`; `m8_world.rs` | §4 A; MW.1, MW.3; §8 |
| 2 | MW.7 tested the house mind only; the ruling also binds the player's sealed mind, the IC weapons hold does not bind doctrine, and Hostile does not mean attacked | `field/admit.rs` `claim_seal`; `arena/party.rs` | §4 C; MW.7; Q-44-38 |
| 3 | "Presence decides" does not say who is entitled: a passenger, staggered losses, a field restart | `space/commands.rs` `all_aboard_crew_for_hull`; `space/tick.rs` `handle_crew_disconnected`, `handle_crew_revived_or_connected`; `field/boot.rs` | §4 C; MW.7, MW.8; Q-44-35 to Q-44-37 |
| 4 | MW.10 promised an edit to an immutable store; MW.13 allowed a draft on a governed core, and the space seal loses its class and principal on reload | `store.rs` `check_mind_source`; doc 17 (the owner's-licence text: an unfiled draft runs IC only on an ungoverned core); `space/picket.rs` `compact_mind_seal` | §4 F; §4a; MW.10, MW.13 |
| 5 | A tape replay does not rerun a think, and the first caller sizes the process's pool | `space/bubble.rs` `replay_from_snapshot`; `minds/pool.rs` `shared`; `space/picket.rs` | §4 F; MW.11, MW.20 |
| 6 | Contact ids are numbered before the detection filter, so an unseen ship changes what a mind reads | `space/picket.rs` (the view); `minds/api.rs` (`Contact.id`, `designator`) | §4 F; MW.9 |
| 7 | "Complete" shrank in the hand-off: C4 is five items, not six stories; doc 45 ends at "open"; work after M8 and the space journey over SSH had no unit and no deferral | doc 17g §3 C4; `35-cut-notes.md` SF.R31; doc 17 §22 | MW.5, MW.16; §7.1's list of what is not a unit |
| 8 | The NPC baseline was wrong: staff verbs exist; a canned line can pass for an answer | `mush-npc/src/module.rs` (`@agent`); `mush-npc/src/turn.rs`; `docs/help/llm.md` | §3; §4 G; MW.17; §8 |
| 9 | One story should tie the market, the mind, the hull and the returning player together | doc 17f §9; doc 35d Q-35d-2 | MW.21 |

The review also named three things the plan had right, which stay: a unit is proved by changing
the mind and seeing play change; the commander as a deterministic doctrine on the existing order
path; and the field's claim seal as the precedent for a mind on a ship.

## 5. Owner survey

**Ruled 2026-11-11.** The owner chose the alternative for Q-44-16 (no stopgap; wait for the
backup mind), amended Q-44-29 (nothing deleted, nothing retargeted), and accepted Q-44-6 and
Q-44-32 with a clarification each. Every other default is accepted as written. Each of those four
carries its decision first, with the proposed text kept under it for the record. Draft 0.3 added
Q-44-35 to Q-44-38 (§5.11), raised by the first review; the owner accepted all four at their
defaults the same day (Draft 0.4).

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

### 5.11 Questions the first review raised (Draft 0.3)

**Ruled 2026-11-11: all four defaults accepted as written.**

**Q-44-35 — Who keeps a hull crewed.**
- **Proposed default:** the characters who hold a bridge station on the hull, and its captain
  (the charter or title holder) when aboard. A passenger does not: their link neither holds the
  backup mind off nor takes the controls back. In the field it is the seated pilot.
- **Alternative:** anyone aboard, as today.
- **Why:** a passenger cannot fly her; counting one leaves a ship adrift with a stranger aboard,
  and lets one hand the controls back to nobody.

**Q-44-36 — When the five minutes start.**
- **Proposed default:** when the last character of Q-44-35 loses their link. An earlier crew
  member's own disconnect, when their grace ends, does not shorten it.
- **Alternative:** when the first one does.
- **Why:** the ruling's five minutes are for the crew to get back; today the first to drop can
  cut the last one's window to nothing.

**Q-44-37 — What a hand-back cancels.**
- **Proposed default:** everything the mind had not yet done. A think in flight is discarded, an
  intent not yet applied is dropped, and the player's first command is the next thing the hull
  does. What the mind already did stands: a passage it engaged and paid for continues until the
  player changes it. A restart while the mind holds the controls resumes the mind, and a seated
  player who is not connected is not re-seated.
- **Alternative:** the mind finishes its current act first.
- **Why:** "takes the controls back at once" is the ruling; an action landing after the player
  is back is the mind overruling them.

**Q-44-38 — What "defends itself" means.**
- **Proposed default:** a mind flying for an absent player may fire only at a unit that has fired
  on this unit since the field or the bubble opened. A leader's hostile mark, or fire on a friend,
  is not enough. The engine refuses any other fire intent, whoever's script asked for it, and
  the same gate refuses mining and trading.
- **Alternative:** it may fire at anything its party holds hostile.
- **Why:** "never opens fire first" has to be true of the unit, and a player's own sealed mind
  takes the house default's place under the same ruling.

## 6. Which category gets which document

| Document | Categories | Shape |
|---|---|---|
| **This document, §7** | A, D, G, I | the units below; H has none (Q-44-32 deferred) |
| **Docs 35 and 27** | C | the ruling's rules; the field cut beside the space one |
| **Doc 45** | E | a survey in this form |
| **Doc 35d §9's successor** | F | rows after SF.R31 |
| **Docs 17g and 17h** | B | the defects and C4 appended to their lists |

## 7. Units of work (Draft 0.3)

Twenty-one units, `MW.1` to `MW.21`, cut from the rulings and tightened by the first review.
"Waits on" is a unit that must be merged first. Every acceptance is through registered commands
with a non-GOD actor, and each story that waits out a real-time window does so at 24:1. **A unit
that wires a mind is done only when changing the mind changes what happens in play**; a script
that thinks, traces and is overruled is the failure this document found.

### 7.1 The units

| Unit | Category | What it is | Mostly in | Waits on | Done when |
|---|---|---|---|---|---|
| **MW.1** | A | The launch switch and the bring-up script (Q-44-4, Q-44-5), walking the runbook's whole sequence; a procurement policy for each House with what a purchase needs (Q-44-8); the raid director on (Q-44-7) | `deploy/instance/` (`render.sh`, `instance.env.example`, `mushd.toml.tmpl`, a new bring-up script), `deploy/dev1`, `deploy/dev2`, `deploy/README.md` | — | on a throwaway Redis the script takes a fresh store through boot, the identities, the Initialize boot, all seven grants, the four initialize actions and the Active restart, and `@mind/launch` names every operator `ready`; the same run interrupted once and started again ends the same way; a House with three titled hulls, a funded treasury and a rated listing buys a licence by its own agent; rendering with the switch off gives today's file byte for byte |
| **MW.2** | B | The two defects ruled to be fixed before the go-live (Q-44-14, Q-44-15): a withdrawn listing leaves the counter; a renewal that cannot land stops, and `+minds/renew` says why in the ruled words | `crates/mush-market` (`offers.rs`), the renewal service under `crates/tacsim/src/minds/` | — | a story withdraws a listing and the counter no longer offers it; a refused renewal is tried once and the holder reads the reason |
| **MW.3** | A | The go-live on dev1 and dev2: §8, run with the owner | the boxes | MW.1, MW.2 | §8's steps recorded for each box, the provider audit and the House's purchase among them |
| **MW.4** | docs | The rulings written into their own documents: D-UM-56, Q-UM-12, Q-UM-14, Q-UM-18 (doc 17); paid Follow deferred, the renewal wording (17g); the heirloom and the House term (17h); the raid director on the dev boxes (doc 36); doc 03's "No code exists yet" replaced by what is live and by the storyteller role's state | `docs/design/` | — | each question reads "ruled 2026-11-11" where it was asked |
| **MW.5** | B | Doc 17g's C4, all five of its items: 17f §9's stories 1, 2, 3, 5, 6 and 7 on the production composition, each with a restart; story 4 through the launch's governed hardware, with a refit and a restart; `cargo test -p mush-bot` green on every new sentence; the cross-host deterministic cases; Registry load beside live arenas, reported | `crates/tacsim/tests/`, `crates/tacsim/src/minds/`, `crates/mush-bot` | MW.1, MW.2 | all five items pass and "C4 as built" is written in 17g, with any item not built named there and why |
| **MW.6** | B | 17g's follow-ups: the recall notice drain that aborts on one bad holding; Registry pacing of 3 to 7 s a lease | `crates/tacsim/src/minds/registry*` | MW.2 | one bad holding no longer stops the drain; a measured lease time in the cut note |
| **MW.7** | C | The field's backup mind and the absent-player gate. The house default doctrine takes a unit whose pilot is gone to its party's home and boards; with no carrier on the site it holds at the site's edge on its side (Q-44-17). The engine refuses, for any mind flying for an absent player, the player's own sealed mind included, a fire intent at a unit that has not fired on this one, and any mining or trading (Q-44-38). A stricter order the player left stays; an order to advance or attack lapses (Q-44-18). A reconnect by the seated pilot takes the controls back at once and drops what the mind had pending (Q-44-37); a restart does not re-seat a pilot who is not connected. The simulator is unchanged (Q-44-19) | `crates/tacsim/src/arena_bridge.rs` (the release for an IC task), `crates/tacsim/src/arena/` (where an intent is accepted), `crates/tacsim/src/field/`, the doctrine library | the design section MW.8's session writes for the field | four stories, each failing today: the house mind, with a hostile in view, does not fire and boards its carrier; a filed mind written to fire first is refused the shot and still returns fire at a unit that hits it; a HOLD FIRE left by the pilot holds through an attack; a mind whose script faults falls to the house default and goes home. Then a restart while the mind holds the unit resumes the mind, and the pilot's first command on reconnect wins |
| **MW.8** | C | The space backup mind. **In flight** (the session started 2026-11-11). The first review adds to what it is done by | `crates/tacsim/src/space/` | — | as that session's brief has it, and: a connected passenger neither holds the backup off nor takes the helm (Q-44-35); with two crew dropping four minutes apart the hull's window is five minutes from the second (Q-44-36); a restart while the mind holds the helm resumes it; a reconnect with a think in flight discards it (Q-44-37) |
| **MW.9** | F | One applier from intents to bubble commands, used by every space actor, movement included; the two trace-string hooks removed; the interdictor's stand-down per carrier, on state (Q-44-24); and one view for every space mind, whose contact ids are numbered for each observer after detection | `crates/tacsim/src/space/picket.rs`, `tick.rs`, the three doctrine scripts | MW.8 | a doctrine changed to hold where it used to close makes its actor hold; an escort that heaves to does not clear the runner beside her; a Dray can heave to against the interdictor; adding a ship the observer cannot detect leaves a mind's view byte for byte the same |
| **MW.10** | F | A doctrine named by the pack's cast with its digest (Q-44-25, as corrected in §4a): staff publish a new revision with the workshop's verbs and select it for an actor; the next bubble flies it | `crates/tacsim/src/minds/content.rs`, the cast in the pack, the seal sites in `space/` | MW.9 | staff publish revision B and select it; the next picket behaves as B; a bubble that was open, and a replay of an earlier one, behave as A; publishing other bytes under A's digest is refused |
| **MW.11** | F | The think's cost measured, then thinks taken off the world thread: the snapshot of step N, intents at N+1, one think a second an actor (Q-44-28), on a pool sized by `[tacsim.minds].workers` and never by a caller | `space/tick.rs` (the step), `space/picket.rs` (the think host), `minds/pool.rs` | MW.9 | the measured cost before and after in the cut note; the same encounter run twice from before a decision, with the workers' timing varied, gives the same commands, the same mind memory and the same outcome; a server whose first think is in space has the configured number of workers |
| **MW.12** | F | Drones as mind-driven sorties: a view of the rock and the hold, intents that move and work, the yield from what the drone did, the sortie record closed (Q-44-27) | `space/mining.rs`, the drone think in `space/picket.rs`, `drone_mining.rhai` | MW.9 | a drone whose mind is changed to stop short brings back less; the excursion book holds no drone after it lands |
| **MW.13** | F | A player's own mind sealed on a ship: helm and sensors, the class from the hull's core (Q-44-26), under a licence or the owner's licence; the space seal keeps its class, principal and claim across a restart | `space/` seal sites and `compact_mind_seal`, `crates/tacsim/src/minds/sealed.rs`, the registry | MW.1, MW.8, MW.9, MW.10 | on a governed hull a filed mind the player holds is sealed, and an unfiled draft is refused; with her link down that mind flies her to her filed port in place of the house default, by a route the house default would not have taken; after a restart in mid-passage the seal reads the same class, principal and claim, and the claim closes once when she docks |
| **MW.14** | D | The NPC commander: a Rhai commander doctrine, read from the store, issuing the roster orders a player leader can, through the same delivery (authority, the comms path, the recipient's own contacts), and yielding to an eligible player who returns (Q-44-20; doc 27 FO-15) | `crates/tacsim/src/detachment/`, `field/orders.rs`, the doctrine library | — | a detachment launched with no player aboard advances, holds and withdraws on its commander's orders; changing the doctrine changes the orders; with its comms jammed an order does not arrive; the same run twice from before an order gives the same orders |
| **MW.15** | D | The field tests doc 36 still owes for SCREEN and SUPPORT | `crates/tacsim/tests/` | — | both orders pinned in a field |
| **MW.16** | E | Doc 45: the survey of asynchronous attack and defence, on one concept (Q-44-22, Q-44-23) | `docs/design/` | — | the survey open for rulings. Its plan and its cuts are doc 45's own, cut there after it is ruled, as §7 was here; this unit does not close the category |
| **MW.17** | G | The last decision in the staff view (Q-44-30): `@agent/show` gains what woke the NPC, the knowledge tiers it was allowed, and whether its line came from the model or from the profile's fallback, within the audit's rule that a prompt is kept by hash and summary only | `crates/mush-npc` | — | staff read, for a station NPC that has just spoken, the stimulus and "model" or "fallback"; with the provider cut the same view reads "fallback"; a player is refused |
| **MW.18** | G | The ambient timer scheduled: major NPCs, a player in the room, at most once in 30 real minutes each, under the budgets (Q-44-31) | `crates/mush-npc` | MW.17 | a major NPC speaks unprompted once in the window and not again; with no player in the room nothing is spent; the audit shows the call |
| **MW.19** | I | The bot's minds lane on the nightly at 24:1 (Q-44-33): write, file, list, buy, fly in the simulator and on an IC sortie | `crates/mush-bot`, the nightly workflow | MW.1, MW.5 | green three nights running |
| **MW.20** | I | Minds measured at size (Q-44-34): 100 units with every seat on a mind, in the simulator and in a field; eight bubbles in space; and the two together on one server | the fill harness, a space harness | MW.7, MW.11 | the figures in the cut note, with what misses, for each alone and for both at once |
| **MW.21** | I | The completion story, with two accounts (review finding 9). One player writes, files and lists a mind. The other buys it, fits it to a governed hull and takes her out. On link loss the mind makes a choice the house default would not, keeps to the absent-player limits, and spends only the ship's own stores. The server restarts while it holds the helm. The player reconnects with its next action pending | `crates/tacsim/tests/` | MW.5, MW.7, MW.8, MW.13 | the player's first command wins; the mind's pending action never lands; the claim closes exactly once when she docks |

**Not units, and where each lives:**

- The storyteller role (Q-44-29) and the institutions (Q-44-32): deferred by their rulings.
- Paid Follow (Q-44-11), sub-leaders and the squad layer in IC fields (Q-44-21): deferred by their
  rulings.
- Doc 17 §22's work after M8: the per-channel lease and the second seat (T-54), the first
  authorable non-combat station mind (which MW.12's player-authored mining mind is), and
  multi-second fire cycles. **Deferred**, with no unit here; each needs its own design.
- The space journey over SSH (depart, picket, submit, mine, dock) and space on the nightly:
  **owed by doc 35**, on SF.R31's list in `35-cut-notes.md`. It is not mind work and is not
  re-planned here, but MW.21 cannot run over SSH until it exists; MW.21 is a story in the test
  rig for that reason.
- Doc 45's plan and build: doc 45's.

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
| **Field** | MW.15 → MW.14 → MW.7 | `field/orders.rs`, `detachment/`; MW.7 also holds `arena_bridge.rs` and the arena's intent gate alone |
| **NPC** | MW.17 → MW.18 | `crates/mush-npc/src/module.rs` |
| **Docs** | MW.4, MW.16 | no shared file |
| **Acceptance** | MW.19, MW.20, then MW.21 | last; they read what the others built |

The space chain is one writer for its whole length. MW.8's session writes a mapping from intents
to commands for the backup mind; MW.9 makes that mapping the one applier and moves the three older
ones onto it, so MW.9 does not start until MW.8 has merged. MW.11 edits `minds/pool.rs`, which the
market chain does not touch.

### 7.4 The edges between chains

- **MW.7 waits on a design, not on code:** the field section MW.8's session writes into doc 27.
  If that section is late, MW.7's first act is to write it from Q-44-17, Q-44-18 and Q-44-35 to
  Q-44-38.
- **Q-44-35 to Q-44-38 are ruled** (their defaults), and MW.7, MW.8's added acceptance and MW.21
  are built on them.
- **The absent-player gate is one gate.** MW.7 builds it in the arena, where a field's intents are
  accepted; MW.8 and MW.9 must refuse the same things in space. Whichever lands second uses the
  first one's rule, not a copy of it.
- **MW.13 needs the launch** (MW.1): a governed hull takes only a filed mind, so there is no draft
  to test it on.
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
- **Wave 4:** MW.21.

## 8. The dev-box list (dev1 and dev2)

Nothing here is done without the owner; a redeploy wipes the store. Steps 3 to 6 are what MW.1's
script does; the list says what each box must show.

1. **Before the redeploy.** Merge what is open against main. MW.2 fixes the withdrawn-listing
   and renewal defects first (Q-44-15 as ruled).
2. **The launch's files.** MW.1: the bring-up script; the `MINDS_LAUNCH` switch in `instance.env`,
   `instance.env.example` and `render.sh`; the template's `[tacsim.minds]` block and its class
   tables kept in step.
3. **Wipe and redeploy both boxes** at main, with the launch off. Create the identities the
   runbook's steps 2 to 7 name, and write the box's `launch.json` in mode `initialize`.
4. **The Initialize boot**, with the launch live. Give each Registry operator all seven grants on
   the review scope: recall, revoke, inspect, fine, claims, seize, forfeit.
5. **The four initialize actions:** `roll-initialize-commerce`, `initialize-core-die-stock`,
   `initialize-core-factory`, `initialize-core-refit`.
6. **The Active restart.** Set the mode to `active` and restart. `@mind/launch` must end
   `Registry operators: #<op> ready` for every operator; a `lacks` names a grant to give again.
7. **Procurement** (Q-44-8): each House has a policy at the template's defaults, at least three
   titled hulls, money in its root-scope treasury, and a rated line listed for it to buy. The
   check is a licence the House's own agent bought, read from its ledger. **The raid director**
   (Q-44-7): on.
8. **The LLM** (Q-44-6): keys present. Ask a station NPC a question and start a match with an AI
   squad, then read `@llm/log`: there must be a row for an `npc_dialogue_*` route and one for
   `tacsim_deliberate`. A reply alone proves nothing, because an NPC whose provider fails speaks
   its fallback line. Play does not run without the LLM, so a route with no row holds the box
   back.
9. **A smoke by hand, on w11term, at each box:** `@mind/new`, `@mind/save`, `@mind/prove`;
   `SIM MIND` and a match; `@mind/publish`; `+minds` and a purchase; `+tv/<n> launch mind=filed:N`.
10. **Record** what each box runs in the deploy notes: the commit, the switches, the launch
    document's identities.
