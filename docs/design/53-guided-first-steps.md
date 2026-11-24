# 53. Guided first steps — a quest chain for a new captain, and a tracker that is always on screen

Draft 0.1, 2026-11-24, design only. Written against `origin/main` at `a7b9022c9`. It changes no
code and no content. Every question in §9 is open; nothing here is cut until the owner rules.

**Why.** On 2026-11-24 a dev1 playtester said the game felt "more like managing a database than
playing a game" and stopped at the first fuel purchase. The owner's answer, in his words: "the
quest system could help here to provide an in-game flow towards things, if you got a quest from an
npc you spawned in front of that told you to go register a ship, and then when that quest was
completed you got a quest to go buy propellant, and the quest was always there on the side of your
screen like in WoW or such and if you were lost you could click on the quest and it would remind
you what you needed to be doing and how".

**The questions, each with its proposed default** (each in full in §9):

| | Question | Proposed default |
|---|---|---|
| Q-53-1 | May one starter quest unlock the next? | yes, in the new chain only; the four old invitations stay independent, and no step ever gates a service |
| Q-53-2 | How is a guided step accepted? | the first is offered by the giver and taken with `+quest/accept <giver>`, no token; each later step is accepted by the service when the one before completes |
| Q-53-3 | Who records a guided check? | the service, when it sees the work, wherever the player stands; no `+quest/check`, no room rule |
| Q-53-4 | Does a guided step need a walk back to the giver? | no: it finishes when its last check is recorded |
| Q-53-5 | Who is the giver, and where? | a new NPC, an arrivals steward, standing in Port Watch Arrivals; one persistent person, not one per player |
| Q-53-6 | What does a new character see in Limbo, before any NPC? | a pointer from the service: `c` to Ceres, and who to see there |
| Q-53-7 | How far does the chain go? | seven steps, to the first sale of mined ore; built in three content cuts, the first ending at the charter |
| Q-53-8 | Does a guided step verify a state or fresh work? | a state wherever a state says the same thing; receipts only for the scan, the sounding and the sale |
| Q-53-9 | How does a player stop the guide? | `+quest/guide off` pauses it, `on` resumes; a guided step cannot be abandoned or declined for good; no per-step skip |
| Q-53-10 | What do existing characters get? | no prompt if already chartered; they can ask the steward, and steps already true pass at once |
| Q-53-11 | What does the chain pay? | nothing a step; the usual once-only relationship bonus at the end; no credits |
| Q-53-12 | Which quests does the tracker show? | any active quest, the guided chain first; `+quest/track` picks another |
| Q-53-13 | Where is the tracker in the SSH TUI? | top of the scene screen's right rail, the last pane to go; a short form on the status line everywhere else; one line on the under-120 frame |
| Q-53-14 | What does "clicking" a step do? | TUI: Enter on the pane puts the step's command in the input box, a key prints the full step. Web: a click opens the step, a button fills the composer |
| Q-53-15 | What does line mode get? | two lines when the step changes, `+quest/next` on demand, and one line, once, naming `@tui` and the web |
| Q-53-16 | How is "where is it" answered? | a route computed over the port's live exits; authored words when the place is in another port |
| Q-53-17 | Does the step fill in quantities? | yes for propellant and provisions: the tonnes come from the ship's own plan |
| Q-53-18 | What is the model's part? | none in the flow; every word on the tracker is authored; the steward can explain the current step in conversation when the model is on |
| Q-53-19 | One giver for the whole chain, or desk to desk? | one giver; the desk staff are met along the way |
| Q-53-20 | What happens to `docs/playthrough.md`? | Parts 1 and 2 shrink to "follow the tracker" as the content cuts land, and stay as the reference |

A claim about the tree is CONFIRMED only where a file and line are named. §11 lists what was not
checked.

## 0. In brief

- **A chain, "First run".** Seven short quests, one at a time, given by one NPC who stands where
  a new character first sets foot in the corridor. Register at the desk, charter a ship, fit her
  for prospecting, take on propellant and provisions, lift, read the ground, bring ore home and
  sell it. Finishing one starts the next (§3).
- **The service does the bookkeeping.** In a guided step the player types the game's own
  commands and nothing else. The quest service sees the charter, the license, the full tanks or
  the landing, records the check and moves on. There is no token to copy, no `+quest/check`, no
  walk back (§4).
- **A tracker.** One feed, `quest.tracker`, pushed to the player when anything on it changes:
  the step, where it is done and how to get there, and the exact next command. A side pane in the
  TUI, a card in the web client, two lines in line mode (§5).
- **Four earlier rules are reversed, for guided quests only** (§2). Every one is a question.
  Quests that are not marked guided behave exactly as today.
- **Most of it is engine work.** Chaining one quest to the next already exists. The guided flow,
  the tracker, the route pointer and six new objective kinds do not (§6, §7).
- **The model is not in the loop.** The whole chain and the tracker work with the model off, as
  the quest service does today.

## 1. What exists

### 1.1 The quest service

`crates/tacsim/src/quest/` is a durable journal over real evidence. Its first line states the
rule this document bends: "Dialogue can explain a quest; only explicit player commands advance it"
(`mod.rs:1-2`).

- **A definition** (`model.rs:36-48`) has a title, an introduction, a `prerequisite`, an optional
  `choice`, up to eight checkpoints and a completion line. It is bound to an NPC; one NPC holds at
  most eight definitions in a 64 KiB catalog (`docs/help/quests.md`, "Authoring a quest").
- **A prerequisite** is `paid_passage`, `open`, or `completed {quest}` (`model.rs:51-58`). So
  **one quest unlocking another is already built**: Leila's follow-up and Rosalind's paid-work
  quest use it. It is read from the asker's rows **with the same giver** only
  (`commands.rs:59-64`, `:199-209`), so a chain today has one giver.
