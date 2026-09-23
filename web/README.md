# rustmush web client

Svelte 5 (runes) + Vite + TypeScript, pure SPA — no SvelteKit, no xterm.js,
no external CDNs or fonts (system font stack; fully self-contained — the
only `https://` strings in the bundle are Svelte's inert error-doc links).
Built to `web/dist` and embedded into `mush-net-web` via rust-embed at cargo
build time. Dark theme by default; light theme via the header toggle or
`/theme` (persisted in localStorage; "system" follows
`prefers-color-scheme`).

## Dev workflow

```
npm install          # once
cargo run -p mushd   # the game (serves /api + /ws on 127.0.0.1:8080)
npm run dev          # vite dev server; proxies /api and /ws to mushd
```

```
npm run build      # typecheck (svelte-check) + vite build -> web/dist
npm run check      # svelte-check only
npm test           # vitest run (ws, render, stores, auth, sheet, roll, worldgen, unit, ...)
npm run test:watch # vitest watch mode
```

`web/dist` and `node_modules` are gitignored; `src/lib/proto/` is **committed**
(see below), so building the SPA never requires cargo.

## Tests

`vitest` runs with `environment: "node"` — no jsdom, no setup file. Pure
libs and rune stores are tested directly; the two layers above them have
their own conventions:

**The transport** (`ws.test.ts`) injects a `MockWebSocket` through
`WsClientOptions.webSocketFactory` and drives framing, resume, ping/pong,
backoff and the close codes with `vi.useFakeTimers()`.

**The controller** (`game.test.ts`) does the same one level up:
`new GameApp({ url, webSocketFactory, random })` (`GameAppOptions` forwards
all three to `WsClient` unchanged) plus `app.client.connect(token, char)`
drives the real hello → hydrate → route path against an inlined
`FakeSocket`. The outbound `sent` array is the assertion surface for
everything `#req` does, and the public stores (`app.unit`, `app.sheet`,
`app.screens`, `app.scrollback`, …) for everything the frames land in. Four
things the harness will bite you with:

- `FakeSocket.open()` MUST set `readyState = 1`. `WsClient.send` drops every
  frame silently otherwise, so `#req` returns `false` and the storm guards
  release themselves — the test then passes while asserting the opposite of
  the truth.
- `url` and `webSocketFactory` travel together. Node defines a global
  `WebSocket`, so threading only `url` attempts a real connection instead of
  throwing.
- Correlation ids are never reset (`WsClient.#nextReqId` climbs across
  reconnects). Read the id back off the last `{t:"req"}` frame; never
  hardcode one.
- Never `vi.runAllTimers()`: toasts arm 6 s, the mail helpers 400 ms and the
  reconnect backoff 500 ms+, so it mints fresh sockets. Advance to the exact
  boundary (`UNIT_RESYNC_MS - 1`, then `1`).

**Not covered by it**: the auth calls (`login`/`register`/`logout` need a
`fetch` stub) and everything that renders a `.svelte` file, which needs a
jsdom environment and a component-testing dependency — those assertions are
still `readFileSync` source probes (`unit.test.ts`'s widget block,
`vitalsboard.test.ts`'s vocabulary lint). The credentialled reset seam
(`enterGame` → `#resetGameState`) needs a `localStorage` stub, which
`game.test.ts` puts in with `vi.stubGlobal` for the few tests that reach it.

`app.composer` is a plain `ComposerHandle` field, so the shell's key model
needs no DOM either: `game.test.ts` assigns a recording fake and drives
`GameApp.act` through it. `focusComposer` defers to `queueMicrotask`, which
`vi.useFakeTimers()` does NOT fake — one `await Promise.resolve()` is enough
to flush it.

## Generated protocol types — `src/lib/proto/`

Every file in `src/lib/proto/` is generated from the `mush-proto` crate via
ts-rs — **do not edit or hand-mirror wire types**. Regenerate after any change
to `crates/mush-proto/src`:

```
cargo gen-ts                              # alias for:
cargo test -p mush-proto --features ts    # (crates/mush-proto/tests/ts_export.rs)
```

Generation is idempotent and rebuilds the barrel `index.ts`; import from
`$lib/proto` only. Representation notes (asserted by the
`generated_shapes_match_wire_format` test on the Rust side):

- `OutputEvent` / `ClientCommand` / `Query` are adjacently tagged — TS
  discriminated unions on `"t"` with the payload in `"d"`.
- `Envelope` is `{ seq, ts } & (…OutputEvent union…)` — the event flattens
  into the envelope, exactly like the wire (`{"seq":4212,"ts":…,"t":…,"d":…}`).
- `u64`/`i64` are `number` (JSON numbers), never `bigint`.
- Fields skipped when unset (`skip_serializing_if`) are optional (`?`).
- `Color` keeps its three wire forms: `{"ansi":n} | {"idx256":n} | {"rgb":[r,g,b]}`.

## Layout

```
src/
├── main.ts                   # mounts App, imports app.css (theme tokens + tag classes)
├── App.svelte                # screen router: login → character select → game
├── components/
│   ├── Login.svelte          # /api/auth login+register card (friendly errors)
│   ├── CharSelect.svelte     # character grid from the login payload
│   ├── Game.svelte           # the shell (I4): tab strip of numbered screens, WHO rail toggle, the composer + palette under every screen but channels (16b cut B.1), legend chips, key help
│   ├── SceneScreen.svelte    # screen 1: room band + scrollback + vitals strip
│   ├── RoomBand.svelte       # RoomView + ambient: zone, weather, IC clock, scene pill, exits with nav keys, presence pills
│   ├── VitalsStrip.svelte    # every `.vitals` frame as gauges (dominant first)
│   ├── Composer.svelte       # the box: Say/Pose/Emit/OOC sticky mode, preview, history, completion, composing signal
│   ├── CommandPalette.svelte # commands.list + aliases + exits, filtered after the slash
│   ├── Scrollback.svelte     # windowed list, scroll-lock, "N new lines" pill
│   ├── RichLine.svelte       # span renderer (tag class, style, CommandLink)
│   ├── InputBar.svelte       # the channel tab's input: Enter submits, ↑/↓ history (200), masked mode
│   ├── ChannelTabs.svelte    # the channel list, IC and OOC apart: color chips + unread badges (I7)
│   ├── GenericView.svelte    # list/detail/form over a view schema, ruleset-blind (I9)
│   ├── GenericViewScreen.svelte # a screen backed by a `.view` feed (I9)
│   ├── RollBuilder.svelte    # trait pickers from the schema's categories -> one `+roll` line (I6)
│   ├── VitalsBoard.svelte    # every `.vitals` frame as widgets + recent rolls (I6)
│   ├── UnitScreen.svelte     # the unit screen: damage / unit layouts over the `.unit` frame (I8, probed)
│   ├── UnitSectionMap.svelte # one block per section: state glyph + condition word + cell gauges (I8, probed)
│   ├── CockpitScreen.svelte  # screen 0: the arena cockpit — canvas map, status, damage, weapons, contacts (16b Q.5)
│   ├── widgets/              # Gauge, TrackBoxes, KeyValue, Table — ruleset-blind (I6, probed)
│   ├── ChannelView.svelte    # one channel's backlog/live view + alias input
│   ├── WhoRail.svelte        # who table, idle ticking, refresh
│   ├── MailPanel.svelte      # the mail screen: folders + entries; open = `@mail <n>`
│   ├── ComposeModal.svelte   # one-shot `@mail/quick`
│   ├── SheetPanel.svelte     # the sheet screen: schema-driven character sheet (M6b — see below)
│   ├── RollsPanel.svelte     # the rolls screen: wod.roll readout (M6b)
│   ├── ActionsPanel.svelte   # the actions screen: legal.actions offers
│   ├── LlmPanel.svelte       # the llm screen: staff cost dashboard
│   └── WorldgenPanel.svelte  # the build screen: worldgen.draft preview (M7 — see below)
└── lib/
    ├── proto/                # GENERATED wire types (cargo gen-ts) — committed
    ├── ws.ts                 # WsClient: connect/send/dispatch/resume/keepalive/backoff
    ├── game.svelte.ts        # GameApp: WsClient → stores wiring, Req routing, /commands, the shell's actions
    ├── keys.ts               # the modal key model (I4): bare keys vs Alt+key, exits as keys
    ├── cockpit.ts            # the arena feeds' types + decoders, the ONE map composition, the view model (16b Q.5)
    ├── palette.ts            # palette ranking (exact prefix first)
    ├── composer.ts           # sticky-mode line shaping + the "renders as" preview
    ├── composing.ts          # the composing signal (start once, stop on send/idle)
    ├── roomband.ts           # RoomView + ambient → the band's model
    ├── render.ts             # pure span/line renderers (mirrors mush-ansi formatting)
    ├── sheet.ts               # sheet schema/doc types + rendering helpers (hand-written; see below)
    ├── roll.ts                # Owod20Outcome types + roll-readout formatting (hand-written)
    ├── worldgen.ts             # worldgen.draft payload parsing + @gen/* Line builders (hand-written)
    ├── auth.ts               # /api/auth/* wrappers + friendly error mapping
    ├── theme.ts, storage.ts, history.ts
    └── stores/               # rune stores: scrollback, channels, who, mail, session, sheet, rolls, worldgen,
                              #   screens, commands, vitals, presence (I4), unit (I8)
```

## Screens and keys (interface cut I4)

The browser is a set of **numbered screens** (`$lib/stores/screens.svelte.ts`)
sharing names and digits with the SSH TUI where both have the screen: 1
scene, 2 sheet, 3 mail, 4 chan, 5 rolls, 7 actions, 8 build, 9 llm, 0
cockpit. The TUI's
6 is its intel screen and its 8 and 9 are the tactical damage and unit
screens; here 6 is the one digit the built-ins leave free, and since
interface cut I8 a server that
publishes a `.unit` feed claims it for the **unit** screen (see below).
16b cut Q.5 took the tenth digit for the **cockpit** screen, which is last in
the strip — `1 … 9 0`, the order of the digits on a keyboard, and the order
the TUI's tab bar reads — so `activeDef`'s fallback is still the scene screen;
`freeDigit` still hands out only 1–9, so a ruleset screen can never take it.
Each
screen is gated on the hello's manifest; a ruleset screen registers by a
panel-feed suffix (`registerFeedScreen`) and appears only when the server
publishes that feed. One binding table per screen — `local` / `prefill` /
`run` (plus `local: "toggle"` for a screen with two layouts) — with
`run` held to a read-only verb list by `validateScreens` (D-I-18), so a bare
key never sends a mutating command; the one carve-out is walking an exit on
the scene screen (D-I-19).

That verb list (`RUN_ALLOWLIST`) is one of two hand-kept copies — the TUI's
`tui/screen.rs` is the other — and both are pinned to the committed fixture
`src/lib/run-allowlist.json`, by a vitest here and a Rust test there, so a
verb added to one and not the other fails a build instead of drifting in
silence. The look-back took two entries off it: `@mail <n>` marks the
message read and `+orders` writes work orders, and neither belonged on a
list whose whole promise is that a bare key mutates nothing. Eleven verbs are
actually bound: `look`, `inventory`, `+roll/last` and `+contacts`, plus the
cockpit screen's seven reading verbs (`view`, `contacts`, `status`,
`weaponstatus`, `range`, `bearing`, `scan`) since 16b cut Q.5; the rest are
reserved for the screens a ruleset declares.

The shell is **modal** (D-I-17): input mode is "the composer has focus".
While nothing has focus, `1`–`9` and `0` switch screens, `Enter` opens the composer
(in the TUI a highlighted row prefills it; the web has no row selection —
its rows are buttons), `"` `:` `;` `\` open it with the trigger
typed, `/` opens the palette, `?` the key help, the room's one-letter exits
walk, and a letter runs the screen's own binding (the chips on the legend
row — on a phone, where the on-screen keyboard makes focus the only mode,
the chips are the whole story). Inside the composer, `Alt+key` performs the
key's nav action (`Alt+1`, `Alt+n`, `Alt+?`) and `Ctrl/⌘+K` toggles the
palette; `Enter` sends and keeps focus, `Enter` on an empty box and `Esc`
blur it (`Esc` keeps the draft), `Shift+Enter` is a newline collapsed to a
space on send. With the palette open, `Enter` inserts the highlighted row,
`Ctrl+Enter` runs it, `?` asks the game for that row's help page
(`help <cmd>` — read-only, so a bare key may send it; a client-local row
prints `/help` instead) and `Esc` closes it keeping the text. The whole
table is `$lib/keys.ts`'s `navIntent`, pinned by vitest without a DOM; what
each intent then DOES is `GameApp.act`, driven in `game.test.ts` against a
fake `ComposerHandle`. The composer and its palette are the shell's
(`Game.svelte`, 16b cut B.1), under every screen but the channel screen, which
has its own box addressed to the channel; from there alone `focusComposer`
switches to the scene screen, so a tray action or `Enter` still reaches a box.

The composer's Say / Pose / Emit / OOC control is client-side (D-I-8): an
explicit trigger or a channel alias verb always wins (`$lib/composer.ts`).
A prefill — a binding, a click, a palette pick, a tray action — always drops it
back to a plain command (`modeAfterPrefill`, 16b cut B.1): the box outlives a
screen switch, and Say set in a room must not turn `lock BA` into speech.
Every keystroke feeds the **composing signal** (`$lib/composing.ts`): the
client sends `composing { active: true }` on the first keystroke of a draft
and `false` on send or after 5 s of silence; the gateway rate-limits starts
to one per 2 s per connection and the engine fans the hint out to the room
minus the typist, perceive-gated, never for a DARK actor, never when
`pref:composing` is `off`, and never as an idle stamp (D-I-11). The room
band shows the pills (`$lib/stores/presence.svelte.ts`, 8 s TTL; NPC
"thinking" pills from `npc.thinking`, 20 s TTL).

The **vitals strip** (`$lib/stores/vitals.svelte.ts`) renders every
`<ruleset>.vitals` frame the hello manifest names — subscribed on hello and
on every full rehydrate — dominant first, ignoring a lower revision; it
replaces the textual prompt when a frame exists.

**A revision only means anything within one run of the world.** A gateway
session outlives a world restart (a web socket survives with no fresh Hello)
while the world's revision counters restart at zero, so a viewer holding
revision N would drop every frame the restarted world publishes and read
stale gauges as live for the rest of the session. `resetWatermarks()` marks
every held group accept-next — one frame per feed, which re-seeds the
watermark — and is called at each seam where the client may now be looking
at a different run: a `hello`, `onFullRehydrate`, and a replayed `resumed`
(the one reconnect that brings neither). The frames are KEPT: a stale gauge
is a smaller lie than a blanked strip on a routine reconnect.

## Scene typography and comms (interface cuts I5, I7)

The scene screen groups consecutive speech and action by one actor under a
single name header (`groupBlocks` in `$lib/stores/scrollback.svelte.ts`: a
pure function over the visible window, five-minute window measured from the
previous line, broken by any other block, never for emits, spoofs or OOC),
sets prose in a system serif stack (`--font-prose`, class `ev-prose`) and
demotes OOC, channel, system and roll lines to small mono (`ev-demoted`).
Every system line the GAME sent is demoted — the `sys` event, the typed-WHO
echo, the generic reply-error line and an action receipt alike; the one
undemoted system family is the client-local `ev-sys ev-local` line, italic
instead, because it never went to the game at all.
The roller's own `.roll` frame lands inline as a dice card
(`RollCard.svelte`); everyone else reads the server's one-line `roll`
narration. On a phone the composer is a sticky bottom sheet.

Channels carry an `ic` flag (`@channel/set <c>=ic`, reported by
`comsys.state`): the channels screen lists IC and OOC channels apart
(`ChannelTabs.svelte`, `channelsWhere`), the channel view's head names the
half, and an IC channel's line on the scene screen reads as prose.

## Generic view screens (interface cut I9)

A module that publishes a schema-bearing keyframe on a `<name>.view` feed
gets a screen with no bespoke code. `$lib/genericview.ts` is the payload
convention (`{ schema: { shape: list | detail | form, columns, fields,
actionKey }, rows | record }`) — a convention walked at render time like the
sheet schema, not a proto type, so it adds no wire and never touches the
authoritative `ViewEnvelope` payload. `GenericView.svelte` renders it (a
table, or a key/value list); `GenericViewStore` holds one per feed;
`ScreensStore.registerViewScreen` auto-registers the screen on the next free
hotkey digit (only 6 is free, and on a server carrying a `.unit` feed the
I8 unit screen has already taken it — see the recorded cost below; a
digitless-screen policy is the follow-up). Either registration can be
REFUSED, and the reason is now pushed to the scrollback as a client-local
line rather than discarded: a screen that quietly never appears is the
hardest kind of bug to report. A row's action is always a Prefill
(D-I-18): a click opens the composer with the line ready. The SSH TUI walks
the identical convention in `render_view_rows`.

Two rules the look-back added. **`r` on a view screen refreshes its own
feed** — `localAction("refresh")` names every built-in and used to fall
through to `refreshWho()`, so a module's view refreshed the WHO list; the
arm reads `ScreenDef.viewFeed`, so it stays data-driven like the TUI's
`screen_queries()`. And **`parseViewSchema` dedups column and field keys**
(first wins, D-I-4): both renderers key their `#each` block by the
producer-chosen key, and Svelte throws `each_key_duplicate` out of the whole
flush on a repeat — a producer shipping two columns called `id` would take
the client's render down, not just its own view. Deduped before the cap, so
duplicates cannot squeeze real columns out.

## Ruleset screens on existing data (interface cut I6)

The rolls screen is the dice-and-damage screen: `RollBuilder.svelte` builds
one `+roll[/spec] <a>[+<b>] vs <d>` line from the sheet schema's
`category`-tagged dotted fields (`$lib/rollbuilder.ts` — no trait name
lives in the client), and `VitalsBoard.svelte` renders every `.vitals`
frame through the shared widgets in `src/components/widgets/` (`Gauge`
`▰▱`, `TrackBoxes` in rows of three with the level and the frame's
`detail`, `KeyValue` with condition glyphs, `Table`) plus the recent rolls.
`vitalsboard.test.ts` greps those files for ruleset words; `widgets.ts`
clamps every input. `v` re-subscribes the vitals feeds. No screen digit
was free for a separate combat screen (the plan's I8 wants 8 and 9 on the
TUI and the web has only 6 left), so the board lives on 5.

**How that prediction resolved (I8, D-I8-4)**: the web ships ONE screen on
digit 6 hosting both of the TUI's two, with `t` flipping damage/unit — the
TUI keeps 8 damage and 9 unit. The cost is recorded rather than hidden: on
a server publishing both a `.unit` feed and a `.view` feed,
`registerViewScreen` is now refused with "no free hotkey digit"
(deterministically — the unit screen registers synchronously in the hello
pass, before any `.view` keyframe can arrive), and `screens.test.ts` pins
exactly that. The follow-up is a digitless-screen policy (a tab with no
hotkey), not a re-shuffle of the built-in digits, which would break
established muscle memory and the TUI parity the digit table exists for.

## Tactical unit screen (interface cut I8)

One screen on digit 6 (`UnitScreen.svelte`), gated on a `.unit` feed in the
hello manifest and registered exactly once from `GameApp.#hydrate` —
`registerFeedScreen` has no idempotency guard and `ScreensStore.clear()`
keeps `feedScreens`, so a second hello would otherwise be refused with
"screen unit declared twice". A character switch is the opposite case and
has its own seam: `#resetGameState` calls `clearFeedScreens()`, because the
registered screens belong to the manifest of the server being left — a
`foo.view` screen holding digit 6 on server A would otherwise refuse this
screen on server B, silently. Keys: `r` refresh, `t` flip layout, `c` run
`+contacts`, `d` prefill `@unit/damage `. `@unit/*` is staff-gated and
mutating, so it is a Prefill and never on `RUN_ALLOWLIST` (D-I-18);
`validateScreens` would refuse the whole screen otherwise.

**Two layouts, one screen** (D-I8-4). `damage` is the journal read as a
screen: the section map, a before/after table of the newest-arrived
revision, and the ring. `unit` is the piloting side: a compact section
list, the object's own gauges plus one mobility line, its components
grouped by install section, and its capabilities. Both layouts are
followed by the contacts table when its feed is live and by the declared
placeholders. `t` flips them through `localAction("toggle")` — which needs
its own arm, since the `refresh` switch falls through to `refreshWho()`
for anything it does not name.

**Three states before the panes.** A refusal the server sent instead of a
frame renders in the panel; `UnitStore.answered` is false until the server
has answered at all, and the screen says `Loading…` rather than asserting
the pilot has no unit before anything came back; then the no-subject line.
Even under that line the journal and before/after panes render whenever
the ring holds rows — the only actor who can produce an after-action
report is a staffer, who often holds no station, and each row names its
own object. A prefill (`d`, or a contacts row) puts the line in the shell's
composer under this screen, ready to send (16b cut B.1).

**Contract** (`$lib/stores/unit.svelte.ts`'s `parseUnitView` /
`journalRows`; a payload convention, not a proto type — I8 adds no wire):

```jsonc
// keyframe — `data: null` is the honest "you have no unit" frame
{ "object": { "id": "#7", "name": "…" },
  "schema": "owner:kind:id@version#<64 hex>",   // an identity, never a body
  "revision": 3, "state": "active", "record": null,
  "mobility": { "mode": "…", "switching": null, "modes": [ … ] },
  "sections":   [ { "id": "…", "condition": "…", "state": "ok",
                    "facets": [ { "id": "…", "cells": ["…"] } ], "slots": ["…"] } ],
  "cells":      [ { "addr": "facet/<s>/<f>/<cell>", "label": "…",
                    "current": 1, "min": 0, "max": 6, "state": "crit" } ],
  "components": [ { "id": "…", "definition": "…", "install": "slot/<s>/<slot>",
                    "condition": "…", "state": "ok", "asset": false } ],
  "capabilities": [ { "id": "…", "source": null } ],
  "counts": { … }, "truncated": false }

// delta — the projected journal one mutating command publishes
{ "object": { … }, "from_revision": 3, "to_revision": 4,
  "now": 1757000000, "cause": "…"|null,
  "entries": [ { "kind": "cell"|"section"|"component"|"asset"
                        |"capability_granted"|"capability_revoked", … } ] }
```

**Rule 1 — rulesets judge, clients render** (docs/design/10b §4). A `state`
word (`ok` / `warn` / `crit` / `down`) is the ONLY thing that picks a glyph
or a colour; a `condition` is the ruleset's own judgement and is printed
verbatim, never mapped. An unknown `state` falls back to neutral and an
unknown entry `kind` prints its kind and nothing else, so the server can
add either without a client release. `vitalsboard.test.ts` greps the store
and both components for tactical vocabulary alongside the I6 sheet words —
every label, id and word on this screen comes from the frame.

**Revision protocol.** A keyframe replaces the whole view and seeds the
watermark (an equal revision replaces too: some mutations publish no
delta). A SUBJECT change clears the journal ring — including losing one:
held → the no-subject frame clears it, while a frame arriving over nothing
held never does, because the fanned delta legitimately beats the resync's
keyframe. A delta ALWAYS rings its rows — newest DELTA first, entries in
the producer's own order within a delta with the `elided` marker last,
`JOURNAL_RING` 200 rows total, `JOURNAL_ROWS_PER_DELTA` 50 per delta (49
plus that client-internal row) — and then reports `foreign` (another
object, or any delta while the server has said there is no subject at all:
a keyframe request could only come back `null` again), `stale` (at or
behind the watermark) or `resync`. A `from_revision` of 0 is always a
`resync`: it is a subject's first apply after being recreated in place,
and its restarted watermark would otherwise read as stale. On `resync`,
`GameApp` arms ONE `setTimeout(UNIT_RESYNC_MS = 250)` behind the store's
boolean storm guard, so a damage burst of many deltas costs a single
`panel_subscribe`; ANY reply to a unit Req releases the guard before
classification, a refusal to a burst-driven request earns ONE bounded
retry at `4 × UNIT_RESYNC_MS`, a request that never went out (a closed
socket) releases the guard on the spot, and both the timer and the guard
are dropped on `#resetGameState`, `onFullRehydrate` and every hello (a
request armed on a previous socket never answers). The before/after table
takes the NEWEST-ARRIVED ring row's `(object, revision)`, never the
highest revision — a recreated subject restarts at 0, and the maximum
would report the dead one's damage forever.

**Contacts** ride `tactics.contacts` as an I9-convention list keyframe,
held in `GenericViewStore` (which keys on the feed name, not the `.view`
suffix) and rendered by an EMBEDDED `GenericView` — so the table costs no
bespoke code and claims no second screen digit. Its rows are Prefills
(`+contact <id>`), like every generic row.

**Panes with no producer yet** (grid, weapons, comms — and contacts until
its feed answers) render as a plain pane name over one muted line
declaring what is missing, the same bytes the TUI prints. They carry the
`placeholder` class and the narrow-width rule hides them BY CLASS wherever
they sit, so a phone drops the empty panes before anything real;
`unitPaneOrder` puts them last so the wide layout reads the real panes
first. That order is a pure function, which is how it is tested. The
coalescer, its bounded retry and both reset seams are driven end to end
through the `GameApp` harness (`game.test.ts` — see Tests above); only the
`.svelte` render assertions are still source probes, because there is no
component-render harness on this client (vitest runs
`environment: "node"`, no jsdom).

**The web can never name a subject.** `Query::PanelSubscribe` carries only
a feed, so the browser always gets the server's default subject (its own
object, or the unit it is stationed on) or a polite refusal, which renders
inside the panel. Targeting `#n` needs a new `Query` variant and a proto
regeneration — out of I8's no-wire-change boundary.

