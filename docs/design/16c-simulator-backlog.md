# 16c — Simulator backlog

Draft 0.1, 2026-09-20, for `main` at `820ad9af` (tree clean, deployed to dev1). Companion to
[doc 16](16-combat-arena.md) (the specification; §11 holds the as-built amendments) and
[16b](16b-combat-implementation-plan.md) (the plan that was built; §9 is the cut ledger). Every cut 16b
planned has landed: R to N, X.1 to X.6, F, Q, and the networking fix `820ad9af`.

**What this is.** The list the owner asked for on 2026-09-20: the outstanding bugs, the tech debt, and
the new functionality for the simulator, followed by the order in which they are built. It is executed
from the top of §4, with a commit and a push at each sub-cut and a ledger row in 16b §9 per cut.

**How it was built.** Four triage passes re-checked every open finding of the 2026-09-18 readiness
review (161 findings), every residual the cut F, X.6 and Q judges left, and the platform and operations
seeds against `820ad9af` by reading the code. Four hunts read the cut Q surfaces afresh (cockpit and
epistemics, the panel clients, the reflex AI and the lobby, the arena debts). One live smoke played eight
matches on the real `mushd` binary over SSH line mode, the websocket and the TUI. Two lenses proposed
features. Every bug and debt below was then handed to a verifier that tried to refute it; the priorities
here are the verifiers' corrected ones. Where several lenses found one defect it appears once. Where the
verifiers disagreed on a priority the higher confirmed one is used, because dropping a P1 costs more than
scheduling a P2 early.

**How to read it.** §1 and §2 are tables; an id (`B-nn`, `T-nn`) is stable and is what §4 cites. "Verified"
means a verifier confirmed it by reading the code at `820ad9af`; "plausible" means the symptom or the
code fact is confirmed and the scenario or the cause is not. "Live" means the smoke saw it on the running
server. Sizes: S is under about 150 lines with tests, M under about 600, L under about 2,000, XL more.
File paths are relative to `crates/` unless they start with `docs/`, `web/`, `deploy/`, `scripts/` or
`.github/`; `tacsim/src/` is abbreviated to `tacsim:` and `tacsim/src/arena/` to `arena:`.

**The rubric.** P0: a match cannot complete, a player is locked out, a record is corrupted, or hidden
information leaks to a player. P1: a wrong outcome or a confusing dead end in ordinary play (two humans,
or one human against the AI fill, on the shipped map, over line mode, the TUI or the web client). P2: an
edge case, a rare path or a staff-only surface. Debt: no player-visible defect.