- **An objective** is one of eight kinds (`model.rs:72-97`): `observation`, `coop_enrolment`,
  `survey_commission`, `trade`, `passenger`, `contact`, `paid_passage`, `survey`.
- **Accepting** needs the giver present and mutually perceivable (`mod.rs:38-66`) and the exact
  token `+quest/offer` printed, a ten-minute digest of the terms (`commands.rs:250-282`,
  `:368-384`).
- **A check** is the player's `+quest/check` in the checkpoint's bound room; the transition guard
  refuses any other place (`mod.rs:198-218`).
- **Finishing** needs the giver present again (`mod.rs:181-191`). It grants +10 trust and +5
  disposition with the giver, once a quest, and no credits (`commands.rs:279`).
- **Declining or abandoning is final** for that quest (`commands.rs:22`).
- **The service already watches.** Each tick it runs `evidence::watch` for every active row and
  reconciles (`mod.rs:226-234`). Trade receipts are copied into a per-actor book as the market
  settles them, each marked purchase or sale (`evidence.rs:89-102`). It knows the work is done
  before the player says so; today it waits to be told.

### 1.2 Today's starter quests

`crates/tacsim/content/quests/first_steps.json` holds four invitations: Ilse's (a Co-op
enrollment), Dov's (a fittings trade), Ma's (meet Nadia), Rosalind's (a scan and a sounding). Its
brief says: "Four independent invitations... None unlocks another or gates the services it
explains" (`first_steps.json:14`). Four more packs add follow-ups. None of them is where a new
character arrives, and none says what to do first.

### 1.3 Where a new character lands

- dev1 and dev2 start every character in Limbo, room 0: `START_ROOM=0`, and
  `CHARGEN_REQUIRED=false` because a playtest box has no approval staff
  (`deploy/dev1/instance.env:109-135`).
- From Limbo, `c` goes through `exit.limbo_arrivals` to Port Watch Arrivals, `room.c03_arrivals`
  (`crates/tacsim/content/atlas/neighborhood.ceres_freeport.json`). The Registry & Co-op Desk is
  one door north (`exit.c03_c05`, typed `registry`).
- **No NPC stands in Arrivals.** The Ceres cast has thirteen people, at the Registry (Ilse,
  Matthias), the Chandlery (Dov), the Exchange (Rosalind, Margit), the Slack Line (Ma) and four
  other rooms (`crates/tacsim/content/cast/ceres_freeport.json`).
- The OOC starter area (`crates/mush-atlas/content/neighborhood.ooc_area.json`) is a second door
  out of Limbo, with tutorial rooms for moving and talking. It teaches the MUSH, not the game.
- Where the onboarding gate is on, it holds a new character to a chargen command set
  (`crates/tacsim/src/sheet/onboarding.rs:1-20`). `+quest` is not on that set, so nothing in this
  document reaches a character before approval.

### 1.4 The route a new captain takes

`docs/playthrough.md` Parts 1 and 2: `c`, `registry`, `+coop`, `+coop/enroll <name>`, then
`+coop/refit prospector`, `+license/buy go`, walk to the Exchange, `+tv/lease go`,
`+ship/plan ahuna`, walk to the Chandlery, quote and accept reaction mass, the same for
provisions, walk four rooms to the berth, `+ship/board`, `+ship/lift ahuna`, then launch the rig,
scan, sound, drive, mine, recover, fly home and sell. About 35 typed lines before the first lift,
and at no point does the game say which is next. The fuel purchase is the first place the player
must read one command's output (the plan's burn), choose a number, and feed it to another.

### 1.5 The clients

- **Line mode is the default** after an SSH login; the TUI is entered with `@tui`
  (`crates/mush-net-ssh/src/server.rs:716-721`). A playtester who never typed `@tui` has no panes
  at all.
- **The TUI** has thirteen screens (`crates/mush-net-ssh/src/tui/screen.rs:444-458`). Screen 1,
  `scene`, is the scrollback with a right rail of three 24-column panes: `who`, `scene`, `actions`
  (`screen.rs:471-565`). At the 120-column floor the rail is 24 wide and the scrollback 96. Panes
  drop for height in a declared order (`TuiApp::visible_panes`, `app.rs:5106`). Below 120 columns
  no screen is drawn: one sentence and the input box (`screen.rs:36-44`, `draw_floor`,
  `app.rs:5640`). A tab strip and a status line are on every screen (`draw_tabs`, `draw_status`,
  `app.rs:5668`, `:5710`).
- **The TUI has no mouse input.** A pane can be `selectable`: a highlight moves in nav mode and
  a key acts on the row (`app.rs:4597-4680`). There is no general text popup; `draw_help`
  (`app.rs:6278`) is the nearest.
- The `journal` pane at `screen.rs:835` is the damage screen's unit journal, read from
  `tactics.unit`. It has nothing to do with quests.
- **The web client** has a header, a left aside holding the who list, and one screen at a time
  in the center over the composer (`web/src/components/Game.svelte:188-286`). No right rail, and
  no player-facing quest view; `QuestStudio.svelte` is the staff editor.
- **Feeds.** A module registers a feed with `reg.panel_feed(name)` and pushes
  `OutputEvent::Panel` frames to `Audience::Actor(player)` (`crates/mush-core/src/module.rs:1122-
  1160`, `:5083`). `tv.status` is the model: pushed only when the record changed, marked
  `retain: "latest"` (`crates/tacsim/src/port/site_map.rs:739-761`). The web subscribes with
  `subscribePanel` (`web/src/lib/game.svelte.ts:315`).
- **Nothing computes a walking route** between rooms on the server. `+map` exists; the walk
  view's path search is client-side over pack geometry (`port_walk.rs:3323`).

## 2. The earlier rules this reverses

