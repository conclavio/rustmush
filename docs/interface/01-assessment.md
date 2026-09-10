# 01 — Interface mockups: assessment

**2026-09-04, recorded 2026-09-05.** The Claude Design handoff bundle in
[`mockups/`](mockups/) read in full and checked against the tree at `a0142e3`
(working tree, E7 in flight). Three canvases: `Current State` (a recreation of
today's web client, TUI and line mode), `Redesign` (turn 1: two web directions,
the shared TUI parity floor, line-mode typography, and a list of server asks),
and `Ruleset TUI` (turn 2, the primary: five HUD screens in the Console
vocabulary, plus a "what exists vs. what these assume" note). The plan that acts
on this is [02-implementation-plan.md](02-implementation-plan.md).

**Notation.** The mockups number their screens T1–T3 (TacSim) and S1–S2
(Shadowrun). Doc 10 already uses T1–T16 for addenda and S1A–S7A for the
foundational survey, so this folder always writes **mockup T1**, **mockup S2**,
never a bare T or S number.

## 1. Verdict

The bundle splits into three cost classes.

1. **Unblocked and cheap:** the web, TUI and line-mode redesign, the damage
   and refit screen (mockup T3), and an IC/OOC comms split. Roughly 12 to 18
   build sessions in total, all startable now.
2. **Blocked on E7:** the piloting HUD's grid, contacts, to-hit, weapon cycle
   timers and heat rules (mockup T1), and comms nets with signal strength
   (mockup T2). The data these render does not exist; it arrives with E7 cuts
   6 (placement), 7 (propagation and contacts) and 8 (the pulse driver), and
   with `ruleset-tacsim` content.
3. **Not UI work at all:** the rigger and hacker screens (mockups S1, S2)
   describe Shadowrun subsystems that do not exist. Each is a milestone in its
   own right; the panes are the last and smallest part.

## 2. What is real today versus what the mockups assume

Most "already there" claims in the ruleset canvas hold. These corrections
change the plan; every one was opened in the tree.

- **The NPC "is thinking" signal already exists.** `mush-npc` publishes an
  `npc.thinking` panel keyframe to the room (`profile.rs` `THINKING_FEED`,
  `module.rs` `emit_thinking`), and both clients drop it: the web router
  (`web/src/lib/game.svelte.ts` `#routePanel`) and the TUI (`tui/app.rs`
  `apply_event`) ignore unknown feeds. The proposed `AgentTurn` event is
  unnecessary. There is no end-of-turn frame yet.
- **WHO location is a privacy policy, not a plumbing gap.** `WhoEntry.location`
  is populated only for staff viewers (`mush-core/src/commands/info.rs`
  `who_entries`). A "Where" column, or the "in this room / elsewhere" split
  both web directions draw, needs a visibility rule.
- **The unit journal is already on the wire.** `@unit/damage` applies a
  `UnitMutationPlan` and pushes the resulting `Journal` to the actor as a
  delta on the `tactics.unit` feed (`mush-tactics/src/module.rs`, the
  `damage` arm), so mockup T3 is a render of existing data. Missing: a
  keyframe on subscribe (there is no `tactics.unit` query handler), and
  neither client routes the feed.
- **There is no map.** The seam-test stub's map was a toy 2x2 grid (feed
  `tacseam.map`; retired by doc 14's S4 for the atlas window). The
  production module registers only `tactics.unit`; there is no grid crate
  dependency anywhere in the workspace; `mush-sim`'s `PlacementRecord` is
  parent-based with opaque poses. The grid pane has no producer until E7 cut
  8 fixes domain geometry.
- **Contacts have no bearing, range, speed or lock.** K6's `ContactState`
  (`mush-core/src/contact.rs`) is an opaque truth-subject mapping with an
  audience and a lifecycle. `+contacts`, provenance and age are E7 cut 7
  (10b §3.7).
- **E7 ships no TacSim panel by decision.** 10b §1.1 keeps the "one internal
  view feeds every projection" rule and explicitly defers the panel. A HUD is
  therefore this programme's work, not E7's, and must respect that rule when
  it reads unit state.
- **Comsys has no IC/OOC concept** (`mush-comsys/src/data.rs` `Channel`,
  `ChannelFlags`: LOUD, NO_TITLES, LOGGED) and no signal strength. Mockup T2
  conflates comsys channels with the propagation layer's nets.
- **Scenes have a record and a witnessed event log** (`mush-scenes/src/data.rs`
  `SceneRecord`, `SceneEvent`, `ScenesState::active_scene_of`) but no panel
  feed or query. A scene pill and a pose counter are a small addition.
- **The TUI has no input history and no cursor editing.** `tui/app.rs`
  handles Enter, Backspace, Ctrl-U, Tab, Ctrl-], PgUp/PgDn and Up/Down as
  scroll; the line-mode `LineEditor` (`editor.rs`) has both. The mockups
  assume both and their parity-floor list omits the gap.
- **Direction 1a's serif is Newsreader from Google Fonts.** `web/README.md`
  forbids external fonts and CDNs, so a font must be vendored or a system
  serif stack used.
- **The IC clock is not on the wire.** `GameClock` (`mush-core/src/clock.rs`)
  has `ic_hour`, `season` and `time_of_day`, and `World::world_ic_now` exists,
  but `RoomView` carries only name, id, desc, contents, exits and zone.
- **Weather reaches `look` as a tagged `Sys` line**, appended by the
  `LookRendered` hook after the `Room` event (`mush-weather/src/lib.rs`
  `WeatherLook`, tag `weather`; `mush-core/src/commands/mod.rs`
  `show_room_with_action_state`). The structured `WxSnapshot` is available to
  core through the registered `WeatherQuery`.