## The cockpit screen (16b cut Q.5, doc 16 §5)

Screen `0`, present while the hello's manifest carries `arena.cockpit` AND
this pilot holds a seat (16b cut B.3, 16c Q21: `GateInput.cockpitSeated`, fed
by `CockpitStore.seatedOnce`) — from a match's first seated frame to the
cleared one, kept across a full rehydrate, so a bystander has no tab at all —
and opened by itself on the FIRST seated frame of each match id (never again
for that id — a pilot who switches away is not dragged back by the next
pulse). When the seat goes, a pilot standing on the tab falls back to the
scene screen (`activeDef`); the TUI's tab keeps the same hours.

**Two feeds, by what changes.** `arena.cockpit` is a whole-state keyframe
pushed per arena pulse; `arena.map` is the whole arena's terrain, **pulled and
never pushed**, so it is not in the session's resume ring either. `CockpitStore`
owns the self-heal: a frame of match M with no held map of M asks for one, at
most once every `MAP_ASK_MS` (2 s), until it has it; while a frame is held, a
map of another match and the unseated answer are dropped rather than drawn
(`applyMap`, 16b cut B.1), and dropping one does not re-arm the latch. A map
that arrives before any frame is held, and judged by `holdsTheGround` when the
first frame names the match. The hello pass's own `arena.map` query counts as the first
ask (`noteMapAsked`) — it is armed before any frame has named a match, and the
server answers it with the ground of whichever match this pilot is seated in —
so a re-attaching pilot is not sent the whole arena's terrain twice on a socket
that has just reopened. `seated: false` — the last frame a pilot gets — clears
everything. `GameApp.#followTheCockpit` is the one place that acts on it, so a
frame arriving bare, inside a reply, or replayed out of the ring goes the same
way.

