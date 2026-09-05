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
npm test           # vitest run (158 tests: ws, render, stores, auth, sheet, roll, worldgen, ...)
npm run test:watch # vitest watch mode
```

`web/dist` and `node_modules` are gitignored; `src/lib/proto/` is **committed**
(see below), so building the SPA never requires cargo.

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
│   ├── Game.svelte           # the shell (I4): tab strip of numbered screens, WHO rail toggle, legend chips, key help
│   ├── SceneScreen.svelte    # screen 1: room band + scrollback + vitals strip + composer + palette
│   ├── RoomBand.svelte       # RoomView + ambient: zone, weather, IC clock, scene pill, exits with nav keys, presence pills
│   ├── VitalsStrip.svelte    # every `.vitals` frame as gauges (dominant first)
│   ├── Composer.svelte       # the box: Say/Pose/Emit/OOC sticky mode, preview, history, completion, composing signal
│   ├── CommandPalette.svelte # commands.list + aliases + exits, filtered after the slash
│   ├── Scrollback.svelte     # windowed list, scroll-lock, "N new lines" pill
│   ├── RichLine.svelte       # span renderer (tag class, style, CommandLink)
│   ├── InputBar.svelte       # the channel tab's input: Enter submits, ↑/↓ history (200), masked mode
│   ├── ChannelTabs.svelte    # the channel list, IC and OOC apart: color chips + unread badges (I7)
│   ├── RollBuilder.svelte    # trait pickers from the schema's categories -> one `+roll` line (I6)
│   ├── VitalsBoard.svelte    # every `.vitals` frame as widgets + recent rolls (I6)
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
                              #   screens, commands, vitals, presence (I4)
```

## Screens and keys (interface cut I4)

The browser is a set of **numbered screens** (`$lib/stores/screens.svelte.ts`)
sharing names and digits with the SSH TUI: 1 scene, 2 sheet, 3 mail, 4 chan,
5 rolls, 7 actions, 8 build, 9 llm (6 is the TUI's intel screen). Each is
gated on the hello's manifest; a ruleset screen registers by a panel-feed
suffix (`registerFeedScreen`) and appears only when the server publishes
that feed. One binding table per screen — `local` / `prefill` / `run` — with
`run` held to a read-only verb list by `validateScreens` (D-I-18), so a bare
key never sends a mutating command; the one carve-out is walking an exit on
the scene screen (D-I-19).

The shell is **modal** (D-I-17): input mode is "the composer has focus".
While nothing has focus, `1`–`9` switch screens, `Enter` opens the composer
(a highlighted row prefills it), `"` `:` `;` `\` open it with the trigger
typed, `/` opens the palette, `?` the key help, the room's one-letter exits
walk, and a letter runs the screen's own binding (the chips on the legend
row — on a phone, where the on-screen keyboard makes focus the only mode,
the chips are the whole story). Inside the composer, `Alt+key` performs the
key's nav action (`Alt+1`, `Alt+n`, `Alt+?`) and `Ctrl/⌘+K` toggles the
palette; `Enter` sends and keeps focus, `Enter` on an empty box and `Esc`
blur it (`Esc` keeps the draft), `Shift+Enter` is a newline collapsed to a
space on send. The whole table is `$lib/keys.ts`'s `navIntent`, pinned by
vitest without a DOM.

The composer's Say / Pose / Emit / OOC control is client-side (D-I-8): an
explicit trigger or a channel alias verb always wins (`$lib/composer.ts`).
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

## Scene typography and comms (interface cuts I5, I7)

The scene screen groups consecutive speech and action by one actor under a
single name header (`groupBlocks` in `$lib/stores/scrollback.svelte.ts`: a
pure function over the visible window, five-minute window measured from the
previous line, broken by any other block, never for emits, spoofs or OOC),
sets prose in a system serif stack (`--font-prose`, class `ev-prose`) and
demotes OOC, channel, system and roll lines to small mono (`ev-demoted`).
The roller's own `.roll` frame lands inline as a dice card
(`RollCard.svelte`); everyone else reads the server's one-line `roll`
narration. On a phone the composer is a sticky bottom sheet.

Channels carry an `ic` flag (`@channel/set <c>=ic`, reported by
`comsys.state`): the channels screen lists IC and OOC channels apart
(`ChannelTabs.svelte`, `channelsWhere`), the channel view's head names the
half, and an IC channel's line on the scene screen reads as prose.

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
panel manifest by suffix — never a hardcoded `"wod"`). The `wod.roll` panel
feed (`SessionStore.rollFeed`, same suffix convention) is pure push — one
frame per `+roll`, to the roller only — and feeds the Rolls tab's compact
readout (dice with successes/1s highlighted, doubled-10 marker, Botch/
Failure/N-successes label).

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

**Known gap — not fixable from `web/`**: `crates/mushd/src/web_bridge.rs`'s
`Query::Sheet` arm still answers with a hardcoded "Character sheets are not
available yet." `Sys` fallback — a pre-M6 stub that was never updated to
route to the ruleset's registered `wod.sheet` query handler (the way
`Query::Who`/`MailList`/`ChannelBacklog` already do; `Query::PanelSubscribe`
has the same gap for any feed other than `"comsys"`/`"who"`/`"mail"`). Until
that's fixed, the Sheet tab (feature-gated on `hasPanel("*.sheet")`, so it
*does* show up — `ruleset-wod` registers the panel feed at boot) will always
land in the graceful "sheet query isn't wired up on this server yet." state
this client renders for exactly that fallback. No client change is needed
once the bridge is fixed — `refreshSheet()`/the panel's poll already speak
the real `Query::Sheet` contract described above.

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
`SheetStore.applyKeyframe` accepts the pushed frame. Frames are whole
keyframes; a true `Delta` remains a later refinement and
`SheetStore.applyDelta` is ready for it.

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
`hydration.test.ts`): `who`, `panel_subscribe {feed: "comsys"}` (the ack IS
the `comsys.state` keyframe), `mail_list`, `sheet`, `commands` (the
command table, once), a `panel_subscribe` for every `.vitals` feed and for
`scenes` — each gated on the hello's features/panels (a screen hides when
its module/panel is absent; `sheet` is gated on `sheetFeed` — see the Sheet
+ Rolls section above). Replies route by correlation id; reply errors
arrive as `Sys` and land in scrollback (a `sheet` reply's `Sys` fallback
also flips `SheetStore.unavailable` — see above).

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
- **/commands** are client-local: `/clear`, `/theme [dark|light|system]`,
  `/help`. Everything else goes to the game verbatim; the palette lists the
  three beside the server's command table and keeps their slash.
