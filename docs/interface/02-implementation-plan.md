# 02 — Interface programme (I): implementation plan

**Draft 0.3 — 2026-09-05.** Scoped from [01-assessment.md](01-assessment.md)
against the tree at `a0142e3` (working tree; E7 in flight on main). Every
symbol named here was opened in that tree; line numbers are deliberately not
cited (08i's confirm-by-symbol discipline).

**Owner's brief (2026-09-05).** Put the assessment on record and lay the
groundwork. E7's TacSim build-out is the only parallel activity, so WoD and
Shadowrun are free to touch for vertical slices as needed.

**Owner 2026-09-05: §8 defaults confirmed as written, with q5 replaced by a
ruling — the TUI is modal.** A nav mode where bare keys act (`1`, `h`, `?`),
`Enter` toggling into a text-input mode where commands are typed MUSH-style;
`/`, `"` and `:` also open the box (command, say and pose triggers); and
while the box is active every nav hotkey is still reachable as Alt+key
(`Alt+1` where `1` worked in nav mode). **Room exits work the same way on
the scene screen:** an exit `North <n>` is walked by pressing `n`, so the
common exit letters are reserved from the scene screen's own legend and
free for other screens (a mech designer, mail) to bind. Folded into §3.2,
§3.4, D-I-17, D-I-18, D-I-19 and the key table in §7.

**Notation.** Programme letter **I**; cuts **I1–I8**; decisions **D-I-n**;
verify lenses are single letters per cut. The mockups' screens are always
written **mockup T1 … T3** and **mockup S1, S2**, because doc 10 already owns
T1–T16 and S1A–S7A. "Session" means one focused build session of the shape
that produced the TUI proof of concept (`380691d`) or the web MVP (`a1b2729`).

Companion docs: 02-connectivity (SSH, session, web, the protocol treaty),
10 and 10b (TacSim foundations, the E7 plan), 08f (E6: scenes, Shadowrun
economy), `web/README.md` (the web client's contracts and known gaps),
`docs/help/tui.md` (the TUI as shipped).

---

## 1. Scope — what I is, in the owner's terms

The Redesign canvas names four problems: the wall of text, no sense of place,
unguessable commands, and a dated shell. The Ruleset canvas adds the target the
shell must grow into: dense, legible ruleset HUDs whose panes are renderings of
data the engine already models. The owner asked for direction, not pixels.

I builds, in this order:

- **Contracts first** (I1) — the small set of wire additions every client
  needs: a ruleset-neutral vitals feed, a command-table query, structured room
  ambience, scene state, and sheet republication on mutation. Nothing here is
  a skin; all of it is data the mockups assumed and the tree lacks.
- **The parity floor** (I2, I3) — the TUI grows a screen model, a status line,
  a speaker gutter, gauges and real line editing; line mode gets typography,
  not new semantics.
- **The web shell** (I4, I5) — screens as a first-class concept, a command
  palette over the real table, a composer with modes, a status strip; then the
  prose typography of direction 1a as a skin on the scene screen.
- **Vertical slices through the rulesets that exist** (I6, I7) — a Shadowrun
  combat screen and WoD vitals and roll builder prove the manifest-driven HUD
  on real sheets before any tactical data exists; an IC/OOC split gives comms
  its structure now.
- **The tactical unit screen** (I8) — mockup T3 and mockup T1's unit-side
  panes, rendered from `TacticalUnit` through a read-only view, coordinated
  with E7 and gated on nothing E7 has not landed.

**Exit criterion.** A player on SSH (line and TUI) and a player on the web see
the same screens by name and number, the same vitals, the same room band, and
the same command palette; the TUI's two modes are always visible and a bare
hotkey can never send a mutating command; a Shadowrun character's condition
monitors and a tactical unit's sections render through one gauge and one table
widget with no ruleset knowledge in either client; every screen degrades in a
declared order down to 80×24; the tiny-terminal and projection-cleanliness
tests stay green.

### 1.1 Explicitly out of I

- **Mockup T1's grid, contacts, to-hit, weapon cycle timers and heat rules.**
  No producer exists: placement is parent-based with opaque poses, K6's
  `ContactState` carries no bearing or range, and the pulse driver is E7 cut
  8. These panes ship as declared placeholders in I8 and fill in when E7 cuts
  6, 7 and 8 land and the `ruleset-tacsim` pack names its cells (§9).
- **Comms nets with signal strength and degraded text.** That is the
  propagation layer (E7 cut 7). I7 ships an IC/OOC flag on comsys channels
  and nothing that pretends to be signal.
- **Mockups S1 (rigging) and S2 (Matrix).** New Shadowrun subsystems; each
  needs its own design doc before any pane. What I keeps from them is the
  layout rule "two bodies get two panes" (D-I-7) and the widgets they share
  with everything else.
- **Client-side rules math.** Clients never compute a threshold, a penalty,
  a to-hit or a "next box" warning; those arrive as `state` and `detail`
  strings from the ruleset (§4 rule 1).
- **A ruleset silhouette for the section map** (the mech outline in mockup
  T3). I8 renders a generic section list; a ruleset-supplied layout is a
  later addition and an owner question (§8 q8).
- **Composing over SSH**, terminal emulation in the browser (02-connectivity
  already rejects xterm.js), and any font that is not a system stack unless
  the owner chooses to vendor one (§8 q4).
- **A second protocol.** Every addition is an additive `mush-proto` change
  regenerated through `cargo gen-ts`; older clients ignore unknown fields and
  event tags (treaty ruling 1's additive evolution).

---

## 2. What I consumes as landed (by symbol)

- **Protocol** — `OutputEvent::{Room, Narrative, Sys, Channel, MailNotice,
  Who, Panel, View, Weather, Reply, Resumed, Prompt, Ping, Hello, SessionInfo,
  Disconnect}`; `Panel { feed, kind: PanelFrameKind, data }`; `ViewEnvelope`
  with `ViewFrameDisposition` (keyframe/delta/stale/resync); `Hello.panels`
  (the manifest); `Query::{MailList, Who, Sheet, ChannelBacklog,
  PanelSubscribe}`; `ClientCommand::{Line, Req, Resume, Pong, Caps}`;
  `ClientCaps { kind, color, width, height, utf8, structured }`;
  `MAX_VIEW_DATA_BYTES`; ts-rs export pinned by CI (`ts_export`).
- **Core** — `Registrar::{panel_feed, query_handler, hook, subscribe}`;
  `PanelFeedHandle::{keyframe_to, delta_to}`; `QueryHandler::query(w, actor,
  params) -> Result<OutputEvent, RichText>` (queries never stamp `last_input`,
  pinned in `engine.rs`); `HookPoint::LookRendered` and
  `EventHook::look_rendered(ctx: &HookReadContext, looker, room)`;
  `WorldEvent::AttrSet { obj, name, setter }` (emitted today only by the
  `@set` path in `commands/building.rs`); `Registry::{specs, help_topic,
  help_topic_names, panel_feeds, query_handler}`; `CommandSpec { name,
  aliases, switches, shape, min_role, power, lock, help, scene_class }`;
  `perm::{has_role, has_power, controls}`; `commands::info::{who_entries,
  WhoQuery}`; `output::room_view_with_action_state` and
  `commands::show_room_with_action_state` (the one room-render path);
  `World::{world_ic_now, session_stat}`; `GameClock::{ic_hour, season,
  time_of_day}`; `Audience::{Actor, Room { room, exclude, subject }}`.
- **Line mode** — `mush_ansi::render::{render, render_room, render_narrative,
  render_channel, render_who}`, `wrap::wrap(s, width)`, `AnsiTheme::style_for`,
  the golden suite `crates/mush-ansi/tests/golden.rs`.
- **TUI** — `TuiSession`, `TuiApp { scrollback, who, input, secondary, mail,
  intel, actions, actions_cursor }`, `InputResult { redraw, submit, detach,
  query: Option<&'static str> }`, `SecondaryPane`, the CSI parser in
  `feed_input`/`handle_csi`, `draw` (rail and secondary drop on narrow
  widths), `render::{render_event, render_who_rail, render_mail_panel,
  render_intel_panel, render_legal_actions_panel}`, `SshBackend`
  (`take_output`, `set_geometry`); `editor::LineEditor` (buffer, cursor,
  history, echo; `EchoMode`, `EditorAction`); `hooks::SshHooks::query(conn,
  name)`; `docs/help/tui.md`.
- **Bridges** — `WebBridge { panels }` and its `Query::PanelSubscribe` arm (a
  fixed match: comsys, who, legal.actions, mail, llm.dashboard; else ack);
  `ssh_bridge`'s `query` (a fixed match: who, mail.list, intel.list,
  legal.actions); `GameBridge::query(conn, id, name, args)`; the panels
  manifest assembled in `mush-server/src/lib.rs` (registry feeds plus `who`
  and `legal.actions`).
- **Web** — `GameApp` (`#wire`, `#hydrate`, `#routePanel`, `#routeView`,
  `submit`, `#localCommand`), `SessionStore::{hasFeature, hasPanel,
  sheetFeed, rollFeed}` (suffix convention), `ScrollbackStore`,
  `render.ts::{narrativeLine, channelLine, roomLines, renderRich}`,
  `Game.svelte`, `RightPanel.svelte`, `Scrollback.svelte`,
  `InputBar.svelte`, `WhoRail.svelte`, `SheetPanel.svelte` and
  `sheet.ts` (the schema walk), `app.css` tokens (the palette the mockups
  reuse), `web/src/lib/proto/` (generated, committed).
- **Rulesets** — `SheetSchema::{describe, field}`, `SheetSection`,
  `FieldDef { id, label, ty, category, default, staff_only }`,
  `FieldType::{Dots, Number, Text, Enum, Track { size, levels }, List, Map}`,
  `DamageLevelDef`; `SheetStore::{get, put, put_by, set_field,
  set_map_entry, track_size, wound_penalty}` (`put_by` is the one write
  path: validate, then `World::set_attr`); `register_sheet_commands` (the
  `<ruleset>.sheet` feed and query); `wod.roll` and `sr.roll` feeds;
  Shadowrun `derive::{condition_monitors, ConditionMonitors, wound_modifier,
  limits, initiative_base}`, `schema::{EDGE_CURRENT, INITIATIVE_BASE,
  INITIATIVE_DICE}`; WoD's tagged confirmations (`sys_to_actor(…, "wod",
  …)`, the heuristic the web client refreshes on).
- **Tactics** — `TacticalUnit::{object, schema, revision, sections, cells,
  components, capabilities, capability, mobility}` (private fields,
  accessor-only), `ScalarCell { min, max, current }`, `CellAddr`,
  `SectionState { condition, facets, slots }`, `InstalledComponent {
  definition, install, condition, asset }`, `CapabilityKey { id, source }`,
  `Journal { from_revision, to_revision, entries }`, `UNIT_FEED =
  "tactics.unit"`, the `@unit/damage` delta (`feed.delta_to(actor,
  journal)`), `unit_show` (the text render), `UnitStore` (legacy `reg.state`
  until E7 cut 8 migrates it), `MAX_UNIT_RECORD_BYTES`.
- **Scenes** — `SceneRecord { id, title, kind, mode, status, participants,
  observers, facilitator, created_at, ic_started_at, turn, round,
  turn_index, ended_at }`, `SceneEvent` (witnessed, ledger-sequenced),
  `ScenesState::{active_scene_of, active_scene_at, latest_scene_of}`.
- **NPC** — `THINKING_FEED = "npc.thinking"`, `emit_thinking` (a room-audience
  keyframe `{ npc, name }`; no end frame).
- **Comsys** — `Channel { id, name, description, perms, color, flags, history,
  seq }`, `ChannelFlags` (LOUD, NO_TITLES, LOGGED), `Membership { alias, … }`,
  the `comsys.state` keyframe (`name, alias, title, gagged, color { ansi,
  web_hex }, unread, logged`).
- **Weather** — `WeatherLook::look_rendered` (one `weather`-tagged line),
  `resolve_room_weather`, `conditions_line`, `WxSnapshot { state, visibility,
  precipitation, wind, temperature }` through the registered `WeatherQuery`.
- **Contract tests** — `assert_projection_text_clean`; the rule-1 allowlist
  `crates/mush-contract-tests/legacy-handlers.toml` and its tripwire test.
- **CI** — `cargo fmt --all --check`, clippy `-D warnings`, `cargo test
  --workspace`, the ts-rs export test, `npm ci && npm test && npm run build`.

---

## 3. Cut structure

| Cut | Builds | Sessions | Depends on | Touches |
|---|---|---|---|---|
| I1 | Contracts: vitals feed, `commands.list`, `RoomView.ambient`, scene state, sheet republication, thinking end frame, bridge routing by registry | 2 | nothing | mush-proto, mush-core, mush-ruleset, ruleset-wod, ruleset-shadowrun, mush-weather, mush-scenes, mush-npc, mush-gateway, web proto |
| I2 | TUI screen model and parity floor | 3 | I1 | mush-net-ssh (tui, editor, hooks), docs/help/tui.md |
| I3 | Line-mode typography; roll narration; WHO where | 1 | I1 (ambient) | mush-ansi, mush-core (who policy), ruleset-wod, ruleset-shadowrun (roll lines), docs/help |
| I4 | Web screen shell: screens, palette, composer, status strip, room band, thinking and composing | 2 | I1 | web, mush-gateway (composing), mush-core (composing fan-out) |
| I5 | Web scene typography (direction 1a) | 2 | I4 | web |
| I6 | Ruleset screens on existing data: SR combat screen, WoD vitals and roll builder, shared widgets | 2 to 3 | I1, I2, I4 | web, mush-net-ssh, ruleset-shadowrun, ruleset-wod |
| I7 | Comms IC/OOC | 1 | I2, I4 | mush-comsys, web, mush-net-ssh |
| I8 | Tactical unit screen: mockup T3, mockup T1 unit-side panes, `tactics.vitals` | 2 | I1, I2, I4, I6; coordinated with E7 cut 8 | mush-tactics (`view.rs` only), web, mush-net-ssh |

Order: I1, then I2 ∥ I3 ∥ I4 (disjoint files), then I6 ∥ I7 ∥ I5, then I8.
Roughly 15 to 16 sessions, inside the assessment's 12 to 18.

### 3.1 Cut I1 — Contracts

**What it is.** The one cut that changes the wire. Everything after it is a
renderer. Five contracts, each additive and bounded.

**Vitals** (D-I-2). A ruleset-neutral shape for "the numbers that kill you":

```rust
// mush-proto/src/event.rs — all new, all ts-exported
pub enum VitalRole  { Dominant, Track, Pool, Readout }
pub enum VitalState { Ok, Warn, Crit, Down }
pub enum VitalValue {
    Bounded { current: i64, min: i64, max: i64 },               // heat, blood, edge
    Track   { boxes: Vec<Option<SmolStr>>, level: Option<String> }, // health, condition monitors
    Text(String),                                               // "hdg 047", "walk"
}
pub struct Vital { pub id: SmolStr, pub label: String, pub role: VitalRole,
                   pub value: VitalValue, pub state: VitalState,
                   #[serde(default)] pub detail: Option<String> }   // "shutdown 24", "−2 to every pool"
pub struct VitalsFrame { pub subject: ObjRef, pub revision: u64, pub vitals: Vec<Vital> }
```

A vitals frame rides `Panel { feed: "<owner>.vitals", kind: Keyframe, data }`
and is always whole: at most `MAX_VITALS_PER_FEED` entries, `revision`
monotonic per subject within a process (module-state counter; clients reset
their watermark on every `hello` and on `Resumed { replayed: false }`). Not a
`View`: vitals carry no legality and need no delta algebra, so the
gap-detecting envelope is the wrong tool. Clients key off the `.vitals`
suffix exactly as `SessionStore.sheetFeed` keys off `.sheet`, and keep one
group per feed (a rigger's body and their drone are two groups, D-I-7).

Declared, not hard-coded: `FieldDef` gains `vital: Option<VitalRole>`
(additive, `skip_serializing_if`, builder `.vital(role)`), and
`mush_ruleset::vitals::from_sheet(schema, doc, store)` turns flagged fields
into `Vital`s generically — `Track` → boxes plus the deepest filled level's
label, `detail` from `wound_penalty`; `Number` → `Bounded`. Derived values
that no field holds come from a new default-empty `Ruleset::extra_vitals(doc)
-> Vec<Vital>`. WoD flags health, blood pool (vampire), willpower, rage and
gnosis (werewolf), quintessence and paradox (mage). Shadowrun flags the two
condition monitors and `edge_current`, and `extra_vitals` adds the wound
modifier (Readout, `detail` "−N to every pool"), limits (Readout "P4 M6 S4")
and initiative (Readout "24 + 4D6"). Each ruleset registers `<ruleset>.vitals`
as a panel feed **and** a same-named query handler, so a subscribe yields a
keyframe.

**Sheet republication.** Today no mutation pushes the sheet (the web client
polls and guesses, `web/README.md` "live-enough"). `SheetStore::put_by` is the
single write path but takes `&mut World` and cannot emit. I1 adds
`SheetStore::put_in(ctx, obj, doc, by)`: `put_by` then
`ctx.emit_world_event(AttrSet { obj, name: sheet attr, setter: by })`.
`register_sheet_commands` subscribes to `WorldEventKind::AttrSet`, filters on
its own attribute name, and pushes the `<ruleset>.sheet` keyframe and the
`<ruleset>.vitals` frame to `Audience::Actor(obj)` — never to the room. The
49 call sites across 20 files in `mush-ruleset`, `ruleset-wod` and
`ruleset-shadowrun` migrate mechanically (`put_by(ctx.w, …)` → `put_in(ctx,
…)`); a pinned probe asserts no `put_by(ctx.w` or `.put(ctx.w` remains in
those crates' command code, so a new site cannot silently go quiet. The
first cut pushes whole keyframes; `SheetStore.applyDelta` on the web is
already written for the day a true delta lands.

**The command table** (D-I-5). `Query::Commands` (web) and the query name
`commands.list` (TUI) reach one handler beside `WhoQuery` in
`commands/info.rs`: iterate `reg.specs()`, keep a spec when `min_role` and
`power` pass for the viewer (`perm::has_role`, `perm::has_power`; locks are
per-target and are not evaluated), and emit

```jsonc
{ "commands": [ { "name": "@desc", "aliases": ["@describe"], "switches": [],
                  "usage": "@desc <object>=<text>",      // first line of `help`
                  "summary": "Set what players see …",   // the rest, ≤ MAX_COMMAND_SUMMARY_BYTES
                  "topic": "desc" } ],                    // when a help topic of that name exists
  "truncated": false }
```

sorted by name, stopped at `MAX_COMMANDS_LIST_BYTES` with `truncated: true`
rather than split. Comsys aliases and exits are merged **client-side**: the
client already holds `comsys.state` and the `RoomView`. Staff-gated verbs are
absent for non-staff, not greyed (§8 q7).

**Room ambience** (D-I-4). `RoomView` gains
`ambient: BTreeMap<SmolStr, serde_json::Value>` (defaulted, skipped when
empty). A new `HookPoint::RoomAmbient` with
`EventHook::room_ambient(&self, ctx: &HookReadContext, looker, room) ->
Vec<(SmolStr, Value)>` mirrors `look_rendered`; `room_view_with_action_state`
collects contributions (first writer wins on a key collision, with a
`tracing::warn`), bounded by `MAX_AMBIENT_KEYS` and `MAX_AMBIENT_BYTES`. Core
itself adds `ic_time: { unix, hour, season, time_of_day }` from
`World::world_ic_now` and `GameClock`. Weather contributes `weather: { state,
icon, line }` from `resolve_room_weather`; its `look_rendered` `Sys` line
stays until I3 relabels line mode. Scenes contribute `scene: { id, title,
mode, round, current_actor }` from `active_scene_at(room)`. The label
"03:14, 12 Brumaire" is a client or ruleset concern; core sends the parts.

**Scene state.** `mush-scenes` registers a `scenes` panel feed and a
same-named query: `{ scene: { id, title, kind, mode, status, round, turn,
participants: [ObjRef], observers: [ObjRef], facilitator: ObjRef,
current_actor, started_at, ic_time } | null }` for the actor's
`active_scene_of`, pushed to members on start, advance, pause, resume and
end. A pose counter is deferred: `SceneEvent`s are ledger-sequenced, not
counted in RAM (§8 q13).

**The thinking signal** (D-I-13). No new event. `emit_thinking` gains an end
frame `{ npc, name, active: false }` when the NPC's action lands or the turn
is abandoned; clients also expire a pill after `THINKING_TTL_S`.

**Composing, wire only.** `ClientCommand::Composing { active: bool }` and
`OutputEvent::Composing { actor: ObjRef, active: bool }` are defined here so
the TS types exist; the fan-out and its rules are I4's (D-I-11).

**Bridge routing** (D-I-3). `WebBridge`'s `PanelSubscribe` arm keeps its
legacy matches and adds one generic rule before the bare ack: if a query
handler is registered under the feed's own name, call it for the keyframe.
`ssh_bridge`'s `query` stops filtering by a fixed list and forwards any name
matching `^[a-z0-9_.]{1,64}$` with the caller's args; the engine refuses
unknown names with a polite `Sys`. This is safe because query handlers are
read-only by construction and never stamp idle (the existing pin). To carry
args, `SshHooks::query(conn, name)` becomes `query(conn, name, args:
serde_json::Value)` and `InputResult.query` becomes `Option<(SmolStr,
Value)>`.

**Queries.** `Query::Commands` · `PanelSubscribe { feed: "<ruleset>.vitals" }`
· `PanelSubscribe { feed: "scenes" }` · TUI names `commands.list`,
`<ruleset>.vitals`, `scenes`. **No new mutating verb**; the allowlist is
untouched.

**Tests (~30).** *Proto:* round-trips and TS shapes for every new type; an
empty `ambient` serializes to nothing; a client that predates `Composing`
ignores it (the additive-evolution test extends). *Core:* `commands.list`
hides a wizard-only verb from a player and shows it to a wizard; is
byte-identical across two calls for one viewer (deterministic order); stops
at the cap with `truncated`; `usage` is the first help line and never exceeds
one line; ambient key collision warns and keeps the first; ambient bytes
refuse past the cap; `ic_time` is present on a default clock. *Rulesets:* a
vampire, a werewolf and a mage sheet each yield exactly their flagged vitals;
health level and penalty match `wound_penalty`; Shadowrun yields two tracks,
the wound readout, edge and initiative, and the wound `detail` matches
`wound_modifier`; a `staff_only` field is never a vital for a non-controller;
`put_in` emits `AttrSet` and the subscriber republishes sheet and vitals to
the owner only — a room member receives nothing; `revision` increases across
two writes and a frame never exceeds `MAX_VITALS_FRAME_BYTES`; the no-`put_by(ctx.w`
probe. *Scenes:* the feed fires on every transition and the query answers
`null` outside a scene. *NPC:* the end frame follows every start frame,
including the abandoned-turn path. *Bridges:* web subscribe to `wod.vitals`
yields a keyframe; an unknown feed still refuses; the TUI's dynamic name
reaches the engine and an unknown name yields a `Sys`; no new path stamps
`last_input` (the Query pin re-asserted for each new name).

**Verify lenses.** **P (projection)** — can any vitals, ambient or scene frame
carry a field the sheet renderer hides, a staff-only value, or another
actor's state? **B (bounds)** — can `commands.list`, ambient or vitals grow
with world size or exceed their caps? **Q (queries)** — can a dynamic query
name reach a mutating path, a handler another module owns, or stamp idle?

**Hands to I2, I3, I4:** frames every client can render without knowing the
ruleset.

**As built (2026-09-05, branch `i1-contracts`).** Everything above landed
as specified, with these deviations, each for a reason found in the tree:

1. **`SheetStore::put_in(obj, doc, by, ctx)` takes the context last**, so
   every call site can pass `ctx.actor` as the audit owner without a borrow
   clash (the arguments before `ctx` are evaluated before it is reborrowed).
   The 43 command-side sites in `ruleset-wod`, `ruleset-shadowrun` and
   `mush-ruleset::commands` migrated mechanically; the three `&mut World`
   sites (NPC sheet bootstrap in both rulesets, the store's own helpers) stay
   on `put_by` by design. The probe is
   `crates/mush-contract-tests/tests/sheet_write_republish.rs`.
2. **`Ruleset::extra_vitals` became the `VitalsSource` trait**
   (`mush_ruleset::vitals`): a query handler runs under a `HookReadContext`
   and an observer under a `Ctx`, and the K8.4 authoritative overlays
   (WoD's temporary Willpower, Shadowrun's condition monitors) are applied by
   each ruleset's own readers, so the source supplies `vitals`, `vitals_in`
   and `sheet_keyframe` and the framework supplies the feed, the query, the
   `AttrSet` subscription and the revision counter (`register_vitals`).
   Both rulesets refactored their sheet query onto one `sheet_panel` builder
   that the republish reuses (10b §4 rule 4).
3. **Registry routing moved into the engine, not the bridges.** Instead of
   a query-name list carried to the gateway (which has no registry), the
   engine's `handle_query` answers a name that is a registered panel feed
   with no same-named handler by acknowledging the subscription; the web
   bridge forwards every unknown feed under its own name (the `llm.dashboard`
   arm is gone, `comsys`/`who`/`mail`/`legal.actions` keep their differently
   named queries); the SSH bridge forwards any well-formed name with the
   caller's args (`SshHooks::query` gained `args`; the TUI's
   `InputResult.query` stays a static name until I2).
4. **`HookReadContext::registry()`** — the command table is registry-owned
   and a query handler sees only the read context, so the engine attaches
   the live registry to the context it builds for query dispatch;
   `commands.list` answers only through it and refuses off-loop.
   `commands.list` is also a core panel feed, so it is in the manifest.
5. **Vitals ids are `<ruleset>.<field>`.** WoD flags `health`,
   `willpower_temp`, `bloodpool`, `rage_temp`, `gnosis_temp` (werewolf and
   kinfolk) and `quintessence`; `paradox` is deliberately unflagged, because
   the generic thresholds read "low is bad" and Paradox is the opposite — an
   inverted role or a ruleset-judged state is a later addition. Shadowrun
   flags the two condition monitors and `edge_current`, and adds the wound
   modifier (with its state), the three Limits and Initiative as readouts.
6. **`npc.thinking` frames carry `active`** (`true` on start, `false` on
   completion); no new event.
7. **Composing is wire-only**, as planned: the web transport accepts and
   drops the command until I4 fans it out.
8. **Scenes**: the `scenes` feed, its query and the room glance live in
   `crates/mush-scenes/src/panel.rs`; a released member receives a `null`
   keyframe so their screen clears; the pose counter stays deferred (§8
   q13).
9. **`RoomView.ambient`** is typed `Record<string, unknown>` on the wire;
   core adds `ic_time` after the hooks, so a module calendar may claim the
   key first.

Tests: `crates/mush-proto` (wire round-trips, the ts shapes),
`crates/mush-core/tests/i1_contracts.rs` (command table, ambient), the
engine's subscribe-ack and `commands.list` round-trip, `mush-ruleset`'s
`vitals` unit tests, `crates/ruleset-wod/tests/i1_vitals.rs`,
`crates/ruleset-shadowrun/tests/i1_vitals.rs`,
`crates/mush-scenes/tests/i1_panel.rs`, `crates/mush-weather/tests/i1_ambient.rs`,
the NPC end-frame test in `agents.rs`, and the `put_in` probe. The
TypeScript bindings regenerated (`Vital*.ts`, `VitalsFrame.ts`, the new
`RoomView.ambient`, `Composing`, `Commands`).

### 3.2 Cut I2 — The TUI screen model and parity floor

**What it is.** The TUI stops being three boxes and one Tab-cycled widget
and becomes numbered screens of titled panes with a tab strip, a status line,
a legend, gauges, a speaker gutter and real line editing. Everything mockup
T1–S2 assumes of the shell, and nothing ruleset-specific.

```rust
// mush-net-ssh/src/tui/screen.rs
pub struct ScreenId(u8);                        // 1..=MAX_SCREENS, the hotkey digit
pub struct Screen { name: &'static str, panes: Vec<PaneSpec>, keys: Vec<KeyBinding>,
                    degrade: Vec<PaneId> }      // drop order on narrow/short terminals
pub struct PaneSpec { id: PaneId, title: String, right_title: fn(&TuiApp) -> String,
                      role: BorderRole,         // Neutral | Unit | Ic | Ooc | Danger | Input
                      content: PaneContent, min_w: u16, min_h: u16,
                      select: Option<SelectFn> } // the highlighted row's prefill line, if the pane has rows
pub enum PaneContent { Scrollback, Who, Panel { feed: SmolStr, render: PanelRenderer },
                       Vitals { suffix: &'static str }, Table { feed, columns }, Text(fn) }
pub enum Mode { Nav, Input }                    // D-I-17; TuiApp { mode, draft: Option<String>, .. }
pub struct KeyBinding { key: Key, label: &'static str, action: KeyAction }
pub enum KeyAction { Local(LocalAction),         // a client toggle: focus, filter, zoom, overlay
                     Prefill(&'static str),      // enter input mode with this line ready; nothing sent
                     Run(&'static str) }         // send now — read-only verbs only (D-I-18)
```

Built-in screens: **1 scene** (scrollback, WHO rail, a right column with the
vitals strip and the legal-actions pane), **2 sheet** (the `.sheet` keyframe
walked generically — dots, tracks, numbers, maps — a Rust port of
`web/src/lib/sheet.ts`'s rules; §8 q12), **3 mail** (existing render),
**4 chan** (`comsys.state` channels plus a backlog pane per channel; the TUI
subscribes to `comsys` on entry and requests backlog through the arg-carrying
query path from I1), **5 rolls** (the `.roll` feed ring), **6 intel**,
**7 actions**. `SecondaryPane` and its Tab cycle retire; **Tab / Shift-Tab**
move pane focus inside a screen, **PgUp / PgDn** scroll the focused pane.

Chrome: the tab strip on row 0 (`rustmush  1 scene  2 sheet  3 mail 2 …
<name> ● <n> on <hh:mm>`, unread badges from `MailNotice` and
`comsys.state`), the status line on row 1 (room name and id, zone, weather
state, IC hour, exits — from `RoomView` plus `ambient`), the legend on the
last row. Both chrome rows drop below 12 rows; the WHO rail drops below 100
columns; each screen's `degrade` list drops the rest in order, so 80×24
shows scrollback, input and the status line, 120×34 (the budget) shows the
whole scene screen, and 160×44 is the HUD case.

Scrollback: the ring stores `Entry { gutter: Option<Span>, body: Vec<Span>,
at: i64 }` and paint wraps the body with a hanging indent of `GUTTER_W`;
`render_narrative` fills the gutter with the speaker (player colour) and other
lines with `HH:MM` (the TUI stamps `now` at apply — the envelope timestamp is
web-only). Rolls tagged `roll` (I3) go in the gutter as `⚄ roll`.

Gauges: one row above the input renders every `.vitals` group as `[ label
▰▰▰▱ 6/10 ]` boxes, `Readout`s as text, `Dominant` first; a textual `Prompt`
still shows when no vitals frame exists (line-mode compatibility).

Input (D-I-6): extract `LineBuffer { buf, cursor, history }` from `editor.rs`
(`LineEditor` keeps echo and escape handling on top of it) and drive it from
`TuiApp`: Left/Right/Home/End, Ctrl-A/E, Ctrl-W, Up/Down history
(`TUI_HISTORY`), Ctrl-U. **^P** cycles the sticky mode (`say`, `pose`,
`emit`, `ooc`) shown as `‹pose›` in the input frame; an explicit prefix
(`"`, `:`, `;`, `\`) or a channel alias verb always wins (D-I-8).

**Modes and keys** (D-I-17, D-I-18; owner 2026-09-05). The TUI is modal. It
starts in **nav** mode, where bare keys act: `1`–`9` switch screens,
`Tab`/`Shift-Tab` move pane focus, `↑`/`↓`/`PgUp`/`PgDn` scroll the focused
pane, `g`/`G` jump to its top and bottom, `?` opens the help overlay for the
current screen, `Esc` closes an overlay, and every other key is the current
screen's own binding, printed on the legend (mockup T1's `t target`, mockup
T3's `r file refit job`, mockup S2's `m mark target` are exactly this).
**`Enter` enters input mode**, prefilled from the focused pane's highlighted
row when it has one (a mail row → `@mail 3`, a contact → `+contact 1`, an
exit on the status line → its name) and empty otherwise. The MUSH triggers
open the box too: `"`, `:`, `;` and `\` enter input mode with that prefix
already typed, and **`/` enters input mode with `/` typed and the command
palette open**, filtering `commands.list`, comsys aliases and exits on what
follows the slash — picking an entry replaces the `/query` with the
command's usage (a game command without the slash, a client-local command
such as `/clear` with it), `Esc` closes the palette and keeps the text. An
unbound printable key in nav mode does nothing; the input frame is dim and
reads `‹nav› Enter to type`, so the mode is never a guess.

**Exits are keys on the scene screen** (D-I-19; owner 2026-09-05). Every
exit in the current `RoomView` whose name or alias is exactly one letter —
the classic `North <N>;n;north` — binds that letter while the scene screen
is showing: `n` walks north, `o` goes out, `u` climbs. The binding is
rebuilt from each `Room` event, so it is always the room you are standing
in; the status line prints each exit as `n·north` so the key is visible;
an exit with no one-letter alias (`quai`, `tabac`) is reached by `Enter`
and typing, or by the palette, which lists exits. Precedence on the scene
screen is global keys, then exits, then the screen's own bindings, and the
scene screen never binds the reserved exit letters **`n e s w u d o`** (a
probe over its table enforces it), so a room can always use them; an exit
whose alias collides with a global key (`?`, `/`, a digit, `g`) is not a
hotkey and is never silently remapped. Other screens are free to bind those
letters: on the mail screen `s` may mean *send* and on a designer screen
`n` may mean *new*, because exits are not shown there. On the web the same
letters walk when nothing has focus, beside the room band's exit buttons.

In **input** mode the box is a normal MUSH line: printable keys insert, the
D-I-6 editing keys apply, `↑`/`↓` are history, `Tab` is completion over
`commands.list` (pane focus is a nav-mode key), a `/` typed into an empty
box opens the palette exactly as it does from nav mode, `^P` cycles the
sticky mode, **`Enter` submits and stays in input mode** so a conversation
costs one Enter per line, `Enter` on an empty box returns to nav mode, and
`Esc` returns to nav mode keeping the draft (restored on the next entry).
The input frame is bright with the cursor in it.

**Every nav hotkey stays reachable from the box as Alt+key**: `Alt+1`
switches to screen 1 where a bare `1` did in nav mode, `Alt+t` targets,
`Alt+?` opens help. A terminal sends Alt+x as `ESC x`, so one parser rule —
`ESC` followed by a printable in the same burst is Alt+that key — covers
digits and letters alike; `F1`–`F9` remain a fallback for the digits on
terminals that swallow Alt, and on a terminal where Alt is not Meta (macOS
without "Option as Meta") `Esc` then the key does the same in two
keystrokes. `ESC` that ends a burst is `Esc`, so a split Alt+key degrades to
`Esc` then the key — leave input mode, keep the draft, act in nav mode —
which is the same outcome, so the ambiguity costs nothing. From either mode
**^K** toggles the palette and **^]** detaches.

A binding is `Local` (focus, filter, zoom, overlay), `Prefill` (enter input
mode with the line ready; nothing is sent until Enter), or `Run`, and `Run` is
allowed only for verbs on a checked-in read-only list (`look`, `WHO`,
`+sheet`, `+contacts`, `+orders`…) **plus the one carve-out: an exit's name
on the scene screen** (D-I-18, D-I-19): a probe over every screen's table
refuses anything else, so a bare key can never fire, eject, jack out, spend
or send. Walking is the MUSH's first verb and its undo is walking back;
locked, hex-locked and exclusive-scene rooms already refuse the move
server-side, so the carve-out inherits every existing guard. Mockup T1's
`^F fire group` is a `Prefill("+fire a")`. The legend shows the screen's
bindings plus `Enter type · ? help` in nav mode, and `↑ history · ⇥ complete
· ^P mode · Esc nav` in input mode. The mode resets to nav on TUI entry and
after a resume.

Panel state: `HashMap<SmolStr, PanelState { data, revision, updated_at }>`
keyed by feed; keyframes replace; a delta applies through a per-feed merge
function or requests one resync (the `View` disposition logic generalized,
with the same storm guard as `actions_resync_pending`); at most
`MAX_TUI_FEEDS` feeds, each ≤ `MAX_VIEW_DATA_BYTES`.

Golden frames: a `frame_text(app, w, h)` helper reads the backend buffer as
text; checked-in frames under `crates/mush-net-ssh/tests/golden/` at 80×24,
120×34 and 160×44 for the scene screen with fixture data. `docs/help/tui.md`
is rewritten for the new keys in the same cut.

**Tests (~44).** Screen switching by bare digit in nav mode, by Alt-digit and
F-key in both modes, including the split burst, which must leave input mode
with the draft kept and then switch; Alt+letter in input mode performs the
letter's nav binding and inserts nothing; Shift-Tab (`ESC [ Z`) moves focus
backwards in nav mode and is inert in input mode, and is never mistaken for
Alt+`[`; `Enter` enters input mode empty, or prefilled from a focused mail
row; `Enter` on an empty box returns to nav; `Enter` on a typed line submits
and stays in input mode; `Esc` keeps the draft and the next `Enter` restores
it; `"` and `:` enter input mode with the prefix; `/` enters input mode with
the palette open and a pick replaces the `/query`, keeping the slash only for
a client-local command; a bare letter in nav mode with no binding changes
nothing (frame diff is empty); a bound letter in input mode inserts and never
acts; a `Run` binding to a mutating verb fails the binding-table probe, and
the scene screen's static table never binds `n e s w u d o`; `n` on the scene
screen sends the exit's name after a `Room` with a `North <N>;n` exit and
sends nothing after a `Room` without one; an exit aliased `?` or `3` binds
nothing; an exit letter is inert on the mail screen and returns on the scene
screen; the status line prints `n·north`; the
mode resets to nav on entry and after `Resumed`; every screen at every golden size
and at (1,1), (2,1), (80,1), (1,24), (10,3) without panic; the degrade order
is honoured pane by pane; history recall and cursor edits match
`LineEditor`'s on the same byte sequence (shared-core property test); the
sticky mode prefixing and the explicit-prefix override; a gutter line wraps
with the hanging indent and never bleeds into the gutter column; the vitals
strip renders a track, a pool and a readout and prefers `Dominant`; a textual
prompt shows only without vitals; a delta with no merge function triggers
exactly one resync; an unknown feed is stored, bounded and never painted;
control characters in panel data are stripped; opaque tokens in the actions
pane stay hidden (the existing `assert_projection_text_clean` probe, re-run
through the new pane path); the tab strip badge counts follow `MailNotice`;
the legend text follows the mode.

**Verify lenses.** **G (geometry)** — any size and pane combination that
panics or draws outside its rect? **K (keys)** — any byte sequence that
desyncs the parser, lets a hotkey leak into the input line, lets typed text
act as hotkeys, or lets a bare key send a mutating command? **P
(projection)** — any pane that renders what the line renderer hides? **R
(resync)** — can a malformed delta wedge a pane or storm the engine?

**Hands to I6, I7, I8:** a `PaneContent::Panel` renderer plugs in without
touching `app.rs`; screens are added by a registration, not a rewrite.

### 3.3 Cut I3 — Line-mode typography

**What it is.** Line mode is the parity floor and the surface people script
against, so it changes typographically and keeps its sentences (§8 q3).

- **Room heading.** `render_room` prints `Name #id` (id dim), a rule of `─`
  (`-` when `!caps.utf8`) at `min(width, 72)`, the description, a blank line,
  then labelled facts in a nine-column gutter: `Here     a, b, c` · `Exits
  x  y  z` · `Weather  …` (from `ambient.weather.line` when present; the
  weather module's `look_rendered` contribution is dropped in this cut, other
  `LookRendered` lines still follow as `Sys`).
- **Hanging indent.** `wrap::wrap_indent(s, width, indent)` continues wrapped
  lines under the first word of the body, so a long pose stays one block.
  `Name says, "…"` and `Name pose` keep their classic form.
- **Rolls on one line** (WoD and Shadowrun, the vertical slice). The public
  roll narration becomes `Narrative { kind: Emit }` whose first span is
  tagged `roll`: `roll  Mireille  perception+alertness  8 6 4 3 = 2
  successes  diff 6` (Shadowrun: `roll  Pellet  hacking+logic  … = 4 hits
  (limit 6)`). Line mode prints it as-is; the TUI and web put `roll` in the
  gutter. The private `.roll` panel to the roller is unchanged.
- **WHO Where** (D-I-12). `who_entries` fills `location` when it equals the
  viewer's room (players) or always (staff, unchanged); `render_who` gains a
  `Where` column and the footer `N connected, M in your room.`
- Golden bytes in `crates/mush-ansi/tests/golden.rs` are regenerated and
  reviewed as a diff; `docs/help/getting-started.md`, `communication.md`,
  `wod.md` and `shadowrun.md` update where they show output.

**Tests (~14).** The heading rule never exceeds `caps.width` and collapses to
`-` without UTF-8; labelled facts align at any width ≥ 40 and wrap below the
gutter; a hanging-indent line is CRLF-clean and re-wraps on resize; a `roll`
line round-trips through both rulesets' narration with the same span layout;
`Where` is absent for a player viewing someone elsewhere and present for a
staffer; control characters cannot break the gutter.

**Verify lenses.** **W (width)** — any line that exceeds the width or breaks
the CRLF-only invariant? **I (injection)** — can a name or a description
misalign the gutter or smuggle an escape?

### 3.4 Cut I4 — The web screen shell

**What it is.** Screens become the top-level concept in the browser, and
the mockups' shared chrome lands without choosing a skin.

- **Screens** (`stores/screens.svelte.ts`): built-ins (`scene`, `channels`,
  `mail`, `sheet`, `rolls`, `actions`, `build`, `llm`) plus feed-registered
  ruleset screens (`.vitals` → a status strip on the scene screen; I6 and I8
  add screens). `Game.svelte` becomes a shell: header with the tab strip
  (the same numbers as the TUI), the active screen, the WHO rail as a global
  toggle. `RightPanel` dissolves into screens; the Room panel folds into the
  scene screen's room band.
- **Modes, the same model** (D-I-17): input mode is the composer having
  focus. While no text field has focus, the §7 global keys and the screen's
  bindings act as bare keys; `Enter`, `"`, `:`, `;`, `\` and `/` focus the
  composer, prefilling as on the TUI, with `/` opening the palette behind
  the caret; `Esc` blurs it and keeps the draft; while the composer is
  focused, Alt+key performs the key's nav binding (`Alt+1` for screen 1,
  `Alt+t` to target) with `preventDefault` on handled combos, and `^K`
  toggles the palette regardless of focus; on the scene screen the room's
  one-letter exits walk when nothing has focus (D-I-19), beside the room
  band's exit buttons; a click in the composer is the mouse's `Enter`. The legend renders as a key row on desktop and as
  tappable chips on mobile, where the on-screen keyboard makes focus the
  only mode. One binding table per screen, shared by name with the TUI, with
  the same `Local` / `Prefill` / `Run` kinds and the same read-only rule for
  `Run` (D-I-18).
- **Room band** from `RoomView` plus `ambient`: name, id, zone, weather icon
  and state, IC hour and season, exits as buttons, the scene pill from
  `ambient.scene`.
- **Command palette** (`CommandPalette.svelte`, Ctrl/⌘+K): `commands.list`
  merged with comsys aliases and exits, filter as you type, Enter inserts,
  Ctrl+Enter runs, `?` opens `help <cmd>` in scrollback. `/` stays the
  client-local prefix.
- **Composer** (`Composer.svelte`): a Say / Pose / Emit / OOC segmented
  control with a "renders as" preview, Shift+Enter for a newline collapsed to
  a space on send (the `@mail/quick` precedent), history as today.
- **Status strip** (`VitalsStrip.svelte`) from `.vitals` frames, one group
  per feed, `Dominant` first; replaces the textual prompt when a frame exists.
- **Thinking pill** from `npc.thinking` (cleared by the end frame or
  `THINKING_TTL_S`); **composing indicator** from `Composing`
  (`COMPOSING_TTL_S`).
- **Composing fan-out** (D-I-11): a `WireInbound::Composing { conn, actor,
  active }` the engine answers with `OutputEvent::Composing` to
  `Audience::Room { exclude: [actor] }` under the same perceive gate as room
  contents; the bridge rate-limits to one per `COMPOSING_MIN_INTERVAL_MS`
  per connection (an `active: false` always passes); it never stamps
  `last_input`, never fires for a DARK actor, and is opt-out through a player
  attribute `pref:composing` (§8 q1). The client sends `true` on the first
  keystroke of a draft and `false` on send or after `COMPOSING_IDLE_MS`.
- Hydration adds `panel_subscribe` for every `.vitals` feed and for
  `scenes`, and `Query::Commands` once per hello.

**Tests (vitest ~32).** Screen switching by bare digit when unfocused, by
Alt+digit when focused, and by click; a bare key while the composer is
focused inserts and never acts; `Enter` while unfocused focuses the
composer and a focused list row prefills it; `Esc` blurs and keeps the
draft; a `Run` binding to a mutating verb fails the binding-table test; a
feed-registered screen appears only when the manifest carries it; the
palette merges the three sources and ranks exact prefix first; a staff-only
verb absent from the list never appears client-side either; the composer's
four modes produce the right line and the preview matches `narrativeLine`;
the strip prefers `Dominant` and ignores a lower `revision`; the room band
renders with `ambient` absent; thinking and composing pills expire; a
composing burst sends at most one `true` per interval and always a trailing
`false`; the resume seam re-subscribes vitals and scenes.

**Verify lenses.** **X (injection)** — can server data reach a class name, a
style or a URL the sanitizers do not guard? **R (resume)** — does every
screen survive a reconnect and a full rehydrate without stale state?
**A (access)** — is every screen and the palette operable by keyboard alone?

### 3.5 Cut I5 — Web scene typography (direction 1a)

**What it is.** The skin the Redesign canvas argues for on the scene screen,
applied after the data exists (§8 q11).

- **Speaker grouping** in `ScrollbackStore.push`: consecutive `narrative`
  blocks from the same actor within `GROUP_WINDOW_S`, with no intervening
  room or system block, merge under one name header; emits and spoofs never
  group; OOC, channel and system lines render demoted (small mono, reduced
  contrast).
- **Prose** with a system serif stack (`Georgia, "Iowan Old Style",
  "Palatino Linotype", "Book Antiqua", Charter, serif`); vendoring a font is
  the owner's call (§8 q4).
- **The dice card** inline for the roller from `.roll`; everyone else sees
  the `roll` line from I3.
- **Mobile:** the composer as a bottom sheet with the mode chips, exits as a
  chip row, rails as drawers (existing).

**Tests (~15).** Grouping rules including the window, the room-change break
and the never-group kinds; demoted classes; the serif class applies only to
narrative prose; the card renders for the roller only.

**Verify lens.** **T (typography)** — can grouping ever attribute a line to
the wrong speaker, or hide a system line a player needed?

### 3.6 Cut I6 — Ruleset screens on existing data

**What it is.** The vertical slices the owner opened WoD and Shadowrun for:
the manifest-driven HUD proven on sheets that exist today, with the widgets
every later HUD reuses.

- **Shared widgets** in both clients: `Gauge` (`▰▱` plus digits), `TwoTrack`
  (Shadowrun's monitors with overflow), `Table` (columns declared by the pane
  spec), `KeyValue`, `ConditionMap` (`✓ ◐ ✗` glyphs from opaque condition
  ids through a small display map the ruleset ships as data).
- **Shadowrun combat screen** (TUI screen 6 and web screen `combat` when the
  ruleset is Shadowrun): physical and stun monitors with the wound modifier,
  limits, edge, initiative (base, dice, and the last `+init` result), recent
  `sr.roll` entries; the "next box → −3" warning arrives as `detail` from
  `extra_vitals`. This is mockup S1's meat pane and mockup S2's damage pane
  without the drone or the deck.
- **WoD**: the scene screen's vitals strip (health boxes and level, blood,
  willpower, splat pool) and a roll builder — web: attribute and ability
  pickers from the schema's `category` tags, difficulty 3–9, `/spec` and
  `/wp` toggles, sends `+roll <a>+<b> vs <d>`; TUI: `^R` opens the same as a
  pane overlay.

**Tests (~22).** The combat screen renders from a fixture sheet with no
ruleset knowledge in the client (a probe greps the client for `stun`,
`edge`, `blood`); a `staff_only` field never reaches a widget; a frame with
a lower `revision` is ignored; the roll builder's line round-trips through
the WoD parser; widgets clamp and never panic on `min > max` or empty boxes.

**Verify lenses.** **P (projection)**, **B (bounds)**, **S (staleness)** —
can an older vitals frame ever overwrite a newer one, or a sheet keyframe
and a vitals frame disagree on screen?

### 3.7 Cut I7 — Comms IC/OOC

**What it is.** Mockup T2's structural separation, on comsys, honestly.

- `ChannelFlags.ic: bool`, set through the existing `@channel/set` handler
  (`ic` / `!ic`; baseline in the allowlist — no new verb); `comsys.state`
  adds `"ic": bool` per channel; `Channel` events are unchanged.
- **TUI screen 4** splits into an IC pane (`BorderRole::Ic`, cyan) and an
  OOC pane (`BorderRole::Ooc`, magenta), each listing its channels with
  unread and a merged log; the transmit bar shows the active channel
  (`^N` next channel in the pane, `^O` toggle panes) and prefixes the alias
  verb. **Web** `channels` screen: two columns; the composer frame reads
  `IC · Lance` or `OOC · Public`.
- Line mode is unchanged.

**Tests (~12).** An IC line never renders in the OOC pane and vice versa; a
channel flipped IC↔OOC re-homes its backlog; `^N` wraps within the pane;
the flag survives restart and appears in `comsys.state`.

**Verify lens.** **L (leak)** — can an IC message reach an OOC surface, or a
transmit go to the wrong net after a pane switch?

### 3.8 Cut I8 — The tactical unit screen

**What it is.** Mockup T3 as a screen, then mockup T1's unit-side panes,
from a read-only view of `TacticalUnit`, with every pane E7 has not yet fed
present as a declared placeholder.

- **The view** (D-I-9, D-I-15): a new `crates/mush-tactics/src/view.rs`
  owning `UnitView` and a `tactics.unit` query handler (`{ object: "#n" |
  null }`; default the actor's own object when it is a unit, else a polite
  refusal; permission `perm::controls` or staff — E7 cut 5's active-station
  focus is the eventual authority and the accessor `unit_for_view(ctx, obj)`
  is the one place it will plug in). `unit_show`'s text is refactored to
  render **from the same `UnitView`** (10b §4 rule 4). The DTO carries
  `{ object, name, schema, revision, sections: [{ id, condition, facets: [{
  id, cells }], slots }], cells: [{ addr, current, min, max }], components:
  [{ id, definition, install, condition, asset? }], capabilities: [{ id,
  source }], mobility }` — never `rules`, never a definition body — bounded
  by `MAX_UNIT_RECORD_BYTES`.
- **Journal.** The existing `@unit/damage` delta is the after-action source:
  the client keeps a `JOURNAL_RING` of entries and requests a fresh keyframe
  after each delta, coalesced at `UNIT_RESYNC_MS`.
- **`tactics.vitals`** from unit-level `CellAddr::Unit` cells (role `Pool`,
  label = cell id) until the rules pack declares display metadata (§9).
- **Screens.** TUI screen 8 **damage** and web `damage`: a generic section
  map (each section with facet bars and structure, condition glyphs), the
  before/after table from consecutive frames, the journal pane with `rev ·
  cause · entry`. TUI screen 9 **unit** and web `unit`: sections, plant
  (unit-level pools), components grouped by install section with condition
  and their capabilities, the capability list; grid, contacts, weapons cycle
  and comms strip panes present with the title `awaiting E7 cut 6/7/8` and
  first in the degrade order.
- **Both clients** route the `tactics.unit` feed and subscribe when the
  manifest carries it.

**Tests (~18).** The DTO never contains `rules`, a definition body, an
opaque offer token or another object's id (`assert_projection_text_clean`);
the text render and the DTO name the same sections, cells and components
(one view, one test); DTO bytes ≤ `MAX_UNIT_RECORD_BYTES` for a unit at
`MAX_UNIT_CELLS`; a non-controller is refused without learning whether the
unit exists; a journal delta followed by a keyframe leaves the after-action
table consistent; the placeholders render and degrade first.

**Verify lenses.** **P (projection)**, **C (coordination)** — can the DTO and
`@unit/show` drift, or can the view read state E7's migration has moved?
**B (bounds)**.

---

## 4. Build discipline

10b §4's rhythm applies per cut: (1) confirm the cut's slice against the
tree **by symbol**; (2) build, `cargo clippy --workspace --all-targets --
-D warnings`, `cargo test --workspace`, `cargo fmt --all --check`; (3) a
probe-confirming adversarial verify with the cut's lenses before the next
cut builds on it; (4) fold every confirmed finding with a regression test
named `vf_<lens><n>_…`; (5) commit with the verify outcome in the message;
**push only on the owner's word**.

Added for this programme:

- `cargo gen-ts` after any `mush-proto` change, with `web/src/lib/proto/`
  committed in the same commit (CI's `ts_export` pins it).
- `npm run check && npm test && npm run build` for every cut that touches
  `web/` (CI runs them).
- Every new DTO gets an `assert_projection_text_clean` probe naming the
  fields it must never carry, in the cut that introduces it.
- Every new mutating handler lands on `legacy-handlers.toml` in the same
  commit. I1–I8 add none; I7 rides `@channel/set`, which is baseline.
- Help pages change in the same cut as the behaviour (`docs/help/tui.md`,
  `channels.md`, `getting-started.md`, `communication.md`, `wod.md`,
  `shadowrun.md`).
- Golden frames (TUI) and golden bytes (line mode) are reviewed diffs, never
  regenerated blind.

Six standing rules:

1. **Clients render; they never compute rules.** Thresholds, penalties,
   to-hit, and "next box" warnings arrive as `state` and `detail` from the
   ruleset. A probe greps each client for ruleset vocabulary.
2. **One internal view feeds every projection** (10b §4 rule 4). The unit
   DTO and `@unit/show`, the sheet keyframe and `+sheet`, the vitals frame
   and the textual prompt each derive from one function.
3. **Whole where it can be, gap-detecting where it cannot.** Vitals, ambient
   and scene frames are whole and bounded; legality-bearing state stays on
   `View`.
4. **Additive wire evolution.** Every new field is defaulted and skippable;
   every new event tag is ignorable; TS types are generated, never
   hand-mirrored.
5. **No new idle-stamping path.** Queries, subscriptions and composing never
   touch `last_input`; each cut re-asserts the pin for what it adds.
6. **Degrade, never crowd.** Every screen declares its drop order; the
   tiny-terminal tests stay green; a pane that cannot fit is absent, not
   truncated into nonsense.

---

## 5. Coordination with E7

- **Files E7 owns while I runs:** `crates/mush-tactics/src/{module.rs,
  unit.rs, mobility.rs, contact.rs}`, the `mush-sim` records and handoff,
  and every `mush-core` action, authority, window and kernel file. I touches
  `mush-tactics` only in I8, through the new `view.rs`, one registration
  line, and the `unit_show` refactor to a shared view — agreed with E7 before
  I8 starts.
- **Shared files both may touch:** `mush-proto/src/event.rs` (I adds types;
  E7's `unit.status` is a projection under `Panel`/`View` and needs no proto
  change), the two bridges (I1 generalizes routing; E7's verbs are line
  commands), `mush-server` wiring (I adds no modules). Whoever touches a
  shared file rebases on the other's last commit the same day; a conflict
  resolves in favour of the landed cut.
- **I8's timing.** E7 cut 8 migrates `UnitStore` from legacy `reg.state`
  onto K's record types. `unit_for_view` isolates the read, so I8 lands
  either before or after with a one-function change; the default is after
  I6, and E7 confirms the window (§8 q9).
- **Nothing in I1–I7 waits on E7**, and nothing in E7 waits on I.

---

## 6. Decisions

- **D-I-1** The programme letter is **I**; cuts I1–I8; decisions D-I-n; the
  mockups' screens are cited as *mockup T1* etc. because doc 10 owns T and S
  numbering.
- **D-I-2** Vitals are a **per-ruleset feed** `<ruleset>.vitals` with a fixed
  proto shape, whole frames, a monotonic `revision`, and the `.vitals`
  suffix convention the clients already use for `.sheet` and `.roll`. They
  are **declared** on `FieldDef.vital` and computed generically by
  `mush-ruleset`, with `Ruleset::extra_vitals` for derived values. TacSim
  publishes `tactics.vitals` from unit cells (I8) and, later, from the rules
  pack's display metadata (§9). Rationale: 01 §4 item 1 — one gauge for a
  health track, a condition monitor and a heat cell; the sheet schema is the
  precedent for "declared once, rendered everywhere".
- **D-I-3** Bridges route panel subscriptions and TUI queries **by registry
  lookup** (a query handler under the feed's own name yields the keyframe),
  not by hard-coded arms; the TUI's query names become dynamic and carry
  args. Rationale: every new pane cost an arm in two files; handlers are
  read-only by construction and never stamp idle.
- **D-I-4** `RoomView.ambient` is an **open, bounded map** filled by a
  `room_ambient` hook; core adds `ic_time`; weather and scenes contribute;
  `look_rendered` lines stay for line mode until I3 relabels. Rationale:
  core cannot name module types, and the hook shape already exists.
- **D-I-5** `commands.list` is core-owned, viewer-filtered server-side,
  bounded, deterministic; `usage` is the first help line; aliases and exits
  merge client-side. Rationale: dispatch order (treaty ruling 5) puts
  aliases and exits before the table, and the client already holds both.
- **D-I-6** **One line-editing core** (`LineBuffer`: buffer, cursor,
  history) shared by `LineEditor` and the TUI. Rationale: the TUI has neither
  today and must not grow a second implementation.
- **D-I-7** **Screens are first-class in both clients**, share names and
  numbers, and split by kind: the scene screen is prose-first (direction 1a),
  ruleset screens are console-dense (direction 1b). "Two bodies get two
  panes" is a client rule: a remotely operated unit keeps the operator's
  body as its own vitals group and pane. Rationale: 01 §4 items 3 and 4.
- **D-I-8** Mode prefixing (`^P`, the composer) is **client-side**; an
  explicit prefix character or alias verb always wins. Rationale: one parser
  (02-connectivity's "the buttons submit Lines"); veterans keep their habits.
- **D-I-9** The unit DTO carries structure, cells, conditions, capabilities
  and mobility — **never `rules`, never a definition body** — and derives
  from the same `UnitView` as `@unit/show` (10b §4 rule 4). Rationale:
  balance data and definitions are the ruleset's; projection cleanliness is
  the standing rule.
- **D-I-10** IC/OOC is a **comsys channel flag** now; nets with signal are
  the propagation layer's, and comsys never fakes signal. Rationale: 01 §4
  item 5.
- **D-I-11** Composing is **room-scoped, perceive-gated, rate-limited at the
  bridge, opt-out, and never stamps idle**; an `active: false` always
  passes; DARK actors never emit it. Rationale: presence must not become
  surveillance or an idle exploit.
- **D-I-12** WHO location for players is **same-room only**; staff unchanged.
  Rationale: today's policy is privacy by omission; "who is near me" is the
  question the mockups answer, and the room already shows those people.
- **D-I-13** The thinking signal is the existing `npc.thinking` feed plus an
  **end frame**; no new event. Rationale: 01 §2.
- **D-I-14** Typography stays **self-contained**: a system serif stack; no
  CDN; vendoring is an owner choice. Rationale: `web/README.md`'s rule.
- **D-I-15** I touches `mush-tactics` only through `view.rs` and one
  registration; E7 owns the module and unit files; I8 isolates its read
  behind `unit_for_view`. Rationale: §5.
- **D-I-16** Line mode keeps its classic sentences (`Name says, "…"`); the
  speaker gutter is a screen thing (TUI and web). Rationale: line mode is
  what people script against and read on old clients; typography there is
  rule, gutter-labels, hanging indent and one-line rolls.
- **D-I-17** **The screens are modal** (owner 2026-09-05). Nav mode is the
  resting state: bare keys navigate and act; `Enter`, the MUSH triggers `"`
  `:` `;` `\`, and `/` (with the palette) enter input mode; `Enter` on an
  empty line and `Esc` leave it; a typed `Enter` submits and stays. **Every
  nav hotkey is Alt+key from inside the box.** On the web, input mode is
  composer focus. The mode is always visible in the input frame and the
  legend, and resets to nav on entry and resume. `^K` and `^]` work from any
  mode. Rationale: the mockups' legends are single letters, which only a
  modal shell can honour; `?` and `/` become real keys; one Enter per line
  keeps conversation cheap; the trigger keys keep the MUSH reflex; Alt+key
  means switching modes is never required to reach a hotkey.
- **D-I-18** **A bare key never sends a mutating command, with one
  carve-out.** Bindings are `Local`, `Prefill` or `Run`; `Run` is confined
  to a checked-in read-only verb list and a probe enforces it in both
  clients; anything that fires, spends or sends is a `Prefill` that waits
  for the user's `Enter`. The carve-out is D-I-19's exit walk on the scene
  screen. Rationale: mode confusion is the failure mode of every modal UI,
  and this rule makes its worst case "a screen switched, or a room walked
  out of" rather than "a missile launched".
- **D-I-19** **Exits are nav keys on the scene screen** (owner 2026-09-05).
  A one-letter exit name or alias in the current `RoomView` binds that
  letter while the scene screen shows; the binding is rebuilt on every
  `Room` event; precedence is global keys, exits, then the screen's own
  table; the scene screen reserves `n e s w u d o` and never binds them;
  other screens may. Rationale: `n` for north is the oldest MUSH reflex
  there is; the reservation keeps rooms and the shell from fighting over the
  same letters, and confining it to the scene screen leaves every other
  screen its full alphabet.

---

## 7. Constants

| Constant | Value | Where |
|---|---|---|
| `MAX_VITALS_PER_FEED` | 16 | proto (checked at construction) |
| `MAX_VITAL_DETAIL_BYTES` | 64 | proto |
| `MAX_VITALS_FRAME_BYTES` | 8 KiB | ruleset publisher |
| `MAX_COMMANDS_LIST_BYTES` | 64 KiB | core query |
| `MAX_COMMAND_SUMMARY_BYTES` | 160 | core query |
| `MAX_AMBIENT_KEYS` / `MAX_AMBIENT_BYTES` | 8 / 2 KiB | core `RoomView` builder |
| `COMPOSING_MIN_INTERVAL_MS` | 2000 | bridge |
| `COMPOSING_IDLE_MS` | 5000 | web client |
| `COMPOSING_TTL_S` / `THINKING_TTL_S` | 8 / 20 | clients |
| `MAX_SCREENS` / `MAX_PANES_PER_SCREEN` | 9 / 8 | TUI |
| `GUTTER_W` (screens) / line-mode fact gutter | 10 / 9 | TUI, web, `mush-ansi` |
| `TUI_HISTORY` | 200 | TUI (matches the web) |
| `SCROLLBACK_CAP` | 1000 (existing) | TUI |
| `MAX_TUI_FEEDS` | 32 | TUI, each ≤ `MAX_VIEW_DATA_BYTES` |
| Golden sizes | 80×24, 120×34, 160×44 | TUI tests |
| `GROUP_WINDOW_S` | 300 | web grouping |
| `JOURNAL_RING` / `UNIT_RESYNC_MS` | 200 / 250 | clients (I8) |
| Unit DTO bound | `MAX_UNIT_RECORD_BYTES` (existing) | `view.rs` |

**Global keys (both clients; a screen may not rebind these).**

| Key | Nav mode | Input mode |
|---|---|---|
| `1`–`9` | switch screen | insert |
| `Enter` | enter input mode, prefilled from the focused row if any | submit and stay; on an empty box, leave to nav |
| `"` `:` `;` `\` | enter input mode with the trigger typed | insert |
| `/` | enter input mode with `/` typed and the palette open | insert; on an empty box, open the palette |
| `Esc` | close overlay | leave to nav, keep the draft (close the palette first if open) |
| `Tab` / `Shift-Tab` | pane focus forward / back | completion |
| `↑` `↓` | scroll the focused pane | history |
| `PgUp` `PgDn` | scroll the focused pane | scroll the scrollback |
| `g` / `G` | top / bottom of the focused pane | insert |
| `?` | help overlay for the screen | insert |
| Alt+key | the key's nav action | **the key's nav action** (`Alt+1` = screen 1, `Alt+t` = target, `Alt+?` = help) |
| `F1`–`F9` | switch screen | switch screen |
| `^K` | toggle the palette | toggle the palette |
| `^P` | cycle the sticky mode | cycle the sticky mode |
| `^]` | detach (TUI) | detach (TUI) |
| one-letter exit (scene screen only) | walk that exit | insert (Alt+letter walks) |

Everything else in nav mode belongs to the current screen and is printed on
its legend; an unbound key does nothing. A screen's bindings are therefore
reachable two ways always: the bare key in nav mode, Alt+key in input mode.
The scene screen reserves **`n e s w u d o`** for exits and never binds them
itself; every other screen may use them.

---

## 8. Open questions (defaults chosen; ★ marks an owner decision)

**Owner 2026-09-05: every default below is confirmed; q5 is closed by the
modal ruling (D-I-17, D-I-18) rather than by its default.**

1. ★ **Composing default.** On with per-player opt-out (default), or off
   with opt-in. Never for DARK actors either way.
2. ★ **WHO Where for players.** Same-room only (default), or every room with
   a `@hide`-style opt-out.
3. ★ **Line mode keeps classic sentences** and gets typography only
   (default, D-I-16), or adopts the gutter form too.
4. ★ **Serif font.** System stack (default) or a vendored face for 1a.
5. **Screen keys.** *Closed by the owner:* the shell is modal — bare digits
   and letters act in nav mode; `Enter`, `/`, `"` and `:` open the text box;
   every nav hotkey is Alt+key from inside the box (F-keys as the digit
   fallback). §3.2, D-I-17, D-I-18, §7.
6. **Palette.** Enter inserts, Ctrl+Enter runs (default).
7. **Staff-gated verbs** absent from `commands.list` for non-staff (default),
   not greyed.
8. ★ **Section map.** A generic section list ships in I8 (default); a
   ruleset-supplied silhouette is a later data addition.
9. **I8 timing** relative to E7 cut 8: after I6, window confirmed with E7
   (default); the accessor makes either order safe.
10. **Vitals ownership.** Per-ruleset feeds on the suffix convention
    (default) rather than one core-owned `vitals` feed with providers.
11. ★ **I5 before or after I6.** After (default): data before skin.
12. **TUI sheet screen.** Walk the `.sheet` keyframe in Rust (default) rather
    than showing `+sheet`'s text; it is the same walk the HUD tables need.
13. **Pose counter.** Deferred (default) until scenes keep a RAM count or
    the ledger exposes one; the scene pill shows `round` and `turn`.

---

## 9. What I hands back

- **To E7.** The `.vitals` contract and suffix convention, so cut 8's unit
  projection publishes `tactics.vitals` on tick commits to the focused
  station's actor; the `UnitView` function (rule 4) and the screen model the
  HUD hangs on. **One ask for cut 1:** display metadata on cells in the rules
  pack — `{ label, unit, role, warn, crit, dominant }` — and a `keys` legend
  list per screen — `{ key, label, prefill }` — so `tactics.vitals`, mockup
  T1's plant and sections panes, and the HUD's own hotkeys are data, not
  code (a pack can only ever declare `Prefill` and `Local` keys; D-I-18).
  **One ask for cut 7:** shape the contacts DTO (`tactics.contacts`) as a
  table-shaped keyframe with declared columns so the screen model renders it
  unchanged, and its rows carry the prefill line `Enter` uses.
- **To E6.** The `scene` ambient key and the `scenes` feed read only what
  E6b owns; if `SceneRecord` changes, the ambient contributor is the one
  place to update. `scene.outcome` stays E6b's.
- **To the rulesets.** `FieldDef.vital`, `Ruleset::extra_vitals`,
  `SheetStore::put_in` and its probe, the `roll` narration tag.
- **To the web client's known gaps** (`web/README.md`): the sheet keyframe on
  mutation closes "live-enough, not truly live"; `SheetPanel`'s poll and the
  refresh heuristic can retire after I1.

---

## 10. Changelog

- **Draft 0.3 — 2026-09-05.** Owner ruling: one-letter room exits are nav
  keys on the scene screen (§3.2 "Exits are keys", D-I-19, the D-I-18
  carve-out, the §7 row and reserved letters, the I2 test cases, the §3.4
  web line).
- **Draft 0.2 — 2026-09-05.** Owner rulings folded: the shell is modal
  (§3.2 "Modes and keys", the web parity item in §3.4, D-I-17, D-I-18, the
  global key table in §7, q5 closed); `/`, `"` and `:` open the box (`/`
  with the palette); every nav hotkey is Alt+key from inside the box; every
  other §8 default confirmed. The screen sketch gains `keys`, `select` and
  `Mode`; I2 and I4 test lists grow the mode cases; §9's cut-1 ask adds the
  `keys` legend list.
- **Draft 0.1 — 2026-09-05.** First plan, scoped from 01-assessment.md on
  the owner's brief.
