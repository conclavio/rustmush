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
npm test           # vitest run (76 tests: ws, render, stores, auth, ...)
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
│   ├── RightPanel.svelte     # tabs: Mail (feature-gated) / Room / Sheet stub
│   ├── MailPanel.svelte      # folders + entries; open = `@mail <n>`
│   ├── ComposeModal.svelte   # one-shot `@mail/quick`
│   └── RoomPanel.svelte      # latest RoomView; click-to-run exits
└── lib/
    ├── proto/                # GENERATED wire types (cargo gen-ts) — committed
    ├── ws.ts                 # WsClient: connect/send/dispatch/resume/keepalive/backoff
    ├── game.svelte.ts        # GameApp: WsClient → stores wiring, Req routing, /commands
    ├── render.ts             # pure span/line renderers (mirrors mush-ansi formatting)
    ├── auth.ts               # /api/auth/* wrappers + friendly error mapping
    ├── theme.ts, storage.ts, history.ts
    └── stores/               # rune stores: scrollback, channels, who, mail, session
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

## Panel hydration (GameApp)

On every `hello` the app issues `Req`s: `who`, `panel_subscribe {feed:
"comsys"}` (the ack IS the `comsys.state` keyframe) and `mail_list` — the
latter two gated on the hello's features/panels (the Mail tab hides when the
mail module is absent). Replies route by correlation id; reply errors arrive
as `Sys` and land in scrollback.

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
