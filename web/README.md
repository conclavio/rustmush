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
npm test           # vitest run (139 tests: ws, render, stores, auth, sheet, roll, ...)
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
│   ├── Game.svelte           # grid: WHO rail | tabs+scrollback+input | right panel
│   ├── Scrollback.svelte     # windowed list, scroll-lock, "N new lines" pill
│   ├── RichLine.svelte       # span renderer (tag class, style, CommandLink)
│   ├── InputBar.svelte       # Enter submits, ↑/↓ history (200), masked mode
│   ├── ChannelTabs.svelte    # tab bar: color chips + unread badges
│   ├── ChannelView.svelte    # one channel's backlog/live view + alias input
│   ├── WhoRail.svelte        # who table, idle ticking, refresh
│   ├── RightPanel.svelte     # tabs: Mail / Room / Sheet / Rolls (each feature-gated)
│   ├── MailPanel.svelte      # folders + entries; open = `@mail <n>`
│   ├── ComposeModal.svelte   # one-shot `@mail/quick`
│   ├── RoomPanel.svelte      # latest RoomView; click-to-run exits
│   ├── SheetPanel.svelte     # schema-driven character sheet (M6b — see below)
│   └── RollsPanel.svelte     # wod.roll readout: dice, successes/1s, botch (M6b)
└── lib/
    ├── proto/                # GENERATED wire types (cargo gen-ts) — committed
    ├── ws.ts                 # WsClient: connect/send/dispatch/resume/keepalive/backoff
    ├── game.svelte.ts        # GameApp: WsClient → stores wiring, Req routing, /commands
    ├── render.ts             # pure span/line renderers (mirrors mush-ansi formatting)
    ├── sheet.ts               # sheet schema/doc types + rendering helpers (hand-written; see below)
    ├── roll.ts                # Owod20Outcome types + roll-readout formatting (hand-written)
    ├── auth.ts               # /api/auth/* wrappers + friendly error mapping
    ├── theme.ts, storage.ts, history.ts
    └── stores/               # rune stores: scrollback, channels, who, mail, session, sheet, rolls
```

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

**Live-enough, not truly live**: the server has no push-on-mutation delta
for sheet changes yet (`+harm`/`+xp` approval/splat pool spends never emit a
`Panel` frame — only the initial `wod.sheet` query does, always as a
keyframe). `GameApp` compensates two ways, neither perfect: (1) any `Sys`
line tagged with the loaded sheet's own ruleset id (`"wod"`) debounce-
refreshes it — covers `+harm`/`+heal`, private rolls, splat pool commands,
XP approval, but **misses** a public (non-`/priv`) `+roll/wp`, whose
Willpower-spend narrates to the room untagged; (2) `SheetPanel` polls every
12s while the tab is mounted (and only then — the interval is cleared on
unmount). Plus a manual "↻" refresh button. The real fix is a `wod.sheet`
`Delta` frame pushed alongside `+harm`/`+xp`/splat mutations, symmetric with
how `wod.roll` already pushes on every `+roll` — out of scope here (crosses
into `crates/ruleset-wod`), but `SheetStore.applyDelta` is already written
and tested for the day it lands.

## Panel hydration (GameApp)

On every `hello` the app issues `Req`s: `who`, `panel_subscribe {feed:
"comsys"}` (the ack IS the `comsys.state` keyframe), `mail_list`, and `sheet`
— the latter three gated on the hello's features/panels (a tab hides when its
module/panel is absent; `sheet` is gated on `sheetFeed` — see the Sheet +
Rolls section above). Replies route by correlation id; reply errors arrive
as `Sys` and land in scrollback (a `sheet` reply's `Sys` fallback also flips
`SheetStore.unavailable` — see above).

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
  `/help`. Everything else goes to the game verbatim.