Each is written down in the tree, not in a numbered ruling that was found (§11). The reversals
apply to a definition marked `guided` and to nothing else.

| | The rule today | Where it is written | What a guided quest does | Question |
|---|---|---|---|---|
| R1 | starter quests are independent: "None unlocks another" | `first_steps.json:14`; `docs/help/quests.md:5` | each step's prerequisite is the step before | Q-53-1 |
| R2 | accepting takes the exact printed token | `commands.rs:368-384`; help | no token | Q-53-2 |
| R3 | "only explicit player commands advance it" | `mod.rs:2`; `commands.rs:22` | the service accepts the next step, records checks and finishes | Q-53-2, -3, -4 |
| R4 | a check needs the player in the objective's room | `mod.rs:198-218`; help | a check is recorded wherever the player is | Q-53-3 |
| R5 | accepting and finishing need the giver present | `mod.rs:38-66`, `:181-191`; help | only the first acceptance does | Q-53-2, -4 |
| R6 | declining or abandoning is final | `commands.rs:22`; help | the guide is paused and resumed | Q-53-9 |
| R7 | every finished quest pays +10 trust, +5 disposition | `commands.rs:279`; help | one bonus for the whole chain | Q-53-11 |

**What is kept.** The second half of R1: nothing in the chain gates a service. A player who
ignores the steward can do everything the chain teaches, in any order. The steward is one
persistent person; the service never summons or copies an NPC. A check is still the service's
reading of the world, never the player's claim or the model's.

## 3. The chain

### 3.1 The giver

A new NPC, written here as **Jonah Adeyemi-Strand, arrivals steward for the Port Watch**, stands
in Port Watch Arrivals. Every character who types `c` in Limbo walks into his room. He is defined
in the chain's own pack (a same-pack giver, `docs/help/quests.md`, "Authoring a quest"), so no
station pack is re-signed. The name and the post are the owner's to change (Q-53-5).

The owner's words were "an npc you spawned in front of". The default reads that as an NPC who is
already standing there. A steward made for each player, or one who follows the player around,
would be the first NPC in the world that is not one shared person, and every NPC rule in the
quest service assumes one (Q-53-5).

### 3.2 Before the giver: the pointer

A character in Limbo has met no one. So the first thing the tracker shows is not a quest. A
character with no charter and no record in the chain gets, at connect, a **pointer** from the
service: in the tracker, "New captain? `c` steps into Ceres Freeport. The steward at Port Watch
Arrivals will show you your first run."; in line mode, that sentence once.

When that character first enters the steward's room, the service prints one authored line from
him, privately, and the tracker changes to the offer: his name, the chain's title, and
`+quest/accept Jonah`. Neither line comes from the model (Q-53-6, Q-53-18).

### 3.3 The steps

One quest a step, each with `prerequisite: completed` on the one before. "Kinds" names the
objective; **new** marks a kind §6 has to build.

| # | Quest | Where | What the service verifies | Kinds | The commands the tracker shows |
|---|---|---|---|---|---|
| 1 | The desk north of here | Registry & Co-op Desk | the player and Ilse in the same room | `contact` | `registry` |
| 2 | A ship of your own | Registry & Co-op Desk | the player's own active Co-op enrollment, charter and stake | `coop_enrolment` | `+coop`, then `+coop/enroll <ship name>` |
| 3 | Fitted to dig | Registry, then the Exchange Hall | the ship is a Dray-P; a prospecting license in force; a rig leased into her bay | **new** `ship_fit`, `licence`, `rig_lease` | `+coop/refit prospector`; `+license/buy go`; `+tv/lease go` |
| 4 | Mass and rations | Co-op Chandlery | tanks cover the plan to Ahuna Flank and home; stores cover its crew-days | **new** `provisioned` (twice: propellant, provisions) | the buy command for each, with the tonnes filled in (§5.5) |
| 5 | Cast off | Visiting Berth Gallery, then Ahuna Flank | the player's ship is down at Ahuna Flank | **new** `at_place` | `+ship/board`; `+ship/lift ahuna` |
| 6 | Read the ground | Ahuna Flank | a fresh scan, then a fresh sounding | `survey` scan, `survey` sound | `+tv/1 launch go`; `+tv/1 scan go`; `+tv/1 sound go` |
| 7 | Bring it home | Ahuna Flank, then the Exchange Hall | ore in the hold; the ship back at Ceres; a settled sale of that ore | **new** `cargo`, `at_place`, `sale` | `+tv/1 drive <pocket> go`; `+tv/1 mine metals go`; `+tv/1 recover go`; `+ship/lift ceres`; the sell command |

Notes on the table:

- **Seven definitions on one NPC** is inside the limit of eight (Q-53-19).
- **Step 2 commits the account.** The Co-op stakes each account once, and the help for Ilse's
  quest says "never enroll just to clear a journal entry". The chain does tell a new captain to
  enroll, because that is the route. The step's text says what the charter costs and that it is
  once an account, and points at `+coop` first. A character who already holds a charter passes
  the step at once.
- **Step 3 spends money**: 2,500cr for the license, 1,400cr and a 5,000cr deposit for the rig
  (`docs/playthrough.md`, Part 2). Each checkpoint's text names its price before its command.
- **Step 6 overlaps Rosalind's "Wide first, then close".** One scan can satisfy both if both are
  active; each quest holds its own receipt. Nothing is changed in her quest.
- **The buy and sell commands** in steps 4 and 7 are the by-name forms the `market-friction`
  session is building. This document assumes they land and does not spell them (§11).
- **Step 7 has a wait in it**: drilling takes a few minutes and the flight home a few more. The
  tracker shows what is being waited on, not a command (§5.2).

### 3.4 How it ends