The once-per-match switch to screen 0 is the **only** server-driven screen
switch in this client, and it is taken on the frame that owes it, even
mid-sentence (16b cut B.1): the composer is the shell's (`Game.svelte`), so a
switch between screens that carry it never unmounts it, and the draft, the
sticky Say/Pose mode, the history buffer and the focus all survive.

**One composition, three renderers.** `$lib/cockpit.ts`'s `composeWindow(map,
frame, cols, rows)` is the rule the TUI's `render_cockpit_map_rows` and the
server's `view_window` also run, differing only in the size asked for: centred
on the pilot, clamped to the arena, the pilot's `@` down first and the contacts
in frame order after it, last write wins. The committed fixtures under
`src/lib/fixtures/cockpit/` are read by all three, and `cockpit.test.ts`'s
parity case pins this client's composition of `arena_map.json` +
`cockpit_frame.json` at exactly 78 by 20 against `view_grid.txt`, byte for byte
— the same file `mush-net-ssh/tests/tui_golden.rs` is held to. Three renderers
each pinned to a golden of its own would be regression; one file all three must
reproduce is parity. `web_view_model.txt` beside it is this client's own
addition: the whole of the screen's text for that frame (re-bless with
`COCKPIT_BLESS=1 npx vitest run src/lib/cockpit.test.ts`).

**The observer rule is in the type.** `CockpitFrame` keeps no contact `id`, no
`lock` digits, no `unit` object number and no notice `text`: a renderer cannot
leak a field its decoder never kept. The notices ring is composed from `kind` +
`designator` + `was` + `section` and the refusal's parts, because the arena
already sent each sentence to the scrollback as a `Sys` line and one sentence
has one owner. While seated the ring also takes the pilot's own `Sys` replies
(`CockpitStore.pushReply`, 16b cut B.1: on screen 0 it is the only reply
surface) — every one but the bridge's copy of a frame's own notice, which is
tagged `arena-notice` (`NOTICE_SENTENCE_TAG`) and composed from the frame
instead, so no notice is rung twice. The status block prints the degraded
`receiver` row and the tank's own `DESTROYED` / `IMMOBILE`. The screen looks
nothing up in another store — `game.test.ts` seeds a decoy name and object
number into who/presence/unit and asserts neither reaches the rendered text,
and a probe pins that `CockpitScreen.svelte` reads only `app.cockpit`.

**Nothing is sent by a key.** The seven reading verbs (`view`, `contacts`,
`status`, `weaponstatus`, `range`, `bearing`, `scan`) are on `RUN_ALLOWLIST`
and run; the five ordering verbs (`startup`, `heading`, `speed`, `lock`, `fire`)
are not on it and only ever prefill — including the contact table's click and
its `scan` button. A prefill goes through `focusComposer`, which puts the line
in the shell's composer and leaves the pilot on screen 0 (16b cut B.1, Q5), as
the TUI does; a prefill always drops the sticky mode back to a plain command
(`modeAfterPrefill`). The channel screen has its own box and no shell box, so
from there alone a prefill switches to the scene screen. Pinned by
`game.test.ts`'s "a prefill leaves the pilot on the cockpit".

## WsClient (`src/lib/ws.ts`)

```ts
const client = new WsClient({
  onStatus: (s) => { /* idle|connecting|connected|reconnecting|closed */ },
  onFullRehydrate: () => { /* resume gap too old: mark the seam, re-req panels */ },
  onSessionExpired: () => { /* server closed 4401: back to login */ },
  caps: () => ({ kind: "web", color: "true_color", width, height, utf8: true, structured: true }),
});
client.on("narrative", (env) => { /* env: Envelope narrowed to t:"narrative" */ });
client.connect(token, charKey);  // token from POST /api/auth/login; key from characters[]
client.sendLine("look");
client.req({ t: "who" });        // auto correlation id, match on t:"reply"
client.close();                  // clean close: no reconnect
```

Protocol behavior baked into the client:

- **Auth** — the token rides the `Sec-WebSocket-Protocol` subprotocol
  `rustmush.v1.<token>` (never the URL); the server echoes it back.
- **Keepalive** — answers every `ping` with `pong` (same nonce) automatically.
- **Resume** — tracks the last session-stamped `seq` (out-of-band frames are
  `seq: 0` and never resume points); on an unclean close it reconnects with
  exponential backoff + equal jitter (500 ms base, 30 s cap) and puts the
  resume point in the URL: `GET /ws?char=<key>&resume=<last_seq>`. Resume is
  connection-setup only — the server closes 1008 on an in-band `Resume`
  command, so the client never sends one.
- **Too-old gap** — the server answers `resumed { replayed: false }` and
  falls back to a fresh attach; `onFullRehydrate` fires (the app drops a
  "Reconnected (some history skipped)" line — panel state re-hydrates via
  the hello pass, which runs on every (re)connect). `replayed: true` means
  the gap replays in-order, silently.
- **Session expiry** — close code 4401 stops the reconnect loop and fires
  `onSessionExpired` (the app clears the stored token and shows login).
- **Clean close** — `close()` cancels pending reconnects; the instance is
  reusable via `connect` (which resets the resume point — new stream).

## Sheet + Rolls panels (M6b)

The Sheet tab renders **any** ruleset's character sheet purely from the
declarative `SheetSchema` the server sends — no field id, section name, or
splat is ever special-cased in the web client. Adding Mage/Werewolf (or a
future ruleset entirely) needs zero changes here; only `crates/ruleset-wod`
(and, for Vampire, the mushd wiring below) does.

**Contract** (`$lib/sheet.ts`'s `SheetPayload`, from
`crates/ruleset-wod/src/sheet_cmds.rs`'s `wod.sheet` query handler — the
generic `mush-ruleset::commands::register_sheet_commands` version emits the
identical shape under `<ruleset>.sheet`):

```jsonc
{
  "target": "#42",                 // ObjId::to_string() — "#N" form
  "schema": { "ruleset": "wod", "version": 2, "sections": [ … ] }, // SheetSchema::describe()
  "approved": { "state": "approved", "approved": true, "detail": { … } },
  "data": { "strength": 3, "disciplines": { "Potence": 2 }, … } // staff_only fields
                                                                  // already stripped for
                                                                  // non-staff viewers
}
```

The client asks for it with `Req { q: { t: "sheet" } }` (`ClientCommand`'s
`Query::Sheet` — deliberately target-less: it is always the viewer's own
sheet) and the reply/live-push both arrive as a `Panel` frame on whichever
feed ends in `.sheet` (`SessionStore.sheetFeed`, derived from the `hello`
panel manifest by suffix — never a hardcoded `"wod"`). A `.roll` panel feed
is pure push — one frame per `+roll`, to the roller only — and feeds the
Rolls tab's compact readout (dice with successes/1s highlighted, doubled-10
marker, Botch/Failure/N-successes label).

Two look-back corrections here. The suffix tie-break is **sorted-first**
(`SessionStore`), not first-in-manifest: the manifest's order is the
server's registration order, which the gateway bridge and this client
walked differently, so two clients on the same server could disagree about
which sheet is "the" sheet. The TUI already sorted. And `#routePanel`
renders **every** `.roll` feed rather than only `session.rollFeed` — a world
with two rulesets publishes two, and the other one's rolls were being
dropped silently. `RollPanelData.outcome` is nullable for the same reason:
`Owod20Outcome` needs a `difficulty`, which a Shadowrun pool or an
Initiative score has no notion of, so requiring it emptied the recent-rolls
table and the dice card on any other ruleset. A frame without it is kept and
rendered from its server-side `summary` — the one field every ruleset's roll
feed carries — and only the dice detail is gated.