- **The command table already carries what a palette needs.** `CommandSpec`
  has name, aliases, switches, `min_role`, `power`, `help`; the registry
  exposes `specs()` and `help_topic_names()`. There is no listing query.
- **The bridges hard-code feed routing.** `web_bridge.rs` maps
  `PanelSubscribe` to a state query by a fixed `match` (comsys, who,
  legal.actions, mail, llm.dashboard; anything else only acks), and
  `ssh_bridge.rs` maps the TUI's query names the same way (who, mail.list,
  intel.list, legal.actions). Every new pane today costs an arm in each.
- **Shadowrun's sheet has the fields the meat-body panes read**:
  `derive::condition_monitors`, `wound_modifier`, `limits`,
  `initiative_base`, `schema::EDGE_CURRENT`; the agent grounding has a
  `matrix` domain tag. There is no rigging, drone, Matrix, host, IC or
  Overwatch state anywhere.

## 3. Effort

Calibration: the TUI proof of concept was one commit (`380691d`, about 3,000
lines) and the web MVP one commit (`a1b2729`, about 18,400 lines). Sizes below
are build sessions of that shape.

| Item | Sessions | Blocked on |
|---|---|---|
| Line-mode typography: heading rule, labelled facts, speaker gutter, one-line rolls | 1 | nothing; golden tests change |
| TUI screen model and parity floor: numbered screens, status line, gutter, gauges, mode switch, history, cursor keys | 2 to 3 | nothing |
| Server asks: command list query, room ambient, scene feed, composing signal, unit keyframe query | 1 to 2 total | nothing |
| Web 1b Console restyle | 1 to 2 | the server asks, for full effect |
| Web 1a Scene: grouping, prose, room band, composer, palette, roll builder, status strip | 3 to 4 | the server asks |
| Mockup T3, damage and refit, TUI plus web | 1 to 2 | the TUI screen model |
| Mockup T1 unit-side panes: sections, plant, weapons, capabilities | 1 to 2 | cell names from the E7 cut 1 rules pack |
| Mockup T1 grid, contacts, to-hit, cycle timers, heat rules | blocked | E7 cuts 6, 7, 8 and ruleset content |
| Mockup T2 IC/OOC split via a comsys channel flag | 1 | nothing |
| Mockup T2 nets with signal strength | blocked | E7 cut 7 |
| Mockups S1 rigging, S2 Matrix | a milestone each, UI last | new Shadowrun subsystems |

## 4. Suggestions

1. **Drive the HUD from a manifest, the way the sheet panel is driven by the
   sheet schema.** Every gauge in mockups T1, S1 and S2 is a bounded scalar
   cell (`ScalarCell { min, max, current }` in `mush-tactics/src/unit.rs`, a
   `Track` or `Number` field in `mush-ruleset`), and every table is a state
   family. The client should render "bounded cell as bar plus digits" and
   "family as table" generically, with the ruleset supplying which cells sit
   in which pane and which one is the dominant number. A drone's condition
   monitor, a mech's heat and a deck's Overwatch become one widget, and the
   client never hard-codes "heat". The prompt gauges are the same mechanism:
   today the prompt is text (`[Bruised] [BP 6/10]`), and a gauge strip needs
   the ruleset to name its vitals.
2. **Build the screen model before any HUD.** Replace the single Tab-cycled
   secondary pane with numbered screens, titled panes, a legend line and a
   declared degrade order per screen. Mockup T1 as drawn wants 44 rows; at
   the 120 by 34 budget the comms strip and grid must collapse first, and
   that order should be written down.
3. **Don't choose between 1a and 1b; split by screen.** Prose wants 1a's
   typography, a HUD wants 1b's density. Making "screen" a first-class client
   concept, with a prose-first scene screen and console-dense ruleset screens,
   matches the ruleset-owned scene modes idea (`docs/next_ideas.md` §16) and
   lets the TUI and web share one layout vocabulary where it matters.
4. **The rigger's two-pane rule generalizes.** "Two bodies get two panes" is
   TacSim's active-station focus (doc 10 C2A, C4A) in UI form: any remotely
   operated unit keeps the operator's body in a persistent pane. Adopt it as a
   client rule now rather than a Shadowrun special case.
5. **Do the IC/OOC split cheaply now.** An `IC` flag on comsys channels gives
   the two-pane comms screen, the border colours and the transmit bar in one
   session. Signal strength and degraded text belong to the propagation layer
   and should not be faked in comsys.
6. **Scope the command-list query around dispatch order.** Filter by the
   viewer's role and power server-side, and merge comsys aliases and exits
   client-side, or the palette omits the verbs people use most.
7. **Be careful with the composing signal.** Coalesce client-side, cap the
   rate, make it opt-out, and never let it stamp idle. Queries already never
   stamp `last_input` (`engine.rs` pins it); a composing inbound follows that
   precedent.
8. **Defer the grid pane, not the HUD.** Ship mockup T1 with a contacts table
   once cut 7 exists, and add the window render when cut 8 fixes geometry.
9. **Treat mockups S1 and S2 as ruleset design inputs.** Rigging can ride the
   tactical unit model once cut 8's pulse exists, since a drone is a unit. The
   Matrix is a new abstract-space subsystem. Both deserve their own design
   conversation before any pane is drawn.

## 5. Bundle notes

- The handoff README's file paths (`rustmush-modern-interface-design/…`) are
  the export's own; here the project lives at `mockups/project/`.
- `mockups/project/uploads/pasted-1788403248790-0.png` is a screenshot of a
  third-party BattleTech MUX client (Thud) used as visual reference; the
  mockup text notes that its unit designations and names are invented.
- `support.js` is the design tool's runtime; the `.dc.html` files need it
  beside them to render.