When step 7's sale settles, the chain is complete. The steward gets the usual verified work
experience with the completion line, the player gets the one relationship bonus (Q-53-11), and
the tracker shows a closing card for the rest of that session: "First run complete." with three
pointers, each an existing thing: Dov's fittings trade at the Chandlery, Ma at the Slack Line,
and `help trade`. After that the tracker is empty unless the player tracks another quest
(Q-53-12). The four independent invitations are where a captain goes next, unchanged.

The default builds the chain in three content cuts (§7): steps 1 and 2 first, which need no new
objective kind and prove the flow end to end; then 3 to 5; then 6 and 7. The alternative is to
stop the chain for good at the first lift (Q-53-7).

## 4. The guided flow

A definition gains one optional field, `guided`, naming the chain and the step's place in it.
Everything in this section applies only where it is set.

### 4.1 Accepting

- **The first step is offered and accepted.** `+quest/accept <giver>` with no token is taken
  when the giver is present and has exactly one guided offer for the asker. The token exists so
  that a player accepts the exact terms they read, against terms that change or a model that
  misdescribes them. A guided step has fixed authored terms, costs nothing and can be paused, so
  there is nothing for the token to protect. With two or more offers, or any quest that is not
  guided, the token is required as today.
- **Each later step is accepted by the service** in the transaction that completes the one
  before. The giver need not be present. The player sees the tracker change and, in the
  scrollback, two lines: what was just done, and what is next.
- The alternative for the first step is no acceptance at all: walking into Arrivals starts the
  chain. The default keeps one explicit yes, because a journal entry the player never asked for
  is the one thing the old rule was right to refuse (Q-53-2).

### 4.2 Checks

- **The service records a guided check** on the tick where the objective's proof is current,
  wherever the player is. The proof functions are the ones `+quest/check` calls today
  (`evidence.rs:1291` for the enrollment, for one). The room rule and the command are waived for
  guided checkpoints in `transition_allowed`; every other guard stays.
- A guided checkpoint's bound room stops meaning "where you acknowledge the work" and means
  "where the work is done". It is what the tracker's pointer aims at.
- **Checkpoints stay ordered.** The service records only the next unrecorded one. If a later
  one is already true it is recorded on the following tick, so a player who does things out of
  order loses nothing.
- The service records a check only while the player is connected, which the guard already
  requires (`mod.rs:132`). A ship that lands while its captain is offline is credited at the
  next connect.
- `+quest/check` on a guided quest answers with the current step instead of an error.

### 4.3 State or fresh work

Today's trade and survey objectives demand work done after acceptance, so that old cargo cannot
be passed off as new. That matters when a quest pays for the work. A guided step pays nothing
and exists to get the player to a state: a ship, a license, full tanks. So a guided step verifies
the **state** wherever a state says the same thing (Q-53-8):

- A player who bought propellant before meeting the steward, or by a route the step did not
  name, passes. So does a veteran (§4.6).
- The tracker can show progress against a state ("tanks 6 of 18 t"), which it cannot against a
  receipt.
- A state can lapse: tanks empty on the burn, a license runs out. That does not undo a step. A
  recorded check is a receipt of what was true then, as a `contact` check already is, and each
  step finishes on the tick its last check is recorded.

The scan, the sounding and the sale stay receipts of fresh work, because there is no state that
says "has scanned".

### 4.4 Finishing

A guided step finishes when its last check is recorded, with no return to the giver (Q-53-4).
Walking back to Arrivals from the Chandlery to be told to walk to the berth is the kind of
errand the playtester's remark was about. The alternative keeps one walk back, at the very end,
to close the chain in person.

### 4.5 Stopping and starting

- **`+quest/guide off`** pauses the chain for that character: no prompts, no acceptance of a
  next step, no guided checks, an empty tracker. **`+quest/guide on`** resumes it from the step
  it stopped at, and steps that became true in between pass one after another.
- **A guided step cannot be abandoned.** `+quest/abandon` on one answers "`+quest/guide off`
  pauses the guide; nothing is lost." Abandoning is final today, and a new player who typed it
  once would lose the guide for good.
- Declining the steward's first offer is the same pause, not a permanent refusal.
- **No per-step skip** in this draft. A player who would rather trade than mine pauses the guide
  at step 3 and goes to Dov. Since steps verify states, the order a player does things in does
  not matter; only a player who never does one of them stays on that step (Q-53-9).

### 4.6 Existing characters

dev1 and dev2 were wiped on 2026-11-22, and the game has no players to migrate. The rule is
still worth stating, because test characters and staff exist:

- The pointer (§3.2) and the steward's greeting go only to a character with no active Co-op
  enrollment and no record in the chain.
- Any character can ask the steward with `+quest/offer` and accept. Steps whose states are
  already true pass on successive ticks; the scrollback gets one summary line, not seven
  announcements. A veteran with a fitted ship lands on step 6 within a few seconds (Q-53-10).

### 4.7 The bonus

One bonus for the chain, paid when step 7 finishes: the usual +10 trust and +5 disposition with
the steward. Steps 1 to 6 carry a new `bonus: false`. Seven bonuses would make the steward the
closest friend of every captain in the corridor for an hour's errands. No credits: the Co-op's
stake is the new captain's money, and a credit reward for each character would be a faucet an
account could run again with every new character (Q-53-11).

## 5. The tracker

### 5.1 One feed

The quest module registers `quest.tracker`. A frame goes to `Audience::Actor(player)`, so every
connection of that character sees the same thing. It is pushed only when its content changes, as
`tv.status` is, and answered as a keyframe when a client subscribes. It changes when:

- a quest is accepted, a check is recorded, a quest finishes, the guide is paused or resumed, or
  the tracked quest is switched;
- the player moves (the "here" mark and the route change);
- a progress figure changes (propellant aboard), read on the quest tick.

A frame holds, for the one tracked quest:

| Field | Example | Notes |
|---|---|---|
| `chain`, `step`, `of` | "First run", 4, 7 | absent for a quest outside a chain |
| `title` | "Mass and rations" | the quest's |
| `objective` | "Take on propellant for Ahuna Flank and home" | the next unrecorded checkpoint's title |
| `where` | "Co-op Chandlery, Ceres Freeport" | the checkpoint's room |
| `here` | false | the player is in that room |
| `route` | `["concourse", "chandlery"]` | exit words to type, from where the player stands; empty when `here`; absent when there is none (§5.4) |
| `command` | the buy command with 12 t filled in | the one line to type next; may end at a blank the player fills (`+coop/enroll `) |
| `progress` | `{have: 6, need: 18, unit: "t"}` | only for objectives that have one |
| `waiting` | "drilling: cycle 2 of 4" | set instead of `command` when the next thing is not the player's to do |
| `detail` | the checkpoint's full instructions | authored text; what the full view shows |
| `others` | 2 | active quests not shown |

Each checkpoint gains an authored `commands` list (up to four lines) beside its `instructions`.
The tracker's `command` is the first line whose effect the service cannot yet see; where it
cannot tell, the first line. The pointer of §3.2 and the closing card of §3.4 are frames with a
`detail` and no objective.

The tracker shows one quest: the guided chain while it is active, otherwise the most recently
accepted one. `+quest/track <giver>[=<quest-key>]` picks another. A quest that is not guided
shows its next checkpoint, its room and its `+quest/check` line, which the journal already
prints (Q-53-12).

### 5.2 The SSH TUI

**The pane.** A fourth pane, `next`, at the top of the scene screen's right rail, 24 columns
wide like its neighbors, at least six rows:

```
┌ next ────────────────┐
│First run · 4 of 7    │
│Mass and rations      │
│▸ Take on propellant  │
│  6 of 18 t           │
│at Co-op Chandlery    │
│  go: concourse       │
│> (the buy command,   │
│   12 t filled in)    │
└──────────────────────┘
```

The command wraps inside the pane; it is never cut, because a clipped command is worse than
none. The pane is the **last** in the screen's `degrade` list, after `who`: a short terminal
loses the action list and the who list before it loses the next step.

**Everywhere else.** The pane is on one screen, and the fuel purchase may be made from the
market screen. So the status line, which every screen draws, carries the short form whenever the
pane is not on screen: `next: propellant 6/18 t · Co-op Chandlery`. That is what makes the
tracker "always there" (Q-53-13).

**Selecting it.** The TUI has no mouse (§1.5), so "click" is the keyboard's version of it
(Q-53-14):

- The pane is selectable. **Enter** on it puts the step's `command` in the input box, not sent.
  A mutating command is never a bare key in this client (the interface plan's D-I-18,
  `screen.rs:68`); the player reads it and presses Enter again.
- **`g`** runs `+quest/next`, which prints the full step into the scrollback: the detail, the
  whole route, every command in order. No new popup is built.

**Below the floor.** Under 120 columns the TUI draws one sentence and the input box, and doc 21
rules that nothing is laid out for a narrower terminal. The default adds one line of text under
that sentence, the status line's short form, and nothing else: it is a line, not a layout. The
alternative is to leave the floor frame alone and let `+quest/next` do the work (Q-53-13).

**When to wait.** While `waiting` is set the pane shows it in place of the command, and Enter
does nothing. A tracker that offers a command during a five-minute flight teaches the player to
press it.

### 5.3 The web client and line mode