**Counts.** 73 bugs (0 P0, 14 P1, 59 P2); 80 debts; 18 ranked features and 6 held for a design
conversation; 12 lettered cuts and an unlettered tail of 5. Since cut D's record (2026-09-21): 75 bugs
(`B-74` and `B-75` added, both P2) and 86 debts (`T-81` to `T-86` added), of which cut D closed 25 bugs
and 9 debts outright and one bug (`B-12`) and two debts (`T-59`, `T-60`) in part; a closed row keeps its
text and says where it landed in its id cell. After the owner-supplied external review of 2026-09-22: 80 bugs (`B-76` to `B-80`
added, all P2, each confirmed and narrowed by two lenses reading `c3852a0c`; the reviewer's probes were not
re-run), 86 debts unchanged, and a thirteenth cut, AA, placed after E and before B. Cut L's source review
at `8366b3fe` adds `B-81` (P2), bringing the total to 81 bugs (0 P0, 14 P1, 67 P2); no runtime probe
was run for that finding. The 2026-09-26 dev1 deploy recon adds `B-82` (P1, the split's world task never
holds the LLM service; read at `50397c1b` and seen live in dev1's journal), bringing the total to 82 bugs (0 P0,
15 P1, 67 P2), and the owner's fix cut AC is placed after I; cut AC's record adds `B-83` (P2, a `BOARD` lost on the split at the walk's commit window) and `T-89` (no per-feature LLM budget caps on the features dev1 now spends on), 83 bugs and 89 debts; cut AE's sweep adds `B-84` (P1, the reflex controller's two path-search wastes), 84 bugs (0 P0, 16 P1, 68 P2), and its fix cut AI is placed after AF. Cut E's record
(2026-09-22) closed ten bugs (`B-04`, `B-05`, `B-39` to `B-46`) and six debts (`T-32`, `T-46`, `T-47`,
`T-55`, `T-56`, `T-81`) outright and one debt (`T-42`) in part; it added no row, its residuals being listed
in 16b section 9, row E, and its deferrals in section 4's cut E entry. Cut AA's record (2026-09-22) closed
its three bugs (`B-76`, `B-78`, `B-79`) and added one owner question (`Q41`, the default AA.3 was built
on); its residuals are in 16b section 9, row AA, and its deferrals in section 4's cut AA entry.
Cut L closes five bugs and seventeen debts outright, and corrects T-54's
comment while leaving its per-channel leases open. Its rows retain the historical
problem, location and proposed payment columns; validation and integration are
recorded in the Cut L entry and PR #1.
Cut B's record (2026-09-23) closed nineteen bugs (`B-08`, `B-09`, `B-10`, `B-13`, `B-14`, `B-23`, `B-24`,
`B-26`, `B-38`, `B-59` to `B-67`, `B-80`) and ten debts (`T-33`, `T-43`, `T-44`, `T-58`, `T-61` to `T-65`, `T-86`)
outright, `T-60`'s two carried slips, and two debts (`T-31`, `T-42`) in part; it added no row, its residuals
being listed in 16b section 9, row B, and its deferrals in section 4's cut B entry.
Cut M's record (2026-09-23) closed fourteen bugs (`B-27`, `B-47` to `B-57`, `B-74`, `B-77`) and twenty-four
debts (`T-01` to `T-07`, `T-09` to `T-17`, `T-23`, `T-26` to `T-28`, `T-80`, `T-82` to `T-84`) outright, `T-42`
in part (the split twin, the driven pose, `@domain/retire`), and reopened and re-closed `B-44`; it added no
row, its residuals being listed in 16b section 9, row M, and its deferrals in section 4's cut M entry.

No P0 survives at `820ad9af`. The one leak-shaped item is `B-01`, which the verifier graded P1 because
what leaks is lobby roster state and not a position; it is first in cut D all the same. The live smoke
scanned 3,216 cockpit frames and found nothing about an undetected unit on any of them, and every unheld
designator (including a live contact's own sixteen-digit id read off the web payload) answers the one
byte-identical `No such contact.`

---

## 1. Bugs

### 1.1 P1

| Id | What happens, to whom | Where | Fix | Size | Status |
|---|---|---|---|---|---|
| B-01 — landed D.1 | A seated pilot mid-match types `SIM STATUS` and reads the other side's whole roster with `, seated` / `, left` on each row: they learn that the human on `RA` has dropped and a machine has the tank, which no sensor told them. Cut Q.8 bracketed the chassis row to `Selection` for exactly this reason and left the rows unbracketed. | `tacsim:match_/lobby.rs:2014-2074`, reach through `sim.rs:172` | During `Running` print the caller's own side in full and the other side as callsigns alone; the whole board in `Selection`, in the terminal states and on `@arena/status`. | S | verified |
| B-02 — landed D.1 | A pilot whose turret is gone reads `cannon ready 40 of 40 round(s)` on `WEAPONSTATUS`, types `FIRE`, is told `Firing cannon.` and 45 ms later `FIRE refused. That unit carries no weapon called cannon.` An empty frame weapon list falls back to the pre-match world record. Three of six fought smoke matches hit it. | `tacsim:match_/cockpit.rs:2944-2949` (stale doc at `:1071-1078`, `:2938-2943`) | Fall back to `weapons_of` only when there is no frame at all; a frame with no weapons means the unit carries none. One cockpit test on `weapons = []`. | S | verified, live |
| B-03 — landed D.2 | A destroyed pilot in a match that is still running is told `Power not online: STARTUP first.`, types `STARTUP`, reads `Startup sequence begun` and nothing ever follows. Orders the arena refuses in pulse phase 2 (`Wreck`, `MotionDisabled`, `QueueFull`) are dropped without a word; only phase 4 stages `FrameNotice::Refused`. Nothing says the options are `EXIT`, `SIM ABORT` or wait. | `arena:step.rs:484-489` against `:509-518`; `arena:intents.rs:409-411`, `:437-439`; `tacsim:match_/cockpit.rs:1605-1667`; `docs/help/cockpit.md:171-176` | Stage `FrameNotice::Refused` for a seated unit's phase-2 refusals (`refusal_line` already has the sentences); local pre-checks off the frame for a wreck and for disabled motion; one guidance line on the pilot's own `UnitDestroyed`; fix the help paragraph. | M | verified, live |
| B-04 — landed E.4 | With the human dead, a 1 v 2 machine endgame went `quiet` for five minutes and had not ended at arena time 533 s; the only exits are `SIM ABORT` (no result) or the one-hour cap. A 1 v 1 machine endgame took 7 m 42 s. The cause was not observed: a rallied hurt unit patrols and fights on (`arena:reflex.rs:930-931`, `:1121-1126`), so the recorded rally seed does not explain it, and the order seed needs deliberation, which was off. No staff view of the machines existed to see more. | `arena:reflex.rs:1108-1127` and the patrol goal choice | A harness case first: one hurt unit against two healthy ones, no humans, must end by elimination inside a stated number of arena minutes. Then the fix the case points at; if it points at patrol coverage, `Patrol` sweeps the other entry zone when nothing has been held for N seconds. Pairs with `B-33`. | M | plausible, live |
| B-05 — landed E.1 | A machine that holds a contact with no line sets its stand-off goal at 70 per cent of the platform's longest outer band (3 km, so 2.1 km) measured from the plot along the ray toward itself: up to 210 cells behind it on a 256-cell map. Where that cell is on the window and passable it drives away from a tank two cells off, loses the contact, patrols back and repeats. An off-window fallback masks it on much of the map. | `arena:reflex.rs:1458-1487`, `:1976-1980`; `arena:weapons.rs:236-242`; `tacsim/content/weapons/cannon_105.json` | `band_hold_cm` becomes the least of the doctrine band, the order's reach and the current plot range, priced off the weapon a bare `FIRE` fires; `stand_off_cell` never returns a cell further from the plot than the unit already stands. A shipped-scale reflex test with two stock tanks on the proving ground. Closes `T-47`. | M | verified |
| B-06 — landed D.1 | After `STARTUP` every motion and fire verb still answers `Power not online: STARTUP first.` until the next frame is published, which at the `Quiet` tempo a match opens in is up to 5 s. A second `STARTUP` with power online answers `Startup sequence begun; power comes online shortly.` and the pilot waits for a `Power online.` that never comes. | `tacsim:match_/cockpit.rs:441-462`, `:1605-1614`; `arena:intents.rs:508-510` | `startup` reads the mirror first (online: `Power is already online.`; starting: the hold-on line) and stamps a `Starting` hint on the registry mirror the bridge already owns, which the next frame confirms or clears. | S | verified, live |
| B-07 — landed D.2 | On the web cockpit a refused `FIRE` prints `refused` and a stopped tank prints `held`, with no order, weapon, reason or cause. The frame's notice object publishes five keys and the prose lives only in `text`, which a panel client correctly never reads. `TooSteep` exists so that a pilot drives round; the web cannot tell it from a wall. | `tacsim:match_/cockpit.rs:2359-2370`; `web/src/lib/cockpit.ts:265-276` | Add `cause`, `refusal`, `order` and `weapon` to the notice object (`BlockedCause::as_str` at `arena:motion.rs:225-235` and `Refusal::as_str` at `arena:intents.rs:267-288` exist and are unused here); extend `CockpitNotice` and `noticeLine`; re-bless the parity fixtures. | S–M | verified |
| B-08 — landed B.1 | On the web every order (the contact click, the scan button, `k` `f` `h` `m` `s`, the `Enter type` chip) switches to the scene screen, because the one `Composer` is mounted there; nothing returns the pilot to screen 0, and the composer swallows the bare `0`. One ejection per order for the whole fight. The behaviour is designed, documented and test-pinned, so the fix changes `web/src/lib/game.test.ts:1286-1299` and `docs/help/cockpit.md:195-198`. | `web/src/lib/game.svelte.ts:1086-1091`; `web/src/components/SceneScreen.svelte:18-25`; `web/src/lib/stores/screens.svelte.ts:195-200` | Mount the composer in the shell beside the legend so every screen has it, and stop `focusComposer` switching screens (the TUI's shape). | M | verified |
| B-09 — landed B.1 | At the web cockpit's default 128-cell view (and at 256) no marker carries its designator: `cellPx` is 4 against a label gate of 6, so two unlocked hostiles are two identical red dots and the pilot matches them to the legend by position. The lock ring is drawn at every zoom. | `web/src/components/CockpitScreen.svelte:61-64`, `:132-137`; `web/src/lib/cockpit.ts:33`, `:36` | Open at 64 cells and draw the label at every zoom for the lock and for hostiles, with a smaller font and a one-pixel halo below 6 px. | S | verified |
| B-10 — landed B.3; the eighty-column layout superseded — landed AB.1 | The TUI cockpit's three columns want 108 columns. At 80 it drops `damage`, `weapons` and `contacts`, and `contacts` is the only selectable pane; at 120 its rows are cut at the pane edge (a scan row loses two sections, a contact row its age), because `draw_pane` neither wraps nor scrolls. The smoke's 80×24 pty showed read-verb output wrapped into a 21-column notices pane. | `mush-net-ssh/src/tui/screen.rs:854-923`, `:993-1003`; `tui/app.rs:2284-2330`, `:2521-2526` | Wrap in `draw_pane` for the cockpit panes (the 76-column wrapper exists in `scan_lines` and `damage_lines`), narrow the side panes' `min_w`, and move `contacts` to the end of the degrade order. New 80×24 and 100×30 goldens. | M | verified |
| B-11 — landed D.5 | Five shipped exits carry the alias `l`, and dispatch matches exits (step 5) before the command table (step 6). In the Tutorial: Building, the Welcome Hall, the Bus Stop, the staff Forecourt and the Breakroom, `l` walks the player through a door instead of looking. The Welcome Hall is the room every pilot crosses to the simulator. | `mush-atlas/content/neighborhood.ooc_area.json:456`, `:470`, `:485`; `neighborhood.staff_lobby.json:553`, `:573`; `mush-core/src/cmd.rs:19-24` | Drop the `l` segment from the five names (keep `<L>`, the word and the compass words); a pack-apply check that refuses an exit segment equal to a registered command name or alias. `mush-atlas/tests/lot.rs:330-331`, `:361-362` and `mushd/tests/persistence.rs:1917` pin the old behaviour and change with it. Two pack versions. | S | verified, live |
| B-12 — room details landed D.5; details on items stay open | Every pack-authored room detail (about 36 keywords in 29 blocks) is written to `atlas:detail:<kw>` while `look <keyword>` reads `detail:<kw>`, so none can be read. The Welcome Hall's sign, which tells a new player how to reach the simulator, answers `NoMatch`. Multi-word keywords are stored with underscores, so the namespace alone is not the whole fix. | `mush-atlas/src/apply.rs:1034-1042`; `mush-core/src/commands/look.rs:70-84`; `mush-atlas/src/module.rs:55-56`, `:308` | `look` tries `detail:` and then each detail prefix a module registered at composition, with spaces folded to underscores; the atlas registers `atlas:detail:`. A test that reads a shipped detail through `look`. | S–M | verified |
| B-13 — landed B.5 | A line that arrives while another command's commit is in flight is refused with `The world is committing another action; please retry in a moment.` Four web lines sent in one burst lost three. `must_defer_behind_commit` covers four connection messages and no command, by design (head-of-line blocking); every test rig in the tree retries the string and a player has no such loop. Bursts of cockpit verbs inside a match were all served, so the committing commands (moves, claims) open the window. | `mush-server/src/world_task.rs:954-956`, `:979`, `:1030-1040`, `:1093-1099` | Measure how often a command meets the window; then a bounded per-session defer queue (depth 4, 250 ms) replayed after the receipt, refusing only on timeout and in a player's words. | M | verified, live |
| B-14 — landed B.4 | Attaching a second connection to a character prints the whole room on the first connection too: the attach dispatches a real `look` as the actor, and its output goes to every connection of that actor. A pilot who opens the browser cockpit beside an SSH window gets a room description in the fight's scrollback. | `mush-gateway/src/bridge_common.rs:955-962` (the per-connection half at `:930-938` is correct) | Address the attach-time room view to the attaching connection only, the way `SessionInfo` already is. | S | verified, live |
| B-82 — landed AC.1 | On the split, production shape (`mush-gateway` supervising `mush-server`) the world task is never handed the LLM service the boot built: every tacsim deliberation logs `WARN tacsim Effect::ModuleAsync: no LlmService on this world task; the squad stays on doctrine` and every NPC turn `npc Effect::ModuleAsync dropped: no LlmService on this world task`, so LLM commanders and NPC minds never fire in production, while `llm service built enabled=true providers=2 routes=12` is logged a few lines earlier by the same boot. The all-in-one `mushd` (the local rig) sets the handle, so no test and no local play meets it. Live on dev1 since the fork (M16b, `89a05672`), on every deliberation from 2026-09-21; found in the 2026-09-26 deploy recon. | `mush-server/src/lib.rs:334-346` (`ServerDeps::in_memory()`, then store, guard, staff, quarantine_spool and outbox_adapters overwritten; `llm` left `None`; `metrics` the throwaway registry) against `mushd/src/main.rs:429-440` (`llm: Some(Arc::clone(&booted.llm))`); `mush-server/src/world_task.rs:1810`, `:9951`, `:9982`, `:10048` | One production `ServerDeps` constructor shared by both binaries (WS04's rule: the split delegates, never transcribes), carrying the booted `llm` and the one metrics registry the sink and the world loop share; a parity test in `mush-server/tests/boot_parity.rs` that the split's deps carry the handle; a split-shape proof over the two-process rig with `[llm]` on and the mock provider that a tacsim deliberation reaches a provider and the WARN never prints. | S | Verified by reading at `50397c1b`; live on dev1. Cut AC. |
| B-84 — landed AI.1 | The reflex controller wastes its wake on two path searches that go nowhere. (1) Orbiting: a follower steers at an adjacent route cell inside its own turning circle, so its heading falls about thirty degrees a decide tick at 5 to 8 m/s, and the `follow` wandered-off branch (cut Q.7) clears the route and sets `searched_ms = None`, so a 7,000 to 16,000-cell search runs every 1,000 ms for as long as the orbit lasts. (2) The corner cut: a unit stopped `impassable` on a corner cut (154 of the harness cohort's 160 stops) has its route cleared by `note_blocked`, and `path_with`'s shoulder rule skips impassable shoulders, so the identical 13,667-cell route is searched again every `repath_ms` without the unit moving. In play a wingman circles instead of arriving and a tank sits on a corner; on the harness the authored lane's per-wake p99 tracks `path_cells` (r 0.77) and reads 8 to 17 ms by seed (2350: 11.6; 2351: 16.0 with a per-window p99 of 27.4 past the 20 ms lens; 2353: 13.3 before cut AE), so the 12 ms per-wake lens holds at the harness seed by chance. Found by cut AE's sweep repairer (2026-09-28) after B-58 moved the seed-2350 fight; both wastes predate AE. | `arena:reflex.rs` (`follow`'s wandered-off branch, `note_blocked`, `repath_ms`); `arena:path.rs` (`path_with`'s shoulder rule, the class gap its doc names); `arena:motion.rs` (the corner-cut refusal) | A follower keeps its route while the next route cell is inside its turning circle and steers at the first cell outside it; a stop on a corner cut is answered by a one-cell shoulder step or a refusal that keeps the route, not by a re-search of the same route; a re-search of a route whose refusal has not changed is refused by name and counted; the per-wake `path_cells` peak reported and bounded; every change proved at three cadences and measured on both lanes at four seeds. Moves the CI deterministic block (a bless with the reason) and the hash. | L | plausible (the sweep repairer's instrumented diagnosis at seed 2350 and a four-seed sweep; not yet reproduced in play) |

### 1.2 P2

| Id | What happens, to whom | Where | Fix | Size | Status |
|---|---|---|---|---|---|
| B-15 — landed D.2 | Rounds that land on an already-destroyed section keep printing `armour gone, 0 of 150 inside.` to the target and `Hit on BA: turret.` to the shooter; six in ten seconds in the smoke. Cut Q.6 took them out of the shooter's `hits`. | `arena:damage.rs:638-666`; `tacsim:match_/cockpit.rs:821-825`; `arena:step.rs:1237-1279` | The round stays absorbed and journalled; both pilots get one distinct sentence for a spent section, at most once per 5 s per section. | S | verified, live |
| B-16 — landed D.2 | `FIRE refused. Overheated: 60 of 60. Let it cool.` does not name the weapon; after a laser shot it is the machine gun that was refused, because heat is the platform's. | `tacsim:match_/cockpit.rs:923-965`, `:1983-1990`; `arena:weapons.rs:561-569` | Every `FIRE` refusal opens with the weapon; a weapon whose `heat_per_shot` is 0 is exempt from the ceiling. | S | verified, live |
| B-17 — landed D.2 | `help cockpit` says `FIRE mg`; the weapon id is `machine_gun` on all six units and `choose_weapon` matches whole ids only. | `docs/help/cockpit.md:144`; `tacsim:match_/cockpit.rs:2954-2968` | Accept a unique prefix and content-declared short names, print them on `WEAPONSTATUS`, and correct the help. | S | verified, live |
| B-18 — landed D.2 | `LOCK RB` then `FIRE` on an identified teammate works with no word; callsigns differ by one letter between sides. | `tacsim:match_/cockpit.rs:1692-1711`; `arena:weapons.rs:535-573` | Keep it possible and say it: `(FRIENDLY)` on the lock line and the hit line. | S | verified, live |
| B-19 — landed D.1 | A contact inside half a rounding step prints `0 m` (under 25 m on `CONTACTS`, under 5 m locked). | `tacsim:match_/cockpit.rs:546-553` | Floor the rounded figure at one step. | S | verified, live |
| B-20 — landed D.4 | A contact that expires and is re-detected on the very next pass is silently renamed from `BA` back to `X2`; `LOCK BA` then answers `No such contact.` until the rename three seconds later. A locked pilot loses the lock with it. | `arena:step.rs:1179-1187`; `arena:perception.rs:643-654`, `:814-841`; pinned by `tacsim/tests/perception.rs:1296-1327` | A notice on the regression (`BA is unidentified again: X2.`). Never an alias: accepting `BA` would say that BA is still out there. | M | verified |
| B-21 — landed D.4 | A target this platform once numbered, lost and then received over the mesh prints `New contact BA` and at once `Contact X2 identified: BA`; the `was` field reads the permanent book instead of the current hold. | `arena:step.rs:1211-1217`; `arena:state.rs:1780-1820` | Carry "provisional during this hold" from the insert site; a mesh arrival sets none. | S | verified |
| B-22 — landed D.4 | A pilot who drives into an ECM bubble reads `Contact lost` three times and nothing else; neither the frame nor any verb says the receiver is degraded. | `arena:perception.rs:1583-1644` | A frame field for the observer's own attenuation budget against its unjammed value, one `STATUS` line and a notice on the transition. No source and no bearing. | M | verified |
| B-23 — landed B.4 | `VIEW` prints one blank line when it holds a frame and the window cannot be drawn (registry row gone, pose off the window). | `tacsim:match_/cockpit.rs:2152-2177`, `:2609-2615`, `:2667-2669` | Check `mush_ansi::panel::is_grid` on the payload and say the dark-window line otherwise. | S | verified |
| B-24 — landed B.4 (square ground; the 30-cells-north symptom is `CONTACTS`' and the beyond-window line's) | The line client's `VIEW` window is 78×20 against a 60-cell circular sight, so a contact 30 cells north is off the map (the beyond-window line counts it). | `tacsim:match_/cockpit.rs:185-188`, `:2670-2677` | Draw two columns per cell (39×20 cells, square ground) and add a scale line; re-bless `view_grid.txt` and the three renderers together. | M | verified |
| B-25 — landed D.1 (the pilot's word; the commander's is `T-81`) | `CONTACTS` can only print `close` or `effective`: the furthest plot is 900 m (600 m sight plus one 300 m mesh hop) and `far` starts at 1.5 km for every shipped weapon. The deliberation brief's `band` and the order schema's `max_range_band` share the dead words. | `tacsim:match_/cockpit.rs:607-621`; `tacsim/content/weapons/cannon_105.json:24-33`; `tacsim/content/sensors/visual_mk1.json:14`, `:18` | Band the `CONTACTS` word against the lesser of the sensor's reach and the weapon's last band; the weapon's own bands stay on `WEAPONSTATUS`. | S | verified, live |
| B-26 — landed B.4 | A wrapped `SCAN` run can be 77 columns: the width test runs before the comma is pushed. No shipped chassis reaches it. The status wrapper is not affected. | `tacsim:match_/cockpit.rs:1870-1874` | Test `run.len() + 3 + cell.len()`. | S | verified |
| B-27 — landed M.3 | `Held: the ground ahead will not take the tank.` printed twice one second apart for one stop at the river bank, at `Quiet`. `held` is carried on `UnitState`, and a unit test pins one notice per stop, so the path that re-arms it between pulses is not yet known. | `arena:step.rs:751-789`; `arena:motion.rs:278-297` | A failing test first (one approach, one stop, two pulses at `Quiet`), then the gate it points at. Settled with `T-04`. | S | plausible, live |
| B-28 — landed D.1 | `SIM AI` from a non-lead answers the `SIM START` sentence; the documented bare read is refused outside `Selection` and for non-leads; bare `SIM UNIT` before a claim is refused although the help says it lists the catalog. | `tacsim:match_/lobby.rs:1725-1747`, `:310-311`; `docs/help/simulator.md:30-32` | Serve both bare reads before the state and lead gates; give `SIM AI` its own `NotLead` sentence. | S | verified, live |
| B-29 — landed D.1 | `SIM STATUS` prints `arena time 0 ms` for the first minutes of a match and then lags by a minute or more, in raw milliseconds: the figure is the last closed 16 KiB ledger chunk's. | `tacsim:match_/lobby.rs:2101-2103`; `tacsim:arena_bridge.rs:1370`, `:1437-1449` | Mirror `now_ms` from the per-pulse frame and print `m:ss`; keep the chunk figure on the staff row. | S | verified, live |
| B-30 — landed D.1 and D.5 | With AI fill on, the claim line and the Simulator Selector still say `SIM START` needs a pilot on both sides. | `tacsim:match_/lobby.rs:620-627`; `mush-atlas/content/neighborhood.ooc_area.json:612` | Branch the sentence on `match_cfg.ai_fill`; make the console text neutral. | S | verified, live |
| B-31 — landed D.3 | `@arena/reset` prints `already idle; nothing changed` above the rows naming the tanks it could not reset. | `tacsim:match_/verbs.rs:297-301`, `:335-347`; `tacsim:match_/reset.rs:127-135` | Drive the headline off `ResetReport::is_empty`, which folds `failed` and `deployed` and has no caller. | S | verified |
| B-32 — landed D.3 | A `run()` that refuses before the check-out (`Unavailable`, `NobodyPlaying`, `EmptySide`) leaves the centre in `Boarding` with the deadline cleared; a seated pilot's only exit is `SIM ABORT`. | `tacsim:match_/lobby.rs:1629`, `:1678-1680`, `:990-996` | Release to `Aborted` with the reason announced, as the check-out failure arm does. | S | verified |
| B-33 — landed D.3 | Every human can `EXIT` and the arena keeps stepping machines at 500 ms for up to an hour, holding the centre. Nothing counts un-`left` slots during `Running`, and no verb concedes with a result. | `tacsim:match_/lobby.rs:1445-1460`; `arena:step.rs:646` | `SIM CONCEDE` for a side's lead (a recorded loss), and an unattended match ends with its own reason after `match.unattended_ms`. | M | verified |
| B-34 — landed D.3 | A tank released before `STARTUP` stays cold in `Withdraw` for the match and keeps its side standing; it ends only when a machine finds and kills it. | `arena:reflex.rs:826-842`; `arena:damage.rs:1072-1105` | A released cold tank runs `STARTUP`; driving home is what `Withdraw` is for. | S | verified |
| B-35 — landed D.3 | A check-in that refuses skips `release_seat`, so the pilot keeps the station and the focus on a unit that is in no match until the server restarts. | `tacsim:match_/mod.rs:981-996`, `:1023` | Release the seat on the error arm, and retry `check_in(ctx, unit, None)` once. Pairs with `T-49`. | S | verified |
| B-36 — landed D.3 | `waiting_to_start` answers `the match has not started yet` for a cockpit billet on any undeployed unit anywhere. Reachable through staff fielding only. | `tacsim:match_/cockpit.rs:3123-3134` | Narrow the walk to the centre's roster while it is `Boarding`. | S | verified |
| B-37 — landed U.4 | The boot pod sweep runs on the first 1 s tick and not inside boot, because a pod resolves through the content latch. The window fails closed on the lease mirror and is unguarded. | `tacsim:match_/sim.rs:271-292` | A post-boot hook in `mush-core` that runs after every module's `on_world_loaded`. | M | verified |
| B-38 — landed B.4 | `BOARD` and the end-of-match ejection move the pilot with no room view, so the web and TUI room bands go stale and a line pilot is not shown where they landed. A walked move calls `show_room` before it emits `Moved`. | `tacsim:match_/cockpit.rs:1343-1360`; `mush-core/src/commands/movement.rs:128-133` | Send the mover the room view after `put_out` and after the `BOARD` move. | S | verified, live |
| B-39 — landed E.1 | A hurt machine in `Withdraw` stands and fights whenever it holds a line on its best hostile, at any health: the stand is gated on the platform's whole weapon reach (3 km; 2.4 km for the laser tank), which is always true on this map, and the rally is suppressed while it holds one. | `arena:reflex.rs:1280-1301`, `:1117` | Stand only inside the fired weapon's close band, and never below half of `withdraw_permille`. | S | verified |
| B-40 — landed E.2 | A live `Objective::Withdraw` order re-enters `Withdraw` the tick after a rally and throws the route away, once per `rally_ms` for the order's life. Needs deliberation on. | `arena:reflex.rs:893-902`, `:1119-1127` | No rally while an order naming `Withdraw` stands. | S | verified |
| B-41 — landed E.2 | An `advance` order to an impassable cell re-asserts the goal every decide tick, cancelling the waypoint fallback and paying a failed search per unit per tick for the whole time to live. | `arena:reflex.rs:1006-1012`, `:1749-1771` | Remember the ordered cell that failed and skip the re-assert while the live order names it. | S | verified |
| B-42 — landed E.3 | The controller orders speed 0 only inside its one-cell arrive window and drives flat out until then; a stock tank needs 56 m to stop, so machines coast past goals and through their firing stop, and a withdrawn tank's rally clock restarts on each overshoot. | `arena:reflex.rs:1606-1610`, `:1699`; `arena:motion.rs:579-583` | Order the stop a braking distance early (`v² / 2·brake_cms2`, both on `UnitState::mobility`). | M | verified |
| B-43 — landed E.3 | The remembered-stop list evicts by insertion order and ages by stamp, so the cell a machine keeps being stopped at is the first it forgets. | `arena:reflex.rs:666-677`, `:768-769` | Evict the oldest stamp; a test that refreshes an entry and then overflows the list. | S | verified |
| B-44 — landed E.3, reopened and re-closed M.2 | A routeless machine tests its next step only at the 1 Hz decide tick and drives at a bluff in between; five `Blocked` cells on the X.6 plateau fixture. Not reachable on the shipped ground (`T-18`). | `arena:reflex.rs:1687-1697`, `:1716-1744` | Test the next step every substep a unit is routeless. | M | verified |
| B-45 — landed E.2 | The reflex controller reads the target's live wreck flag in four places, so a machine drops a target the pulse it dies however stale its plot: the one live read left in a target choice that F.3 otherwise moved onto the plot. | `arena:reflex.rs:740`, `:865-875`, `:1311-1320`, `:1399-1404` | Read the contact's own `seen_condition` where one exists and assume alive until the contact expires; `reflex_has_line` stays physics. | M | verified |
| B-46 — landed E.3 | Every refused crossing zeroes the whole speed. A follower whose id sorts below its leader's is stopped dead at the cell the leader has not vacated yet and takes 11 to 15 s to rebuild way, repeatedly at the ford; a pilot who clips one of 2,228 impassable cells at a shallow angle pays the same. The terrain half is doc 16 §3.4 step 4 as written and is test-pinned (`tacsim/tests/arena.rs:1090`). | `arena:motion.rs:686-746`; `arena:step.rs:742-790` | An occupancy refusal holds at the boundary and keeps the speed, with no `blocked` event; terrain, edge and slope refusals stay literal. | M | verified |
| B-47 — landed M.4 | A tank with both tracks destroyed still turns one way and still moves at 50 per cent: `no_turn_toward` is one `Option<Side>` and the second consequence overwrites the first; `turn`'s neither-way branch is unreachable. | `arena:motion.rs:461-509`; `arena:hash.rs:206-210`; the five `tacsim/content/units/line_tank*.json` (`:52-68`) | Author `disable_motion_when_all_of` on both tracks of the five tanks (and the biped's legs) and make `no_turn_toward` a pair of flags; both land where the hash already moves. | M | verified |
| B-48 — landed M.1 | `Task::end` drains the ledger backlog with `try_send` and no retry, then waits to deliver `Ended` with a digest; under egress backpressure the result records a digest `log:match.<id>` cannot reproduce. Nothing asserts the digest at all. | `tacsim:arena_task.rs:543-587`, `:609-622` | Drain with `send().await` racing the stop signal; `arena_story.rs` concatenates the chunk lines, hashes them and asserts the digest. | S | verified |
| B-49 — landed M.1 | `send_stop` maps a full inbox to `Gone`, so a `SIM ABORT` during a busy pulse restores every unit from its check-out snapshot and discards the match's damage (`finish` aborts the task, so there is no second writer). | `tacsim:arena_bridge.rs:1081-1083`; `tacsim:arena_task.rs:220-224`; `tacsim:match_/mod.rs:1163-1186` | The `is_live` split `forward_intent` uses: a full inbox is `Busy` and the verb says try again. | S | verified |
| B-50 — landed M.1 | A `Control` dropped from the full pre-pulse buffer leaves the world's epoch one above the arena's and every later order from that pilot is refused `Epoch`. Needs 17 `Control` messages in one pulse window of a two-tank match; the bound scales with units. | `tacsim:arena_task.rs:333-344`, `:365-366`, `:382-385` | Never drop a `Control`: collapse per `(unit, channel)` keeping the highest epoch. Pairs with `T-11`. | S | verified |
| B-51 — landed M.2 | A pilot's beam schedules its impact at `now_ms` in phase 4, which drains nothing: it resolves in the next pulse's first substep against later positions under the fire timestamp (an AI beam does not slip), and the zero-length timer can spin zero-substep pulses for up to a millisecond. Extension units only. | `arena:weapons.rs:166-174`; `arena:step.rs:527-573`, `:819`; `tacsim:arena_task.rs:632-635`; `arena:clock.rs:370-373` | Drain due events at or before `now_ms` once more at the end of phase 4, and floor the timer at 1 ms. | S | verified |
| B-52 — landed M.4 | A shot past the weapon's last band resolves at the `p_min` floor (1 per cent) while the cockpit prints `beyond`; `resolve_fire` has no range gate. | `arena:resolution.rs:557-574`; `tacsim:match_/arena_def.rs:592` | Past the last band resolves as `Resolution::blind`, which is what the module note already claims. | S | verified |
| B-53 — landed M.4 | The gunnery `sensor_condition` term reads the first component in id order with a `condition` cell, which on the ECM and ECCM tanks is the pod and not the sight. | `arena:resolution.rs:809-845`; `tacsim:definitions/unit.rs:1289`, `:1350`, `:1418` | Carry the sensor component's id across the custody seam (`match_/arena_def.rs:347` knows it) and address the cell. | M | verified |
| B-54 — landed M.4 | `Eccm::read` refuses a filter at condition 0 and `Jammer::read` never reads condition. Both are read once at check-out, so the asymmetry bites a unit that enters a match already damaged; neither is re-read after damage inside a match. | `arena:perception.rs:335-355`, `:398-426`, `:1016-1029` | Add the guard to `Jammer::read`, and say in both docs that component condition is read at check-out only. | S | plausible |
| B-55 — landed M.4 | `[tacsim.sensors].max_signature_factor` below 1.0 boots clean and makes the candidate cull shorter than the range test, so contacts silently vanish. | `mush-server/src/config_ext.rs:455-477`; `arena:perception.rs:903-911` | Refuse under 1.0 by name. Pairs with `T-26`. | S | verified |
| B-56 — landed M.4 | `line_cells` is a bare Bresenham with no endpoint canonicalisation: measured on the shipped map over 300,000 pairs within 60 cells, 31 per cent walk different cells in the two directions. A sees B and B does not see A on an arithmetic tie, in detection and in the mesh. | `mush-atlas/src/trace.rs:252-281`, `:368` | Walk from the lesser endpoint and reverse; a property test that `sees(a, b) == sees(b, a)` at equal eye heights over the shipped heightfield. | S | verified |
| B-57 — landed M.4 | The 50 ms substep floor beats the one-cell bound when `max_speed > 20·(cell_cm − 1)`: above 19,980 cm/s at 10 m cells, and for the stock tank below 76 cm cells. Motion then refuses every step and says `impassable` about grass. | `arena:clock.rs:32`, `:325-340`; `arena:motion.rs:691-698`; `arena:state.rs:96-121` | Refuse the match at `Arena::new` by name when the bound does not hold. | S | verified |
| B-58 — landed AE.1 | Every cell is priced once from the locomotion table's tracked row, so the fieldable biped is refused the 798 heavy-woods cells its own row prices at 200, and pays tracked costs on rubble and light woods; its slope limit is its own. The help admits the gap. | `tacsim:definitions/arena.rs:656-666`; `arena:state.rs:54-69`; `mush-atlas/content/locomotion.json`; `docs/help/simulator.md:85-86` | A cost layer per locomotion row on `ArenaTerrain`, chosen per unit; needs `T-15` first. Scheduled with `N15`. | L | verified |
| B-59 — landed B.2 | Both panel clients re-ask a lost `arena.map` answer only when a cockpit frame arrives, and the arena sends a frame only when one changes: a stationary pilot with no contacts keeps `Waiting for the arena map…`. A read verb heals it, which nothing tells the pilot. | `web/src/lib/game.svelte.ts:841-847`, `:914`; `mush-net-ssh/src/tui/app.rs:696-745`, `:806-858` | Drive the heal from a timer while the cockpit screen is active and no ground is held; a refresh binding on both cockpit screens. | S | verified |
| B-60 — landed B.1 | The web cockpit's `−` button zooms in and `+` zooms out. The aria-labels and the cell readout are truthful; do not swap those. | `web/src/components/CockpitScreen.svelte:151-152` | Swap the two glyphs. | S | verified |
| B-61 — landed B.1 | A late `arena.map` answer for another match replaces a correctly held one for up to 2 s; the doc comment promises the opposite. | `web/src/lib/stores/cockpit.svelte.ts:104-112` | Gate the assignment on the match id. | S | verified |
| B-62 — landed B.3 | The TUI map legend names a contact whose glyph a later marker covered. | `mush-net-ssh/src/tui/render.rs:2295-2311`, `:2334` | Build the legend from the glyphs that survived. | S | verified |
| B-63 — landed B.3 | The TUI keeps a `0 cockpit` tab after the seat goes, and `v` on the cockpit screen prints a line about `VIEW`'s own window. | `mush-net-ssh/src/tui/app.rs:419-427`, `:810-822`; `tui/screen.rs:952-956` | Clear `cockpit_seat` on the cleared keyframe; drop `v` from that screen. Settled with `T-63`. | S | verified |
| B-64 — landed B.3 | The TUI cockpit's `notices` pane is the whole scrollback, so channel chatter pushes a hit off a nine-row pane; the web shows the arena's notices only. | `mush-net-ssh/src/tui/screen.rs:884-898`; `tui/app.rs:2671-2710` | Render the held frame's `notices` array there, as the web does. | S | verified |
| B-65 — landed B.3 | The TUI map pane emits heading, grid and legend into a pane that may have one or two inner rows; the legend is clipped silently. | `mush-net-ssh/src/tui/render.rs:2152`, `:2287-2290`, `:2313-2328` | Drop the heading and then the legend as the pane shrinks. | S | verified |
| B-66 — landed B.3 | `@tui` always opens with `(no room yet)` until the next `look`; `entry_queries` asks for nothing about the room. | `mush-net-ssh/src/tui/render.rs:2745-2747`; `mush-net-ssh/src/server.rs:717-740`; `tui/app.rs:626-680` | Seed the room on entry. | S | verified, live |
| B-67 — landed B.4 | A cockpit keyframe that renders to nothing on a line client still erases and redraws the pilot's input line, twice a second at `Engaged`. | `mush-net-ssh/src/server.rs:1678-1689`, `:1850-1853`, `:1869`; `tacsim:arena_bridge.rs:1230` | Render into a scratch buffer and skip the erase and redraw when the event produced no bytes. | S | verified |
| B-68 — landed D.5 | `help` still lists `wod`, `shadowrun` and `action`, and `help perception`'s worked example is four deleted verbs. The pages also give wrong usage for five live verbs (`+sheet`, `@sheet`, `+chargen`, `+stat`, `+roll`), and are baked into the bot's primer (`mush-bot/build.rs:5`). | `docs/help/wod.md`, `shadowrun.md`, `action.md`; `perception.md:16-49`, `:93-97`; `behavior.md:28`; `npc.md:55`, `:74`; `intel.md` | Delete the three, rewrite `perception`'s example on the arena's own contacts and `SCAN`, fix the references; then `T-45`. | S | verified |
| B-69 — landed L.1 | A password-failure tempban (SSH password auth and the welcome screen's `connect`; public-key misses are not counted) has no lift: `IpGuard` can clear a siteban only, and the gateway has no operator surface. | `mush-config/src/ipguard.rs:141`, `:253-263`, `:336`, `:413`; `mush-server/src/staff.rs:42-62` | `@siteban/lift` on the store route: a short-lived `tempban-lift:{ip}` key read by the `SitebanMirror` refresh. | M | verified |
| B-70 — landed L.1 | `reset-instance.sh` deletes the SSH host key with the data directory; every returning client and every bot pinned by `known_hosts` or `host_key_sha256` then refuses the box. | `scripts/reset-instance.sh:202`; `mush-gateway/src/ssh_bridge.rs:457`; `mush-net-ssh/src/hostkey.rs:41-60`; `mush-bot/src/transport.rs:30-46` | Preserve `ssh_host_ed25519` across the wipe and name it in the script's inventory. | S | verified |
| B-71 — landed L.1 | The deploy's stale-bundle guard compares mtimes under `web/src` only: a rollback passes (`git archive` stamps the commit time) and `index.html`, `package.json` and `vite.config.ts` are outside it. No refusal branch has ever run. | `deploy/dev1-split/deploy.sh:29-47` | Write the commit into `web/dist/.commit` at build and refuse a mismatch. | S | verified |
| B-72 — landed L.1 | The stdout log layer keeps ANSI colour where the file layer strips it, and under systemd stdout is journald. | `mush-config/src/logging.rs:225-243`, `:253-256` | `.with_ansi(IsTerminal::is_terminal(&stdout()))`. | S | verified |
| B-73 — landed L.1 | A refused-token cool-off answers 429 before the upgrade, which a browser sees as 1006; a legitimate tab behind the same address learns nothing for up to 60 s. | `mush-net-web/src/ws.rs:269-275`, `:303-305` | Complete the upgrade and close with a distinct code, as 4401 now does for the refusal. | S | verified |
| B-74 — landed M.2 | From cut D.4's judge: a mesh share that cancels a held loss inside the hysteresis window leaves a mesh-inserted row (`numbered: false`), so the rename is said by nobody and the pilot's column goes from `X1` to `BA` with no line. Before D.4 that case read `Contact X1 identified: BA, blue.`, so it is a narrow regression, pinned on purpose by `a_mesh_share_inside_the_window_cancels_the_loss_and_says_no_regression`. | `arena:step.rs` (`stage_pilot_notice`, the `Detected` cancel arm); `tacsim/tests/perception.rs` | In the cancel arm, when the row is not regressed and the name moved, stage a `Classified` contact notice with `was: Some(was)`; no new state. | S | verified by the D.4 judge |
| B-75 | From cut D.5's judge: pack apply now refuses an exit named with a kernel or host command word, and `@dig`, `@open` and `@name` do not. A Builder can hand-dig an exit aliased `l` and take `look` from that room, and a green test pins it (`@dig Lounge = library;l`). Staff-only. | `mush-core/src/commands/` (the three building verbs); `mushd/tests/persistence.rs:1908` | An owner ruling first (refuse, warn, or leave to builders); then `Registry::kernel_command_named` at the three verbs and the test moved. | S | verified by the D.5 judge; needs a ruling |
| B-76 — landed AA.1 | From the 2026-09-22 review: a reflex-flown tank whose engine is destroyed (`disable_motion`, struck through `hull.rear` on the stock `line_tank`; `arena_story.rs` calls an engine kill without a wreck the routine end of a rear-quarter duel) while it holds a classified hostile between `engage_cm` and `release_cm` (420 m to 600 m under doctrine or `far`; 280 to 400 under `effective`; 140 to 200 under `close`) never fires again. `Approach` enters `Engage` only inside `engage_cm` and reads no motion fact, so every decide tick it re-plans a stand-off cell it cannot drive to and emits Heading and Speed intents that are refused `MotionDisabled`; `engage_orders` is never called, although its own ceiling is `release_cm` and `resolve_fire` has no arc or motion gate, so the intact cannon may legally shoot at that range. Carrying a weapon it is never `disabled`, so it neither shoots back nor lets its side be eliminated. Health after the engine and the rear armour is 909 of 1000, above `withdraw_permille`; `Withdraw` would not rescue it either, since `withdraw_stand` fires out to `stand_cm` (200 m) until `arrived`, which a machine that cannot move never is. A healthy tank closing through the same annulus also holds fire, which 16b's cut E row defers on purpose; immobility is what makes the window permanent. Two-step exploit: kill the engine, back off to 450 m, take it apart. | `arena:reflex.rs` (`Arena::decide`, the `ReflexState::Approach` arm `:1329-1352`; `engage_orders` `:1585`; `withdraw_stand` `:1675-1685`; `best_hostile` `:1771-1794`); `arena:intents.rs:437` (`Refusal::MotionDisabled`); `arena:damage.rs:1090-1113` (`settle_disabled`, `out_of_action`); `tacsim/content/units/line_tank.json` (the `engine` section) | In the `Approach` arm choose the closing edge off the unit's own hull: `release_cm` when `limits.motion_disabled` (optionally also `mobility.max_speed_cms == 0`, which covers N13's zero-speed target), `engage_cm` otherwise. The existing `Engage` arm is the stationary path (its speed 0 and bearing orders are refused harmlessly) and `engage_orders` keeps `hold_fire`, the order's band cap and the `release_cm` ceiling; the `Engage` release edge is also `release_cm`, so there is no oscillation. Do not let `Approach` fire while closing (16b's deferral). Decide with it whether `arrived` reads true for a machine that cannot move, so a hurt immobile tank stands too. Tests in `tacsim/tests/reflex.rs`: the immobilized fixture (flag set mid-approach, not at construction) engages and stages a lock and a fire between the edges, its mobile twin does not; the ceiling past `release_cm`, a `close` band order at 450 m and `hold_fire` all still hold; the immobile tank below `withdraw_permille` fires; a shipped-scale case with the engine struck through `hull.rear` (909 of 1000, not out of action, a `Fired` event within stated arena seconds); three cadences, same `state_hash`; the D.4 harness counters unmoved. | S | verified by reading at c3852a0c by both lenses (`reflex.rs` is mid-edit for E.4 and was read at HEAD; E.4's diff leaves the `Approach` arm untouched; E.3 did not touch it); the reviewer's two probes not re-run; moves the hash (`row.state.code()`), which cut E already declares |
| B-77 — landed M.1 | From the 2026-09-22 review: `Task::publish` discards the `Full` error of `try_send(ArenaOut::Frames)` after `Arena::frames` has already cleared `pending_notices` and committed the new view to `last_frames`, so a frame the world never received is not re-sent while nothing changes, and every notice staged in that pulse for every pilot of the match (a refusal, a hit, a contact, `Power online.`) is lost for good: nothing re-derives a notice from the ledger, an empty clock update writes no mirror, and `Lagged` is the ledger ring's signal, never the channel's. If the dropped frame is the `StartupComplete` flip, an idle pilot's mirror stays at `Starting`: `power_of` short-circuits on any non-`Off` mirror, `STARTUP` answers the hold-on line without forwarding, motion and fire are refused locally, and only a frame-changing event (a contact, a tempo change) or `EXIT` clears it. Narrowed: the egress is one 512-slot channel shared by every match and steady traffic is about one message per match per arena second, so the `Full` branch needs the world's drain arm disarmed for minutes (a pending commit that does not clear, a retained-write seal, legacy persistence work, a non-writing admission mode), not an ordinary busy pulse; the view half heals on any contact, the notice half never does. Doc 16 §3.1 promises a latest-wins slot that drops stale frames and never events; the slot does not exist. | `tacsim:arena_task.rs:548-571` (`Task::publish`, the discarded `try_send`; contrast `drain_backlog` `:661-673`); `arena:step.rs:1295-1310` (`Arena::frames`: `pending_notices.clear()`, `last_frames.insert`); `tacsim:arena_bridge.rs:1271-1342` (`on_frames`), `:2052-2057` (`egress_capacity`); `tacsim:match_/cockpit.rs:527-531` (`power_of`); `mush-server/src/world_task.rs:3986-3990` (`arena_egress_armed`) | A held-frames field on `Task` beside `backlog`: on `Full` keep the message; on the next pulse merge per pilot (latest view wins, the held notices prepended to the newer frame's, a pilot only the held message names carried as-is, the notice vector bounded with `push_bounded` and logged once at the drop) and re-offer it before the D.1 clock clause; a pilot released in between is dropped by `on_frames`' seating check. The arena library stays untouched (it never learns whether the world took a message) and `Frames` is never awaited (`Ended` is the one message the task waits on). One rule: a notice is delivered at most once. Tests in `tacsim/tests/arena_task.rs` on a small-capacity egress (the helpers hard-code `mpsc::channel(256)` at `:241` and `:598`): a refusal survives a full egress exactly once; the startup flip delivers `Power online.` and a frame reading `Online`, and `power_of` then answers `Online`; latest-wins on the pose with both notices in order; no duplicate and no extra `Frames` on later pulses; a released pilot gets nothing; the held vector stops at the cap. | S–M | verified by reading at c3852a0c by both lenses (neither file mid-edit; E.1 to E.3 touched neither path); the reviewer's paused-clock probes not re-run; how often the 512-slot channel fills under real load was not measured |
| B-78 — landed AA.2 | From the 2026-09-22 review: when the boarding deadline drops a side's lead while that side keeps a seated slot, `centre.leads` still names the dropped account: `board_deadline` retains the seated slots and runs the match without the succession `leave` performs ("a side is never left with no one who can start"). Nobody on that side can `SIM CONCEDE` for the rest of the match: the survivor reads `Only a side's first pilot can concede for it.` and the dropped lead reads `You hold no slot in the simulator.` Shipped defaults reach it with three players and no staff (A and B claim red, C claims blue, either lead starts, B and C `BOARD`, A does not, 120 s pass) under either `ai_fill` value. Narrowed: `SIM START` and `SIM AI` read `leads` in `Selection` only, so the one reachable consumer after the deadline is `concede`; the match still ends by elimination, the cap, the unattended deadline or `SIM ABORT` (no result), and `release_rows` clears `leads`, so nothing leaks into the next match. It is the result-bearing exit `B-33` was written to add, reopened for one composition; the shipped deadline test drops a non-lead and the concede test seats everyone, so neither sees it. | `tacsim:match_/lobby.rs:1786-1800` (`board_deadline`'s drop block, `:1797`), `:729-744` (`leave`'s succession), `:2050-2053` (`concede`'s `leads_side` gate), `:124` (`Centre::leads`), `:209-213` (`leads_side`) | One writer of the succession rule: `Centre::resettle_leads` beside `leads_side`, rebuilding `leads` over both teams (keep the lead if that account still holds a slot on the side, else the first remaining slot in claim order, else no entry), called after the `retain` in `board_deadline` inside the same borrow, and in place of `leave`'s inline block. Not the concede gate (doc 16 §473 and `B-33` rule it deliberately) and not a recompute on read (it would move `SIM START`'s and `SIM AI`'s gates too). Tests in `tacsim/tests/match_lifecycle.rs`: the three-player trigger (`Running` with two slots, B concedes for red, the board names blue, A reads `NoSlot`); a two-a-side double drop; a dropped non-lead leaves `leads` unchanged; with `ai_fill` on an emptied side carries no entry; the existing `SIM LEAVE` and deadline pins stay green; one SSH-level pass in `mushd/tests/sim_lobby.rs` beside `d3_a_lead_concedes_and_the_board_names_the_winner` driving real `BOARD`, which the reviewer's probe did not. | S | verified by reading at c3852a0c by both lenses (`lobby.rs` unchanged since D.3, not mid-edit); the reviewer's lifecycle probe not re-run; the refute lens grades the harm P3-shaped (one verb, one side, one match) |
| B-79 — landed AA.3 (on the Q41 default, not the owner's word) | From the 2026-09-22 review: `Task::pulse` publishes and returns on `out.ended` before it reads `took`, so a pulse that overruns `tempo.stall_ms` and also ends the match is accepted with its own reason (`Timeout`, `Elimination`, `Spent`, `Conceded`, `Unattended`, `Abort`) and its arena-derived units are committed to the records, where doc 16 §3.1 and `EndReason::Void`'s own doc say a stalled pulse is voided and every unit restored from its check-out snapshot; the same stall on a non-terminal pulse takes the watchdog. Hoisting the check is not enough: `Arena::stop` is first-wins and `step` returns early on an ended arena before it reads the stop, so the watchdog's `Void` would be recorded and never honoured. Narrowed: not reachable by a player on shipped content (`stall_ms = 2000` on all three configs, the validator floors it at `engaged_ms`, measured p99 26.4 ms, the fault verb is test-only); the trigger is host pathology, and the `max_duration_ms` end is the exposed one because it is settled after a full-cost pulse. Nothing wedges; what lies is the result word and the custody. The refute lens asks whether voiding a finished match on a two-second host stall is the right ruling at all, since the pulse's own record is deliberately published and the final cells are the match's evidence. | `tacsim:arena_task.rs:499-541` (`Task::pulse`: the `ended` return `:506-509`, the watchdog `:516-541`), `:601` (`Task::end`'s `void` test); `arena:step.rs:525-538` (the ended short-circuit), `:749-756` (the `Timeout` end); `arena:state.rs:3273-3277` (`Arena::stop`); `tacsim:match_/mod.rs:950-952` (`custody_in` on void) | Keep the publish where it is (the stalled pulse's chunks are black-box evidence). Compute `stalled = took > self.stall` before acting on `ended`; on a stalled terminal pulse log and call `Task::end` with `MatchEnd { reason: Void, t_ms: self.arena.now_ms }` directly, with no `arena.stop` and no second step, so `units` is empty and the world restores the snapshots; the non-terminal watchdog body is unchanged. Record that the forced void is then not journalled (the ledger ends in the original `match_ended`); the journalled variant means letting a `Void` stop override an ended arena inside `step`, a library change and larger. Tests in `tacsim/tests/arena_task.rs`: a stall armed on the ending pulse (a new seed in `SEEDS`, a small `max_duration_ms`) voids and carries no units; a fast terminal pulse keeps its reason and its units; the existing non-terminal proof unchanged; the stalled pulse's chunks still left the task; the custody consequence beside `mushd/tests/arena_custody.rs`'s injected-fault lens (every record byte-identical to its snapshot). | S | verified by reading at c3852a0c by both lenses (`arena_task.rs` unchanged since D.1, not mid-edit); the reviewer's fault-injection probe not re-run; the owner confirms doc 16 §3.1's ruling stands for a finished match before it is built |
| B-80 — landed B.2 | From the 2026-09-22 review: `CockpitStore.applyFrame` invalidates the map on a change of match and keeps the notices ring, relying on an unseated frame to `clear()`; that frame is not guaranteed, because `cockpit_cleared` carries `RETAIN_LATEST` and the session ring coalesces it away once the next match's first frame is recorded, and neither automatic reconnect path resets the store (`resumed` touches the vitals watermarks only; `onFullRehydrate` resets unit, requests, channels and vitals and not the cockpit, and its hydrate pass is answered with the new match's mirror, stored with its notices cleared; `cockpit.clear()` runs only from `enterGame` and the disconnect modal's button). A tab that resumes into the next match shows its healthy tank the last twelve lines of the old one, `your tank is destroyed` and `BA destroyed` among them, the old designators reading as the new match's. Narrowed: a single connection cannot reach it (nobody can board an away client), so the trigger is a second connection on the same character (another tab, or SSH beside the browser) finishing match 7 and taking a seat in match 8 while the first tab is down; the window is not bounded by `linkdead_grace_s`, since the session stays online. Display only: nothing but the pane reads `notices`, and every stale line was the pilot's own. The doc on `cockpit_cleared` claims the clear is the one frame left in the ring, which is true only until the pilot is seated again. | `web/src/lib/stores/cockpit.svelte.ts:88-100` (`applyFrame`'s seated arm), `:156-164` (`clear`); `web/src/lib/game.svelte.ts:276-315` (`onFullRehydrate`), `:411-444` (`#resetGameState`); `mush-session/src/manager.rs:717-719` (retain-latest coalescing); `tacsim:match_/cockpit.rs:3225-3232` (`cockpit_cleared` and its doc) | Hoist `open` (`this.#switchedFor !== frame.match`) above the notices append and empty the ring when it is true, so the first frame of a new match starts the ring with its own notices; the two map latches are not touched. Correct `cockpit_cleared`'s doc sentence. Tests: `cockpit.test.ts`'s existing match-7-then-match-8 case (`:54`) asserts the ring holds only match 8's lines and a same-match frame still appends; the reconnect-across-matches case in `game.test.ts`'s cockpit describe, a match-8 keyframe delivered with no cleared frame between, the screen still opening once. `B-64` must carry the same per-match reset when the TUI's notices pane moves onto the frame's array, or it clones this. | S | verified by reading at c3852a0c by both lenses (no web file mid-edit; the store unchanged since Q.5); the reviewer's decoder-and-store probe not re-run |
| B-81 | After a human pilot's grace expires or they `QUIT`, the arena binding and focus are released but the human cockpit billet remains. Reconnecting while still in the pod admits `+focus #<tank>/cockpit`; `flying` then accepts that focus and `STARTUP` can pass world-side admission using the current arena epoch. The arena remains reflex-controlled and no frames are restored; motion/fire still fail the missing-frame power checks. | `mush-sim/src/seat.rs:771-835`, `mush-sim/src/commands.rs:1748-1839`; `tacsim:match_/cockpit.rs:475-482`, `:530-592`, `:2016-2033`; `tacsim:arena_bridge.rs:696-701`, `:965-1053`; `arena:intents.rs:395-469` | In B.4, remove the ordinary cockpit billet through the seat seam on final release, or require the matching live arena binding at cockpit focus/context and intent admission. Add post-grace and explicit-QUIT regressions that attempt `+focus` and `STARTUP` and verify no pilot intent is accepted; preserve within-grace revival. | M | verified by reading at `8366b3fe`; no runtime probe; full control takeover not established |
| B-83 | On the split shape a `BOARD` typed the instant the `red` walk's room view arrives is neither applied nor refused nor answered with a transient (the AC.1 fixer reproduced it in 2 of 6 direct runs of the two-process rig with `[llm]` on and 4 of 10 without it); the likely cause is a line that meets the walk's commit window entering the B-13 `DeferQueue` and never being replayed or refused on the split. `crates/mush-gateway/tests/two_process_llm.rs` hides it with a 1 s sleep before `BOARD`, the one fixed wait in an otherwise bounded-poll test. Needs an SSH-level repro on the real front door. Found by cut AC.1 (2026-09-26). | `mush-gateway/tests/two_process_llm.rs` (the sleep); the B.5 defer queue in `mush-session` / the gateway's line path on the split | Reproduce over SSH on the two-process rig; then either replay the deferred line after the walk commits or refuse it in the player's words, and replace the sleep with a bounded poll on the refusal or the seat. | M | plausible (the rig reproduces it; the cause is not confirmed) |

---

## 2. Tech debt

Every row is S unless it says otherwise, and verified unless it says otherwise.

### 2.1 The arena's clock, hash and record

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-01 — landed M.2 | `match.max_duration_ms` is settled in phase 5 against the pulse's end, so its ledger timestamp is a function of the cadence; every other end is settled inside the substep. | `arena:step.rs:646-653` | `DueKind::MatchDeadline` scheduled in `Arena::new`; phase 5 stays as the backstop. |
| T-02 — landed M.2 | A reflex tick's shot is stamped at the substep's end and not at the tick's own timestamp; `reflex.rs`'s module head claims the opposite. `fired` and `impact` timestamps move with tempo. M. | `arena:weapons.rs:528`, `:588-636`; `arena:step.rs:800`, `:945`; `arena:reflex.rs:49-53` | Thread `at_ms` into `resolve_fire`, `resolve_lock` and `resolve_startup`; a cross-tempo test with an AI shot in flight. |
| T-03 — landed M.2 | The cross-tempo property for discrete actions is proved only through a zero-`dt` driver production never uses; a pilot's `FIRE` resolves at a different arena millisecond at each cadence and no test says so. M. | `tacsim/tests/weapons.rs:1058-1079`; `tacsim:arena_task.rs:378-417` | Amend D-16-2 in doc 16 §11 (a discrete action resolves at the end of the pulse that accepted it) and add a production-shaped test that states it. |
| T-04 — landed M.3 | The ledger's `blocked` line fires on `!was_blocked` and the pilot's notice on `!was_held`, so the line count depends on the partition (`[1, 6, 278, 1]` over four partitions) and is unbounded against a bank. `EventKind::Blocked` carries no cause; `held` is unhashed; `path.rs` and `motion.rs` each carry a false sentence about it; the frame publishes `held` under the name `blocked`. M. | `arena:step.rs:768-789`, `:1106`; `arena:ledger.rs:107-116`; `arena:path.rs:266-270`; `arena:motion.rs:275-277` | Journal on the held edge, add `cause` to the event, fold `held` into `state_hash`, rename the frame field, correct the two comments. |
| T-05 — landed M.2 | Substeps are cut from each pulse's own start with the remainder last, so two partitions of one arena time give two event lists; the three shipped cadences agree only because 250 ms divides all three, which ends above 3,996 cm/s. L. | `arena:clock.rs:354-384`; `arena:motion.rs:72-79` | By default not built: amend doc 16 §3.1 and assert in the harness that the fielded catalog's substep divides the three periods. |
| T-06 — landed M.3 | Heat moves every substep with no ledger event and no hash fold, although it gates firing: the argument Q.6 used for the motion carries. | `arena:hash.rs:192-336`; `arena:heat.rs:160-208`, `:230-239` | Fold `heat_now()` and `carry_milli`; extend the unhashed-carry probe at `hash.rs:593-618`. |
| T-07 — landed M.3 | The `facet:<shot_seq>` draw never reaches the ledger and the seed is not in the result document, so which section a round found cannot be re-derived. | `arena:damage.rs:696-712`; `arena:ledger.rs:366-369`; `tacsim:match_/record.rs:420-434` | Add `seed` to the result document and widen `LedgerEvent.draw` to a list. |
| T-08 — landed D.4 | `in_flight` is set by `Impact` and never cleared; two of its readers fire after the pair has resolved, and since Q.2 one of them writes hashed state. Latent: nothing but a round destroys a section today. | `arena:step.rs:991-998`, `:1236`, `:1281-1341` | Carry the shooter on `SectionDestroyed` and `UnitDestroyed`, as `Impact` does. |
| T-09 — landed M.3 | `observe_section_lost` mutates hashed contact state after the substep's hash is taken; `hash.rs` does not say so. | `arena:step.rs:988-1005`, `:1329`; `arena:hash.rs:314` | One paragraph in `hash.rs`, or move the write above the record. |
| T-10 — landed M.2 | `StepOutput::substeps` reports the whole partition on the pulse a substep ended the match. | `arena:step.rs:495-507`, `:696` | Count the pieces run. |
| T-11 — landed M.1 | `Intent::seq` is stamped and documented as an ordering guarantee and nothing reads it; the buffer drops the oldest, so a `LOCK` can be dropped while its `FIRE` survives. | `arena:intents.rs:192-194`; `tacsim:arena_bridge.rs:963`; `tacsim:arena_task.rs:333-344` | Sort by `(unit, seq)` and drop the newest discrete action, or delete the field and the claim. |
| T-12 — landed M.1 | `match_::start` drops the record it built when one exists in `Boarding`, while the check-out uses the fresh seed. No reachable path at `820ad9af`. | `tacsim:match_/mod.rs:599-611`, `:632-638` | Refuse when the two disagree. |
| T-13 — landed M.3 | A struck-through hit journals the chosen section's name beside another section's facet armour. | `arena:damage.rs:736-751`, `:800-809` | Name the armour's section on the `Hit` event. |
| T-14 — landed M.4 | `Arena::new` never tests the opening cell's `passable()`, and `from_pack` never checks that an entry cell has an in-limit passable neighbour; a unit so placed sits with 0 kph, no `HELD` and no ledger line. Unreachable on shipped content. S–M. | `arena:state.rs:2532-2755`; `tacsim:definitions/arena.rs:513-600`; `arena:motion.rs:345-347`, `:561-578` | `ArenaError::CellImpassable`, and a `from_pack` invariant refused by name at load. |
| T-15 — landed M.4 | The one-cell bound rests on `move_cost >= 100`, which only the tracked row guarantees; the wheeled row authors 80 and 90. | `arena:motion.rs:561-571`; `arena:clock.rs:295-327` | Size the substep on the true ceiling, or floor the cost in `ArenaTerrain::new`; pin it. Blocks `B-58`. |
| T-16 — landed M.4 | `carry.speed` survives an accel-to-brake flip and `carry.turn` a reversal of direction; under 1 cm/s and 1 millidegree. | `arena:motion.rs:461-504`, `:575-598` | Zero each on its sign flip. |
| T-17 — landed M.4 | The search refuses diagonals motion allows (the shoulder rule), which is the safe direction and is stated and tested nowhere. M. | `arena:path.rs:402-415`, `:502-529`; `arena:motion.rs:686-706` | A property test: every diagonal the search accepts under a limit, `refusal()` accepts. |
| T-18 — landed AE.1 | The shipped map's steepest orthogonal edge is 30 per cent (one edge) against limits of 45 and 70, so `TooSteep`, the shoulder rule and the follower's cliff memory never run in play. M, a pack version. | `tacsim/content/atlas/proving_ground.json`; `arena:slope.rs:117-119` | Author one bluff with `N15`; until then say in doc 16 §9 item 6 that the coverage is fixtures. |
| T-19 — landed D.4 | `FrameNotice::Contact`'s `class` is hard-coded `Unknown` on `Acquired`, also for a mesh row inserted `Classified`. Nothing reads it. | `arena:step.rs:1189-1197` | Pass the live class through. |
| T-20 — landed D.4 | The 10 m locked range is finer than a plot up to one pass old can carry (17 m at 60 kph), and `stale_clause` is silent at one pass. | `tacsim:match_/cockpit.rs:529-537`, `:1742-1761`, `:1940-1946` | Widen the step to what the target could have covered since the plot, floor 10 m. |
| T-21 — landed L.3 | `EndReason::LedgerOverflow`'s doc says "twice" the bound; the code is the bound. | `arena:state.rs:2163-2164`; `arena:ledger.rs:580-590` | Drop the word. |
| T-82 — landed M.1 | `intents.max_pending_per_unit` bounds a queue that is drained once a pulse, so the size of a burst a pilot may send is a function of the tempo: eight discrete orders per 5 s at `Quiet`, eight per 0.5 s at `Engaged`. Pre-existing since cut A.2; cut D.2 made the drop audible (`Too many orders at once; try that again.`) and proved only the common-boundary case at three cadences. | `arena:step.rs` (phase 2, `apply_intent`); 16b section 9's constant | Make the bound a rate in arena time, or drop it and let the inbox bound stand alone. |
| T-83 — landed M.2 | `sweep_held_losses` runs once a pulse from `frames()`, so the end of the loss hysteresis is a pulse boundary and not an arena time. With stock sensors a re-detection past the deadline is cancelled at `Quiet` (and since cut D.4 says `BA is unidentified again: X2.`) but swept at `Engaged` (`Contact BA lost.` then `New contact X2`): two tempos, two non-empty stories. The mechanism is S.2's and Q.1's; D.4's cross-tempo flicker fixture is arranged never to observe the fork. | `arena:step.rs:1869` | Make the hysteresis expiry a `DueKind` at its own timestamp, and drive the flicker fixture across the deadline at three cadences. |

### 2.2 Per-pulse cost at the scale target

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-22 — landed U.1 | Every unit has its own recurring `SensorPass` due event and `next_due_ms` has no floor, so the task wakes about once per unit per second whatever the tempo; the harness steps at a fixed period and never reads `next_due_ms`, so A2 measures a cadence the task never runs. M. | `arena:step.rs:1806-1816`, `:893-902`; `arena:perception.rs:1399-1412`; `tacsim/tests/harness.rs:1937-1940` | Drive the harness through `next_due_ms`; coalesce due events that fall inside one substep length and record the amendment to §3.1 phase 5. |
| T-23 — landed M.3 | Phase 4 takes a full `state_hash` every pulse before the `ended` guard and with nothing queued. | `arena:step.rs:527-532`, `:936-944` | Fill `action_hash` on first use inside phase 4, as the `ReflexTick` arm does. `Arena::event_hash` is not the equivalent place. |
| T-24 — landed U.1 | The two per-pulse tempo signals root a range where the squared membership test is exact. | `arena:perception.rs:1374-1380`; `arena:reflex.rs:732-744`; `arena:index.rs:31-35` | The squared form, with `reach − 1` for the strict one. |
| T-25 — landed U.1 | The pass stagger is `id % pass_ms`, so contiguous ids put every pass in the first 300 ms of each second. | `arena:perception.rs:1394-1412` | `splitmix64(seed ^ id) % cadence`; re-measure before touching a bound. |
| T-26 — landed M.4 | `sensors.max_signature_factor` defaults to 1.5 and no factor above 1.0 can exist, so the cull circle is 2.25 times the area it needs. | `tacsim:config.rs:517`; `mush-config/src/config.rs:1242`; `arena:perception.rs:226`, `:300`, `:906-911` | Default 1.0 and say so in the key table. |
| T-27 — landed M.3 | The whole monotonic designator book is folded row by row into every substep hash. | `arena:hash.rs:317-330` | A running digest updated in `DesignatorBook::assign`. |
| T-28 — landed M.3 | `set_unit` recomputes every condition band on every heat write, and before it knows the arena holds the unit. | `arena:state.rs:2893-2905`; `arena:heat.rs:230-239` | Recompute only when the plan touched a section or a facet cell. |
| T-29 — landed U.2 | `share_pass` folds the sharer's jamming above the return that would have made it free, and re-scans each receiver's bubble once per sharer; `jam_checks` has no bound. The mate's fold cannot move below the line test, because the budget feeds it. M. | `arena:perception.rs:1030-1065`, `:1147-1173`, `:1205-1210` | One jamming fold per unit per pass in a per-pulse table; `harness.max_jam_checks_per_pulse`. |
| T-30 — landed U.2 | `Frame::same_as` compares `age_ms`, which moves every pulse, so every pilot holding any contact is sent a whole keyframe every pulse; nothing bounds it. | `arena:step.rs:244-277`, `:1136-1149`, `:1732`; `arena:perception.rs:1476-1533` | Compare the age at one second, the only resolution a renderer prints; a frames-per-pulse assertion. |
| T-31 — landed B.5 in part (the per-feed latest slot; the per-session `Envelope` clone stays) | `deliver_to_session` deep-clones every event and retain-scans a 512-entry ring per cockpit frame for every session, and only the web bridge can replay. M. | `mush-session/src/manager.rs:706-726`, `:302-308`; `mush-gateway/src/web_bridge.rs:554` | A per-feed latest slot beside the ring, or record only for a session that has had a structured connection; a bound. |
| T-32 — landed E.1 | `alert_range_cm` derives to 6 km on a 3.6 km diagonal and `hostile_in_weapons_range` reads the 3 km reach, so `Engaged` means "somebody holds a classified hostile" and `Alert` is the classification delay. M. | `arena:state.rs:2779-2789`; `arena:reflex.rs:732-744`; `arena:clock.rs:134-140` | Cap the derived range at the window's diagonal and price `Engaged` off the fired weapon's inner band; report time in each tempo on a shipped-content match. |
| T-33 — landed B.2 | The web cockpit repaints every terrain cell on every keyframe (65,536 `fillRect` at the widest view) with no cache. Unmeasured. M. | `web/src/components/CockpitScreen.svelte:54-56`, `:80-139` | Profile at 256 cells; if it matters, an offscreen terrain canvas repainted on origin or zoom change. |
| T-87 (candidate, from cut J) | The candidate cull `SpatialIndex::in_range` walks a `(2 × span + 1)²` slot window per pass with `span = reach / cell_cm + 1`: 15,129 slots at the authored 60 cells (at most about 1.42 million slot reads a wake at 94 passes for 3,657 candidates) against 2,601 at the CI 24, 9,801 at 48 and 5,625 at 36, and 3,969 per sharer at the 30-cell mesh (361 at 8). A cost no trace change touches and no knob separates from the traces (every sight knob moves both); part of the per-window gap between the cohorts after J. M. | `arena:index.rs:313-327` | A coarser-bucketed cull, or a bucket side sized to the reach, measured by `candidates` and the per-window p99 at 60/30 on the CI lane; a companion to `Q26`'s answer, not a J sub-cut. |

### 2.3 The harness and CI

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-34 — landed U.1 (the report-only row; N5 is the remedy) — landed J.3 (closed on the number: 8.156 ms per wake, 2,994 traces and 19,858 cell probes at the peak, every one inside its bound; the per-window lens and the share-ops bound carry as `Q26`) — J.3b (the per-window lens 17.5 ms and the share-ops bound 768 settled on the owner's answer; the lane asserted) — J.3c (the lens 20 ms on the owner's second word) | The scale harness runs at 24 sensor cells and 8 share cells against the shipped 60 and 30. At the authored reaches with the AI on the field the recorded run is p99 26.4 ms against 25, 13,278 traces against 4,000 and 435,829 cell probes in the peak pulse. The choice has been carried for four cuts. L. | `tacsim/tests/harness.rs:1138-1184`, `:2515-2540`; `tacsim/content/sensors/visual_mk1.json:14`, `:18` | A report-only row at 60 and 30 now; the remedy is `N5`. |
| T-35 — landed U.1 | Harness units declare no locomotion (`mobility: None`), so the slope limit, the shoulder rule and their A* cost are in zero of 300 units; all 300 share one chassis and one sensor. M. | `tacsim/tests/harness.rs:1093-1136`, `:1467`; `arena:slope.rs:111-119`; `arena:path.rs:402-412` | A cohort table by unit id in the shipped shapes (tracked, legged, the extension variants), relief steep enough to refuse edges, and refusal counters in the report. |
| T-36 — landed U.2 | Cell probes are reported and never bounded; resident memory is printed over 30 s of arena time and never compared; the arena's frames go to a sink, so the arena-to-session fan-out that 16b §11 names as the risk has no number. M. | `tacsim/tests/harness.rs:109-110`, `:1621`, `:1634-1636`, `:1806-1814`, `:1946`, `:2428-2482`; `tacsim:config.rs:551-579` | `harness.max_cell_probes_per_pulse`; a long-run mode with an RSS growth bound; a third lane that boards N real sessions through `arena_bridge` and bounds bytes per frame and output events per pulse at 24 pilots. Landed U.2 with `max_output_events_per_pulse` at 512, not the planned 256: the lane's first measurement was 288 on the acquisition pulse (24 keyframes plus eleven contact-notice lines per pilot), so the default is sized on the measurement with headroom; the ruling is Q-U2. |
| T-37 — landed U.3 | The one measurement of the scale target runs weekly or on demand, never sets `TACSIM_ASSERT`, and compares nothing to the last run; its world lane asserts three counters hard-coded to zero and measures a passive domain. M. | `.github/workflows/ci.yml:345-367`; `tacsim/tests/harness.rs:99`, `:841-844`, `:885-891`, `:1028-1049` | A committed baseline and a weekly delta check (D-H-8 keeps the absolutes report-only); delete the zeros. |
| T-38 — landed L.4 | The LLM cost run is an env-gated early return the inventory gate cannot see, and it runs in no job. | `mush-contract-tests/tests/ci_gated_test_inventory.rs:14-40`; `tacsim/tests/harness.rs:2678-2692` | A third gate form with an allowlist row. |
| T-39 — landed L.4 | No `cargo doc` job and no rustdoc lints, in a tree that navigates by intra-doc links. | `.github/workflows/ci.yml`; `Cargo.toml:216-230` | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items`; the first run says how many links are broken. |
| T-40 — landed L.4 | CI never release-builds the deployed binaries, so the web-embed guard and thin LTO first run on the VPS. | `.github/workflows/ci.yml:41`, `:91`, `:128`; `mush-net-web/build.rs:33-47` | A release build with the `tls` features after `npm ci && npm run build`. |
| T-41 — landed L.4 | The simulator e2e suites wait on wall-clock sleeps (66 in eight suites), which is what load-sensitive means; six suites that flaked under load are on no list, and three tests race `classify_after_ms`. M. | `mushd/tests/sim_reflex.rs:499-507`, `:555`; `mushd/tests/support/mod.rs:21` | One `wait_until` helper and the arena waits converted; `classify_after_ms` set from the test config; a written list in 16b §0. |
| T-42 — landed E.4 in part (the `no line` edge, the `avoid` clause, `Spent` at three cadences) and B.2 in part (the resumed and rehydrated web cockpit) and M.1 in part (the split twin, the driven pose, `@domain/retire`) and U.3 (`FIRE` then `EXIT`, the edge set; closed) | Properties the code claims and no test holds: the split twin restarts with nothing in custody (`mush-gateway/tests/arena_custody_split.rs:520-533`); the driven pose is an inequality (`mushd/tests/arena_custody.rs:489-501`); `@domain/retire`'s refusal is asserted as `!Huh?` (`mush-sim/tests/deployed.rs:178-185`); `FIRE` then `EXIT` fires a `FIRE` the cockpit refused locally (`mushd/tests/sim_cockpit.rs:409-441`); the `no line` edge (`arena:reflex.rs:988-995`); the `avoid` clause of `first_step_is_open` (`arena:reflex.rs:1741-1743`, whose doc also misstates motion's rule); `EndReason::Spent` at one cadence only (`tacsim/tests/reflex.rs:2334-2420`); a resumed or rehydrated web cockpit (`web/src/lib/game.test.ts:1139-1396`); `tacsim`'s edge set (`tacsim/tests/composition_edges.rs`). M. | as cited | Each test beside the fix it guards; the edge set recorded as deliberately open in the R-a ledger row. |
| T-43 — landed B.3 | The TUI cockpit is driven over SSH only in the all-in-one binary; the split is production. M. | `mush-gateway/tests/` (no `@tui`) | One split test that opens `@tui`, presses `0` and asserts the cockpit pane. |
| T-44 — landed B.2 | `vitest` runs in a node environment with no DOM, so none of 32 components is rendered in a test; marker colours, the wedge and the lock ring are unpinned. M. | `web/vite.config.ts`; `web/package.json` | `jsdom` and `@testing-library/svelte`; mount `CockpitScreen` against the committed fixture. |
| T-45 — landed D.5 | Nothing cross-checks `docs/help` against the command table, which is how `B-68` and `B-17` stayed green for four days of cuts. M. | `mush-server/src/help.rs:84-110` | A contract test that scans the topics for command-shaped tokens against the registered verbs, with an allowlist. |
| T-46 — landed E.1 | Nothing closes the range when no carried weapon reaches the plot, and neither `is_dry` nor `spent()` notices; safe only because the coaxial's 1.2 km out-reaches the longest possible plot. M. | `arena:reflex.rs:1246-1255`, `:1930-1936`; `arena:damage.rs:1145-1173` | An `Engage` edge that closes to the loaded weapon's reach; a content test that every fieldable unit's shortest loaded weapon out-reaches sight plus mesh. |
| T-47 — landed E.1 | `Engage` pins the goal where the line first appeared and can never re-enter `Approach` (the edge is `range > 3 km`), so `engage_band_permille` governs nothing. M. | `arena:reflex.rs:1038-1069` | Closed by `B-05`'s pricing. |
| T-84 — landed M.2 | `Cadenced` in `tests/perception.rs` compares RENDERED sentences across cadences, which holds only because its observers stand still: `notice_line` reads bearing and band off a per-pulse frame, so a moving-observer fixture added there would fail for a reason that is not the engine's. Cut D.4's `Q6Replay` compares the staged `FrameNotice` lists instead. | `tacsim/tests/perception.rs:2435-2486`; `tacsim/tests/extensions.rs` (`Q6Replay`) | Compare staged notices per pilot in `Cadenced` too; keep the rendered lines for verbatim sentence assertions only. |

### 2.4 Lobby, lifecycle and cockpit

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-48 — landed D.3 | `lobby::leave` writes `Centre::default()` in place, a second writer of `Idle`, reachable from `Selection` and `Boarding`; the Q.8 tank reset had to be bolted on beside it. | `tacsim:match_/lobby.rs:673-717`, `:1953-1973` | `release_rows(ctx, Idle, None)`. |
| T-49 — landed D.3 | No staff verb clears a stuck `deployed` marker; the cure is a server restart. | `tacsim:match_/verbs.rs:48-58`; `tacsim:match_/mod.rs:988-993` | `@arena/release <unit>`: the void check-in and `release_seat`, refused while the unit's match is live. |
| T-50 — landed D.3 | An abort from `Selection` or `Boarding` leaves a `SIM UNIT` refit standing on the pod's tank until the next claim. | `tacsim:match_/lobby.rs:1941-1971` | `reset_stock_tanks` on every pre-`Running` end. |
| T-51 — landed D.3 | `ai_side_size` clamps to the pod cap and `largest_side` does not; no path reaches the disagreement. | `tacsim:match_/lobby.rs:389-436` | Clamp both. |
| T-52 — landed U.4 | The only non-standard match start is a raw-JSON `@spatial/place`, and every return to `Idle` undoes it. M. | `tacsim:match_/reset.rs:389-420` | `@arena/pose <slot>=<x> <y> [heading]`, honoured until the next `Idle`. |
| T-53 — landed U.4 | Every cockpit read is keyed on `ctx.actor` while the unit is resolved through the proxy driver; latent until a proxy-driven cockpit exists. | `tacsim:match_/cockpit.rs:406-413`, `:1625`, `:1962` and ten more | Resolve `(driver, unit, station)` once in `cockpit_command`. |
| T-54 — comment corrected in L.3; per-channel leases stay open (not built in AE.3: no unit of cut AE gained a second station) | A three-channel cockpit seat holds no K6 control lease: `submit_seat_handoff` stages nothing for a station whose channel set is not exactly one, so `forward_intent`'s step 4 passes on vacancy. The cockpit's module head says the path "asks K6 for the control lease". Re-read while writing this document, because two triage passes took the comment's word. M. | `mush-sim/src/seat.rs:697-712`; `tacsim:arena_bridge.rs:866-874`; `tacsim:match_/cockpit.rs:42-45` | Correct the comment now; build a lease per channel in the cut that gives a unit a second station. |
| T-55 — landed E.2 | `would_overheat` refuses a shot the engine would accept and clamp, for any shot that reaches the ceiling. | `arena:reflex.rs:1961-1972`; `arena:heat.rs:129` | The engine's own predicate. |
| T-56 — landed E.2 | An omitted `ttl_ms` becomes a 1,000 ms order, dead at the next decide tick; the forced tool schema requires the field, so only the JSON-in-text fallback can omit it. Plausible. | `tacsim:deliberation/request.rs:180`, `:329` | Default to `deliberate_min_ms`. |
| T-57 — landed U.4 (the bound at apply; `pub(crate)` not taken, both methods have integration-test callers) | The 256 KiB `arena.map` bound is a test against the shipped ground and refuses nothing; `frame_scan_tables` and `frame_contacts` are `pub` with test callers only. | `arena:step.rs:1661`, `:1673`; `tacsim/tests/cockpit.rs` | Enforce the bound where `map_frame` is built; `pub(crate)`. |
| T-58 — landed B.1 | An unknown `SCAN` band letter paints green on the web; the TUI leaves it uncoloured. Cannot fire today. | `web/src/lib/cockpit.ts:612-634` | An uncoloured fallback. |
| T-59 — landed D.1 and D.2 (the printed word; `tempo` stays on the frame) | From the live smoke, not re-verified: `STATUS` prints the arena's one global tempo word to every pilot, including a destroyed one, so `engaged` says somebody somewhere has contact. Plausible. | `arena:step.rs:259-298` (the frame's `tempo`) | Print the pilot's own alert state, derived from their own contacts; keep the cadence in the frame for the clients' timers. |
| T-60 — six of eight landed D.3; the two carried (the 50 m edge, `VIEW`'s footer) landed B.4 | Voice and small slips from the live smoke, not re-verified: `SIM ABORT` by one claimant in `Selection` wipes every claim; the board reads `aborted (match 3) — abort`; `That unit makes 54 kph; SPEED 0 to 54` while `SPEED -10` is accepted; a biped is named `Red Tank 1`; bare `LOCK` answers `No such contact.`; `EXIT` prints both the actor and the room line to one person; a range near a 50 m edge flips every pulse; `VIEW`'s footer is in window cells. M. | `tacsim:match_/lobby.rs`, `cockpit.rs` | One sweep with the transcript (`scratchpad/backlog/smoke/logs/ada.out`) open. |
| T-81 — landed E.1 | `cockpit::range_band` (read by the deliberation brief and inverted by `deliberation::order::RangeBand::ceiling_cm`, which the reflex controller reads for `max_range_band`) still bands against the weapon alone, while since cut D.1 the pilot's `contact_band` bands against the lesser of sight and weapon: a lead's `close` is 500 m and a pilot's is 200 m. F.3's "one distance" sentence is suspended by this row. The other half of `B-25`. | `tacsim:match_/cockpit.rs` (`range_band`, `contact_band`); `tacsim:deliberation/request.rs`; `tacsim:deliberation/order.rs` | Re-price the order ceiling with E.1 and make the two one function again. |

### 2.5 Session, network and clients

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-61 — landed B.5 | The retain-latest rule ignores `PanelFrameKind`, so a future delta would evict its own base keyframe. | `mush-session/src/manager.rs:302-308` | Require `Keyframe`. |
| T-62 — landed B.5 | `arena.map`'s retain marker can never fire (conn-scoped sends are not ring-recorded), so the clients' 2 s re-ask is what resumes a map; the resume paragraph does not say so. | `mush-session/src/manager.rs:239-243`; `tacsim:arena_bridge.rs:1845-1846` | Say it on `MAP_FEED` and in the crate's resume paragraph. |
| T-63 — landed B.3 | The cockpit tab's gate is a seated frame on the TUI and the manifest alone on the web; neither leaks and doc 16 records neither. | `mush-net-ssh/src/tui/app.rs:419-427`; `web/src/lib/stores/screens.svelte.ts:210` | Seated-once on both, as a §11 amendment. |
| T-64 — landed B.3 | `PanelStore::forget` is `pub` and leaves `view_cursors` and `resync_pending` behind. | `mush-net-ssh/src/tui/panels.rs:101-137` | Remove all three. |
| T-65 — landed B.2 | `MAP_HEAL_LATCH_S`'s rationale quotes frame rates the tempo config contradicts by five and ten times, which is what hid `B-59`. | `mush-net-ssh/src/tui/app.rs:97-99`; `tacsim:config.rs:350-353` | Correct the comment. |
| T-66 — landed L.1 | Three transport hooks are defaulted to permissive, so a second embedder ships with the cool-off off; `auth_failed` in the same traits is required. | `mush-net-web/src/hooks.rs:326-335`; `mush-net-ssh/src/hooks.rs:195-197` | Make them required. |
| T-86 — landed B.1 | After cut D the panels lag the line client: neither status block has a `receiver` row or prints `wreck` or `immobile` (both decoded and unused, though the help says the panel shows a destroyed tank); the web ring prints the regression as `X2 unidentified` with no old name, because the notice JSON has no `was` key and `NOTICE_WORDS` no entry for the kind; the web's `struck_spent` line has no section word; a blocked notice with a null `cause` prints the `occupied` wording; `noticeLine` indexes its word tables as plain objects. | `web/src/lib/cockpit.ts` (`noticeLine`, `statusLines`); `mush-net-ssh/src/tui/render.rs`; `tacsim:match_/cockpit.rs` (`status_frame`) | A `was` key and a `section` on the victim's kinds in `status_frame`; the rows on both panels; `Map` lookups; the parity goldens re-blessed once. |

### 2.6 Platform, perception and operations

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-67 — landed U.4 (`@find` and the gate; the lens is H6) | The kernel matcher resolves `#N` for any object and `*name` for any player with no observer test; 165 call sites against 40 `can_perceive` uses. `look` guards itself; `@find` consults no exclusion. L for the lens. | `mush-core/src/matchit.rs:35-70`; `mush-core/src/commands/look.rs:50-54`; `mush-core/src/commands/building.rs:770-785` | Now: `deployed::excluded` in `@find`, and a contract test that lists the player-facing call sites against an audited list. The lens is `H6`. |
| T-68 — landed U.4 | `mush-propagation`'s observer roster and its candidate side enumerate `SpatialState` with no exclusion consult, so the world-side detection pass can mint sightings on units an arena holds. Nobody stands in the muster domain today. | `mush-propagation/src/pass.rs:611-630`, `:801` | Filter both through `deployed::excluded`. |
| T-69 — landed L.2 | `[tacsim].ground_cell_cm`, `[tacsim.time.ephemeris]` and `[atlas].path_max_cells` are live, boot-read and in no shipped config; `REQUIRED_SECTIONS` does not require their blocks. | `deploy/instance/mushd.toml.tmpl`; `mush-config/tests/shipped_configs.rs:54-79` | Write the keys with their commentary; add `tacsim.time` and `tacsim.time.ephemeris`. |
| T-70 — landed L.2 | `max_slots_per_side = 12` in every shipped config is silently clamped to four pods. | `deploy/instance/mushd.toml.tmpl:286`; `tacsim:match_/lobby.rs:389-393` | Comment the key; log the clamp once beside `simulator centre ready`. |
| T-71 — landed L.2 | A mistyped `[tacsim]` header boots the layer off, and the only sign is one INFO line. | `mush-server/src/boot.rs:344-347`; `mush-config/src/config.rs:1579-1582` | WARN, and refuse an unknown top-level section within edit distance 2 of a known one. |
| T-72 — landed L.2 | dev2 keeps a hand-written config lane, a pre-split deploy shape and three gates alive; Q.7 had to edit it. Whether the box still runs an August binary could not be checked from the repository. | `deploy/dev2/`; `mush-config/tests/shipped_configs.rs:665-685`; `mush-config/tests/ipc_endpoint_validation.rs:225`; `mushd/tests/boot_composition.rs:216-222` | Delete the lane; a second box is an `instance.env`. |
| T-73 — landed L.1 | A fresh store has no scripted first staff login; `reset-instance.sh` stages nothing; no binary emits an argon2id hash. Two manual routes exist. M. | `deploy/README.md:384-402`; `scripts/reset-instance.sh:207-210`; `mush-config/src/accounts.rs:98-163` | `--bootstrap-staff <account>`, refused once any account exists, and `--staff` on the reset script. |
| T-74 — landed L.1 | The operator log says nothing when a match starts, logs `pre-auth timeout` for connections that already closed, and prints an INFO audit line per disabled deliberation. | `mush-net-ssh/src/server.rs:1398-1418`; `tacsim:arena_bridge.rs:1495` | INFO at `SIM START` and at `Running`; cancel the watchdog on close; DEBUG for the disabled outcome. |
| T-85 — landed L.1 | A first-boot pack that apply refuses is one `tracing::error!` line and the world comes up without the lot: a production boot with a bad shipped pack is not a boot failure, and a test sees a missing room four frames from the cause (found by cut D's sweep, when D.5's new exit check refused a `mush-housing` fixture). | `mush-content/src/module.rs:127-129`; `mush-testkit` | Fail the boot, or at least fail `TestWorld` loudly, on a refused first-boot pack; an owner-visible choice for production. |
| T-88 (from the 2026-09-26 deploy review) — landed AC.2 | The IC clock is hard-coded in the instance template (`[game].ic_ratio = 1`, `ic_epoch = "2020-01-01 00:00"`) and not an instance-env value, so an operator's edit to a rendered `mushd.toml` (dev1 carries `ic_ratio = 4`, `ic_epoch = "2026-09-22 00:00"` since 2026-09-22, never yet read by a boot) is silently reverted by the next `deploy.sh` render, and because the timeline is seeded from `[game]` only on an empty store, a wipe after that render freezes the template's clock into the new world; nothing but the config sets the epoch. S. | `deploy/instance/mushd.toml.tmpl:143-144`; `deploy/instance/render.sh`; `mush-server/src/timeline.rs:29-30` | `IC_RATIO` and `IC_EPOCH` as instance-env values rendered into the template (the example at 1 and 2020-01-01, dev1 at 4 and 2026-09-22 00:00 as the owner set it by hand), the completeness check and the provisioning smoke covering them. Cut AC.2. |
| T-89 (from cut AC.1's record) | Since the 2026-09-26 redeploy of `1ea6a58e` dev1 spends on the provider for the first time (AC.1 handed the world task the service): `tacsim_deliberate` (a malformed answer costs two calls, the original and one repair), `npc_dialogue_extra` (Sharon, where `ooc` lands every player), `npc_dialogue_minor` (Aaron) and `npc_summarize` run with no per-feature cap: `[llm.budgets.per_feature]` in the template names only `worldgen_room` and `npc_dialogue_major`, so those four sit under the shared daily `global = { tokens = 5_000_000, cost_usd = 5.0 }` and the `max_background = 3` / `max_background_queued = 12` caps alone. S. | `deploy/instance/mushd.toml.tmpl` (`[llm.budgets.per_feature]`), `mushd.toml.example` | Four per-feature rows in the template and the example with figures the owner sets (a deliberation cap sized to the lance lane's orders per minute), the provisioning smoke pinning that the rendered dev1 config carries them, and a boot line that says which features have no cap. |

### 2.7 Documents

| Id | The debt | Where | The payment |
|---|---|---|---|
| T-75 — landed L.3 | Doc 16 drifts: §1 and §8 name `mush-server/tests` for the A1 twin (it is `mush-gateway/tests/arena_story_split.rs`); §5 still says `Keyframe \| Delta`; §6 still calls deliberation the durable decision pipeline; §11's "Before IC play" still lists AI takeover, which N.1 and Q.7 built; four places and `docs/design/README.md:91-94` say seven seam messages where `arena_task.rs:133-146` carries eight; `arena_story_split.rs:124` counts the selectors wrong. | `docs/design/16-combat-arena.md:26`, `:85`, `:526`, `:558`, `:585`, `:608`, `:658`, `:690-692`, `:700` | One pass of §11 amendments. |
| T-76 — landed L.3 | The interface plan still says `MAX_SCREENS` 9, `ScreenId` `1..=MAX_SCREENS`, and that the digits are exhausted. | `docs/interface/02-implementation-plan.md:450`, `:1246-1248`, `:2418` | Three edits and a line that digit `0` is the cockpit in both clients. |
| T-77 — landed D.2 | The cockpit's module head counts twelve, fourteen and twelve verbs and its table omits `RANGE` and `BEARING`; `HELP STATUS` omits the callsign; the help never says designators fold case, and its `SCAN` sentence over-promises. The help also never says that moving makes a tank visible further out, or that facing matters, which cost the smoke's first-time pilot three of four duels. | `tacsim:match_/cockpit.rs:1`, `:10-23`, `:58-61`, `:3227-3229`; `docs/help/cockpit.md` | Four corrections and two sentences. |
| T-78 — landed L.2 | Four passages in the boot path describe a ruleset selector R3 deleted; `ruleset_modules`, `onboarding_ruleset` and a dead `strip_prefix("ruleset_")` survive; four test configs write a `[rulesets]` section nothing parses. | `mush-server/src/boot.rs:264-288`, `:327`; `mush-server/Cargo.toml:64`; `mush-server/src/config_ext.rs:162`, `:171`; `mush-contract-tests/tests/state_slot_owners.rs:388`; the three `cli_args.rs` and `mush-gateway/tests/authoritative_process_kill.rs:189` | Rename, rewrite, delete. |
| T-79 — landed L.3 | `README.md` still advertises two deleted rulesets, a `[rulesets].enabled` key and a scaffold phase; `docs/PLAN.md` leads with the same. M. | `README.md:18-29`, `:91-92`, `:109-119`, `:144` | Rewrite the front door around the one game; mark the plan historical. |
| T-80 — landed M.3 | `arena:path.rs`, `motion.rs` and `hash.rs` each state an invariant about `blocked`, `held` or a staged write that the code does not hold (`T-04`, `T-09`). | as cited there | Corrected with the code they describe; listed so the ledger row counts them. |

---

## 3. New functionality

The engine is ahead of the surfaces on it. The black box records everything a debrief needs and no verb
reads it; the order pipeline is complete and only a switched-off LLM can issue one; four key spots are
authored, digested and read by nothing; W8's `command.max_span` exists only in prose; the centre caps at
four a side on content while the config asks for twelve and the harness carries 300 units. So the top of
the ranking is mostly a surface for something built. Ranking is by what it gives a pilot in a fight per
line of work, with the scale target's one blocker (`N5`) placed where its dependents need it.

### N1. Name the ground — landed V.1

- **Pitch.** `The Ford`, `The Crossroads`, `North Barn` and `East Compound` go on the map, on `BEARING`
  and `RANGE`, on `STATUS` and into orders, so two pilots can say where they are in words.
- **Grounding.** `tacsim/content/arenas/proving_ground.json` authors four `key_spots`; they are typed
  (`tacsim:definitions/arena.rs:78-85`, cap 32), carried into arena state (`arena:state.rs:463`) and
  folded into the definition digest (`:520-521`). No behavioural consumer exists. Doc 16 §3.2 rules the
  ground surveyed: known to every pilot at check-out, its units not.
- **Sketch.** Carry the spots on `arena.map` and the map payload. `VIEW` marks those in the window with a
  letter and names them in a legend beside the grid (never in it, or the three renderers stop matching
  byte for byte), and names those just outside with a bearing. `BEARING <spot>` and `RANGE <spot>` are a
  second resolution arm that answers plainly; a name that is neither spot nor held contact keeps the
  byte-identical refusal. `STATUS` names the nearest spot.
- **Depends on.** Nothing. **Size.** S. **Risk.** The parity fixtures.
- **Owner's choices.** Legend or in-grid names (default: legend plus a one-letter marker). May a pilot
  name a spot they cannot see (default: yes, surveyed ground). More spots now (default: no).

### N2. The debrief: `SIM REPORT`, `SIM HISTORY`, `@arena/report` — landed V.2 and V.3; the hit rate landed O.3

- **Pitch.** A match ends in one line. This gives each pilot a report of their own match (shots, hits,
  what they killed, what killed them, how long) and the centre a history of its last matches.
- **Grounding.** `write_result` (`tacsim:match_/record.rs:374`) writes one `tacsim:match:<id>` document
  per match and only tests read it (`mushd/tests/arena_story.rs:1351`, `sim_solo.rs:545`). The ledger
  (`arena:ledger.rs:70-318`) is appended to `log:match.<id>` (`tacsim:arena_bridge.rs:1293`, `:1306`) and
  nothing reads it. Four crates already page a ledger log through `mush_core::read_cold_range`
  (`mush-core/src/ledger.rs:996`, with `recent_sealed` at `:821` and `typed_entries` at `:196`). W16 calls
  the match ledger an in-world artifact.
- **Sketch.** `tacsim:match_/report.rs` and two arms on `Selector::ALL` (`sim.rs:99-107`), both gated by
  `lobby::outside_centre`. `SIM HISTORY` reads result documents only. `SIM REPORT [<match>]` pages the log
  with the visited bound the precedents use and folds `LedgerEvent` through one pure function (unit-tested
  against a harness match with no world). The projection is the design: a player's report carries their
  own `fired`, `hit` and `miss` lines, the hits on their own unit, the public outcome and per-side losses,
  and nothing of the other side's ammunition, movement or detections. Staff read the whole through
  `@arena/report`. A match whose tape has been archived says so.
- **Depends on.** Nothing. **Size.** M. **Risk.** A report that starts as a ledger dump leaks; it is built
  as a projection from its first commit and the observer-rule test grows a report column.
- **Owner's choices.** What a player may read (default: their own projection plus the public outcome;
  staff the whole). Does the fog lift after an OOC match (default: no). History depth (default: 20).
  A panel feed (default: line text only).

### N3. `ORDER`: commanding the AI wingmen, and W8's knob — landed Y.1 and Y.2

- **Pitch.** `ORDER red-1 ENGAGE BA`, `ORDER ALL HOLD`, `ORDER red-2 WITHDRAW`, `ORDER red-1 ADVANCE The
  Ford`: one pilot and three machines become a lance.
- **Grounding.** `Objective` has four arms (`tacsim:deliberation/order.rs:66-88`) and two constraints
  (`:187-193`); `apply_intent` takes an order latest-wins (`arena:intents.rs:420-432`); `send_order`
  offers it (`tacsim:arena_bridge.rs:2130-2159`); `squads_of` cuts pod pairs (`deliberation/squad.rs:134-172`).
  The only issuer is the LLM seam, floored at 30 s, and the shipped server has `[llm].enabled = false`.
  Doc 07 §2's first case is the commanded wingman. `command.max_span` appears in doc 16 line 59 and 16a
  line 887 and nowhere in `crates/`.
- **Sketch.** A cockpit verb staged as `IntentKind::Order` through `forward_intent`, which already checks
  focus, the live `helm` grant and the epoch. Add `issuer: Option<ObjId>` to `Order` and resolve a target
  hint against the issuer's contacts first (`arena:reflex.rs:1330-1337` resolves against the squad
  commander today, and a pilot's tank is in no squad); `best_hostile` still chooses only among contacts
  the receiving unit holds, so a commander may point and a tank that cannot see still cannot see.
  `[tacsim.command].max_span` (default 4 squads per commander, with a per-minute order floor), validated
  by name at boot and asserted by the harness, as W8 says. A squad the pilot has no authority over takes
  the one byte-identical refusal. The acknowledgement says what the order means, because an ordered
  `WITHDRAW` still stands and fights for three ticks unless it carries `hold_fire`. `FOLLOW` is a fifth
  arm and a follow-on.
- **Depends on.** N1 for `ADVANCE <spot>`; cut E for machines worth ordering. **Size.** M.
  **Risk.** `Order` and `Objective` are hashed; the fixtures re-bless in one commit.
- **Owner's choices.** Who may order (default: the side's lead, and any pilot their own squad). The span
  (default 4). Does a human order outrank a live LLM order (default: yes, latest-wins). A radio line
  (default: no).

### N4. The reclaim — landed Y.3 and Y.4

- **Pitch.** A pilot whose link drops past the grace takes the tank back from the machine, with a report
  of what happened while they were gone.
- **Grounding.** The arena returns a unit on `Control { Live }` (`arena:intents.rs:358-365`); the world
  makes it one-way, because `Slot::left` is written from every route to `Released`
  (`tacsim:match_/lobby.rs:77-89`). Doc 07 §1.3 specifies the catch-up report. The smoke confirmed that
  revival inside the grace already works.
- **Sketch.** Split `Slot::left` into `left` (a deliberate `EXIT`, still one-way) and `dropped_at`. A
  `BOARD` from a dropped slot's account in a `Running` match re-runs the F.1 seat path and sends `Live`
  at a new epoch. `SIM STATUS` reads `flown by doctrine`. The report is built from arena state (contacts
  new and lost, sections against a small `at_release` snapshot, rounds spent, the controller's state and
  goal), rendered as one `Sys` block and carried on the frame as `catchup`. Doc 07 §1.4's invariants are
  assertions; `reclaim_min_ms` closes flapping.
- **Depends on.** Nothing. **Size.** M. **Risk.** A reclaim racing a `Released` in flight; the epoch
  table settles it.
- **Owner's choices.** `EXIT` stays one-way (default: yes). The tank comes back where it is (default:
  yes). The report is projected through the observer rule (default: yes).

### N5. The authored reach fits the pulse — landed J.1, J.2 and J.3 (steps 1 and 2; step 3, the work budget, not built: `Q26` answered by the owner on the bounds arm, J.3b)

- **Pitch.** The answer to `T-34`, the choice carried for four cuts: make the trace cheap before choosing
  between the reach and the bound.
- **Grounding.** `walk` allocates three vectors per call and by its own comment never returns early
  (`mush-atlas/src/trace.rs:359-459`, `:415-416`); the arena reads only `sight` and `visited`
  (`arena:perception.rs:980-987`, `:1191`). Arena terrain is immutable for a match
  (`arena:state.rs:76-111`). A pass is already a timestamped due event.
- **Sketch.** Three steps, each measured on `T-35`'s cohort at 60 and 30. (1) `sight_only`: shares
  `line_cells` and the per-cell rules, allocates nothing and breaks at the first blocker; proved by a
  differential test over every ordered cell pair of the shipped ground. (2) `arena:horizon.rs`: a
  per-match max-top and any-opaque pyramid, conservative in both directions, that proves a line clear or
  blocked without walking it and descends on partial opacity; a property test and byte-identical
  `TACSIM_ASSERT` ledgers. (3) Only if the honest number is still over: a work budget per substep,
  nearest candidates first, the remainder resumed as a due event whose cursor is hashed.
- **Depends on.** `B-56` (the canonical walk is what the differential test is against), `T-35`, `T-22`.
  **Size.** M, then L, then L. **Risk.** Step 3 is the first thing in the arena whose outcome depends on
  load; the cursor is state, hashed and driven by arena time, and it is built only on an owner's word.
- **Owner's choices.** Which gives: the 600 m sight, the 4,000-trace bound, or the 300-unit target
  (default: build steps 1 and 2, re-measure, and only then choose; if still over, step 3 keeps both).
  RAM per arena for the pyramid (default: accepted; `T-36`'s memory bound covers it).

### N6. Lances — landed Z.1 and Z.2

- **Pitch.** Squads of four with two more mechanical orders, so that twelve machines a side is an
  opponent and not twenty-four duellists.
- **Grounding.** `SQUAD_SIZE = 2` (`tacsim:deliberation/squad.rs:40`); the reflex machine has no notion
  of a neighbour; `BriefContact` carries no condition (`deliberation/request.rs`).
- **Sketch.** `[tacsim.arena].squad_size` (default 4). Two `Objective` arms the reflex machine can obey
  from state it already has: `Screen { sector }` (hold an arc, fight what enters, do not pursue) and
  `Support { of }` (stay within a range of a named unit and prefer its target). The brief gains the
  squad's own picture. A 12 v 12 harness cohort with the LLM stubbed asserts orders per commander per
  minute under `max_span` and p99 unchanged.
- **Depends on.** N3, `T-35`. **Size.** L. **Risk.** A doctrine vocabulary the owner does not want; two
  arms, then ask again.
- **Owner's choices.** Lance size (default 4). Whether these are the right two (default: ship them and
  watch them fight). May an LLM command in a match with humans (default: yes, doctrine is the fallback).

### N7. Six a side — landed Z.3

- **Pitch.** The cap is content: eight pods and eight tanks. Raise the centre toward the size the engine
  carries.
- **Grounding.** `slots_per_side` clamps to `PODS_PER_SIDE = 4` (`tacsim:match_/centre.rs:60`) while the
  config default is 12; `POD_ENTRIES` is a compile-time table (`centre.rs:66-75`); the arena pack authors
  eight tanks; `MAX_ARENA_UNITS` is 24. Callsigns (bijective base 26), `ai_side_size` and the squad cut
  all scale already.
- **Sketch.** The OOC pack authors the extra pods in the two team rooms, `POD_ENTRIES` follows, the
  arena pack authors the extra tanks on the ford road muster. Two pack versions with migration chains;
  the stock digests move.
- **Depends on.** N3 (machines with no commander are noise at this size) and `T-36`'s fan-out bound.
  **Size.** M. **Risk.** The item budget of the OOC pack was not read; 12 real cockpit frames a side
  through the world's output path has no number until `T-36`.
- **Owner's choices.** Six or twelve (default: six first; twelve is one more bump once the fan-out has a
  number). New rooms (default: no).

### N8. A side channel — landed Z.4

- **Pitch.** Each side's pilots share a channel for the match, aliased by callsign.
- **Grounding.** `mush_comsys::api::force_join` and `force_leave` are public
  (`mush-comsys/src/api.rs:65`, `:122`) and comsys is first in tacsim's composition. `docs/help/cockpit.md:59-60`
  already says a callsign on the radio is a person you can look up; there is no radio.
- **Sketch.** `force_join` at check-out, `force_leave` in the release sweep and on `EXIT`. Machines do
  not speak.
- **Depends on.** Lands with N7. **Size.** S. **Risk.** A jammed mesh and an unjammed voice is a fiction
  decision and is taken on purpose.
- **Owner's choices.** Jammed by ECM (default: no, revisit for IC play). Alias (default: callsign). A
  cross-side channel (default: no).

### N9. Replay — landed I.1 and I.2

- **Pitch.** Watch a finished match from its black box.
- **Grounding.** `PositionSummary` names every unit every `ledger.summary_ms` (`arena:ledger.rs:103-106`)
  and `on_chunk` already parses it (`tacsim:arena_bridge.rs:1306`); with `arena.map`'s terrain and the
  shot events that is a whole replay with no seam change.
- **Sketch.** `SIM REPLAY <match>` over N2's reader, stepping keyframes on the one map-window rule; a
  scrubbing screen on the TUI and the web. Interpolation between summaries five seconds apart is a
  renderer's drawing and is labelled as such.
- **Depends on.** N2, N1. **Size.** L. **Risk.** A replay is as public as its projection rule.
- **Owner's choices.** Is a finished OOC match's replay public and objective (default: yes; the tanks are
  back at spec and it is practice, and this is deliberately the opposite of N2's default, because a
  replay is sat down to and a report is handed over). `ledger.summary_ms` (default: leave 5,000).

### N10. The referee's levers — landed I.3 and I.4

- **Pitch.** Staff can start, drive, abort and reset a match and can see nothing while it runs.
- **Grounding.** Five switches (`tacsim:match_/verbs.rs:48-58`); `MatchMirror` holds tempo, time and
  counts and no positions (`tacsim:arena_bridge.rs:399-414`).
- **Sketch.** `@arena/tempo <match>=<period_ms>`, a floor under the adaptive tempo (rate independence
  makes slow motion a supported state; a true pause is a fourth clock state and is not built).
  `@arena/place <unit>=<x>,<y>`, refused with an impact scheduled against the unit, re-running the cell
  entry test, journalled as its own kind and hashed. `@arena/watch <match>`: an `Observer` arm on the
  `Frames` message, a type distinct from `Frame` with no path into a pilot feed, pinned by a contract
  test, built only after `T-36` bounds the fan-out.
- **Depends on.** N9 for the screen, `T-36`. **Size.** M. **Risk.** `@arena/place` writes a pose outside
  the motion path.
- **Owner's choices.** Pause or slow (default: slow). `@arena/place` at all (default: yes, in `Running`,
  nothing in flight). Announced in-world (default: journalled always, announced never).

### N11. `SIM MODE`: Hold — landed O.1

- **Pitch.** A side that keeps a named key spot uncontested for N seconds of arena time wins.
- **Grounding.** `eliminated` is the only positive win (`arena:damage.rs:1086-1105`); `EndReason` and
  `MatchOutcome` are closed enums (`tacsim:match_/record.rs:105-171`), which doc 16 §7 says is where a
  new arm goes.
- **Sketch.** A `SIM MODE` selector in `Selection`, announced like `SIM UNIT`; `modes: []` on `ArenaDef`
  so a pack declares what its ground supports. `Hold` is one range test per unit per decide tick against
  at most 32 cells. The objective is settled before elimination and `spent` after it. The AI fill takes
  `Objective::Advance` to the spot as its default goal, which is N3's machinery used by the arena on
  itself. `Defend` and `Escort` follow.
- **Depends on.** N1, N3. **Size.** L. **Risk.** A wide shallow diff through every `EndReason` consumer;
  the compiler finds it. A §4 amendment.
- **Owner's choices.** First mode (default: `Hold`). Lobby choice or arena property (default: both).
  Does elimination still end it (default: yes, settled second). Public in selection (default: yes).

### N12. `EMCON` — landed O.2

- **Pitch.** A tank carrying a pod jams its own side with no switch.
- **Grounding.** Doc 16 §11 X.4 and Q.6: powered pods within their radius of the observer fold into
  detection and the mesh; `line_tank_ecm` is fieldable; `tacsim.power` is the precedent for a toggled
  component state.
- **Sketch.** A `tacsim.ecm` capability state, `EMCON on|off` as a discrete intent on `Channel::Sensors`,
  one clause in the two jamming folds, one ledger kind, one notice that says what switching off costs,
  one `STATUS` line. The stock digests do not move. Posture (hull-down scaling the presented height) is
  the follow-on and is not scheduled: it moves the hit curve for every shot and every hash.
- **Depends on.** `B-22` for the words. **Size.** S–M. **Risk.** Low.
- **Owner's choices.** Default state at check-out (default: on, as today).

### N13. The range — landed O.3

- **Pitch.** Targets that do not shoot back, and a match that does not end when one is hit.
- **Grounding.** An unarmed stationary unit is not `out_of_action` until wrecked
  (`arena:damage.rs:1053-1067`); `spent` excludes an unarmed side; `refit_unit` can field a type on a
  machine slot, which the lobby cannot ask for today.
- **Sketch.** `target_mk1` in the fielded catalog (the stock sections, no weapon, zero speed) and
  `SIM RANGE [<n>]`, which fields targets on the machine slots. Ends when every target is wrecked or on
  `SIM ABORT`; N2's report gives the hit rate. Check by test that a `UnitSpec` with no weapon survives
  `create_unit`, `refit_unit` and check-out.
- **Depends on.** N2. **Size.** M. **Risk.** Low, mostly content.
- **Owner's choices.** A mode of the centre or a room (default: a mode). Still or shuffling (default:
  still).

### N14. `@macro` — landed AD.1

- **Pitch.** `@macro l=lock $1`. Platform-wide, most useful in a cockpit whose action verbs are typed on
  every surface by design.
- **Grounding.** `AliasStage` and `add_alias_stage` exist (`mush-core/src/cmd.rs:377`, `:626`). A narrow
  grep found no player-facing alias verb; if one exists under another name this is smaller.
- **Sketch.** `@macro <name>=<line>`, read and `/del`; one expansion only (a macro that expands to a
  macro is refused); `$1` to `$3`; never over a registered verb; 20 per player.
- **Depends on.** Nothing. **Size.** M. **Risk.** A macro must never reach a verb its player could not
  type.
- **Owner's choices.** Platform-wide (default: yes). Positional arguments (default: three).

### N15. Content breadth, with the legged cost row — landed AE.1, AE.2 and AE.3

- **Pitch.** Six unit types and three weapons prove the mechanisms. A scout, a heavy, a second biped, an
  autocannon, a missile with a flight time and one capped drone make a catalog.
- **Grounding.** `tacsim/content/` and the extension tables at `tacsim:definitions/content.rs:162-267`;
  W2 and `arena.max_units_by_class`; `B-58`, `T-15`, `T-18`.
- **Sketch.** Three tranches. Chassis, which pays `B-58` and `T-15` and authors `T-18`'s bluff. Weapons,
  which makes `FIRE`'s default weapon a choice. One drone under W2's cap. No infantry: at 10 m cells with
  one unit per cell the model has nothing to say about a person.
- **Depends on.** `T-35`. **Size.** M and L. **Risk.** Low; definitions are versioned with migration
  chains.
- **Owner's choices.** Infantry ever (default: no). Drones as content (default: one, capped).

### N16. Per-pulse durability and resume — landed AF.1, AF.2 and AF.3

- **Pitch.** Doc 16 §11's first "Before IC play" item: a restart stops voiding the match.
- **Grounding.** D-16-3. `Arena` and `UnitState` derive `Clone, Debug` only (`arena:state.rs:1015`,
  `:2282`); the ack-gated ledger lane exists (`tacsim:arena_bridge.rs:281-343`); every substep has a
  state hash; `write_restart_abort` is what would stop voiding (`tacsim:match_/record.rs:450`).
- **Sketch.** `Serialize` on the state tree behind a versioned `snapshot` module whose deserialised
  `state_hash` must equal the recorded one; the definition and terrain are re-derived from the digest.
  An `arena -> world Snapshot { at_ms, hash, bytes }` message every `arena.snapshot_ms`, written on the
  arena writer lane with disjoint key ownership. Boot re-creates the arena from the last snapshot at or
  after the last acked chunk, replays the ledger tail, and re-opens the task; a hash mismatch voids
  loudly. Pilots return through N4. The arena still holds no store handle.
- **Depends on.** N4; cut M's hash and timestamp work. **Size.** XL. **Risk.** A snapshot that diverges
  silently; the test snapshots every pulse of a scripted match and asserts a byte-identical ledger from
  any resume point.
- **Owner's choices.** Cadence (default: every 5,000 ms plus tail replay, not every pulse). Do OOC
  matches resume (default: yes, it is the cheapest proof). Re-read `deliberation/mod.rs:54-88` when this
  lands, because its second argument stops being true.

### N17. `VIEW BRIEF`

- **Pitch.** The frame in sentences, for a screen reader and for a pilot who would rather read.
- **Sketch.** The ground on eight bearings, run-length described, then the contacts as `CONTACTS` words
  them, then the key spots. A second renderer of one frame; nothing new crosses the seam.
- **Depends on.** N1. **Size.** M. **Owner's choices.** Build it before a player needs it (default: yes,
  small, after the fighting features).

### N18. First-time hints

- **Pitch.** One sentence naming the next verb at five points, once per account, `@tutorial off`.
- **Depends on.** Nothing. **Size.** S. **Owner's choices.** Now or with the second player (default: with
  the second player; `T-77`'s two help sentences are the part worth doing now).

### 3.1 Not before a design conversation

| Id | Proposal | What the conversation must settle |
|---|---|---|
| H1 | The first IC fight: an arena on real ground with units that belong to somebody. An arena can already stand on a real atlas region, `@arena/start` already runs the lifecycle over arbitrary units, and check-in already commits a wreck and spent rounds. A `TacticalUnit` has no owner and the match lifecycle has no edge to `mush-economy`. | Is a destroyed unit lost or a repair bill (doc 10 §10 says meaningful loss softened by insurance). Who may start a fight. Proving ground with owned units first, or real ground. Whether the ownership layer lands here or in its own conversation. N16 is arguably a prerequisite. |
| H2 | Wrecks and salvage: a second `SalvageSource` beside `SimSalvageSource` (`tacsim:composition/salvage_source.rs:19-60`). | Who may claim a wreck they did not own; hangs off H1 and the ownership layer. |
| H3 | Rearm and repair with a price: two verbs on `refit_unit`'s governed write and the first consumer of the ten commodity families. | Instant or timed; anywhere or at a facility; hangs off H1. The free OOC path and the priced IC path are one lane with two policies. |
| H4 | The 10 km arena. The grid takes it; four budgets sized against 256 do not, and 1M cells of authored relief is about 2.8 MB against a 1 MiB pack cap, so the relief must be generated. | 10 m or 25 m cells (25 m silently changes every tuned fight). Whether it is wanted before a fight that needs the room. |
| H5 | Live spectating for players, including a dead pilot watching their own side's shared picture. | Does a spectator see the world or a projection. The strict reading is replay only, which N9 already gives. |
| H6 | A perception lens on `match_object`. | This is the Epistemic Engine's kernel seam (observer-dependent identity); that conversation shapes it. `T-67`'s audit gate is the part built now. |

---

## 4. Execution order

Fourteen serial cuts and a tail. Ledger letters used before this document: R G P H A T C K S W N X F Q;
the twelve below use the rest of the alphabet, and the review cut AA (after E) opens the second series, the owner's ruling cut AB (after Y) follows it, and the owner's fix cut AC (after I, before the 2026-09-26 dev1 redeploy) follows that; the tail's five cuts AD to AH (after O, started on the owner's word of 2026-09-27) close the second series, with the fix cut AI (`B-84`, from cut AE's sweep) placed after AF.
Each cut is a functional boundary of roughly cut F's size: three to five sub-cuts of 1.5k to 3k lines
with their tests. 16b §0's build discipline applies unchanged: the workspace green at every sub-cut, the
gates re-blessed in the commit that moves them, a reviewer that did not write the cut, one commit and one
push per sub-cut, a ledger row in 16b §9 per cut, and doc 16 §11 amended in the cut that makes a sentence
of it untrue. A cut that moves `state_hash` re-blesses the cross-tempo fixtures once, in its last
sub-cut. "Gateway" below means the deploy needs a full restart of the gateway half and not a server-half
reload, because the TUI, the session layer or the embedded web bundle changed.

### Cut D: the cockpit and the lobby tell the truth

Landed 2026-09-21 at `5703862f`; see 16b section 9, row D. Deferred: the item half of `B-12` (details
authored on items stay unreadable until the owner rules on DARK, OPAQUE and carried things), two of
`T-60`'s eight slips (to B.4), the commander's half of `B-25` (now `T-81`, E.1), and what stays of
`T-59` (`tempo` is still a frame key that `Frame::same_as` compares); the record also added `B-74`,
`B-75` and `T-82` to `T-86` from the judges' residuals.

**When it lands.** A seated pilot never reads a fact their sensors did not earn from the lobby board;
never reads a weapon they no longer carry; is told every refusal with its reason on all three surfaces;
a destroyed pilot is told what they can do; `l` looks; the hall's sign can be read.

- **D.1 The board and the panel.** `B-01`, `B-02`, `B-06`, `B-19`, `B-25`, `B-28`, `B-29`, `B-30`, `T-59`.
- **D.2 Refusals reach the pilot.** `B-03`, `B-07` (server and `cockpit.ts`), `B-15`, `B-16`, `B-17`,
  `B-18`, `T-77`.
- **D.3 The lifecycle's ends.** `B-32`, `B-33`, `B-34`, `B-35`, `B-36`, `B-31`, `T-48`, `T-49`, `T-50`,
  `T-51`, `T-60`.
- **D.4 What a contact is called.** `B-20`, `B-21`, `B-22`, `T-08`, `T-19`, `T-20`.
- **D.5 The way in.** `B-11` with its pack-apply check, `B-12`, `B-68`, `T-45`.

Depends on nothing. Gateway: yes (`web/src/lib/cockpit.ts`). Packs: `neighborhood.ooc_area` v5 to v6,
`neighborhood.staff_lobby` v3 to v4. Hash: D.4 moves it (`T-08`). Assumes Q1, Q2, Q3, Q4, Q6, Q12 to Q17,
Q28, Q36 at their defaults. Size: large, five sub-cuts.

### Cut E: the machines fight on the shipped ranges

Landed 2026-09-22 at `b18ee30d` (the sweep's one repair, `145296c2`, is `mush-bot` test hygiene); see 16b
section 9, row E. Deferred: the rest of `T-42` (the split twin, the driven pose, `@domain/retire`, `FIRE`
then `EXIT`, the edge set: cuts M and U), a signal reason that tells the commander WHY a `B-41` end
happened and the bridge's late or missing `mind link dropped` for it, a machine that searches the wreck
it knows it killed, a lone hunter's orbit of its next cell in water, `entry_cell`'s impassable fallback
and `far_entry_cell`'s shared scan budget, the second-stop rule for remembered cells, firing on the move
while closing, `T-04` with `B-27` (cut M), `B-58` and `T-18`; every one is written out in the E row's
Deviations cell.

**When it lands.** One pilot against the fill meets machines that close on a contact, break off when
hurt, stop where they meant to, remember what stopped them, and finish a match among themselves.

- **E.1 Ranges.** `B-05`, `B-39`, `T-47`, `T-46`, `T-32`, `T-81`, and the shipped-scale reflex test.
- **E.2 Orders, rally and knowledge.** `B-40`, `B-41`, `B-45`, `T-55`, `T-56`.
- **E.3 Driving.** `B-42`, `B-43`, `B-44`, `B-46`.
- **E.4 The endgame.** `B-04`'s harness case and the fix it points at, and from `T-42` the `no line`
  edge, the `avoid` clause and `Spent` at three cadences.

Depends on D.3 (`B-33`, `B-34`). Gateway: no. Packs: none. Hash: moves. Assumes Q8, Q9, Q10, Q11.
Size: large.

### Cut AA: the machine, the lobby and the watchdog do not lie

Landed 2026-09-22 at `61597e06`; see 16b section 9, row AA. Deferred: a reach term in `best_hostile`
for a machine that cannot close (it can still aim at a held plot past its `release_cm` and never fire),
the immobile machine's refused drive and stand-off planning, the mobile machine boxed in by wrecks or
terrain, a lead who disconnects or `EXIT`s mid-match keeping the slot (cut B.4 or later, with the
"first pilot" wording), the journalled forced void, the mushd-level stalled-terminal custody lens that
`B-79` listed, and the owner's confirmation of Q41 (AA.3 was built on the default; every one is written
out in the AA row's Deviations cell).

**When it lands.** A tank that cannot move but can shoot still fights out to its allowed reach and can be finished off only by killing it; a side whose lead was dropped at the boarding deadline can still concede with a result; a pulse that overruns the watchdog while ending the match is voided and the tanks restored, as doc 16 §3.1 says.

- **AA.1 The immobile machine.** `B-76`, and the `arrived` question for a hurt immobile tank in `Withdraw` (`withdraw_stand`'s `stand_cm` parks it at 450 m too).
- **AA.2 The dropped lead.** `B-78`, with `leave` and `board_deadline` sharing one succession rule.
- **AA.3 The stalled terminal pulse.** `B-79`, on the owner's word that doc 16 §3.1's ruling stands for a finished match.

Depends on E (E.1's `engage_cm` and `release_cm` split is the gap AA.1 closes, E.3's `hold_routeless` skips the immobile unit, and E.4's `B-04` harness case is where an immobile armed machine holding its side up may show; E.4 was already building when the review arrived, so AA.1 tells its own builder). Gateway: no. Packs: none. Hash: AA.1 moves it (`row.state.code()` for the immobile machine; `motion_disabled` is already a hashed byte), so AA.1 re-blesses the cross-tempo fixtures once; AA.2 and AA.3 touch nothing hashed. Assumes Q8 to Q11 at their cut E values, `stall_ms = 2000`, and no change to the concede gate (doc 16 §473). Size: small, three sub-cuts, each S. Named from the second series because every single letter is a ledger row already.

### Cut B: three clients a pilot can fight from

Landed 2026-09-23 at `db273b1c`; see 16b section 9, row B. Built as five sub-cuts, the world-task and session items listed under B.4 (`B-13`, `T-31`, `T-61`, `T-62`) landing as B.5. Deferred: the other half of `T-31` (the per-session `Envelope` clone), the rest of `T-42` (the split twin, the driven pose, `@domain/retire`, `FIRE` then `EXIT`, the edge set), a timed-out query keeping its typed `Busy`, the TUI damage pane at 120x34 showing three of its five sections (an owner call, 16b row B), and `B-49`, `B-50`, `B-77` to cut M on the defer queue's shape; it added no row, its residuals being listed in 16b section 9, row B.

Merged over the owner's cut L (PR #1, `68cb15dd`) on 2026-09-23; the merge's sweep: Step 3: 315 test binaries and 43 doc-test targets (359 result lines), 5818 passed / 0 failed / 82 ignored. Step 4 (mush-bot): 13 binaries and 1 doc-test target, 262 passed / 0 failed / 0 ignored. Web: 44 files, 493 tests passed. ts_export: 2 passed. Harness: 2 passed, arena p99 6.806 ms.

**When it lands.** A web pilot gives every order from screen 0 and can tell two hostiles apart at the
default zoom; an 80-column TUI keeps its contacts pane and wraps its rows; a lost map heals by itself; a
line pilot's input line is left alone; a player who types ahead is served.

- **B.1 The web composer.** `B-08` with its test and help changes, `B-09`, `B-60`, `B-61`, `T-58`, `T-86`.
- **B.2 The map heals and the web is tested.** `B-59` on both clients, `B-80`, `T-44`, the resumed-cockpit cases
  of `T-42`, `T-33` (profile first), `T-65`.
- **B.3 The TUI at 80 columns.** `B-10`, `B-62`, `B-63`, `B-64`, `B-65`, `B-66`, `T-63`, `T-64`, `T-43`;
  `B-64` carries `B-80`'s per-match reset when the TUI's notices pane moves onto the frame's array.
- **B.4 Line mode and the session layer.** `B-67`, `B-38`, `B-14`, `B-13`, `B-23`, `B-24`, `B-26`, `B-81`,
  `T-31`, `T-61`, `T-62`, and the two slips `T-60` carried (the 50 m edge flicker, `VIEW`'s footer).

Depends on D.2 (the notice fields). Gateway: yes, all of it. Packs: none. Assumes Q5, Q7, Q20, Q21.
Size: large.

### Cut L: the operator is never locked out

**Status:** L.1–L.4 complete. Integration record:
[PR #1](https://github.com/faenix7/rustmush/pull/1).
The frozen Linux run at `5f8353b3` completed 31 of 34 gates, including
66 Redis tests, real bootstrap/provisioning, TLS, strict rustdoc, Clippy and three
release binaries. Its workspace reported 5,747 passed, seven failed and 82 ignored;
the recovery fixture and interrupted bot run also failed. Those failures remain
recorded. Six test-file corrections plus upstream AA.3 are validated at `ffa12f6b`:
all six corrected targets passed 92 tests, the clean bot suite passed 262, and
TacSim passed 884 with two ignored on both Linux and Windows. Matching SPA,
affected-package Clippy and strict workspace rustdoc passed. All twelve final
Linux gates passed, including three TLS release binaries and both report-only
harness cases. Final arena p99 was 14.998 ms against the 12 ms target, with timing
assertions disabled; all deterministic report fields matched earlier runs.
Private Redis/build-cache cleanup was verified. AA's later documentation merge
changes no production or test code; GitHub CI remains blocked by billing.
See [16b's as-built record](16b-combat-implementation-plan.md#cut-l--design-note-2026-09-22-before-implementation)
for Windows/AA history and the [validation report](../reviews/cut-l-2026-09-22/validation.md)
for the retained failures, corrections, resource incident and final evidence.

**When it lands.** An operator can lift a tempban, reset an instance without changing its host key, make
the first staff account from the command line, see a match start in the log, and trust the deploy's
bundle check; the repository's front door describes the game that exists.

- **L.1 The box.** `B-69`, `B-70`, `T-73`, `B-71`, `B-72`, `B-73`, `T-66`, `T-74`, `T-85`.
- **L.2 Config and the second lane.** `T-69`, `T-70`, `T-71`, `T-72`, `T-78`.
- **L.3 Documents.** `T-75`, `T-76`, `T-79`, `T-21`, `T-54`'s comment.
- **L.4 CI.** `T-39`, `T-40`, `T-38`, `T-41`.

Depends on nothing; it may run beside E or B if two builders are ever wanted. Gateway: yes (`IpGuard`,
logging, hooks, the websocket close code). Packs: none. Assumes Q30, Q31, Q33. Size: medium.

### Cut M: the arena's clock, hash and record

Landed 2026-09-23 at `0871e0ab`; see 16b section 9, row M. Built as the four sub-cuts below, each committed and pushed on its own green gate (M.1 `0d896ec6`, M.2 `c7cab0a9`, M.3 `2eb19757`, M.4 `0871e0ab`); the biped was NOT re-versioned (its legs have carried the pair rule since X.2, so only the five tank packs went to version 2), `T-12` landed as a record replace rather than a refusal, `T-82` as a rate counted on the order's source, and `B-44` was reopened by `T-02`'s tick-stamped startup and re-closed in M.2 with a stopping-distance look-ahead. Deferred: the journalled forced void (AA.3's residual, a library change), the `blocked` wire key on the cockpit frame and the seed-derived contact id on the keyframe's row key (both client-touching cuts; the seed on the result document is staff-only meanwhile and `N2` must not print it), the pod and filter condition read at check-out only, `CellOccupied`/`PoseOffWindow` naming a unit to the centre, the -1 cm tail-floor asymmetry of `axis()` (M.3 only stops it re-arming `held`), `T-18`'s bluff, the absolute substep lattice (Q24) and the rest of `T-42` (`FIRE` then `EXIT`, the edge set; cut U); every one is written out in the M row's residuals.

**When it lands.** A match's digest can be re-derived from its log and a test proves it; an abort under
load cannot discard a match; no order is lost to a buffer; every timestamp the arena can make
rate-independent is; the hash covers what gates an outcome; A sees B exactly when B sees A.

- **M.1 The seam.** `B-48`, `B-49`, `B-50`, `B-77`, `T-11`, `T-12`, `T-82`, and from `T-42` the split twin, the driven
  pose and the `@domain/retire` assertion.
- **M.2 Timestamps.** `T-01`, `T-02`, `B-51`, `T-10`, `T-83` with `B-74` and `T-84`, `T-03`'s amendment and test, `T-05`'s amendment
  and harness assertion.
- **M.3 What the hash and the ledger cover.** `T-04` with `B-27`, `T-06`, `T-07`, `T-09`, `T-13`,
  `T-23`, `T-27`, `T-28`, `T-80`.
- **M.4 Weapons, sight and entry.** `B-47`, `B-52`, `B-53`, `B-54`, `B-55` with `T-26`, `B-56`, `B-57`,
  `T-14`, `T-15`, `T-16`, `T-17`.

Depends on nothing; it is placed here because it moves every hash and every later cut re-blesses on top
of it. Gateway: no. Packs: a new version of the five tank definitions and the biped (`B-47`), with the
digest pins. Assumes Q19, Q22 to Q24, Q32. Size: large.

### Cut U: measure the game as shipped

Landed 2026-09-24 at `a9fe77c4`; see 16b section 9, row U. Built as the four sub-cuts below, each committed and pushed on its own green gate (U.1 `6ea71b5b`, U.2 `297294bf`, U.3 `cfa01af4`, U.4 `63dc804b`), then the full sweep with one test-hygiene repair (`a9fe77c4`). Deferred: `T-57`'s `pub(crate)` (both methods have integration-test callers; the bound at apply closes the row), the matcher lens itself (H6, over the audited `none`, `locality` and `staff` rows of `matcher-call-sites.toml`), `B-58`'s per-locomotion cost layer, an engaged fan-out pulse (the lane measures the opening only), the Linux baseline (the owner's to bless on their box), and two owner questions raised and not assumed, Q-U1 and Q-U2 (section 5).

**When it lands.** The harness drives units shaped like the shipped catalog through the loop the task
runs, reports the authored reach beside the reduced one, bounds cell probes, memory, jamming checks and
the arena-to-session fan-out at 24 pilots, and the weekly job fails when a number moves.

- **U.1 The cohort and the loop.** `T-35`, `T-22`, `T-25`, `T-24`.
- **U.2 Bounds.** `T-36`, `T-29`, `T-30`, `T-34`'s report-only row.
- **U.3 The job.** `T-37`, and the tests of `T-42` not yet placed (`FIRE` then `EXIT`, the edge set).
- **U.4 The world side.** `T-67`'s gate and `@find`, `T-68`, `T-52`, `B-37`, `T-53`, `T-57`.

Depends on M (one re-bless of timestamps after the stagger and the coalescing). Gateway: no. Packs: none.
Assumes Q25, Q27, Q29, Q34. Size: medium to large.

### Cut V: talk about the fight

Landed 2026-09-24 at `14430594`; see 16b section 9, row V. Built as the three sub-cuts below, each committed and pushed on its own green gate (V.1 `23dcac43`, V.2 `2ff2d8eb`, V.3 `14430594`), then the full sweep with no repair (its one failure a `mush-server` world-task timing flake, green on both reruns). Six corrections to this section's sketches are recorded in the V row's design note: the debrief is folded as the tape passes the world and written as one report document per pilot at check-in rather than paged back from the store; the callsigns come off `Arena::callsigns()`; the history is a RAM ring of twenty rows loaded from the boot documents; the marker alphabet is 22 letters and a spot's name is checked at load; `VIEW` names every undrawn spot; a pilot's shot line carries no range and no damage figure. Deferred: the width of the designator-shape rule on spot names (it refuses `Ridge` and `Barn`; a candidate row), the rubble `x` and door `+` glyphs sharing a byte with the hostile and friendly markers (pre-existing; a candidate row), the `SIM HISTORY` row wrapping past 80 columns with its suffix (an owner call), the boot skipping its restart write over a finished match whose `deployed` marker survived and deleting the reports of the rows it evicts, `Report.kills` printed nowhere, and the fold of a chunk the ledger later refused; every one is in the V row's residuals.

**When it lands.** A pilot can say `The Ford`, steer by it, and read after a match what they did and
what was done to them; the centre keeps a history.

- **V.1** N1 on the feeds, `VIEW`, `BEARING`, `RANGE`, `STATUS`, and the parity fixtures.
- **V.2** The result reader and `SIM HISTORY`.
- **V.3** The ledger fold, `SIM REPORT`, `@arena/report`, and the report column of the observer test.

Depends on D (the notice and frame shapes), M (`T-07`'s seed and draw are in the record). Gateway: yes
(the legend on the TUI and the web). Packs: none. Assumes N1's and N2's defaults. Size: medium.

### Cut Y: command the machines, and come back to your tank

Landed 2026-09-24 at `2bfa61db`; see 16b section 9, row Y. Built as the four sub-cuts below, each committed and pushed on its own green gate (Y.1 `bdcde6eb`, Y.2 `5e2f6793`, Y.3 `19aed6cd`, Y.4 `47879f39`), then the full sweep with one repair (`2bfa61db`, test code and doc comments only: the observer test's claim that the catch-up's cell equals the carrying frame's corrected to the re-seat's cell within a speed bound, and the shooting test made to wait for its impact notices). The corrections to this section's sketches are recorded in the Y row's design note: a non-lead orders the one machine in their pod pair rather than "their own squad" (`squads_of` makes a four-pod side one squad); the seat is authorised once on the issuer's own tank and the order offered per unit, since `forward_intent` checks focus on the ordered unit; the span is concurrency read off the world's `OrderWatch.issuer` with a per-minute rate beside it, both enforced at the world and never in the arena; the drop is told to the slot by the disconnect hook and `EXIT` clears it; the reclaim needed a stale-control gate and the roster re-add; the catch-up baseline is taken at the first `Linkdead` rather than the `Released`, and the report is carried through a held pulse. Q42, Q43 and Q44 are recorded in section 5, each built on its default. Deferred: `FOLLOW` as a fifth arm and `ORDER <squad> RELEASE` (Q44, cut Z), doc 07 §1.3's heat changes and actions taken in the report, the split of `contacts_more` into two wire fields, `issued_ms` on the `ordered` line for an exact per-order harness count, and `B-81` (the billet that survives a drop, which the reclaim now relies on); every one is in the Y row's residuals.

**When it lands.** A pilot orders their side's squads from the cockpit under W8's span; a pilot who
dropped boards again, takes the tank back where it is, and reads what happened.

- **Y.1** `ORDER` on the four arms, the `issuer` hint, the byte-identical refusal, the help.
- **Y.2** `[tacsim.command].max_span`, validated at boot, asserted in the harness.
- **Y.3** N4: the slot split, the re-board, `Live` at a new epoch, `reclaim_min_ms`.
- **Y.4** The catch-up report on line mode and on the frame.

Depends on E, V.1. Gateway: yes (`catchup` on the clients). Packs: none. Hash: moves (`Order.issuer`).
Assumes N3's and N4's defaults. Size: large.

### Cut AB: the 120-column floor

Landed 2026-09-25 at `d7caa5cb`; see 16b section 9, row AB. Built as one sub-cut, committed and pushed on its own green gate (AB.1 `d7caa5cb`), then the full sweep green on its first round with no repair; `B-10`'s eighty-column layout is superseded, the B row's 120 by 34 damage pane residual closed, `Q-AB1` built on, and `Q-AB2` and `Q-AB3` recorded in section 5 with their defaults. Deferred: a row floor (`Q-AB2`, default no), an Alt+letter guard below the floor, the biped's damage pane whole at 120 by 34 (19 rows; 12 shown, 18 at 160 by 48), the SCAN row's one wrap between 120 and 125 columns, the cockpit strip's who block between 120 and 127, the catch-up report cut from the top at 120 by 34, the contacts pane's five inner rows at 120 by 30, a weighted column split for the status pane's spare rows, the line client's `DAMAGE_COLUMNS` 76, `VIEW` 78 and `SIM HISTORY` at eighty, and the re-flow of help line 229.

**When it lands.** The TUI is laid out for a terminal of at least 120 columns and says so below that,
instead of contorting: the status pane wide enough that the speed row and `DESTROYED` never wrap, the
damage pane drawn whole at 120 by 34 with every section, the contacts pane unwrapped; the goldens, the
fixtures, the help and doc 16 move with it, and `B-10`'s eighty-column layout is superseded.

- **AB.1 The layout.** The TUI's minimum is 120 columns (`Q-AB1`): the pane widths, the wrap rules
  and the height rule of cut B.3 re-derived for that floor; below it, one sentence naming the floor
  on every screen; the 120 by 34 damage pane residual of the B row closed; the TUI goldens re-blessed
  once; the help and the two sizing comments made true.

The owner's ruling of 2026-09-24: the minimum viable width is 120; 160 to 180 is typical; 240 and more
is reasonable. The line client's `VIEW` window (39 cells, two columns each) and the web are unaffected;
`SIM HISTORY`'s rows stay as V.2 built them. Depends on B.3, Y.4 (the last change to the panels).
Gateway: yes (the TUI). Packs: none. Hash: no. Size: small, one sub-cut. Named from the second series.


### Cut J: the authored reach fits the pulse

Landed 2026-09-25 at `c9f89d86`; see 16b section 9, row J. Built as the first three sub-cuts below, each committed and pushed on its own green gate (J.1 `f4625673`, J.2 `3eab4488`, J.3 `c9f89d86`), then the full sweep green on its first round with no repair; `T-34` closed, N5 landed through step 2, the authored lane's number written beside 12 ms and 10.3 ms and `Q26` put to the owner with a figure beside each arm (section 5), the candidate cull's slot window recorded as `T-87`. Deferred: J.4 (the work budget, the owner's word alone), a bound on `horizon_probes`, the fire path on the index, a lazy per-run walk for the index's query (it reads fewer cells than `sight_only` and is slower in wall time on the sight lane), a `pub(crate)` `Horizon::sight`, the coarser cull (`T-87`), and the Linux figure (CI billing-blocked, no Linux baseline); the `docs/ops.md` and `scripts/harness-weekly.sh` sentence that every lane asserts under the script, false while the authored lane was report-only, is made true in J.3b.

J.3b landed 2026-09-25 at `664465ff` on the owner's Q26 answer (the authored reach and the 300-unit target kept, `max_share_ops_per_pulse` 768, the authored lane's per-window lens 17.5 ms, the lane asserted under `scripts/harness-weekly.sh`; the authored baseline and its delta deferred); see 16b section 9, row J. J.3c landed 2026-09-25 at `036633d6` on the owner's second word ("bump the lens up to 20 to give us some headroom but keep a reasonable boundary"): the authored lane's per-window lens 20 ms, the guard test pinning it at the owner's figure, over the worst asserted run on record by the harness's 16 per cent spread and under the 25 ms `engaged_p99_ms` ceiling.

**When it lands.** The harness at 60 and 30 on the shipped-shape cohort has an honest p99 and trace
count, and the owner's choice between reach, bound and target is made on that number or is moot.

- **J.1** `sight_only` and its differential test.
- **J.2** `arena:horizon.rs`, its property test, `horizon_probes`.
- **J.3** Re-measure; write the ledger row; change `harness.max_traces_per_pulse` or the authored reach
  only on the number.
- **J.4** Only on the owner's word: the work budget.

Depends on U, `B-56`. Gateway: no. Packs: none unless J.3 shortens a reach. Assumes Q26. Size: large.

### Cut Z: lances and six a side

Landed 2026-09-25 at `b0f6504c`; see 16b section 9, row Z. Built as the four sub-cuts below, each committed and pushed on its own green gate (Z.1 `de08f2b2`, Z.2 `599a5001`, Z.3 `c75267b8`, Z.4 `b0f6504c`), then the full sweep green on its first round with no repair; N6, N7 and N8 landed at their defaults, `Q39` built on (six a side), `Q44` decided (`RELEASE` built in Z.2). Two of this section's assumptions were found false and corrected in the Z row's design note: the arena pack was a boot pack and not a shelf entry, so Z.3 shelves the game's two packs and a live world takes them by hand with the centre idle, and a version-only pack bump plans as a no-op, so the staff-lobby latch stays at 4 on a live world. Deferred: a bare `FOLLOW` arm, a comsys panel republish on a forced join or leave, the cockpit notices ring carrying the pilot's own net, the TUI channel backlog's comtitle, a relatch or re-make step for a version-only bump or a lost stock tank, a per-issuer order watch, a knob for the support leash, and twelve a side (Q39's second half); every one is in the Z row's residuals.

**When it lands.** Six pilots or machines a side take the field in lances of four that screen and
support, and each side talks on its own channel by callsign.

- **Z.1** `squad_size`, the brief's squad picture, the 12 v 12 harness cohort.
- **Z.2** `Screen` and `Support`.
- **Z.3** The pods, the tanks, `POD_ENTRIES`.
- **Z.4** N8.

Depends on Y, U (`T-36`), J if the number demanded it. Gateway: no. Packs: `neighborhood.ooc_area` and
the proving-ground arena pack, each a version with a migration chain. Hash: moves. Assumes N6's, N7's
and N8's defaults. Size: large.

### Cut I: replay and the referee

Landed 2026-09-26 at `affcea72`; see 16b section 9, row I. Built as the four sub-cuts below, each committed and pushed on its own green gate (I.1 `baa0cf54`, I.2 `8faf886b`, I.3 `24a91080`, I.4 `affcea72`), then the full sweep green on its first round with no repair; N9 and N10 landed at their defaults, `Q37`'s replay half built (objective and public), one owner question added (`Q-I1`, the line printer's grid budget, default assumed). Two of this section's assumptions were corrected in the I row's design note: the replay is folded as the tape passes the world and held on the ring in RAM beside its ground (a panel query cannot read the store, so N9's "over N2's reader" is not the reader), and `@arena/place`'s refusal is any round in flight to OR from the unit, journalled and never announced. Deferred: a scrubber on the TUI (it steps), a live picture on the line client (the observer payload has no shape), announcing a place to its pilot or dropping other platforms' contacts on it, a replay past the byte budget (a twenty-four-unit staff hour keeps about fifty-six minutes), the pull-in of the task's deadline only when a floor actually changes, a pilot-refusal test of the two staff switches, the TUI's live-picture guard on `available: false`, and every residual in the I row's deviations cell.

**When it lands.** Anyone can watch a finished match; staff can slow a running one, move a stuck unit
and watch the field.

- **I.1** The replay fold and `SIM REPLAY` in line mode.
- **I.2** The replay screen on the TUI and the web.
- **I.3** `@arena/tempo` and `@arena/place`.
- **I.4** `@arena/watch` and the `Observer` frame, with its contract test.

Depends on V, U. Gateway: yes. Packs: none. Hash: I.3 moves it. Assumes N9's and N10's defaults.
Size: large.

### Cut AC: the mind reaches the split

Landed 2026-09-26 at `f7f4f705`; see 16b section 9, row AC. Built as the two sub-cuts below, each committed and pushed on its own green gate (AC.1 `15f1afb5`, AC.2 `f7f4f705`), then the full sweep green on its first round with no repair; `B-82` and `T-88` closed; no harness run was owed and none was made; the redeploy that follows is made from the record commit with a wiped store and not before the owner confirms the 2253-10-29 date. Deferred: a `BOARD` typed the instant the `red` walk's room view arrives on the split going unanswered (reproduced by AC.1's fixer with and without `[llm]`, settled by a 1 s sleep in `two_process_llm.rs`; a 16c row with an SSH-level repro is owed), a `tacsim_deliberate` per-feature budget line (from the redeploy dev1 spends on four features inside the shared global cap alone; a 16c row is owed), the source tripwire's blindness to a field overwritten after construction, a `Metrics::default()` registry and `mushd`'s own `Metrics::new()`, the one-route rule that ties the split proof's hit to the tacsim arm, `boot_split`'s discarded server-half boot error, the optional `Ok` reply through an `issue_order` tool call, the template comment's missing re-render sentence, the generation-one-only formula in the clock comments, the provisioning file's unexercised fifth claim and its boot case's fn doc, `t88`'s unquoted-render message, and a committed pin of dev1's date.

**When it lands.** On the production shape the world task holds the LLM service the boot built, so a
squad's commander asks its provider and an NPC's turn runs, exactly as on the all-in-one; the two
binaries build their world-task dependencies through one constructor and a parity test holds them to it.

- **AC.1 The handle.** `B-82`: one production `ServerDeps` constructor shared by `mush-server` and
  `mushd` (the booted `llm`, the staff queue, the store, the guard, the spool, the adapters, the one
  metrics registry the sink and the world loop share); the split half calls it where it built
  `in_memory()` and overwrote five fields; a parity test that the split's deps carry the handle and the
  registry; a split-shape proof over the two-process rig with `[llm]` on and the mock provider that a
  tacsim deliberation reaches a provider and `no LlmService on this world task` never prints; the
  `ServerDeps::in_memory` doc made true (the tests' and the compat path's, never production's).
- **AC.2 The clock in the env.** `T-88`: `IC_RATIO` and `IC_EPOCH` as instance-env values the template renders into
  `[game].ic_ratio` and `[game].ic_epoch` (the example at 1 and `2020-01-01 00:00`, `deploy/dev1/instance.env` at 4 and
  `2026-09-22 00:00`, the values the owner set by hand on the box on 2026-09-22), the render's completeness
  check and the provisioning smoke covering both, the deploy README's redeploy section saying the clock
  is the env's; no code path changes.

The owner's ruling of 2026-09-26: a quick cut after I to fix the LLM service bug before the dev1
redeploy (found in that deploy's recon: dev1 has logged the WARN on every deliberation since 09-21; the
same recon's review found the clock edit the redeploy would revert). Depends on nothing in I. Gateway:
no (the server half alone; a reload takes it). Packs: none. Hash: no. Size: small, two sub-cuts. Named
from the second series.

### Cut O: more reasons to fight and more to do

Landed 2026-09-27 at `944780c0`; see 16b section 9, row O. Built as the three sub-cuts below, each committed and pushed on its own green gate (O.1 `8c2f0388`, O.2 `d547c94d`, O.3 `b78b06a7`), then the full sweep, whose first round found one failure that was not this cut's (`t88_the_shipped_instances_render_the_clock_they_chose` in `mush-config`: `225e7a8a` had moved dev1's `IC_EPOCH` to `2123-01-19 11:15` and left the shipped-instance table behind; repaired at `944780c0`, `T-88`'s test asserting as much as before) and whose second round was green with no rerun; N11, N12 and N13 landed at their defaults and N2's hit rate with them; three owner questions added to section 5 (`Q-O1` to `Q-O3`, defaults assumed); no pack applied by hand beyond the arena latch's 3 to 4 move through the shelf. Deferred: a `mode` argument on `@arena/start` (always a skirmish), a Hold match's clock on the staff observer picture, a machine's own `EMCON` decision (a machine keeps the switch it inherits, so a released `EMCON off` tank fields a silent pod for the rest of the match: an owner ruling), a floor on `EMCON` flips, `SIM UNIT target_mk1` refused for a human, range outcome words beyond the tag, an attacking machine contesting a held square and the shipped ground's blue-first asymmetry, a `Hold` declaration on a ground with no spot in reach of both musters, a second mode (`Defend`, `Escort`), the Hold lane on the weekly job (`Q-O3`) and its `TACSIM_HOLD_LANE` flag's absence from the gated-test inventory, the `harness_lance_goal_cost` line's absence from `docs/ops.md` and the lance prefix list, the `EMCON` hint's three narrow windows (a stale frame's early clear, the expiry under a staff tempo floor, bare `EMCON` reading the mirror), and the comment, test-pin and help-text inaccuracies the O.2 and O.3 judges listed (16b row O).

**When it lands.** A match can be won by holding a named place; a pod can be switched off; a new pilot
can shoot at targets that do not shoot back and read their hit rate.

- **O.1** `SIM MODE`, `modes` on `ArenaDef`, `Hold`, the settle order, the AI's default goal.
- **O.2** `EMCON`.
- **O.3** `target_mk1` and `SIM RANGE`.

Depends on V, Y. Gateway: yes if `STATUS`'s new lines are drawn on the panes. Packs: the arena pack and
the fielded catalog. Hash: moves. Assumes N11's, N12's and N13's defaults. Size: large.

### The tail

Started on the owner's word of 2026-09-27 ("go ahead and start the tail"), as five cuts in the rank order the
first draft gave them, named from the second series: AD (N14, `@macro`), AE (N15, content breadth, which
carries `B-58`, `T-18` and the lease half of `T-54` if a unit gains a second station), AF (N16, durability
and resume, the last engineering item before H1), AG (N17, `VIEW BRIEF`), AH (N18, hints). `B-58` needed
`T-15` first, which cut M.4 landed, and the help already names the gap. `B-75` (added by cut D's record)
is in no cut: it waits on an owner ruling.

### Cut AD: `@macro`

Landed 2026-09-27 at `ee8acbcd`; see 16b section 9, row AD. Built as one sub-cut, committed and pushed on its own green gate, then the full sweep green on its first round with no repair; N14 closed at its defaults; no harness run was owed and none was made, and no baseline row is added. Deferred, none of them a 16c item: `$*` and `$0`, per-account macros (`Q-AD2`), `@wait` inside a macro (`Q-AD3`), a prefix-expanded line (`Q-AD1`), a staff view of another player's macros, and a client-side set of macro names so the TUI's and the web's sticky modes stop wrapping a macro as speech (a gateway-half and `web/` cut of its own).

**When it lands.** A player can name a line and type the name, on every surface, with up to three
positional arguments, and no macro can reach a verb its player could not type.

- **AD.1** `@macro <name>=<line>`, `@macro` (read), `@macro/del <name>`: one expansion only (a macro that
  expands to a macro is refused), `$1` to `$3`, never over a registered verb, twenty per player, stored on
  the player, expanded at the alias stage the kernel already runs before command resolution, and the
  help.

N14 at its defaults (platform-wide; three positional arguments). Depends on nothing. Gateway: no. Packs:
none. Hash: no. One new verb, so one non-baseline `legacy-handlers.toml` stanza (16b section 0.7). The
observer rule: an expansion runs as the typist, under the typist's own permissions, and a refusal for a
registered verb's name is byte-identical whatever the verb. Size: medium, one sub-cut.

### Cut AE: content breadth, with the legged cost row

Landed 2026-09-28 at `e15e6898`; see 16b section 9, row AE. Built as the three sub-cuts below, each committed and pushed on its own green gate (AE.1 `bfd5377d`, AE.2 `1f814420`, AE.3 `e15e6898`), then the full sweep in two rounds with no repair (round 1's one miss was the authored lane's per-wake p99 at 12.369 ms against the 12 ms lens, investigated, not repairable inside the cut and put to the owner as `Q-AE8`; round 2 lost 57 test executables to the 04:00 target prune in the middle of its run and every one was green rerun alone); N15 landed at its defaults, `B-58` and `T-18` closed, the lease half of `T-54` not built because no unit gained a second station; `Q-AE1` to `Q-AE5` built on their defaults and `Q-AE6` to `Q-AE8` raised, all in section 5. Deferred: a declared `default_weapon` (component order is the choice), a machine-flown heavy firing its rack, the class cap re-checked at `SIM START` against the fill's pods, the fill topping a gunless side up (`Q-AE6`), the two world-side way-out prechecks and the loader's zone check left on the tracked row, an elevation glyph on the map, replays losing their map at the restart that moves the spots and the hash pin of `UnitState.class` (both AF), the `Held: … too steep for the tank.` word for every chassis, the cadence-dependent first bearing of the `New contact` notice (a candidate row), the `Withdraw` re-search cost (`Q-AE8`), the authored window lens (`Q-AE7`) and the lock-unaware target prune; every one is in the AE row's residuals.

**When it lands.** A scout, a heavy and a second biped, an autocannon and a missile with a flight time,
and one capped drone make a catalog; the biped pays its own row's costs; the shipped ground has one bluff
steep enough to refuse an edge.

- **AE.1** Chassis: a cost layer per locomotion row on `ArenaTerrain` chosen per unit (`B-58`; `T-15`
  landed M.4), the scout, the heavy and the second biped as versioned definitions with migration chains,
  and the proving ground's bluff (`T-18`) as the next arena pack version.
- **AE.2** Weapons: the autocannon and the missile with a flight time (a scheduled impact, as every shot
  is), `FIRE`'s default weapon a choice, `WEAPONSTATUS` and the help.
- **AE.3** One drone under W2's cap (`arena.max_units_by_class`), and the lease half of `T-54` only if a
  unit of this cut gains a second station.

N15 at its defaults (no infantry; one capped drone). Depends on `T-35` (landed U.1). Gateway: yes if a
pane draws a new weapon or section shape. Packs: the arena pack (the bluff) and the fielded catalog
(versioned definitions, digest pins, the boot refit). Hash: moves (AE.1 changes the biped's costs).
Size: large, three sub-cuts.

### Cut AF: per-pulse durability and resume

Landed 2026-09-28 at `98c020ab`; see 16b section 9, row AF. Built as the three sub-cuts below, each committed and pushed on its own green gate (AF.1 `f7d7e4ef`, AF.2 `18c4510e`, AF.3 `98c020ab`), then the full sweep green on its first round with no repair (its one failure, `sim_units`'s drone `STATUS` read before the first telemetry frame under full-sweep load, passed alone 2 of 2); N16 closed at its defaults with Q40's default amended (the tail is abandoned behind a `resumed` marker, not replayed: a tape records outputs, so a resume rewinds the match by up to `snapshot_ms` plus the egress latency, `Q-AF4`); `Q-AF1` to `Q-AF5` are in section 5 at their defaults; no section 1 or 2 item is closed by it. Deferred: automatic re-seating and a release into anything but `Withdraw` (`Q-AF2`), a shorter cadence under `Engaged` (`Q-AF3`), a final snapshot at shutdown (`Q-AF4`), an incremental snapshot, an `@arena/resume` verb, the human-order watches, span, rate and `@arena/pose` overrides across a resume, the marker's own check-out snapshot scrubbed at boot, a truncating store op for the abandoned tail, and the paged reader's cross-page limit.

**When it lands.** A server-half restart no longer voids a running match: the arena comes back from its
last snapshot and the ledger tail, byte-identical, and the pilots return through the reclaim.

- **AF.1** `Serialize` on the state tree behind a versioned `snapshot` module whose deserialised
  `state_hash` must equal the recorded one; the definition and the terrain re-derived from the digest; the
  test that snapshots every pulse of a scripted match and asserts a byte-identical ledger from any
  resume point.
- **AF.2** The `arena -> world Snapshot { at_ms, hash, bytes }` message every `arena.snapshot_ms`
  (default 5,000), written on the arena writer lane with disjoint key ownership.
- **AF.3** Boot re-creates the arena from the last snapshot at or after the last acked chunk, replays the
  ledger tail, re-opens the task; a hash mismatch voids loudly; `write_restart_abort` stops being the
  restart's answer; the pilots return through N4 (Y.3); `deliberation/mod.rs`'s recorded deviation
  re-read, because its second argument stops being true; doc 16 section 11's first "Before IC play"
  item closed.

N16 at its defaults (every 5,000 ms plus tail replay; OOC matches resume). Depends on N4 (Y.3) and
cut M. Gateway: no (the server half; the proof is `restart_survival`'s shape). Packs: none. Hash: must
not move. The arena still holds no store handle. Size: extra large, three sub-cuts.

### Cut AG: `VIEW BRIEF`

**When it lands.** The frame in sentences: the ground on eight bearings, run-length described, then the
contacts as `CONTACTS` words them, then the key spots; a second renderer of one frame, nothing new
across the seam.

- **AG.1** `VIEW BRIEF` in line mode, the observer assertions, the help.

N17 at its default (built now, after the fighting features). Depends on N1 (V.1). Gateway: no. Packs:
none. Hash: no. Size: medium, one sub-cut.

### Cut AH: first-time hints

**When it lands.** One sentence naming the next verb at five points, once per account, `@tutorial off`;
and `T-77`'s two help sentences.

- **AH.1** The five hint points, the per-account flag, `@tutorial off`, and the two help sentences the
  smoke's first-time pilot was owed (moving makes a tank visible further out; facing matters).

N18 built now on the owner's word rather than with the second player. Depends on nothing. Gateway: no
(hints are `Sys` lines on the existing path). Packs: none. Hash: no. One new verb, so one stanza. Size:
small, one sub-cut.

---

### Cut AI: the machines stop searching in circles

Landed 2026-09-29 at `0a465976`; see 16b section 9, row AI. Built as the one sub-cut below, committed and pushed on its green gate after the programme lead's three rulings on the shipped-scale duel and the ford (e1's claims read at their intent at the pulse that carried the shot, the hunt fanned, no figure moved), then the full sweep in one round with no repair; `B-84` closed whole and `Q-AE8` closed on the record (its second option built). Deferred: nothing of `B-84`'s own scope; `Q-AE8`'s third option (an arena-side per-window cell budget) is not built by design, the bound being a harness threshold and never a throttle, and the residuals the AI row's Deviations cell writes out (the arc-versus-chord `too_steep` stops at seeds other than 2350, the hunt fan running out of rows, an unlimited unit's hold at a both-closed corner, e1's window passing with no pulse margin, the `sim_units` race owed the load-sensitive list) are left there.

**When it lands.** A wingman that wandered off its route keeps the route and steers back onto it; a
tank stopped on a corner cut steps around the corner or holds, and never searches the same route
again without moving; the wake's path cost is reported and bounded on both harness lanes.

- **AI.1** `B-84`: the follower's steering target is the first route cell outside its turning circle
  and a wander inside the circle keeps the route; a stop on a corner cut is a one-cell shoulder step
  or a refusal that keeps the route; a re-search of a route whose refusal has not changed is refused
  by name and counted; `path_cells` per wake reported with a bound in `[tacsim.harness]`; the
  cross-tempo test at three cadences and both lanes measured at four seeds, the CI baseline
  re-blessed once with the reason.

From cut AE's sweep (2026-09-28): the authored lane's per-wake p99 rose from 8 to 9 ms to 11 to 12.4 ms
against the 12 ms lens when `B-58` moved the harness fight, and the repairer's instrumented diagnosis
put the cost on the two searches above, which predate AE and hold at other seeds already. Depends on
AE.1 (the per-row prices) and Q.7. Gateway: no. Packs: none. Hash: moves. Size: large, one sub-cut.
Until it lands the gates' rule for a per-wake lens miss on the authored lane is the per-window one:
re-run once with the deterministic block untouched, a second miss real, nobody moves a lens.

## 5. Open questions for the owner

Each has a default, and the build proceeds on the default unless the owner says otherwise before the cut
that assumes it. Feature choices are listed with their feature in §3; the ones below belong to bugs and
debts, plus the four feature choices that change the most.

| Q | Question | Default | Cut |
|---|---|---|---|
| Q1 | Is the enemy roster lobby knowledge that stays readable mid-match? | No: own side in full, the other side as callsigns alone, no `seated` or `left`; the callsign-to-slot mapping stays public. | D |
| Q2 | What does a destroyed pilot get for the rest of the match? | They stay seated, are told once what the options are, and read nothing new; a view of their side's mesh waits for H5. | D |
| Q3 | A match whose last human has left: finish, stop, or slow down? And may a side concede with a result? | `SIM CONCEDE` for the lead records a loss; an unattended match ends with its own reason after `match.unattended_ms` (300,000). | D |
| Q4 | May the cockpit write one optimistic fact into its own mirror? | Yes, one: a `Starting` hint that the next frame confirms or clears. D-16-1 stands for everything else. | D |
| Q5 | The web composer: shell-level, a cockpit-local row, or return-to-screen? | Shell-level, which is the TUI's shape. | B |
| Q6 | Does `look` learn the module namespaces, or does the atlas write a bare `detail:`? | `look` tries `detail:` and then each registered detail prefix; the namespace gate is untouched. | D |
| Q7 | Is a defer queue for player commands worth it? | Measure first; then depth 4, 250 ms, refused on timeout in a player's words. | B |
| Q8 | At what range and health does a hurt machine turn and fight? | Inside the fired weapon's close band only, and never below half of `withdraw_permille`. | E |
| Q9 | What does `Approach` do with range and no line? | It closes on the plot; the hold is the least of doctrine, order and current range, priced off the fired weapon's inner band. | E |
| Q10 | Does a refusal stop the tank (doc 16 §3.4 step 4) or may it slide? | Terrain, edge and slope stay literal; an occupancy hold keeps the speed. Sliding is not built. | E |
| Q11 | Is a wreck a physical fact the controller may read, or knowledge it must earn? | Knowledge: the contact's own reading, alive until the contact expires. | E |
| Q12 | A round on a destroyed section: absorbed, or transferred inward? | Absorbed and journalled as today, with its own rate-limited sentence. | D |
| Q13 | Is a zero-heat weapon refused at the platform's ceiling? | No. | D |
| Q14 | Short weapon names, or exact ids and a fixed help? | Unique prefixes and content-declared short names, printed on `WEAPONSTATUS`. | D |
| Q15 | Friendly fire: silent, warned or gated? | Warned, never gated. | D |
| Q16 | The silent rename: a new notice, or no hysteresis silence when the class regressed? | A notice; one line per treeline flicker. | D |
| Q17 | Does a platform know it is being jammed? | It knows its own receiver is degraded; it does not know who or from where. | D |
| Q18 | Does the `CONTACTS` band describe the weapon or the scope? | The lesser of the two; the weapon's own bands stay on `WEAPONSTATUS`. | D |
| Q19 | Is an authored band list a hard range limit? | Yes: past the last band a shot resolves blind. | M |
| Q20 | The TUI notices pane: filter the scrollback, or render the frame's notices? | Render the frame's notices; two screens may word one event. | B |
| Q21 | The cockpit tab's rule on both clients? | Seated-once. | B |
| Q22 | Move the `blocked` journal to the held edge and hash `held`? | Yes, with the cause on the event; every hash moves, in cut M with the rest. | M |
| Q23 | Make discrete actions rate-independent, or amend D-16-2? | Amend: a discrete action resolves at the end of the pulse that accepted it, and a production-shaped test says so. Reflex shots do become rate-independent (`T-02`). | M |
| Q24 | Build the absolute substep lattice? | No: amend §3.1 and assert that the fielded catalog's substep divides the three periods. Revisit with the first unit above 3,996 cm/s. | M |
| Q25 | May due events coalesce, amending §3.1 phase 5? | Yes, inside one substep length; and the harness measures the loop the task runs. | U |
| Q26 | Which gives: the 600 m sight, the 4,000-trace bound, or the 300-unit target? | None yet: build the cheap trace and the horizon index, re-measure, choose on that number; if still over, the work budget keeps both. Built to that default through J.3 (2026-09-25) and still over: after `sight_only` and the horizon index the authored lane's per-wake p99 is 7.699 to 8.156 ms and its per-window p99 14.108 to 15.092 ms over three weekly runs against the 12 ms lens (10.3 ms, 12.0 / 1.16, is what a weekly assertion needs), with 452 share operations a wake against a bound of 256; the lane stays report-only. The arms with their figures: shorten the sight (48 cells still over at 14.454 ms per window; 36 cells holds 12 at 11.234 but not 10.3, and needs `max_share_ops_per_pulse` at 768); move the bounds (768 for share operations, the lens still missed by 2.1 to 3.1 ms per window; holding the 16 per cent margin would need a 17.51 ms lens); lower the target (240 units holds 10.3 by 0.39 ms at 9.911 on one run with share operations to 512; 200 units holds every current bound at 9.478); or J.4, the per-substep trace budget with a hashed resume cursor, which keeps all three. Each sizing figure is one run inside the harness's 16 per cent noise; three confirming runs are owed before an arm is relied on. The builder would take the target arm; the owner decides. Answered by the owner on 2026-09-25 ("keep the authored reach and the 300 units, raise the bounds and set the lens with margin"), the bounds arm: keep the authored reach and the 300-unit target, `max_share_ops_per_pulse` 768, the authored lane's per-window lens 17.5 ms, asserted weekly by hand; built as J.3b (`664465ff`): first three asserted runs per window p99 15.339, 14.057 and 14.732 ms and per wake 8.099, 7.785 and 8.150 ms, every bound held (the gate's fourth run 16.298 and 8.377). The lens carries the 16 per cent margin over 15.092, the figure it was set on; over J.3's fourth run (15.807), J.3b's first (15.339) and the gate's fourth (16.298) it holds by 10.7, 14.1 and 7.4 per cent, so a loaded week can miss on noise: leaving it, adding a re-run rule, or setting 18.9 ms (16.298 × 1.16) is the owner's next call, and only the owner moves the lens. Ruled again later the same day: 20 ms ("bump the lens up to 20 to give us some headroom but keep a reasonable boundary"), built as J.3c (`036633d6`): 22.7 per cent over the worst asserted run on record (16.298), under the 25 ms `engaged_p99_ms` ceiling, pinned at the owner's figure by `the_lens_is_the_cohorts_own`; the confirming run read 15.732 ms. | J (J.3b, J.3c), answered |
| Q27 | Is a one-second age column acceptable for frame comparison? | Yes; it is what every surface prints. | U |
| Q28 | A lock's finer range: fixed, or derived from the plot's age? | Derived, floor 10 m. | D |
| Q29 | Does a baseline-delta check ride the weekly harness job? | Yes, failing at a 20 per cent move; the absolutes stay report-only (D-H-8). | U |
| Q30 | Build the tempban lift, or leave the restart? | Build it on the store route. | L |
| Q31 | dev2: delete or convert? | Delete; somebody with SSH looks at the box first. | L |
| Q32 | Both tracks: content, structure, or both? | Both, in cut M where the hash already moves. | M |
| Q33 | Pay the K6 lease for the three-channel seat now? | No: correct the comment now, build a lease per channel with the first second station. | L |
| Q34 | The matcher: an audit gate now, or a lens now? | The gate and `@find` now; the lens is H6. | U |
| Q35 | Validate the arena's config blocks when `[tacsim].enabled` is off? | Yes, always; see §6. | none |
| Q36 | May `STATUS` print the arena's global tempo word? | No: the pilot's own alert state; the cadence stays in the frame. | D |
| Q37 | What may a player read of a finished match (N2), and is a replay public (N9)? | The report is the player's own projection; the replay of an OOC match is objective and public. | V, I |
| Q38 | Who may `ORDER`, and how wide is the span (N3)? | The side's lead, and any pilot their own squad; 4 squads per commander. Built on this default (Y.1, Y.2), "their own squad" read as the one machine in their pod pair, since a four-pod side is one squad. | Y |
| Q39 | Six a side or twelve (N7)? | Six first. Built on this default (Z.3); twelve is one more pack bump once the fan-out lane has an engaged number. | Z |
| Q40 | Durability's cadence, and do OOC matches resume (N16)? | Every 5,000 ms of arena time (and on the opening pulse); yes. Amended by cut AF's plan: the tail is abandoned behind a `resumed` marker rather than replayed, since a tape records outputs and not inputs, so a resume rewinds the match by up to `snapshot_ms` plus the egress latency (`Q-AF4`). Built on this default (AF.1 to AF.3). | AF |
| Q41 | Does the watchdog void the pulse that finished the match? | Yes: void, and every unit restored (doc 16 §3.1); the forced void is not journalled, the black box keeps the arena's own end. Built on this default (AA.3); reversed by reverting the terminal arm of `Task::pulse` alone. | AA |
| Q42 | Does a reclaimed pilot take the side's lead back (N4)? | No: succession is one-way and the heir keeps it; a side whose every claimant had left gives the lead to the first who comes back. Built on this default (Y.3). | Y |
| Q43 | May a non-lead's order to their wingman replace the lead's live order on that unit (N3)? | Yes, latest-wins per unit, and nobody is told; a per-unit watch or an issuer set is a later cut's if a word should be said. Built on this default (Y.1). | Y |
| Q44 | Should `ORDER <squad> RELEASE` hand a squad back to doctrine and free the span (N3, W8)? | Not in Y: the four arms as N3 lists them, and the span holds until the world's watch expires, since the world never learns an early arena end. Cut Z decides. Decided and built (Z.2): `RELEASE` drops the unit to doctrine at once and the issuer's own watch with it, freeing the span. | Y, Z |
| Q-U1 | Should the arena task pass the SCHEDULED wall to `step`, so arena time is exactly the substep lattice with a late wake carried as lag? (raised by cut U's plan, not assumed) | No for now: arena time stays wall time, the wake targets are lattice points in absolute arena time, the task's deadline is anchored to its base, and the overshoot's cost is measured by `TACSIM_WAKE_JITTER_MS` (3 ms: 81 short substeps in 120 wakes). | U, open |
| Q-U2 | The arena-to-session events-per-pulse bound: the shipped opening burst is 288 in one pulse at 24 pilots (24 keyframes plus eleven contact-notice lines per pilot at the acquisition pass) against the planned 256. Keep 512 on the measurement, coalesce a pilot's notices into one `Sys` event, or bound per pilot? | 512 on the measurement (built at U.2); the burst is client-facing text and untouched. | U, open |
| Q-AB1 | What is the narrowest terminal a client surface is laid out for? (raised when `SIM HISTORY`'s rows ran 82 to 88 columns) | Answered by the owner on 2026-09-24: 120 columns is the minimum viable width, 160 to 180 typical, 240 and more reasonable; no client surface is built to 80 again. | AB |
| Q-AB2 | Should the TUI also floor on HEIGHT (blank every screen under some row count), or keep B.3's rule that a short terminal drops panes in the screen's declared order? (raised by cut AB's reviews, not assumed) | No row floor; width alone. Under 30 rows the cockpit's damage pane goes first as B.3 built it, under 12 rows every screen drops its chrome and keeps the scrollback and the box; `COCKPIT_ROWS` 30 is a documented derivation `draw` never reads. Built at AB.1. | AB, open |
| Q-AB3 | Does the notices pane ride the Right column (weapons, contacts, notices) or the Left as B.3 built it? | The Right, as AB.1 built it: seven inner rows at 120 by 34 and twelve at 160 by 48 instead of two on the Left under the 12-row damage pane, and the cockpit whole from 120 by 30 with the Left column status over damage. The owner may put it back on the Left (two rows at 120 by 34, four at 120 by 36) in a later cut. | AB, open |
| Q-I1 | Does the line printer's one grid budget (78 by 20 printed columns; `VIEW` and `SIM REPLAY` at 39 by 20 cells, two printed columns a cell) fall under the 120-column ruling (`Q-AB1`)? (raised by cut I's plan, not assumed) | No: the ruling names client surfaces built new, and the printer is the line client's, which AB left at 78 by 20; `SIM REPLAY` shares `VIEW`'s window. If overruled, the printer's budget moves to a 59 by 20 window in a small follow-up and `VIEW` moves with it. | I, open |
| Q-O1 | How do the machines behave under Hold, and who governs a squad on dev1? (raised by cut O's plan, not assumed) | Doctrine's own goal: a machine drives to the key spot in place of the hunt and the walk, parks inside the square and fights from there, and an attacking machine stops at `engage_cm` and never drives in to contest a held square, so a human can camp The Crossroads against an AI side that shoots but never enters, the shipped ground's blue machines reaching the spot first; on dev1 the commander's order governs whenever it gives one (the forced tool always answers an order). Built on this default (O.1); reversed by an `Approach` clause that closes into the square. | O, open |
| Q-O2 | Should a range's result occupy the twenty-deep history ring at all, flushing a real match's row, report and replay? | Yes: a range is a match like any other, tagged ` [range]` on the ring, the header and the replay; the flush is the ring's. Built on this default (O.3). | O |
| Q-O3 | Should the report-only, flag-gated Hold lane (`TACSIM_HOLD_LANE=1`, `harness_lance_goal_cost` beside the lance lane's) join the weekly job? | No: run by hand in O.1's gate only. Until answered the flag matches neither `RUN_*` nor `*_RUN`, so `ci_gated_test_inventory` neither registers nor exempts it; a yes renames and inventories it. | O, open |
| Q-AD1 | May a macro's line start with a speech prefix (`@macro w=:waves at $1`)? (raised by cut AD's plan, not assumed) | No: the line starts with a word (`say`, `pose`, `semipose`, `@emit` or a verb), and a prefixed line is refused at definition (`A macro's line starts with a word (say, pose, semipose, @emit), not with " : ; or \.`); a prefix-expanded rewrite would have to skip the later stages the way a typed prefix does, and `AliasOutcome::Rewrite` carries no such flag. Built on this default (AD.1). | AD, open |
| Q-AD2 | Are macros per character or per account? (raised by cut AD's plan, not assumed) | Per character: the object the line is typed as, twenty each; an alt has its own, and nothing is kept per account. Built on this default (AD.1). | AD, open |
| Q-AD3 | May a macro's line be `@wait ...`, the one player verb that re-dispatches a stored line later? (raised by cut AD's plan, not assumed) | No (`A macro cannot queue a command.`): `@macro z=@wait 0=z` would re-arm itself on every fire, hold a wait-queue slot per chain and survive a disconnect, the one way a mortal could make the world task loop. Built on this default (AD.1), and wider than asked: a `$` in a line's first word is refused at definition too (`A macro's first word cannot hold a $; the words after it can.`), because `@macro y=$1 $1` typed `y @wait 0=y` reached the queue past every definition-time check, and the stage re-checks the expansion's verb (`@wait`, `@sshkey`, `@newpassword`, `@newchar`) whatever the document holds. The owner may want `@wait` back with a depth mark. | AD, open |
| Q-AE1 | Widen the harness cohort to the new chassis, or price the CI cohort's ground from the shipped table? (raised by cut AE's plan, not assumed) | No to both: the cohort stays U.1's eight and the CI ground keeps `harness_move_cost` for every row; pricing it from the table would make heavy woods impassable to 262 tanks and could move the CI per-window lens. The lance lane and the sight differential fight on the shipped ground priced from the table since AE.1; neither is baselined. Built on this default (AE.1). | AE, open |
| Q-AE2 | Should a missile be guided, and is 250 m/s its speed? (raised by cut AE's plan, not assumed) | No guidance, 250 m/s: a lock is never held past a sight (600 m own, about 900 m on a shared plot), so a missile is 2.4 to 3.6 s in the air in play and a moving tank walks out from under the one-cell re-validation; the 1.5 km / 6 s case lives in the arena's tests, and the 2 mrad band is reachable only on a shared plot. A slower round is one number in `missile_lr.json`. Built on this default (AE.2). | AE, open |
| Q-AE3 | Does the wheeled row ford shallow water? (raised by cut AE's plan, not assumed) | Yes, at 400 (`locomotion.json`, `LOCOMOTION_DIGEST` re-pinned); rubble, rock and heavy woods still refuse it. Built on this default (AE.1). | AE, open |
| Q-AE4 | Is the class cap per side or per map? (raised by cut AE's plan, not assumed) | Per side: `drone = 1` on each side, doc 16 §2 W2 amended in §11. Built on this default (AE.3). | AE, open |
| Q-AE5 | Where does The Bluff sit? (raised by cut AE's plan, not assumed) | East of the hold square at 141..147 by 105..118, 105 cells from blue's muster and 138 from red's, overlooking The Crossroads: the one flat −200 shelf of its size near the centre. Its rows cross the lance lane's blue rows 105 and 107, and the lane's W8 held at its defaults there, so the decided fallback (rows 109..118, clear of every lance row) was probed and not taken; the owner may move it in a later pack version. Built on this default (AE.1). | AE, open |
| Q-AE6 | With the fill on, may a lone drone pilot start a match? (raised by AE.3's judge, not assumed) | Today no: `ai_side_size` is the larger of the stored size and the largest side, one with one pilot, so `fill_pods` adds nothing beside the claimed drone and `SIM START` reads `The red side fields no gun; a side needs a machine that carries one.` with no hint that `SIM AI 2` starts it. Either `fill_pods` tops a gunless side up by one armed pod (shared by `ready()` and `start()`), or the sentence or `help simulator` gains the `SIM AI 2` hint; the test pins today's behaviour. | AE, open |
| Q-AE7 | Does the 20 ms authored per-window lens move? (raised by AE.3's gate, not assumed) | Not by a builder: the worst asserted run on this host read 17.623 ms (past the 17.241 guard, so the recorded constant stays 17.108) and 17.623 × 1.16 = 20.44, so the lens no longer clears the host's worst run by the 16 per cent spread; the deterministic block was equal in every run and AE.3 runs nothing the cohort runs, so it is host noise, and the doc makes the lens the owner's. Cut AF read above the guard four more times (17.773 and 17.579 in AF.2, 17.940 in AF.3, 18.022 in its sweep; its gates 15.815 and 15.660) with the deterministic block equal every time; nothing moved. | AE, open |
| Q-AE8 | The authored lane's per-wake p99 at seed 2350 sits about 5 per cent under the 12 ms lens and crossed it once in cut AE's sweep (12.369 ms; 11.15 to 11.63 on five re-runs), and AE.1 already crossed it at seeds 2351 and 2353: which repair? (raised by the sweep, not assumed) | Treat the margin as known for now. The cost is A star cells expanded by `Withdraw` machines re-searching full-width routes to their entry cell: one orbiting its next route cell so Q.7's wandered-off rule clears the route every `decide_ms`, one stopped `impassable` at a corner motion cuts and the shoulder rule skips (the documented class gap), re-searching the same 13,667-cell route every `repath_ms`. The second option is a reflex cut (no re-search while orbiting, a back-off for a machine stopped at the same cell re-planning the same route, the corner-cut gap closed) and a re-bless of the CI baseline; the third bounds path cells per `decide_ms` window beside the query bound. Each moves the deterministic block, so none is a builder's. Cut AF crossed the lens once more (12.078 ms, AF.2's builder run, re-run under rule (a)) and read 10.25 to 11.35 on its passing runs. Closed by cut AI.1 (`0a465976`, 2026-09-29): the second option built (the follower's turning circle, the closed-corner rule, the bounded refusal memory reading nought, the CI baseline re-blessed once) and the third not, the per-wake `path_cells` peak bounded by `[tacsim.harness].max_path_cells_per_pulse` (48,128) as a harness threshold; the authored per-wake p99 read 8.1 to 8.9 ms at seed 2350 on every asserted run of the cut and 10.5 at seed 2351 report-only (from 17.4); 16b section 9, row AI. | AE, closed by AI.1 |
| Q-AF1 | Is the staff tempo floor (`@arena/tempo`) lifted by a resume? (raised by cut AF's plan, not assumed) | Yes: a floor is a lever set for a process that is gone; the clock re-anchors with no floor and `@arena/status` shows none, and staff set it again. Built on this default (AF.1, `Clock::reanchor`; AF.3). | AF, open |
| Q-AF2 | Should a resumed match's pilots be re-seated automatically when their gateway session survived the server-half restart? (raised by cut AF's plan, not assumed) | No: every seat is released at the resume, the machine flies the tank, and the pilot takes it back with `BOARD` through Y.3's reclaim after `reclaim_min_ms`, reading the resume line, the doctrine line and the catch-up. The cost, said plainly: every released tank goes to the reflex controller in `Withdraw` and drives toward its own entry zone until its pilot boards, on both sides at once, while AI-filled tanks keep fighting, so a resume shifts the fight toward the side with more AI fill for `reclaim_min_ms` plus the pilots' return, and the first resumed pulses pay a burst of route searches (the machine fired during the handover in `sim_rematch`). A resume-marked control that releases into a hold or `Patrol` instead of `Withdraw` is a later cut. Built on this default (AF.3). | AF, open |
| Q-AF3 | May the snapshot cadence be shortened under `Engaged`? (raised by cut AF's plan, not assumed) | No: one cadence in arena time (`snapshot_ms`), because the encode cost is the field's size and not the tempo's, and `harness_arena_snapshot` says what it costs (about 12 to 14 ms on the task thread per snapshot of a 300-unit field). Built on this default (AF.2). | AF, open |
| Q-AF4 | A resume rewinds the match to its last snapshot (up to `snapshot_ms` of arena time plus the egress latency): a hit, a lock or a wreck a pilot read in the abandoned seconds un-happens, and the tape keeps those seconds only as an abandoned tail behind a marker; a planned reload rewinds too, because no final snapshot is taken on the way down (`arena_shutdown` runs after the world task has stopped). Accepted? (raised by cut AF's plan, not assumed) | Accepted: a pilot whose seat the resume released reads one line, `The server restarted and the match resumed from its last save; what happened after that save did not count.`, before the doctrine line. A final snapshot at shutdown is a later cut. Built on this default (AF.3). | AF, open |
| Q-AF5 | Does the inline encode's cache cost on the wake after it move a lens? (conditional, raised by cut AF's plan, to be asked only if AF.2's off-and-on measurement showed it) | Not raised: off against on (per wake / per window p99) read CI 5.625 / 10.059 off and 6.226 / 10.562 on, authored 11.254 / 16.550 off and 10.401 / 15.958 on, no lens move attributable to the encode; the figure itself (p50 11.2 to 13.5 ms, wake plus encode up to 21.2 ms, the encode cloning every unit's opaque `TacticalUnit` once through `UnitRepr`) is the owner's to see, a borrowing `UnitRepr` serialiser or an incremental snapshot being the lever if it costs. | AF, closed at its default |

---

## 6. Not doing

| Item | Reason |
|---|---|
| Skip the arena config validators when `[tacsim].enabled` is off (`mush-server/src/config_ext.rs:375-378`) | Doc 16 §11's Q.7 entry states the behaviour as intended; a stale value refused by name today is cheaper than one that surprises the operator the day the layer is switched on. |
| Move N.2 deliberation onto the K7 durable decision pipeline | A recorded deviation with a three-point case at `tacsim:deliberation/mod.rs:54-88`; since 16b cut AF a match survives a server-half restart, but a deliberation in flight dies with the process and the squad is re-asked at its next trigger about the field as it then stands, so durability would preserve the wrong thing (the second argument rewritten by AF.3). Doc 16 §6 is amended in `T-75` and again by cut AF's record. |
| A true match pause | A fourth clock state and a hole in arena time that `lagged` models differently; `@arena/tempo` and the replay answer what a pause is asked for. |
| Sliding along an obstacle after a terrain refusal | Doc 16 §3.4 step 4 is literal and test-pinned, and the pose clamp is what keeps "no unit occupies an untested cell"; the occupancy half of `B-46` is the part that hurts in play. |
| The absolute substep lattice | L, moves every timestamp, and buys nothing at any shipped speed; amended instead (`T-05`, Q24). |
| Field repair and damage control | The verb existed (`+dc`) and D-16-20 deleted it; W15 keeps fights short because units withdraw, and a tank that repairs itself does not have to. Its one good idea, shedding heat by stopping, already happens. |
| Hull-down posture | It moves the hit curve of every shot, every resolution fixture and every hash for one decision; not before `EMCON` has shown whether pilot agency of that kind is wanted. |
| Smoke | It needs a persistent volume in the line-of-sight trace per cell per time, and neither W4 nor the arena has a place to put one. |
| Infantry | At 10 m cells with one unit per cell the model says nothing about a person. |
| A tutor NPC | A second source of truth to keep in step with every verb; N18's hints quote the registered table. |
| Accepting a former callsign after a rename (`LOCK BA` while the row reads `X2`) | It would tell the pilot BA is still out there, which the byte-identical refusal exists to hide; `B-20` is a notice. |
| A `fire when ready` hold for the machine gun's rhythm | Real, from the smoke, and small; not ranked because a standing fire order is an automation the owner has not asked for. It is one paragraph inside N3 if wanted. |
| The "Locked percentage read at the pulse boundary" seed | The smoke's web frames show `lock_percent` tracking range pulse by pulse; not a defect. |
| `k9` (a rear-quarter duel ends in `unit_disabled`) and "frame notices duplicate the Sys lines" | Both re-checked at `820ad9af` and found harmless: the match still ends, and no screen carries both a notices ring and a log pane. |
| Re-posing dev2 onto the template | Deleted instead (`T-72`). |