**Rendering rules** (`$lib/sheet.ts`, exercised by `sheet.test.ts` against a
realistic Vampire section):
- `Dots` fields and small-span `Number` fields (`max - min <= 20` —
  Willpower, Rage/Gnosis, Generation, Humanity, Merit/Flaw points) render as
  filled/empty pip rows (`● ○`) plus the numeric rating.
- Wide-span `Number` fields (Blood Pool 0..50, Quintessence/Paradox 0..100,
  XP) render as a value + proportional bar instead of 50 dots.
- `Track` fields (Health) render as boxes with the classic marks:
  bashing `/`, lethal `X`, aggravated `*` (an unrecognized future mark
  glyphs to its own first letter rather than breaking); the deepest filled
  level's label + penalty is shown alongside.
- `Map` fields (Backgrounds, Disciplines/Spheres/Gifts, Merits/Flaws,
  Specialties) render each entry keyed by name, alphabetically, with the
  entry's own value rendered per its declared element type — so a Map<Dots>
  (Disciplines) and a Map<List<Text>> (Specialties) each look right without
  the renderer knowing what a Discipline or a specialty *is*.
- A `staff_only` field a non-staff viewer can't see is simply absent from
  `data` (the server strips it) and is never rendered — there is no
  client-side staff check.

**Type decision**: hand-written light interfaces in `$lib/sheet.ts` /
`$lib/roll.ts`, walked at runtime with defensive parsing (`parseSheetPayload`
etc., mirroring `mail.svelte.ts`'s `str`/`num` guards) — **not** ts-rs.
`SheetSchema`/`SheetDoc` (mush-ruleset) and `Owod20Outcome` (mush-dice) don't
carry `#[cfg_attr(feature = "ts", derive(TS))]`; adding it would be a
cross-crate touch outside this agent's web/-only scope, and would cut against
the framework's own design — `SheetSchema::describe()` *is* the wire
description precisely so the schema can be walked generically rather than
mirrored 1:1 into a second type system. If a future ruleset needs the sheet
schema's recursive `FieldType::{List,Map}` element types to type-check more
strictly, ts-rs is still an option then; today's hand-written types cover
every shape the base sheet + Vampire/Mage/Werewolf schemas actually produce
and degrade gracefully (never throw) on anything malformed.

**The bridge (was a "known gap", closed in interface cut I1)**: the web
bridge lives at `crates/mush-gateway/src/web_bridge.rs` (there is no
`crates/mushd/src/web_bridge.rs` — `mushd/src` holds only `inprocess.rs`,
`lib.rs`, `main.rs`, `repl.rs`). Its `Query::Sheet` arm routes to whichever
`.sheet`-suffixed query handler the ruleset registered, and falls back to a
polite `Sys` only when no such feed exists; `Query::PanelSubscribe` checks
manifest membership, remaps four named feeds (`comsys`, `who`,
`legal.actions`, `mail`) and forwards **every other feed under its own
name** with empty params (D-I-3). So a new module feed needs no bridge arm
at all — `llm.dashboard`, `wod.vitals`, `scenes`, `tactics.unit` and
`tactics.contacts` all ride that catch-all, and interface cut I8 depends on
it directly. (An earlier revision of this file described both halves as
broken and named a file that does not exist; the paragraph was stale.)

**Live since interface cut I1** (`docs/interface/02-implementation-plan.md`
§3.1): every command-side sheet write (`+harm`, `+heal`, splat pool spends,
XP approval, `@sheet/set`, chargen steps) now goes through
`SheetStore::put_in`, which republishes the subject's `<ruleset>.sheet`
keyframe **and** a `<ruleset>.vitals` frame (`$lib/proto`'s `VitalsFrame`:
the health track, the pools, the condition monitors, plus ruleset readouts)
to the subject alone. The client-side compensations from before — the
ruleset-tagged `Sys` refresh heuristic in `GameApp` and `SheetPanel`'s 12s
poll — were retired in cut I4 (the Refresh button and the sheet screen's `r`
remain as manual nudges), which also renders the vitals strip;
`SheetStore.applyKeyframe` accepts the pushed frame. Since interface cut I13
(§3.13) the republish is an incremental `Delta` — the whole staff-filtered
`data` with `revision`/`base_revision`, the large static `schema` omitted —
and `SheetStore.applyDelta` is a revision watermark: it full-replaces `data`
on a matching `base_revision`, ignores a stale frame, and on a gap asks
`GameApp` for a fresh keyframe (rule-3 self-heal, storm-guarded). A template
swap or a staff-visibility change forces a keyframe server-side, so the
client never applies a schema-mismatched delta.

**Overlay-only state republishes too.** An authoritative action can mutate
state that writes no sheet attribute (a typed Willpower spend, condition
damage); the server now republishes the subject's sheet on an action-state
commit, so that path is as live as an ordinary sheet write. The client's
interim compensation — a `refreshSheet()` fired from `#routeView` after the
viewer's own `resolution.outcome` receipt — is **retired**: the viewer's own
commit costs no extra round trip, and `game.test.ts` pins that no `sheet`
Req follows a receipt. The panel's Refresh and Try-again buttons, the sheet
screen's `r`, the hello-pass hydration and the rule-3 self-heal are the only
things that ask for a sheet.

## Worldgen draft panel — Build tab (M7)

The Build tab is the builder-facing preview for `@gen` (`crates/mush-worldgen`,
design doc `docs/design/03-llm-integration.md` §5): a room description the
LLM drafted from the builder's brief, shown side-by-side with the room's
current desc, pending `@gen/accept|retry|discard`. **Nothing the model
produces is ever written by the client** — every button sends the existing
`@gen/*` command as a plain Line (same pattern as the mail panel's
`@mail/quick`); only the server, via `@gen/accept`, ever commits a draft,
and only through the ordinary attribute-set path.

**Contract** (`$lib/worldgen.ts`'s `parseWorldgenDraft`, from
`crates/mush-worldgen/src/commands.rs`'s `emit_draft_panel`, payload shape
from `DraftPayload`/`StoredDraft` in `crates/mush-worldgen/src/data.rs`):

```jsonc
{
  "room": "#42",                 // ObjId::to_string()
  "room_name": "The Pawn Shop",
  "builder": "#7",
  "model": "claude-opus-5",
  "current": { "desc": "…the room's desc attribute right now…" },
  "proposed": {
    "desc": "…generated desc…",
    "short_desc": "a cramped pawn shop",
    "details": [ { "keyword": "counter", "desc": "…" } ],
    "exits": [ { "direction": "north", "name": "Out", "desc": "…", "transit_msg": "…" } ]
  }
}
```

`desc` is the model's only server-required field; everything else may be
empty or absent and is parsed defensively (mirrors the sheet/roll panels'
`str`/`obj` guards — never throws, degrades to empty on anything
malformed). `desc` is the only field the server sends a `current`
counterpart for, so that's the only side-by-side compare; `short_desc`,
`details`, and `exits` render proposed-only.

**Gating**: the Build tab is feature-gated on `SessionStore.worldgenAvailable`
(`hello`'s `"worldgen"` feature or `"worldgen.draft"` panel) — fixed ids,
unlike `sheetFeed`/`rollFeed`, since the worldgen module isn't swapped per
ruleset. The client has no way to know a viewer's `BUILDER_LLM` power, so
the tab shows for every viewer once worldgen is enabled server-side; a
non-builder's `@gen` is simply refused, and the refusal rides `Sys` into
scrollback like any other permission error — there is no separate
client-side builder check.

**No query, no delta — accept/discard clear optimistically**: unlike the
sheet panel, there is no worldgen query handler to re-fetch the current
draft on demand, and the server never pushes a "draft cleared" frame — a
`worldgen.draft` keyframe only ever arrives when a fresh draft is *ready*.
`@gen/accept` and `@gen/discard` both unconditionally clear the
server-side draft (`commands.rs`'s `gen_accept`/`gen_discard` both call
`clear_draft`), so `WorldgenStore.clear()` runs the instant the Line is
sent rather than waiting for confirmation. `@gen/retry` is different: the
old draft stays live server-side while the (off-loop, several-second) LLM
call runs, so the panel keeps showing it too, marked `pending`, until the
regenerated keyframe lands and clears the flag — matching what the server
actually does (`WorldgenStore.markPending()`/`applyKeyframe()`).

**Retry guidance**: the panel's Retry button opens a single-line optional
input inline (not a modal); its text is appended verbatim to
`@gen/retry <guidance>` (newlines collapsed, like the mail quick-send
fields) or omitted entirely when left blank.

## Panel hydration (GameApp)

On every `hello` — and on a full rehydrate after a resume gap — the app
issues the `Req`s `hydrationQueries` lists (`game.svelte.ts`, pinned by
`hydration.test.ts`), in this order: `who`, `panel_subscribe {feed:
"comsys"}` (the ack IS the `comsys.state` keyframe), `mail_list`, `sheet`,
`commands` (the command table, once), a `panel_subscribe` for every
`.vitals` feed and every `.view` feed, then — since interface cut I8, as
two discrete arms deliberately outside that suffix loop — the `.unit` feed
and `tactics.contacts`, and finally `scenes`. Each is gated on the hello's
features/panels (a screen hides when its module/panel is absent; `sheet` is
gated on `sheetFeed` — see the Sheet + Rolls section above). The suffix
loop's order is asserted byte-for-byte, which is why the I8 feeds are
separate arms rather than two more suffixes inside it. Replies route by
correlation id; reply errors arrive as `Sys` and land in scrollback (a
`sheet` reply's `Sys` fallback also flips `SheetStore.unavailable`, and a
`unit` reply's fills `UnitStore.unavailable` — both render in the panel
rather than cluttering scrollback).

Design notes / deliberate simplifications:

- **Scrollback virtualization is windowing**: only the last ~300 blocks
  render; "Show older" widens the window (ring caps at 5000 blocks).
- **Channel unread is client-side**: the server's `last_read` cursor
  advances on live delivery (there is no read-marking command), so the tab
  badge tracks the seq last seen while the tab was active, seeded from the
  keyframe cursor.
- **Reading mail is a Line command** (`@mail <n>` — reading mutates), so the
  body renders in scrollback classic-style and the panel re-requests its
  list; folder switches send `@mail/folder <name>` first because `<n>`
  ordinals resolve against the server-side current folder. The compose
  button is one-shot `@mail/quick`; the multi-line compose wizard remains a
  terminal flow.
- **Channel tab input** wraps lines in the member's alias verb
  (`pub <text>`), so `:`/`;` pose prefixes keep working.
- **/commands** are client-local: `/clear`, `/history`,
  `/theme [dark|light|system]`, `/help`. Everything else goes to the game
  verbatim; the palette lists the four beside the server's command table and
  keeps their slash. `/history` is the same verb the TUI and line mode have
  had since I12 — before the look-back the web could only reach it by
  scrolling to the top of the pane.
- **`/clear` draws a floor**, it does not reset the cursor. Clearing throws
  away every held narrative `seq`, so `oldestNarrativeSeq` is `null` again
  and the next page would be the server's NEWEST one — the lines just
  cleared, repainted, with nothing to dedup them against. `clear()` therefore
  sets `moreBacklog = false`, exactly as the TUI states in
  `docs/help/tui.md` "Scene history"; a fresh view (a re-attach, or the
  resume-gap rehydrate) lifts the floor through `beginSegment`.