**Web.** A card at the top of the left aside, above the who list, drawn when the session has the
`quest.tracker` panel. It shows the same fields as the pane. A click opens the card to the full
detail and every command; a button beside each command puts it in the composer, as a notice's
action already does (`NotificationTray.svelte`). Route words are buttons that send the move.
Where the aside is hidden on a narrow window, the card becomes one line above the composer that
opens the same view (§11: the web's breakpoints were not read).

**Line mode.** It has no panes, and it is where an SSH player starts (§1.5). It gets
(Q-53-15):

- two lines in the scrollback when the step changes: what was done, then the next objective,
  where, and the command;
- `+quest/next` at any time, for the full step;
- the pointer of §3.2 at first connect;
- once for a character, with the first step: "This is shown beside the scrollback in `@tui`
  (120 columns or wider) and in the web client."

No line is repeated on a timer. A player who is lost types `+quest/next`.

### 5.4 Where is it

A new bounded search in the quest module: breadth-first over live exits from the player's room
to the checkpoint's room.

- It follows only exits the player could take now: not dark, lock passing.
- It stays inside the port the player is standing in, by the port table in
  `crates/tacsim/src/ic_return.rs`, and never through Limbo's global exits.
- It stops at 12 steps or 256 rooms. Ceres has thirteen rooms.
- It yields, for each step, the exit's first short word (`registry`, not `Registry <N>`).

The pane shows the first word; `+quest/next` shows them all ("go: concourse, chandlery").

When the search finds nothing, because the place is in another port or the player is aboard a
ship or in the OOC area, the tracker falls back to the checkpoint's authored `where` words and
the port's name: "Co-op Chandlery, Ceres Freeport (you are at Ahuna Flank)". Getting between
ports is what the step's own commands are for. The alternative is authored directions only,
which is content and no engine work, and is wrong whenever the player is not where the author
assumed (Q-53-16).

### 5.5 Quantities

The playtester stopped where the game first asks for a number it does not supply. So the
`provisioned` objective and the tracker share one helper: the ship's plan to the named place and
home, plus a margin, less what is aboard. It is the arithmetic behind `+ship/plan`, called, not
copied. The tracker's command for step 4 is the complete buy line with that many tonnes, and its
progress is "6 of 18 t". The margin is a figure in the step's definition; 10% is the starting
value and is provisional (Q-53-17).

The alternative is authored text: "buy a little more than `+ship/plan ahuna` says, for both
legs". That is what `docs/playthrough.md` says now, and where the playtest stopped.

### 5.6 The model

- **The flow never calls the model.** The steward's greeting, each step's text, the two lines on
  a step change and every word on the tracker are authored in the pack. With the model off the
  chain is complete.
- **With the model on**, the steward is an NPC like any other and can be talked to. The existing
  `quest.status` ability (`commands.rs:664-720`) gives him the asker's verified progress; it
  gains the current step's detail and command, so that "what do I do now?" is answered from the
  pack and he does not invent a command.
- The model never advances a step, never writes tracker text, and the steward says nothing
  unprompted beyond his authored lines. A model-written line on each step would cost a call for
  every step of every new player and would differ from the tracker beside it (Q-53-18).

## 6. Engine or content

| Piece | Engine | Content |
|---|---|---|
| One quest unlocking the next, one giver | built (`Prerequisite::Completed`) | the `prerequisite` lines |
| The steward, his room, his lines | | the pack: an `npc` entry and seven `quest` entries |
| Steps 1, 2 and 6 | built (`contact`, `coop_enrolment`, `survey`) | their text |
| `guided`, `bonus`, `commands` on a definition | a new definition schema version; `QuestDef` refuses unknown fields (`model.rs:35`) | |
| Tokenless accept, service acceptance of the next step | new | |
| Service-recorded checks, finish without the giver | new: the tick and three guards in `transition_allowed` | |
| `+quest/guide`, `+quest/next`, `+quest/track`; the refusal on abandon | new, with help text | `docs/help/quests.md` |
| The pointer and the greeting | new: a connect hook and a `Moved` observer | their words |
| `quest.tracker` | new | |
| The route search | new | the fallback `where` words |
| `ship_fit`, `licence`, `rig_lease`, `provisioned`, `at_place`, `cargo` | new: six state proofs | |
| `sale` | new: one leg of the `trade` receipt the book already keeps (`evidence.rs:89-102`) | |
| The TUI pane, the status line's short form, the floor line | new | |
| The web card | new | |
| Quest Studio fields for all of the above | new | |

Accepted work keeps the definition it was accepted under, so a record made before the schema
change reads as it did.

## 7. Units of work

Prefix GS. One pull request each. S is under a day's session, M one to two. Nothing is cut until
§9 is ruled; a ruling against a default removes or reshapes the units named beside it.

| Unit | What | Crates | Size | Needs |
|---|---|---|---|---|
| **GS.1 guided definitions and acceptance** | definition schema 2 with `guided`, `bonus`, checkpoint `commands`; `+quest/accept <giver>` with no token for a single guided offer; the service accepts the next step when its prerequisite completes; `+quest/guide on\|off`; abandon and decline on a guided quest pause instead | tacsim | M | Q-53-1, -2, -9 |
| **GS.2 service-recorded checks** | the tick records a guided checkpoint when its proof is current; the room rule and the giver-present rule waived for guided rows in `transition_allowed`; finish on the last check; one bonus a chain; proven with `contact` and `coop_enrolment` | tacsim | M | GS.1; Q-53-3, -4, -11 |
| **GS.3 the step in words** | `+quest/next`; the two lines on a step change; the connect pointer and the greeting on entering the giver's room; the once-only `@tui` line; `quest.status` carries the step's detail and command; help | tacsim, docs/help | M | GS.2; Q-53-6, -15, -18 |
| **GS.4 the route** | the bounded exit search and its fallback; in `+quest/next` | tacsim | S | GS.3; Q-53-16 |
| **GS.5 the feed** | `quest.tracker`: the frame, push on change, the subscribe answer, `+quest/track`; a frame for a quest that is not guided | tacsim | M | GS.3; Q-53-12 |
| **GS.6 the TUI pane** | the `next` pane on the scene screen, last to degrade; Enter fills the input box, `g` runs `+quest/next`; the status line's short form on every screen; the floor line | mush-net-ssh | M | GS.5; Q-53-13, -14 |
| **GS.7 the web card** | the card in the aside, the opened view, the fill-the-composer buttons, the narrow form; the feed in `hydrationQueries` | web | M | GS.5; Q-53-14 |
| **GS.C1 content: the steward and steps 1 and 2** | pack `tacsim.quests.first_run@1`: the NPC in Arrivals, two quests, every authored line. **Gate: a rig that types only what the tracker's `command` says goes from Limbo to a chartered ship** | tacsim content, tests | S | GS.2, GS.3; Q-53-5, -19 |
| **GS.8 states I** | `ship_fit`, `licence`, `rig_lease` | tacsim | M | GS.2; Q-53-8 |
| **GS.9 states II and quantities** | `provisioned` with the shared plan helper, `at_place`, and `progress` in the frame | tacsim | M | GS.5; Q-53-8, -17 |
| **GS.C2 content: steps 3 to 5** | pack v2. **Gate: the same tracker-only rig reaches Ahuna Flank** | tacsim content, tests | S | GS.8, GS.9, the `market-friction` buy form |
| **GS.10 cargo and the sale** | `cargo`; `sale` as one leg of the trade book; `waiting` in the frame | tacsim | M | GS.9 |
| **GS.C3 content: steps 6 and 7, the close** | pack v3; the closing card. **Gate: the tracker-only rig sells its ore** | tacsim content, tests | S | GS.10; Q-53-7 |
| **GS.11 the Quest Studio** | the staff editor reads and writes `guided`, `bonus`, `commands` and the seven new kinds | web, tacsim | M | GS.10 |
| **GS.12 the playthrough** | `docs/playthrough.md` Parts 1 and 2 rewritten around the tracker | docs | S | GS.C3; Q-53-20 |

**Order.** GS.1, GS.2, GS.3 and GS.C1 in a row put a working guided chain of two steps on a dev
box in line mode, with no client change. GS.4 to GS.7 are the tracker and can run beside GS.8 to
GS.10, since they touch different crates. The three content cuts are each small and each end in
the same test, grown by a few steps.

**The gate is the design's test.** A rig that reads the `command` field and types it, and does
nothing else, must reach the end of each content cut. If it cannot, the tracker did not say what
to do, which is the playtester's complaint as a failing test.

**What waits on other sessions.** GS.C1 adds an NPC to a Ceres room while `ceres-doors` and
`ooc-ic` are re-signing Ceres v4 and changing Limbo's exits; it lands after both. GS.C2 needs the
by-name buy from `market-friction`.

## 8. What this does not do

- **No change to the four independent invitations** or the four follow-up packs.
- **No steward in the OOC area**, and no change to `START_ROOM`. A box that starts characters
  somewhere other than Limbo needs only different pointer words.
- **No marker in the walk view** (the picture of the station on screen 1) pointing at the door.
  The route is words. A marker is a later unit on doc 21's side.
- **No second chain.** A trade chain, a passenger chain or a law chain would each be content on
  this machinery; none is designed here.
- **No guided quests for ports other than Ceres.** The route search is per port, and nothing in
  it names Ceres.
- **No per-step skip, no credits, no timers.**

## 9. Owner survey

Every question has a proposed default, an alternative and a reason. **(expensive to reverse)**
marks an answer that would take a significant refactor to undo.

**Q-53-1 — May one starter quest unlock the next?** Reverses R1.
- **Proposed default:** yes, in the new chain only. `first_steps` keeps its brief and its four
  independent invitations. No step gates a service.
- **Alternative:** keep every starter quest independent; the tracker suggests an order among
  them and nothing is chained.
- **Reason:** the playtester's problem was not knowing what was next. Independent invitations
  cannot say. The engine already chains quests; only the pack's brief forbade it.

**Q-53-2 — How is a guided step accepted?** Reverses R2 and part of R3 and R5.
- **Proposed default:** the first step with `+quest/accept <giver>`, no token, giver present;
  each later step by the service on completion of the one before.
- **Alternative A:** no acceptance at all: entering Arrivals starts the chain.
- **Alternative B:** tokens kept; the tracker prints the token line to copy at every step.
- **Reason:** the token guards terms that can change and cost something. A guided step's cannot
  and does not. One explicit yes keeps a journal the player asked for.

**Q-53-3 — Who records a guided check?** Reverses R3 and R4. **(expensive to reverse)**
- **Proposed default:** the service, on the tick it sees the proof, wherever the player is.
- **Alternative:** the room rule waived but the command kept: the player types `+quest/check`
  when the tracker says the work is seen.
- **Reason:** the owner's sentence is "when that quest was completed you got a quest to go buy
  propellant". A command that tells the game what the game already knows is the database feel.
  It is marked expensive because the chain's content, the tracker and the tracker-only test are
  all written to it.

**Q-53-4 — Does a guided step need a walk back to the giver?** Reverses R5.
- **Proposed default:** no; it finishes on its last check.
- **Alternative:** one walk back, at the end of the chain, as an eighth step.
- **Reason:** §4.4. The alternative is cheap to add later as content: a `contact` step.

**Q-53-5 — Who is the giver, and where?**
- **Proposed default:** a new NPC in the chain's own pack, the Port Watch's arrivals steward,
  in Port Watch Arrivals; one persistent person. The name in §3.1 is a placeholder.
- **Alternative A:** Ilse Oduya-Brandt at the Registry, one room north: no new NPC, and the
  pointer says `registry`. She would then hold her own quest and seven more, over the limit of
  eight unless hers is retired, which the service refuses for a live source.
- **Alternative B:** a steward made for each player, or one who walks with them.
- **Reason:** Arrivals is the one room every new character enters and it is empty. B breaks the
  rule that an NPC is one shared person, for the sake of a walk of zero rooms.

**Q-53-6 — What does a new character see in Limbo?**
- **Proposed default:** the service's pointer (§3.2), in the tracker and as one line.
- **Alternative:** nothing until the character reaches Arrivals; the login banner says `c`.
- **Reason:** the tracker should never be empty for someone who has done nothing yet.

**Q-53-7 — How far does the chain go?**
- **Proposed default:** seven steps, to the first ore sale, in three content cuts.
- **Alternative:** five steps, ending at the first lift; mining is left to `help mining` and
  Rosalind.
- **Reason:** the first sale is the first time the game pays the player back, and the mining run
  is the 45-minute loop the playthrough was built around. The alternative saves GS.10 and GS.C3.

**Q-53-8 — Does a guided step verify a state or fresh work?**
- **Proposed default:** a state wherever one says it; receipts for the scan, sounding and sale.
- **Alternative:** fresh work after acceptance throughout, as `trade` does today: a `purchase`
  objective instead of `provisioned`.
- **Reason:** §4.3. The alternative fails a player who fueled before meeting the steward, and
  cannot show progress.

**Q-53-9 — How does a player stop the guide?** Reverses R6.
- **Proposed default:** `+quest/guide off` and `on`; abandon and decline pause; no step skip.
- **Alternative:** add `+quest/guide skip`, which closes the current step unrewarded and offers
  the next.
- **Reason:** a pause loses nothing and cannot be regretted. A skip needs a third way for a
  quest to close and a prerequisite that accepts it; it can be added if playtests show players
  parked on step 3.

**Q-53-10 — What do existing characters get?**
- **Proposed default:** no prompt for a character with a charter or a chain record; anyone may
  ask the steward; true steps pass at once with one summary line.
- **Alternative:** every existing character gets the pointer once.
- **Reason:** a veteran prompted to register a ship is noise; one who wants the tracker can have
  it in a sentence.

**Q-53-11 — What does the chain pay?** Reverses R7.
- **Proposed default:** one relationship bonus at the end; nothing a step; no credits.
- **Alternative:** a small credit grant at the first lift or the first sale, paid by the Co-op.
- **Reason:** §4.7. A figure for the alternative would be provisional and tunable; the faucet
  across new characters is the part that needs a rule before any figure.

**Q-53-12 — Which quests does the tracker show?**
- **Proposed default:** any active quest, one at a time, the guided chain first; `+quest/track`.
- **Alternative:** guided quests only.
- **Reason:** the feed and the panes cost the same either way, and Ma's and Rosalind's quests
  have a next checkpoint and a room to show.

**Q-53-13 — Where is the tracker in the SSH TUI?**
- **Proposed default:** top of the scene screen's right rail, last to degrade; the status line's
  short form on every screen where the pane is not drawn; one line on the under-120 frame.
- **Alternative A:** a pane on every screen. Each screen's layout is fitted to 120 columns and
  would lose 24 of them.
- **Alternative B:** nothing below the floor.
- **Reason:** §5.2. The status line is the only thing on every screen with room for a clause.

**Q-53-14 — What does selecting a step do?**
- **Proposed default:** TUI: Enter fills the input box with the command, `g` prints the full
  step. Web: a click opens the step; a button fills the composer.
- **Alternative:** a popup over the screen in the TUI, like the help.
- **Reason:** putting the command in the input box is the most useful thing a click can do for
  someone who is stuck, and it needs no new overlay. Mouse input in the TUI is a separate piece
  of work that this does not ask for.

**Q-53-15 — What does line mode get?**
- **Proposed default:** §5.3's four things, and no repeats.
- **Alternative:** also tell every new SSH character at login to type `@tui`.
- **Reason:** line mode is the default and the playtester was likely in it. The alternative
  changes the login for everyone and is a client decision wider than quests.

**Q-53-16 — How is "where is it" answered?**
- **Proposed default:** the bounded route search over live exits inside the port, with authored
  words as the fallback.
- **Alternative:** authored directions only.
- **Reason:** §5.4.

**Q-53-17 — Does the step fill in quantities?**
- **Proposed default:** yes, for propellant and provisions, from the ship's plan plus a margin
  (10%, provisional).
- **Alternative:** authored text that points at `+ship/plan`.
- **Reason:** §5.5. This is the step the playtest stopped at.

**Q-53-18 — What is the model's part?**
- **Proposed default:** none in the flow. Authored text everywhere. `quest.status` carries the
  step so the steward can explain it when asked.
- **Alternative:** the steward speaks a model-written line on each step, with the authored line
  as the fallback.
- **Reason:** §5.6. The service must work with the model off, as today, and two voices saying
  slightly different things about the same step is worse than one.

**Q-53-19 — One giver, or desk to desk?**
- **Proposed default:** the steward gives all seven; Ilse, Dov and Rosalind are met on the way.
- **Alternative:** each desk hands the player on: Ilse gives the charter step, Dov the
  propellant, Rosalind the rig. It needs a prerequisite that reads another giver's quest, which
  the service does not do today (§1.1), and spreads seven bonuses over four people.
- **Reason:** one name on the tracker, one relationship, and no engine change for the chaining.

**Q-53-20 — What happens to `docs/playthrough.md`?**
- **Proposed default:** as the content cuts land, Parts 1 and 2 become "accept the steward's
  offer and follow the tracker", with the command lists kept below as reference. The web's
  guide view shows that file, so it follows.
- **Alternative:** leave it as the full typed walkthrough.
- **Reason:** two guides that disagree is one more thing for a new player to reconcile.

## 10. Risks

- **A wrong command on the tracker is worse than none.** The tracker-only rig in each content
  cut's gate is the guard. It has to run on every change to a command the chain names, which
  means the market's and the ship's suites gain a dependent.
- **The chain goes stale when a command changes.** The pack's `commands` are text. The help
  lint checks that documented commands exist; the same lint should read a guided pack's
  `commands`. That is part of GS.C1.
- **The quest tick does more.** Guided checks add a few state reads for each active guided row
  on each tick. The rows are one a new character, and a state proof is a lookup. GS.2 measures
  it rather than assuming.
- **Step 2 is irreversible for the account.** §3.3. The text has to say so before the command.
- **A tracker that nags.** Nothing here repeats on a timer or blocks input, and `+quest/guide
  off` empties it. The under-120 line and the status line are the two places it shows without
  being asked for; either can be dropped by its question.

## 11. What this did not check

- **No numbered owner ruling** was found for R1 to R7. They are cited from the pack's brief, the
  help topic and the code. If a design document rules them, its number belongs in §2.
- **The `market-friction` buy and sell forms** were not read; steps 4 and 7 assume a by-name
  form and do not spell it.
- **The plan helper.** §5.5 assumes the arithmetic behind `+ship/plan` can be called for a round
  trip from the quest module. The function was not read.
- **State proofs for the license, the rig lease, the fit, the landing and the cargo** are each
  assumed to be a short read of existing state. `licence_standing` exists
  (`crates/tacsim/src/port/licence.rs:437`); the others were not located.
- **The status line's free width** at 120 columns was not measured, nor what it shows today.
- **The web client's narrow layout** and its breakpoints were not read.
- **The tick's cadence** and the cost of a guided check were not measured.
- **Whether a same-pack NPC can be placed in another pack's room** without touching that pack
  was taken from the help topic's authoring section, not tried.
- **The onboarding gate's interaction** with the connect pointer, on a box where it is on, was
  reasoned from its module comment only.
- No playtester was asked which client they used.
