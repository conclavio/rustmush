# 44. Minds, wired through the game — the remaining work, surveyed

Draft 0.6, 2026-11-11, docs only. Surveyed against `origin/main` at `8c8ebc184` (the space train
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
**Draft 0.4, the same day: the owner accepted Q-44-35 to Q-44-38 at their defaults.** **Draft
0.5, the same day, answers the second outside review** (a second comment on PR #530, read at
`95764abb5`). Five read-only lanes checked its claims one by one: most hold, six are refuted, and
several of the symbols and paths it cites do not exist. §4c is the list. It adds four units
(MW.0, MW.22, MW.23, MW.24), splits MW.14, moves MW.6 ahead of the go-live, and raises ten
questions, Q-44-39 to Q-44-48 (§5.12), which are open; each default holds until ruled. **Draft
0.6, the same day, records what the first wave 0 work found:** MW.0 does not stand alone (its
default would take from House and leased hulls what they do today), so Q-44-39's default is
changed and MW.0 is merged with MW.22; MW.4, MW.6, MW.16 and MW.17 are built (PRs #532, #534,
#533, #535); MW.17 found that the NPC decision call cannot carry its NPC without a change to the
decision lease (now MW.17b), and MW.24 was sized: it is a presence-free connection through the
engine, the session manager and the gateway protocol, not an edit to the SSH server alone.

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

Three more facts from the first review (findings 2 and 3). **A player's sealed mind is what
flies** a licensed rig in a field when its claim is open: admission seals an open claim's own
script (`field/admit.rs`, `claim_seal`), so the limits of the ruling have to bind that script,
not only the house default. *(Corrected in Draft 0.5: that is true in the test rigs only. The
production boot never composes the claim path, so on a live server no claim opens and the rig
flies the ROM; §4c item 1, MW.0.)* The IC seat's weapons hold does not: "Doctrine and NPC pilots are untouched by
the gate" (`arena/party.rs`, `weapons_hold`). And `Relation::Hostile` is a leader's mark or fire
on the viewer's party or a friend of it; neither says this unit was attacked. **Presence is
decided by anyone aboard.** In space `all_aboard_crew_for_hull` counts every player in a walkable
room, so a connected passenger with no bridge role keeps the hull "crewed" and the backup away,
and a passenger's reconnect clears the hull's linkdead state. With two crew, the first to drop
starts no window while the second is live; when the first one's own grace ends, the server's
disconnect finds nobody else live and ends the hull's window at once, however recently the second
dropped. In the field, a restart re-seats a player who is still sitting in the unit without asking
whether they are connected (`field/boot.rs`).

And from the second review (§4c items 5 and 9 to 11). **A pilot who reconnects after the grace
cannot take an IC unit back at all**: the release unbinds them (`arena_bridge.rs`,
`unbind_pilot`), the simulator's `BOARD` refuses outside a team room, and the only IC seat
bindings are a launch join and a boot. **Nothing sends a player's own mind home**: the withdraw
objective is seeded for doctrine only, and Rhai may steer to the ramp but may not board (D-UM-11
forbids `order`, `release`, `place`, `board`, `exit_hull` and `eject`). The stock mind's withdraw
rallies back to patrol unless it was ordered home, and its cornered stand can fire first. A rig
has no exit ask (`field/home.rs`, `Then::Rig => None`), a recover that is refused once leaves the
hull beside the ramp for its player, and with no ramp a unit "holds where it stands" (Q-CF-18: no
edge exit; the ground has no edge geometry). **An incapacitated pilot keeps the seat and the
controls** (`field/crew.rs`); a captive holds no station. **A clean QUIT** releases a field unit
at once, and in space starts no window at all: the drive is cut and nothing ever flies her home.
**A hull flying an offline plan** (`+ship/plan`, `port/executor.rs`) is left alone today when her
link drops, and the backup mind would be the first thing to fight it. **SSH `mind push`** attaches
the character and drops it as a lost link, which restarts a running five-minute window and leaves
an offline player link-dead in the room for five minutes.

**What is owed.** The field half: a mind that does not fire first and takes the unit home. What
"home" is, and how the backup mind meets orders the player left. On both tracks: the limits
enforced where the engine accepts an action, whoever's script asked; a rule for who may keep or
take back the controls; when the window starts; what a hand-back cancels; a way back into an IC
seat; and rules for a quit, an offline plan and a pilot who is unconscious.

**Questions.** Q-44-16 to Q-44-19.

### D. NPCs that lead in the field

**What exists.** Hired NPCs and raiders fly the compiled doctrine under roster orders. The raid
director (`surface/raid.rs`) is built and ruled (Q-36-41) and is off by default. A detachment's
plan can name an `npc_commander` (doc 26 D-26-1, doc 27 FO-15), and nothing decides for one. The
simulator's LLM squad layer is not dealt in IC fields.

**What is owed.** A decision layer for an NPC commander, so a detachment with no player aboard is
led. SCREEN and SUPPORT have their gate tests (GC.0) and no story that flies them as delivered
orders (doc 36). Sub-leaders stay deferred (Q-36-43).

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

And from the second review (§4c items 13 to 16). **The view is thin**: a contact's bearing is
flat (no elevation), speed is one number, there is no velocity vector, no propellant, no rock and
no hold, and the four postures of EMCON reach a mind as one switch whose "off" means Standard for
the corvette and Ghost for the raider. **There is a third hook**: with no customs state in the
view, Rust writes `stage` into the corvette mind's memory and reads it back
(`sync_corvette_cleared_stage_in_memory`). **A stand-down leaves the fight on**: clearing a mark
leaves the party's `fired` set, so anyone who fired stays hostile. **A space seal runs no rule
program**: it is bound with no rules, forced to the scripted engine at Class I. The drone's think
blocks the world thread in `space/mining.rs` as well.

**What is owed.** One applier from intents to commands that every space actor uses; a view that
carries what a mind needs to fly and to inspect; doctrines that steer by intents alone; a
stand-down per carrier that ends the fight; a doctrine named by the pack with its digest
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

From the second review (§4c items 17 and 18): the 18 major NPCs ship `"enabled": false` (the
34 minor and extra ones default to enabled), so the ambient timer as ruled would wake nobody;
five majors share one room at Ceres and four at Aitken. "A player in the room" counts a
link-dead character, staff who are DARK and an idle web tab. A turn drains its mailbox, so nothing
records which stimulus woke it (its text is in episodic memory, untagged). The NPC decision call
is made without its NPC, so the per-NPC budget and `@llm/kill` do not reach it. The
`content_assist` route has no block in the deploy template or the example, so content assist
falls back to templates. Every route shares one provider bucket of 60 requests a minute, the
global budget is 5,000,000 tokens and $5 a day, and the ledger is in memory and starts again at a
restart.

**What is owed.** A decision on the dead role; whether NPCs act unprompted, and which; the last
decision in the staff view; budgets sized for a box where play needs the LLM.

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

## 4c. The second review (Draft 0.5)

A second comment on PR #530, 2026-11-11, read at `95764abb5` against main at `0d8f3021`. It
agrees Drafts 0.3 and 0.4 answered the first review, and says several units could not be built in
the crates they name or rest on something a live server does not do. Five read-only lanes checked
its claims in the tree. **Most hold. Six are refuted.** Several symbols it cites do not exist
(`PendingTransfer`, `validate_launch_config`, `HardwareSection`, `SelfDealing`,
`jurisdiction_refusal`) and several paths are wrong; where the behaviour behind a wrong name is
real, the row says so.

| # | Claim | Verdict | What changes |
|---|---|---|---|
| 1 | The production boot never calls `with_ic_minds_in_bays`, so on a live server no claim opens, no claim seal is admitted and a one-seat licence is not seat-limited | **Confirmed** (`mush-server/src/boot.rs` `compose_launch`; only two test rigs call it). The "`; selling` never triggers" part is wrong: resale is composed; what is dead is the claim reader's refusal while a resale is pending | §4 C corrected; **MW.0**; Q-44-39 |
| 2 | Turning it on makes the title fence refuse a filed mind on every leased rig and every organisation-titled hull, until S.4c | **Confirmed** (`surface/dispatch.rs` `untitled_refusal`; the Roll's `TitleFence`) | **MW.22**; Q-44-39 |
| 3 | No launch route issues the Class III end-user entitlement; the example launch has no `ungovern` | **Confirmed** for integration and delivery (doc 17h says so). "The contraband loop is dark" overstates it: inspection, fines, seizure and claims are composed | **MW.23**; MW.1; Q-44-40 |
| 4 | A Registry rating runs on wall time, about three hours each, and no file key sets the pace | **Confirmed** (`bulk_maintenance_ms` is `serde(skip)`; `ci.yml` budgets three hours a story) | MW.6 moves ahead of MW.1 and MW.3; Q-44-41 |
| 5 | Deploy and reset crash-loop on a live `launch_path` | **Partly.** Nothing breaks today (the template ships the key commented, and the README says to take the path out before a reset). It is a real constraint on how MW.1 makes the path live | MW.1 |
| 6 | Players can connect during the bring-up boots | **Confirmed.** The fix it proposes, a private socket, would lock the operator out as well | MW.1 |
| 7 | A procurement policy needs the House's root scope, its own account, its agent standing in the venue and grants on two scopes | **Partly.** The rules are real and are already in `deploy/README.md`; `[tacsim.procurement]` ships defaults only, and a House's policy comes from the launch document | §8 |
| 8 | "A withdrawn listing leaves the counter" cannot be done inside `mush-market`; two stories assert the stale behaviour; procurement reads the same stale list | **Confirmed** (the cited "ceiling" is not the barrier; the reader's lens is) | MW.2 re-scoped |
| 9 | Renewal is two services in `mush-market`, and a stop held in memory is undone by the next scan | **Partly.** Q-44-14 already says "lasting"; the work is telling a lasting refusal from a passing one and filtering terminal states in the Roll's readers | MW.2 |
| 10 | The recall drain is in `mush-roll`, and revocation has the same fault | **Confirmed** | MW.6 |
| 11 | A pilot back after five minutes cannot take an IC unit back | **Confirmed** | MW.7 |
| 12 | Withdraw wipes a player's mind's memory | **Refuted**: only doctrine is seeded. What is true is worse for the ruling: a player's own mind is never told to go home | MW.7 |
| 13 | Rhai cannot board; the stock withdraw rallies and can fire first; a rig has no exit; a refused recover strands it; no edge geometry | **Confirmed** (any refusal strands it, not a full bay) | MW.7; Q-44-42 |
| 14 | A Rhai commander cannot be built in the mind ABI: one hull, its own intents, no `order` | **Confirmed** | MW.14 split; Q-44-44 |
| 15 | A launch needs a player organiser aboard; orders in a field mark nobody hostile; orders stop when the commander's hull is gone | **Partly**: the units may all be NPC-crewed; an order on open ground does mark; standing orders are kept | MW.14a |
| 16 | The space view is flat and blind to velocity; a third hook writes the mind's memory; a stand-down leaves `fired` set | **Confirmed** (own position is three-dimensional) | MW.9 |
| 17 | The script is recompiled on every think | **Refuted**: compiled scripts are cached by digest. What costs is rebuilding the runner each think. Rule programs are ignored and the drone think blocks the world thread: confirmed | MW.11, MW.13 |
| 18 | MW.13 and MW.21 tested a route choice in free flight, where no mind runs | **Confirmed**; this document says as much in §4 F | MW.13, MW.21 |
| 19 | `content_assist` has no route; the NPC decision call carries no NPC; a turn keeps no waking stimulus; no major NPC is enabled; presence counts link-dead, DARK and idle characters | **Confirmed** (paths wrong) | MW.1, MW.17, MW.18; Q-44-47 |
| 20 | The bot may not type `SIM MIND` and has no market verbs | **Partly**: it trades goods; it has no mind-market verbs | MW.19 |
| 21 | Seventy-two Rhai seats already failed to hold on Linux | **Confirmed** (`match_/seal.rs`; it is why the fill is native) | MW.20 |
| 22 | An incapacitated pilot keeps the controls; a captive counts as aboard; yielded raiders keep firing | Incapacitated: **confirmed**. Captive: **refuted** (a captive holds no station and is aboard another's carrier). Yielded raiders: **confirmed** | Q-44-43; doc 42b's list |
| 23 | SSH `mind` exec attaches a presence and restarts the link-dead window; the doorway writes a drain banner into its output | **Confirmed** | **MW.24** |
| 24 | The backup mind would fight an offline plan; a clean QUIT gets no return | **Confirmed** | Q-44-45, Q-44-46; MW.8 |
| 25 | Refit, strip and a change of title keep a hull's seal | **Partly**: true, and harmless until a player's mind can be sealed on a ship | MW.13 |
| 26 | A hull sold with an open claim is bricked; a derelict holds a licence seat for ever | **Refuted**: the close needs no title, and seats count claims, not ship epochs. A wrecked ship's epoch never closes: confirmed, owed by doc 23 | — |
| 27 | One provider bucket and a $5 day; IC sorties have no detachment panel; the chains still share files; 24:1 shortens no wall-clock timer | **Confirmed** in substance ("exhausted in an hour" is the reviewer's arithmetic; the shared files are append-only) | Q-44-48; §7's first paragraph; §7.3 |

## 5. Owner survey

**Ruled 2026-11-11.** The owner chose the alternative for Q-44-16 (no stopgap; wait for the
backup mind), amended Q-44-29 (nothing deleted, nothing retargeted), and accepted Q-44-6 and
Q-44-32 with a clarification each. Every other default is accepted as written. Each of those four
carries its decision first, with the proposed text kept under it for the record. Draft 0.3 added
Q-44-35 to Q-44-38 (§5.11), raised by the first review; the owner accepted all four at their
defaults the same day (Draft 0.4). Draft 0.5 adds Q-44-39 to Q-44-48 (§5.12), raised by the
second review; they are not ruled, and each default holds until they are. Two of them reopen a
ruled answer: Q-44-42 withdraws half of Q-44-17, and Q-44-44 changes the order of Q-44-20's work.

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

### 5.12 Questions the second review raises (Draft 0.5; each default holds until ruled)

**Q-44-39 — Licensed minds on a live server, and the title fence.**
- **Proposed default (changed in Draft 0.6):** MW.0 and MW.22 are built in that order and merged
  together. The claim path is composed in the production boot only with S.4c, lease and
  organisation authority in the fence, beside it.
- **Alternative (Draft 0.5's default):** compose the claim path now, and S.4c straight after;
  between the two a filed mind flies only on a hull its pilot holds personal title to.
- **Why:** the personal-title path is built and tested and nothing on a live server uses it. But
  a starter flies a chartered hull and a House flies its own, so MW.22 is not optional before
  players or before procurement means anything in a field.
- **Why the default changed.** Starting MW.0 showed that the alternative takes something away.
  With no claim path, `untitled_refusal` (`surface/dispatch.rs`) reads no title, and a leased
  or House-titled hull launches a filed mind as S.4a's dormant seal: the seal is written to the
  black box, which is what doc 17h's inspection, contraband and procurement stories read
  (M8f, M8g). With the claim path composed and no S.4c, the same launch is refused
  (`MIND_UNTITLED`, D-23-22). So on a live server every House hull and every chartered rig
  would lose its filed mind until MW.22 landed. The test rigs that prove M8f and M8g compose
  no claim path for the same reason.

**Q-44-40 — The Class III entitlement in a launch world.**
- **Proposed default:** the launch composes a route that issues it (MW.23). C4's story 4 and the
  heirloom ruling (Q-44-12) wait on that unit.
- **Alternative:** staff mint it on the dev boxes and story 4 stays in the test rig.
- **Why:** Q-44-12 and story 4 both assume a Class III core can exist in a launch world, and none
  can.

**Q-44-41 — The Registry's pace on a deployed box.**
- **Proposed default:** `[tacsim.minds]` gains a key for the Registry's pulse, 1,000 ms by default
  as today. The dev boxes set it low (provisional: 50 ms, the story harness's value), so a rating
  takes minutes. A production box keeps real time, as doc 43c records it was decided.
- **Alternative:** real pace everywhere; each rated line a dev box needs costs about three hours
  at every wipe.
- **Why:** the bring-up needs a rated listing for a House to buy, and the boxes are wiped at every
  step.

**Q-44-42 — A field unit with no carrier on the site.**
- **Proposed default:** it holds where it stands, as Q-CF-18 already has it. Q-44-17's "the site's
  edge on its party's side" is withdrawn: the ground has no such geometry and Q-CF-18 ruled out an
  edge exit.
- **Alternative:** build the edge.
- **Why:** it is what the field does today, and the unit is no safer at an edge it cannot leave by.

**Q-44-43 — Conscious and at liberty.**
- **Proposed default:** Q-44-35 gains two words. A character who is incapacitated or held captive
  neither keeps a hull crewed nor holds its controls. A pilot who is knocked out while connected
  loses the controls to the unit's mind at once, with no five minutes.
- **Alternative:** Q-44-35 as accepted.
- **Why:** today an unconscious pilot who is still connected goes on steering and firing.

**Q-44-44 — The commander, in two steps.**
- **Proposed default:** MW.14a is a compiled commander: a host-side issuer for the plan's
  `npc_commander` that gives roster orders through the existing delivery, with a successor when
  its hull is lost. It is done by a detachment with no player in its units; the organiser is still
  a player aboard the carrier. MW.14b is the Rhai commander: its own view, an order channel, and
  D-UM-11 lifted for that class of mind; it is designed after MW.14a.
- **Alternative:** design the commander ABI first and build once.
- **Why:** Q-44-20 ruled a Rhai doctrine, and the mind ABI is one hull that may not order. The
  direction stands; the order of work changes.

**Q-44-45 — A hull on an offline plan.**
- **Proposed default:** she keeps her plan when her link drops. The backup mind takes her only
  when the plan ends or fails. The plan's own buying and selling is the player's, filed before
  they left, and is not the mind trading.
- **Alternative:** the backup mind aborts the plan and flies her to her filed port.
- **Why:** a plan is the game's own way to fly while away (doc 18), and the ruling's "never
  trades" was about a mind acting on its own.

**Q-44-46 — A clean QUIT away from port.**
- **Proposed default:** it hands the unit to its mind at once, with no grace, on both tracks. The
  player chose to leave, so there is nobody to wait five minutes for.
- **Alternative:** QUIT is refused away from port, with a line saying why.
- **Why:** today a ship whose pilot quits is left with her drive cut for ever and nobody told.

**Q-44-47 — Which NPCs act unprompted, and for whom.**
- **Proposed default:** the dev boxes enable the 18 major NPCs. "A player in the room" means
  connected, not link-dead, not DARK, and active within the last ten real minutes (provisional).
  A room hears at most one unprompted line in a window, whichever of its NPCs speaks.
- **Alternative:** the majors stay parked and the timer runs for the minor NPCs.
- **Why:** Q-44-31 named the majors and none is enabled; five of them share one room.

**Q-44-48 — The LLM's budget on a box where play needs it.**
- **Proposed default:** the template's budgets stay ($5 and 5,000,000 tokens a day, 60 requests a
  minute) until MW.20 has measured what a full match's squads spend. The bring-up records when a
  budget ran out, and the figure is set from that.
- **Alternative:** raise the day's budget now (provisional: $25).
- **Why:** spending is the owner's to set, and there is no measurement yet to set it from.

## 6. Which category gets which document

| Document | Categories | Shape |
|---|---|---|
| **This document, §7** | A, D, G, I | the units below; H has none (Q-44-32 deferred) |
| **Doc 23** | S.4c | MW.22 is the cut doc 23 already names; its design note is written there |
| **Docs 35 and 27** | C | the ruling's rules; the field cut beside the space one |
| **Doc 45** | E | a survey in this form |
| **Doc 35d §9's successor** | F | rows after SF.R31 |
| **Docs 17g and 17h** | B | the defects and C4 appended to their lists |

## 7. Units of work (Draft 0.5)

Twenty-seven units: `MW.0` to `MW.24`, with MW.14 and MW.17 each in two parts. They are cut from the rulings and
tightened by two reviews. "Waits on" is a unit that must be merged first. Every acceptance is
through registered commands with a non-GOD actor. **A unit that wires a mind is done only when
changing the mind changes what happens in play**; a script that thinks, traces and is overruled is
the failure this document found.

**Time in a story.** A window measured in IC time is waited out at 24:1. A window measured on the
wall clock is not shortened by the rate: the link-dead grace, the ambient timer and the Registry's
pulse are wall-clock. A story sets `[game].linkdead_grace_s` for the grace, uses a paused clock in
an in-process rig for a timer (doc 43c, TS.10), and uses the Registry's pulse key for a rating
(Q-44-41). No story waits five real minutes.

### 7.1 The units

| Unit | Category | What it is | Mostly in | Waits on | Done when |
|---|---|---|---|---|---|
| **MW.0** | A | Licensed minds composed on a live server (Q-44-39): the production boot's launch composes the claim path the test rigs compose. **Merged only together with MW.22** (Draft 0.6): alone it would refuse a filed mind on every leased and House-titled hull, which launch one today as a dormant seal | `crates/mush-server/src/boot.rs` (`compose_launch`), `crates/tacsim/src/launch.rs` | — (merges with MW.22) | on the production composition, not a rig's own: a filed mind launched on a rig its pilot holds title to opens a claim and is admitted to a field under its own seal; a one-seat licence refuses a second sortie while the first is out; a launch while that licence's resale is pending is refused; a leased rig and a House-titled hull are refused by name |
| **MW.1** | A | The launch switch and the bring-up script (Q-44-4, Q-44-5), walking the runbook's whole sequence; `launch_path` kept out of the rendered file until the box's `launch.json` exists, and taken out again before a reset; the script says how its in-game steps are typed as #1 and how players are kept off until the Active boot; a procurement policy for each House with what a purchase needs (Q-44-8); an ungoverning shop named in the dev launch; the raid director on (Q-44-7); the Registry's pulse set for a dev box (Q-44-41); the `content_assist` route added to the template and the example | `deploy/instance/` (`render.sh`, `instance.env.example`, `mushd.toml.tmpl`, `launch.json.example`, a new bring-up script), `deploy/dev1`, `deploy/dev2`, `deploy/README.md`, `mushd.toml.example` | MW.6 for the purchase check | on a throwaway Redis the script takes a fresh store through boot, the identities, the Initialize boot, all seven grants, the four initialize actions and the Active restart, and `@mind/launch` names every operator `ready`; the same run interrupted once and started again ends the same way; no player can log in before the Active boot; a reset and a second bring-up work; a House with three titled hulls, a funded treasury and a rated listing buys a licence by its own agent; rendering with the switch off gives today's file byte for byte |
| **MW.2** | B | The two defects ruled to be fixed before the go-live (Q-44-14, Q-44-15), in the crates they live in. A listing that can no longer be sold leaves the counter and the House agent's list: a read the Roll owns says whether a listing is withdrawn, recalled, lapsed with its seller's or its parent's licence, or moved with its charter, and both the counter and `ProcurementListings` ask it. A renewal that can never land stops: the Roll's renewal readers leave out terminal entitlements, both renewal services tell a lasting refusal from a passing one, and `+minds/renew` says why in the ruled words | `crates/mush-roll/src/commerce/` (`withdrawal/`, `renewal/reader.rs`, `house_renewal/reader.rs`), `crates/mush-market/src/` (`mind_commands.rs`, `mind_commands/procurement.rs`, `mind_renewal/service.rs`, `mind_house_renewal/service.rs`) | MW.6 | the two stories that assert a withdrawn listing stays visible (`player_market_story.rs`, `player_market_typed.rs`) are rewritten and pass; a recalled listing is gone as well; the House agent does not ask for a dead listing again; a recalled licence is tried once and not again after a restart; a derivative waiting on its parent's renewal is still retried |
| **MW.3** | A | The go-live on dev1 and dev2: §8, run with the owner | the boxes | MW.0, MW.1, MW.2, MW.6 | §8's steps recorded for each box, the provider audit and the House's purchase among them |
| **MW.4** | docs | The rulings written into their own documents: D-UM-56, Q-UM-12, Q-UM-14, Q-UM-18 (doc 17); paid Follow deferred, the renewal wording (17g); the heirloom and the House term (17h); the raid director on the dev boxes (doc 36); doc 03's "No code exists yet" replaced by what is live and by the storyteller role's state; the defects this survey found that belong to another document's list (doc 42b: a yielded hull keeps firing; doc 23: a wrecked ship's epoch never closes) | `docs/design/` | — | each question reads "ruled 2026-11-11" where it was asked, and each defect is on its document's owed list |
| **MW.5** | B | Doc 17g's C4, all five of its items: 17f §9's stories 1, 2, 3, 5, 6 and 7 on the production composition, each with a restart; story 4 through the launch's governed hardware, with a refit and a restart; `cargo test -p mush-bot` green on every new sentence; the cross-host deterministic cases; Registry load beside live arenas, reported | `crates/tacsim/tests/`, `crates/tacsim/src/minds/`, `crates/mush-bot` | MW.1, MW.2, MW.22, MW.23 | all five items pass and "C4 as built" is written in 17g, with any item not built named there and why |
| **MW.6** | B | The Registry's pace as a key (Q-44-41), and the two notice drains that abort on one bad holding, recall and revocation | `crates/tacsim/src/config.rs` and `crates/mush-config` (the pulse), `crates/mush-roll/src/commerce/registry_recall/notice.rs`, `registry_revocation/notice.rs` | — | a server whose file sets the pulse rates a mind in minutes, and one that does not keeps today's pace; one bad holding no longer stops either drain, and the cursor moves past it; the measured time a rating takes at both paces is in the cut note |
| **MW.7** | C | The field's backup mind and the absent-player gate. A way back into the seat: a pilot who reconnects, at any time, is bound and seated again by the field host. The go-home objective reaches every mind, a player's sealed one included; the host boards the unit when it reaches the ramp, since no mind may. The stock mind does not rally while its pilot is absent, a rig has an exit ask, and a recover that is refused is tried again. With no carrier on the site the unit holds where it stands (Q-44-42). The engine refuses, for any mind flying for an absent player, a fire intent at a unit that has not fired on this one, the cornered stand included, and any mining or trading (Q-44-38). A stricter order the player left stays; an order to advance or attack lapses (Q-44-18). A hand-back drops what the mind had pending (Q-44-37); a restart does not re-seat a pilot who is not connected. A pilot who is incapacitated loses the controls at once (Q-44-43); a clean QUIT hands over at once (Q-44-46). The simulator is unchanged (Q-44-19) | `crates/tacsim/src/arena_bridge.rs` (the release and the re-seat for an IC task), `crates/tacsim/src/arena/` (`mind.rs`, `stock_mind.rs`, where an intent is accepted), `crates/tacsim/src/field/` (`home.rs`, `exit.rs`, `boot.rs`, `crew.rs`) | MW.0 for the filed-mind story; the design section MW.8's session writes for the field | six stories, each failing today: the house mind, with a hostile in view, does not fire and boards its carrier; a filed mind written to fire first is refused the shot, still returns fire at a unit that hits it, and is taken home; a HOLD FIRE left by the pilot holds through an attack; a mind whose script faults falls to the house default and goes home; a pilot who reconnects ten minutes later is back in the seat and their first command wins; a pilot knocked out while connected stops steering. Then a restart while the mind holds the unit resumes the mind |
| **MW.8** | C | The space backup mind. **In flight** (the session started 2026-11-11). The reviews add to what it is done by | `crates/tacsim/src/space/` | — | as that session's brief has it, and: a connected passenger neither holds the backup off nor takes the helm (Q-44-35); with two crew dropping four minutes apart the hull's window is five minutes from the second (Q-44-36); a restart while the mind holds the helm resumes it; a reconnect with a think in flight discards it (Q-44-37); a hull on an offline plan keeps flying her plan (Q-44-45); a clean QUIT away from port hands her to the mind at once and she is flown home (Q-44-46) |
| **MW.9** | F | One applier from intents to bubble commands, used by every space actor, movement included. One view for every space mind: contact ids numbered for each observer after detection; a velocity vector and a contact's elevation; propellant and the return reserve; the four postures of EMCON; customs and inspection state, so that nothing writes a mind's memory from outside. The two trace-string hooks and the memory hook removed. The interdictor's stand-down per carrier, on state, ending the fight for the parties it clears (Q-44-24) | `crates/tacsim/src/space/picket.rs`, `tick.rs`, the three doctrine scripts, `crates/tacsim/src/arena/party.rs` (the stand-down) | MW.8 | a doctrine changed to hold where it used to close makes its actor hold; an escort that heaves to does not clear the runner beside her; a Dray can heave to against the interdictor, by a burn a mind computed from the view; after a stand-down a party that had fired is no longer hostile; adding a ship the observer cannot detect leaves a mind's view byte for byte the same; no code outside a mind writes its memory |
| **MW.10** | F | A doctrine named by the pack's cast with its digest (Q-44-25, as corrected in §4a): staff publish a new revision with the workshop's verbs and select it for an actor; the next bubble flies it | `crates/tacsim/src/minds/content.rs`, the cast in the pack, the seal sites in `space/` | MW.9 | staff publish revision B and select it; the next picket behaves as B; a bubble that was open, and a replay of an earlier one, behave as A; publishing other bytes under A's digest is refused |
| **MW.11** | F | The think's cost measured, then thinks taken off the world thread, the drone's included: the snapshot of step N, intents at N+1, one think a second an actor (Q-44-28), on a pool sized by `[tacsim.minds].workers` and never by a caller, with a runner kept for each seal and not rebuilt each think | `space/tick.rs` (the step), `space/picket.rs` (the think host), `space/mining.rs` (the drone think), `minds/pool.rs` | MW.9 | the measured cost before and after in the cut note; the same encounter run twice from before a decision, with the workers' timing varied, gives the same commands, the same mind memory and the same outcome; a server whose first think is in space has the configured number of workers; no think blocks the world thread |
| **MW.12** | F | Drones as mind-driven sorties: a view of the rock and the hold, intents that move and work, the yield from what the drone did, the sortie record closed (Q-44-27) | `space/mining.rs`, the drone think in `space/picket.rs`, `drone_mining.rhai` | MW.9, MW.11 | a drone whose mind is changed to stop short brings back less; the excursion book holds no drone after it lands |
| **MW.13** | F | A player's own mind sealed on a ship: helm and sensors, the class from the hull's core (Q-44-26), under a licence or the owner's licence; its rule program bound, which no space seal does today; the seal keeps its class, principal and claim across a restart, and is cleared or checked again at a refit, a strip and a change of title | `space/` seal sites and `compact_mind_seal`, `crates/tacsim/src/minds/sealed.rs`, the registry, `crates/tacsim/src/port/` (refit, strip, sweep and sale) | MW.0, MW.1, MW.8, MW.9, MW.10, MW.22 | on a governed hull a filed mind the player holds is sealed, and an unfiled draft is refused; with her link down, inside a bubble, that mind makes a helm or sensor choice the house default does not; after a restart the seal reads the same class, principal and claim, and the claim closes once when she docks; a core stripped after sealing leaves no seal of its class; a hull sold or swept carries no seal to its new owner |
| **MW.14a** | D | The compiled commander (Q-44-44): a host-side issuer for the plan's `npc_commander`, giving the roster orders a player leader can through the same delivery (authority, the comms path, the recipient's own contacts), yielding to an eligible player who returns (doc 27 FO-15), with a successor when its hull is lost | `crates/tacsim/src/detachment/`, `field/orders.rs`, `crates/tacsim/src/arena/detachment.rs` | — | a detachment with no player in its units advances, holds and withdraws on its commander's orders; with its comms jammed an order does not arrive; when the commander's hull is lost the next NPC gives the orders; the same run twice from before an order gives the same orders |
| **MW.14b** | D | The Rhai commander (Q-44-20, Q-44-44): a commander ABI with its own view and an order channel, D-UM-11 lifted for that class of mind, and MW.14a's rules written as its house doctrine | `crates/mush-mind`, `crates/tacsim/src/minds/`, the doctrine library | MW.14a; a design section and its ruling | changing the commander doctrine changes the orders a detachment gets |
| **MW.15** | D | The story doc 36 still owes for SCREEN and SUPPORT. Corrected in Draft 0.6: the two gate tests exist since GC.0 (`field::orders::gc0_a_screen_in_a_field_holds_its_bearing`, `gc0_a_support_in_a_field_binds_to_its_partys_unit_only`); what is not built is a story that flies them as delivered orders, which needs a detachment's roster flown into a field in a test rig, and no tacsim test does that yet | `crates/tacsim/tests/domain_field/` | — | a roster's SCREEN and SUPPORT, given by its leader in a running field, are delivered and obeyed: the screening unit holds its bearing and the supporting unit keeps to its charge |
| **MW.16** | E | Doc 45: the survey of asynchronous attack and defence, on one concept (Q-44-22, Q-44-23). It takes up what the second review asked of that concept: an absent defender who is evacuated never defends a site | `docs/design/` | — | the survey open for rulings. Its plan and its cuts are doc 45's own, cut there after it is ruled, as §7 was here; this unit does not close the category |
| **MW.17** | G | The last decision in the staff view (Q-44-30): a turn records which stimulus woke it; `@agent/show` gains that, how many public facts and memories the model was shown, and whether its line came from the model or from the profile's fallback, within the audit's rule that a prompt is kept by hash and summary only. **Built, PR #535** | `crates/mush-npc` (`runtime.rs`, `module.rs`) | — | staff read, for a station NPC that has just spoken, the stimulus and "model" or "fallback"; with the provider cut the same view reads "fallback"; a player is refused |
| **MW.17b** | G | The NPC decision call carries its NPC, so the per-NPC budget and `@llm/kill` reach a decision. Split from MW.17 in Draft 0.6: the worker lease the call is handed (`mush_core::decision::DecisionWorkerLease`) is public-safe by design and names a controller and a decision, not an object, so an audit attribution has to be added to the lease itself | `crates/mush-core/src/decision.rs`, `crates/mush-server/src/decision_llm.rs`, `crates/mush-npc/src/decision.rs` | — | `@llm/kill` on an NPC stops its decisions as well as its dialogue; a decision is charged to its NPC's budget; nothing else about the world crosses the lease |
| **MW.18** | G | The ambient timer scheduled (Q-44-31, Q-44-47): for the enabled major NPCs, when a player is present in Q-44-47's sense, at most one unprompted line a room in 30 real minutes, under the budgets | `crates/mush-npc`, the cast packs | MW.17 | on a paused clock: a major NPC speaks unprompted once in the window and no NPC in that room speaks again within it; with only a link-dead or a DARK character in the room nothing is spent; the audit shows the call |
| **MW.19** | I | The bot's minds lane on the nightly (Q-44-33): the bot's lobby vocabulary takes `SIM MIND`, the bot gains the mind market's verbs, and the lane writes, files, lists, buys, and flies in the simulator and on an IC sortie | `crates/mush-bot` (`arena/lobby.rs`, a new lane), the nightly workflow | MW.1, MW.5, MW.6, MW.24 | green three nights running, at the Registry pace the rig sets |
| **MW.20** | I | Minds measured at size (Q-44-34), as a measurement and not a gate: 100 units on the native doctrine, in the simulator and in a field; the same on Rhai seats, which is expected to miss (72 Rhai seats did not hold on Linux, the reason the fill is native); eight bubbles in space; the two together on one server; and what a full match's squads spend at the LLM (Q-44-48) | the fill harness, a space harness | MW.7, MW.11 | the figures in the cut note, with what misses, for each alone and for both at once |
| **MW.21** | I | The completion story, with two accounts (first review, finding 9). One player writes, files and lists a mind. The other buys it, fits it to a governed hull and takes her out. In a bubble, with the link down, the mind makes a choice the house default would not, keeps to the absent-player limits, and spends only the ship's own stores. The server restarts while it holds the helm. The player reconnects with its next action pending | `crates/tacsim/tests/` | MW.5, MW.7, MW.8, MW.13 | the player's first command wins; the mind's pending action never lands; the claim closes exactly once when she docks |
| **MW.22** | B | S.4c: lease and organisation authority in the title fence (Q-44-39), in the Roll and at the dispatch | `crates/mush-roll/src/commerce/ic_claims/reader.rs`, `crates/tacsim/src/surface/dispatch.rs` | MW.0 | a chartered pilot launches a filed mind on her leased rig; a House's licence flies on a House-titled hull; a pilot with neither a title, a lease nor an office is still refused |
| **MW.23** | B | The Class III end-user entitlement issued by a launch route (Q-44-40) | `crates/tacsim/src/launch.rs`, `crates/mush-economy/src/core_integration/` | MW.0 | in a launch world, with no staff minting, a Class III core is integrated and delivered |
| **MW.24** | C | SSH `mind push`, `pull` and `check` without a presence: the exec neither logs the character in nor drops it as a lost link, and a doorway drain writes nothing into its output. Sized in Draft 0.6: the exec runs an invoke as the character over a connection the engine has to know, so it needs a connection that carries output and an invoke and is invisible to presence: no `Connected`, `Linkdead` or `Disconnected` hook, not counted as a live connection, no session state change. That is a flag from the SSH hook through the gateway bridge and the IPC message (a version bump) to `Engine::conn_up` and `conn_down` and the session manager | `crates/mush-net-ssh/src/server.rs`, `crates/mush-gateway/src/bridge_common.rs`, `crates/mush-ipc`, `crates/mush-session/src/manager.rs`, `crates/mush-core/src/engine.rs` (`conn_up`, `conn_down`), `crates/mush-sshdoorway/src/server.rs` | — | an offline player's `mind push` leaves no link-dead character in the room; one typed four minutes into a link loss neither restarts the window nor takes the controls; `mind pull` during a drain gives the script's bytes and nothing else |

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
- What a client shows of a mind: IC sorties carry no detachment panel (the projection reads the
  practice host only), the TUI and the web have no view of a mind on a ship, and the TUI's market
  screen clips two columns. **Deferred**, with no unit here; it follows MW.13.
- A wrecked ship's epoch, which never closes (doc 23), and a yielded hull that keeps firing (doc
  42b): defects on those documents' lists (MW.4 writes them there).
- Doc 45's plan and build: doc 45's.

### 7.2 What may start now

MW.0, MW.14a, MW.15 and MW.24 wait on nothing, and MW.8 is running. MW.1's script can be
written at once; only its purchase check waits on MW.6, which is built (PR #534). MW.2 follows
MW.6 in the market chain and may start from its branch. MW.4, MW.16 and MW.17 are built; MW.17b
and MW.18 may start from MW.17's branch.

### 7.3 One writer a file

| Chain | Units, in order | The shared files |
|---|---|---|
| **Composition** | MW.0 → MW.22 → MW.23 | `crates/mush-server/src/boot.rs`, `crates/tacsim/src/launch.rs`, `surface/dispatch.rs`, the Roll's `ic_claims/` |
| **Deploy** | MW.1 → MW.3 | `deploy/`, `mushd.toml.example` |
| **Market** | MW.6 → MW.2 → MW.5 | the Roll's `commerce/` (notices, withdrawal, renewal) and `crates/mush-market` |
| **Space** | MW.8 → MW.9 → MW.10 → MW.11 → MW.12 → MW.13 | `space/picket.rs` and `space/tick.rs`; every unit of the chain edits both |
| **Field** | MW.15 → MW.14a → MW.7, then MW.14b | `field/orders.rs`, `detachment/`; MW.7 also holds `arena_bridge.rs` and the arena's intent gate alone |
| **NPC** | MW.17 → MW.17b → MW.18 | `crates/mush-npc/src/module.rs`; MW.17b also edits the decision lease in `mush-core` |
| **SSH** | MW.24 | `crates/mush-net-ssh`, `crates/mush-gateway`, `crates/mush-sshdoorway` |
| **Docs** | MW.4, MW.16 | no shared file |
| **Acceptance** | MW.19, MW.20, then MW.21 | last; they read what the others built |

The space chain is one writer for its whole length. MW.8's session writes a mapping from intents
to commands for the backup mind; MW.9 makes that mapping the one applier and moves the three older
ones onto it, so MW.9 does not start until MW.8 has merged. The composition and market chains both
edit the Roll, in different directories (`ic_claims/` against `withdrawal/`, `renewal/` and the
notices).

**Files every chain appends to.** A new story is a `mod` line in `crates/tacsim/tests/domain_field.rs`
or `domain_world.rs`; a changed verb is a note in `crates/mush-contract-tests/legacy-handlers.toml`;
a new test binary is a row in `test-tiers.toml`; a new config key is a line in both
`mushd.toml.example` and the deploy template, which a contract test keeps in step. These conflicts
are one line each and are resolved by keeping both sides. A story under `crates/tacsim/tests/` may
not write a space book directly (`arena_library.rs`); it goes through commands.

### 7.4 The edges between chains

- **MW.0 is under three other chains.** A filed mind in a field (MW.7's second story), C4 (MW.5)
  and a mind on a ship (MW.13) each need the claim path composed.
- **MW.0 and MW.22 are one merge.** They are two units because they are two changes, built in
  order by one session; neither reaches main without the other (Q-44-39). Where a row above
  waits on MW.0, it waits on that merge.
- **MW.6 is under the go-live.** Without the pulse key a dev box waits about three hours for each
  rated line it needs, at every wipe.
- **MW.7 waits on a design, not on code:** the field section MW.8's session writes into doc 27.
  If that section is late, MW.7's first act is to write it from the rulings.
- **Q-44-39 to Q-44-48 are open.** Each unit is built on the default and says so in its cut note.
- **The absent-player gate is one gate.** MW.7 builds it in the arena, where a field's intents are
  accepted; MW.8 and MW.9 must refuse the same things in space. Whichever lands second uses the
  first one's rule, not a copy of it.
- **MW.24 comes before either backup mind lands.** With a hand-back at reconnect, an SSH
  `mind push` would take the controls from the mind for ten milliseconds and start the window
  again.
- **MW.13 needs the launch** (MW.1): a governed hull takes only a filed mind, so there is no draft
  to test it on.
- **MW.19 needs MW.1's bring-up** to stand a launch up in the bot's rig, and MW.5's stories to
  have found what a story can find first.
- **MW.3 is the owner's.** It also wants what is open against main merged first.
- **Doc 45 (MW.16) may change MW.7 and MW.8** where its rulings widen the hand-over to an absent
  player. Both are built for link loss alone; neither should assume link loss is the only door.

### 7.5 Waves

- **Wave 0, now:** MW.0 then MW.22 (one session, one merge), MW.1, MW.15 then MW.14a, MW.24,
  MW.2 (from MW.6's branch); MW.8 already running. Built and open: MW.4 (PR #532), MW.6 (PR
  #534), MW.16 (PR #533), MW.17 (PR #535). MW.17b and MW.18 follow MW.17 in the NPC chain.
- **Wave 1:** MW.2 (after MW.6), MW.7 (after MW.0 with MW.22, and the field section), MW.9
  (after MW.8), MW.18.
- **Wave 2:** MW.3 (after MW.0, MW.1, MW.2 and MW.6), MW.23, MW.10, then MW.11 and MW.12.
- **Wave 3:** MW.5, MW.13, MW.19, MW.20; MW.14b once it is designed and ruled.
- **Wave 4:** MW.21.

## 8. The dev-box list (dev1 and dev2)

Nothing here is done without the owner; a redeploy wipes the store. Steps 3 to 6 are what MW.1's
script does; the list says what each box must show.

1. **Before the redeploy.** Merge what is open against main. MW.0 (licensed minds composed),
   MW.2 (the two market defects) and MW.6 (the Registry's pace) are merged first.
2. **The launch's files.** MW.1: the bring-up script; the `MINDS_LAUNCH` switch in `instance.env`,
   `instance.env.example` and `render.sh`; the template's `[tacsim.minds]` block and its class
   tables kept in step; the Registry's pulse set low for the box (Q-44-41); the `content_assist`
   route.
3. **Wipe and redeploy both boxes** at main, with the launch off and players kept off. Create the
   identities the runbook's steps 2 to 7 name, and write the box's `launch.json` in mode
   `initialize`.
4. **The Initialize boot**, with the launch live. Give each Registry operator all seven grants on
   the review scope: recall, revoke, inspect, fine, claims, seize, forfeit.
5. **The four initialize actions:** `roll-initialize-commerce`, `initialize-core-die-stock`,
   `initialize-core-factory`, `initialize-core-refit`.
6. **The Active restart.** Set the mode to `active` and restart. `@mind/launch` must end
   `Registry operators: #<op> ready` for every operator; a `lacks` names a grant to give again.
   Players are let in from here.
7. **Procurement** (Q-44-8), as `deploy/README.md` has it: each House's policy names its root
   scope; its agent is a character on an account of its own, standing in the market's venue room,
   with the holder's grant on the root scope and the attestation grant on the House's scope, and
   where the policy names a standing, the faction membership that standing needs. The House has at
   least three titled hulls, money in its root-scope treasury, and a rated line listed for it to
   buy. The check is a licence the House's own agent bought, read from its ledger. **The raid
   director** (Q-44-7): on.
8. **The LLM** (Q-44-6): keys present. Ask a station NPC a question and start a match with an AI
   squad, then read `@llm/log`: there must be a row for an `npc_dialogue_*` route and one for
   `tacsim_deliberate`. A reply alone proves nothing, because an NPC whose provider fails speaks
   its fallback line. Play does not run without the LLM, so a route with no row holds the box
   back. Read `@llm/budget` at the end of the smoke and record it (Q-44-48); the ledger starts
   again at a restart.
9. **A smoke by hand, on w11term, at each box:** `@mind/new`, `@mind/save`, `@mind/prove`;
   `SIM MIND` and a match; `@mind/publish`; `+minds` and a purchase; `+tv/<n> launch mind=filed:N`
   on a rig the pilot holds title to (a leased rig is refused until MW.22).
10. **Record** what each box runs in the deploy notes: the commit, the switches, the launch
    document's identities.
