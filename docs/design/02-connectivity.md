# rustmush — Connectivity Design (SSH server, Web client, Session layer)

Area owner: connectivity. Everything between a player's keyboard and the game core: SSH ingress, WebSocket ingress, session abstraction, output rendering, auth, and abuse protection.

---

## 1. Summary of recommendations

| Concern | Recommendation |
|---|---|
| SSH server crate | **russh 0.63.x** (Eugeny/russh fork — actively maintained, v0.63.1 released Aug 2026 with security fixes) |
| HTTP/WS server | **axum 0.8** (current stable) + built-in `ws` extractor (tokio-tungstenite) |
| Line handling on SSH | **Server-side line editor** (own module, not rustyline) — SSH puts the client tty in raw mode, so we own echo/editing |
| Output model | Semantic `OutputEvent` enum in a shared `proto` crate; **two renderers**: ANSI (SSH) and serde_json (web — the event *is* the wire format) |
| Web auth | Account login → **opaque random server-side session token** (no JWT), token presented in WS handshake |
| Frontend | **Svelte 5 (runes) + Vite + TypeScript**, pure SPA (no SvelteKit), scrollback rendered as HTML spans (no xterm.js), embedded via **rust-embed** |
| Rate limiting | **tower_governor 0.8** on HTTP; custom per-IP tracker for SSH auth failures |
| Password hashing | **argon2** (RustCrypto `argon2` crate, Argon2id) |
| TLS | Reverse proxy (Caddy) recommended for prod; native rustls via `axum-server` supported; `rcgen` self-signed for dev |

---

## 2. Workspace layout (connectivity's slice)

```
rustmush/
├── Cargo.toml                  # [workspace]
├── crates/
│   ├── proto/                  # rustmush-proto: OutputEvent, ClientCommand, RichText, ClientCaps.
│   │                           #   Depends only on serde. Optionally derives TS types via ts-rs.
│   ├── session/                # rustmush-session: SessionManager, ConnectionHandle, attach/detach,
│   │                           #   reconnect buffers, idle tracking. Depends on proto + tokio.
│   ├── net-ssh/                # rustmush-net-ssh: russh server, LineEditor, ANSI renderer consumer.
│   ├── net-web/                # rustmush-net-web: axum app, WS handler, token store, static assets.
│   ├── ansi/                   # rustmush-ansi: OutputEvent -> ANSI bytes, color downgrade, themes.
│   ├── core-api/               # (owned by core agent, consumed here): CoreHandle trait — the ONLY
│   │                           #   thing net-* crates know about the game.
│   └── ...                     # core, modules, llm — other agents' areas
├── bin/rustmushd/              # the single server binary; wires everything together
└── web/                        # Svelte 5 + Vite SPA (npm workspace, built to web/dist,
                                #   embedded by net-web via rust-embed)
```

Dependency direction is strict: `net-ssh`/`net-web` → `session` → `proto`; nothing in the game core imports a net crate. The core talks to connectivity exclusively through `SessionManager` (outbound) and an mpsc of `InboundMsg` (inbound).

Key dependencies (versions verified Aug 2026 where noted):

```toml
tokio = { version = "1", features = ["full"] }
russh = "0.63"            # verified: v0.63.1, Aug 2026, active
axum = { version = "0.8", features = ["ws"] }   # verified: 0.8.x current stable
tower-http = { version = "0.6", features = ["fs", "trace", "compression-gzip"] }
tower_governor = "0.8"    # verified: 0.8.0, GCRA rate limiting for tower/axum
rust-embed = "8"          # verified: maintained
argon2 = "0.5"
rand = "0.9"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
dashmap = "6"
rcgen = "0.13"            # dev self-signed certs
axum-server = { version = "0.7", features = ["tls-rustls"] }  # optional native TLS
ts-rs = "10"              # optional: generate .ts types from proto (build-time)
```

---

## 3. The shared protocol crate (`proto`)

This is the contract between the core, both renderers, and the web client. The core agent defines the *semantics*; this crate defines the *shape*. Everything is `Serialize + Deserialize` and internally tagged so the JSON is self-describing.

### 3.1 RichText — the one text representation

```rust
/// Styled text used inside every narrative event. Never raw ANSI, never HTML.
#[derive(Clone, Serialize, Deserialize)]
pub struct RichText(pub Vec<Span>);

#[derive(Clone, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "Style::is_plain")]
    pub style: Style,
    /// Semantic tag, e.g. "exit", "player", "channel", "damage", "npc".
    /// ANSI renderer maps tag -> theme color; web maps tag -> CSS class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<SmolStr>,
    /// Interactive affordance for web (click-to-run); SSH ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<CommandLink>,   // e.g. { cmd: "look east" }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Style {
    pub fg: Option<Color>, pub bg: Option<Color>,
    pub bold: bool, pub italic: bool, pub underline: bool,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum Color { Ansi(u8), Idx256(u8), Rgb(u8, u8, u8) }
```

Semantic tags are the important part: builders and the LLM pipeline emit `tag: "exit"`, not "cyan". Explicit `Style` exists so classic `%c`-style manual color (if the core offers it) still round-trips. The ANSI renderer resolves `tag → Style` through a server theme; the web client resolves `tag → CSS class` and gets theming for free.

### 3.2 OutputEvent — server → client

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum OutputEvent {
    // -- narrative stream (renders into scrollback on both clients) --
    Room(RoomView),                       // full room: look / movement
    Narrative { kind: NarrativeKind, actor: Option<ObjRef>, text: RichText },
    Sys(RichText),                        // server/system messages, errors
    // -- panel-oriented (web panels; SSH renders a text fallback) --
    Channel { channel: SmolStr, actor: ObjRef, kind: NarrativeKind, text: RichText },
    MailNotice { from: ObjRef, subject: String, id: MailId, unread: u32 },
    MailList(Vec<MailSummary>), MailBody(MailMessage),
    Who(Vec<WhoEntry>),
    Sheet(SheetUpdate),                   // opaque module payload, see §3.4
    Map(MapUpdate),                       // opaque module payload (window cells later)
    Weather { zone: SmolStr, text: RichText, icon: SmolStr },
    // -- session control --
    Prompt(PromptSpec),                   // input mode changes: normal / masked / choice
    SessionInfo { char: ObjRef, account: SmolStr, connections: u8 },
    Disconnect { reason: RichText },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum NarrativeKind { Say, Pose, SemiPose, Emit, Page, Ooc, Spoof }
```

`Room`, `Narrative`, `Sys` are the classic scrollback. Panel events are *additive*: the core emits `Channel` once; the web client routes it to a channel tab, while the ANSI renderer prints the familiar `<Public> Wizard says, "..."` line. One event, two renders — no dual code paths in the core.

### 3.3 Client → server

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum ClientCommand {
    /// A line as typed. THE primary input for both transports.
    /// SSH lines arrive here; web input box submits here too.
    Line(String),
    /// Small RPC surface for panels that need pull, with correlation id.
    Req { id: u64, q: Query },       // Query::MailList, Query::Who, Query::Sheet,
                                     // Query::MapViewport{..}, Query::ChannelBacklog{..}
    Resume { last_seq: u64 },        // web reconnect (see §6.4)
    Pong { nonce: u64 },
}
```

Opinion: keep the structured surface *tiny*. A MUSH is command-driven; the web UI's buttons mostly just submit `Line("@mail/read 3")` under the hood, which keeps web and SSH behavior identical and testable through one parser. `Req` exists only for panel hydration (populating the mail list on login without spamming scrollback).

### 3.4 Module extensibility (WoD now, TacSim later)

`SheetUpdate` and `MapUpdate` carry a module discriminator and an opaque JSON payload:

```rust
pub struct SheetUpdate { pub module: SmolStr /* "wod" | "tactics" */, pub data: serde_json::Value }
```

The connectivity layer never interprets these; the web client lazy-loads a panel component per module id, and the ANSI renderer calls back into the module's registered `fn render_text(&Value, &ClientCaps) -> RichText`. This is the module boundary for connectivity: **modules ship (a) a text renderer registered with `ansi`, and (b) a Svelte panel component keyed by module id.** Tactical windows in TacSim become `MapUpdate { module: "tactics", data: {cells: [...]} }` with a canvas-based panel — no protocol change (square cells, doc 14 D-14-4).

---

## 4. Session layer (`session`)

### 4.1 Concepts

- **Connection**: one physical transport (an SSH channel or a WebSocket). Owns caps, IP, seq counter.
- **Session**: one authenticated attachment of an *account* to a *character*. A session has **1..N connections** (classic multi-connect: all connections receive all output; input accepted from any).
- **Account → Characters**: accounts (login identity, password/ssh keys, email) own multiple characters. Auth yields an account; character selection yields a session.

### 4.2 Types

```rust
pub struct ConnId(pub u64);           pub struct SessionId(pub u64);

#[derive(Clone)]
pub struct ClientCaps {
    pub kind: ClientKind,             // Ssh | Web
    pub color: ColorDepth,            // None | Ansi16 | Ansi256 | TrueColor
    pub width: u16, pub height: u16,
    pub utf8: bool,
    pub structured: bool,             // web: gets JSON events; ssh: false
}

/// Held by the session layer; the transport task holds the rx side.
pub struct ConnectionHandle {
    pub id: ConnId,
    pub addr: IpAddr,
    pub caps: ArcSwap<ClientCaps>,               // resize updates in place
    pub tx: mpsc::Sender<Arc<OutputEvent>>,      // bounded, e.g. 256 — see §8.4
    pub connected_at: Instant,
    pub last_input: AtomicCell<Instant>,
}

/// What the game core receives.
pub enum InboundMsg {
    Command { session: SessionId, conn: ConnId, cmd: ClientCommand },
    Attached { session: SessionId, conn: ConnId },      // core emits "has reconnected"
    Detached { session: SessionId, conn: ConnId, reason: DetachReason },
    SessionClosed { session: SessionId },               // last connection gone + grace expired
}

pub struct SessionManager { /* DashMap<SessionId, SessionEntry>, DashMap<ConnId, ...> */ }

impl SessionManager {
    /// Transport calls after auth + char select.
    pub fn attach(&self, conn: ConnectionHandle, char_id: CharId, account: AccountId)
        -> (SessionId, AttachOutcome);   // AttachOutcome::New | Rejoined { others: u8 }
    pub fn detach(&self, conn: ConnId, reason: DetachReason);
    /// Core-facing output API. Target lets the core address a character without
    /// knowing about connections.
    pub fn send(&self, target: Target, ev: OutputEvent);
    pub fn broadcast_where(&self, pred: impl Fn(&SessionMeta) -> bool, ev: OutputEvent);
    pub fn who(&self) -> Vec<SessionMeta>;               // feeds the WHO command
    pub fn set_idle_policy(&self, p: IdlePolicy);
}
pub enum Target { Session(SessionId), Char(CharId), Conn(ConnId) }
```

`send` wraps the event in `Arc` once and clones the Arc per connection — a room-wide emit to 200 players is one allocation.

### 4.3 Reconnect, linkdead, idle

- **SSH**: no transport resume. New connection + auth + same character ⇒ `attach` returns `Rejoined`; core announces "CONNECTED: X has reconnected." When the last SSH connection drops unexpectedly, the session enters **linkdead** for a grace period (default 300 s, config) before `SessionClosed` — the character stays in the room flagged linkdead, exactly the classic behavior.
- **Web**: real resume. Each session keeps a **ring buffer of the last 512 `(seq, Arc<OutputEvent>)`**. A dropped WS reconnects with its session token and sends `Resume { last_seq }`; the server replays the gap or, if the gap exceeds the buffer, sends a fresh `Room` + panel snapshots. Seq numbers are per-session, stamped by the session layer in the envelope (web only).
- **Idle**: `last_input` feeds WHO idle times. Transport keepalives (§5.6, §6.4) detect dead links; game-level idle never disconnects by default (MU* culture: idlers are furniture), but `IdlePolicy` supports optional idle-to-linkdead.

---

## 5. SSH server (`net-ssh`)

### 5.1 Crate choice

**russh 0.63.x.** Verified actively maintained (v0.63.1 Aug 23 2026, including security fixes; weekly release cadence through summer 2026), pure Rust, tokio-native, and it's the only serious option for *server-side* SSH in Rust:

- Rejected **thrussh**: the original, effectively unmaintained; russh is its maintained fork.
- Rejected **ssh2 / libssh2 bindings**: blocking, C dependency, client-oriented — no good server story.
- Rejected **wrapping OpenSSH sshd** (ForceCommand into our binary): kills the single-binary story on Windows, complicates auth integration and per-connection state.

Pin the minor version and isolate all russh types inside `net-ssh` (russh is pre-1.0 with real API churn; nothing outside this crate may name a russh type).

### 5.2 Server skeleton

```rust
struct SshIngress { sessions: Arc<SessionManager>, auth: Arc<AuthService>, guard: Arc<IpGuard> }

impl russh::server::Server for SshIngress {
    type Handler = SshConnHandler;
    fn new_client(&mut self, peer: Option<SocketAddr>) -> SshConnHandler { /* per-conn state */ }
}

#[async_trait]
impl russh::server::Handler for SshConnHandler {
    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error>;        // §5.3
    async fn auth_password(&mut self, user: &str, pw: &str) -> Result<Auth, _>;
    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, _>;
    async fn channel_open_session(&mut self, ch: Channel<Msg>, s: &mut Session) -> _;
    async fn pty_request(&mut self, ch: ChannelId, term: &str, cols: u32, rows: u32, ...) -> _;
    async fn env_request(&mut self, ...) -> _;          // capture COLORTERM, LANG
    async fn window_change_request(&mut self, ch: ChannelId, cols: u32, rows: u32, ...) -> _;
    async fn data(&mut self, ch: ChannelId, data: &[u8], s: &mut Session) -> _;    // -> LineEditor
    async fn shell_request(&mut self, ...) -> _;        // start the login flow
}
```

Host key: **ed25519**, generated on first boot into the data dir (`russh::keys`), never regenerated (key rotation = user-visible fingerprint change; document it). Windows and Linux paths via `directories` crate.

### 5.3 Auth model — how players get in

Route on the **SSH username**, supporting both modern and classic flows:

| `ssh <user>@mush` | Behavior |
|---|---|
| `guest` | `auth_none` accepted → attached to a pooled guest character (Guest1..N), no persistence |
| `new` | `auth_none` accepted → interactive account-creation wizard: account name → email (optional) → password (masked, entered twice, argon2id-hashed) → offer to paste an SSH public key → first character creation |
| `play` (generic) | `auth_none` accepted → **classic welcome screen**: ASCII banner + `connect <account> <password>`, `create ...`, `WHO`, `quit` — password entry is masked by the line editor. This is the nostalgic front door. |
| `<account>` | `auth_publickey` checked against keys the account registered (in-game `@sshkey/add <pubkey>`); else `auth_password` (argon2id verify). Success → **character-select screen** (or straight into the character if the account has exactly one and autoconnect is set). |

Notes:
- Publickey is the promoted path for regulars: `ssh nick@mush` with an agent = zero-friction login, immune to brute force.
- `auth_none` principals (`guest`/`new`/`play`) are throttled hard (§8.2) and get a restricted pre-auth command set.
- Failed password auth: constant-time verify (argon2 gives this), uniform error, counted by `IpGuard`.
- Account ↔ character mapping lives in the core's DB; `AuthService` is a thin trait here so `net-ssh` doesn't own storage:

```rust
#[async_trait]
pub trait AuthService: Send + Sync {
    async fn verify_password(&self, account: &str, pw: &str) -> AuthResult;
    async fn verify_pubkey(&self, account: &str, key: &PublicKey) -> AuthResult;
    async fn create_account(&self, req: NewAccount) -> Result<AccountId, CreateErr>;
    async fn characters(&self, acct: AccountId) -> Vec<CharSummary>;
}
```

### 5.4 Line mode: the server-side line editor

Critical design fact: when an SSH client allocates a PTY, the *client's* terminal goes raw and **all line discipline is the server's job**. There is no real pty behind us, so we implement it. Rejected pulling `rustyline`/`reedline` (both assume a local terminal fd); instead a small, transport-agnostic, fuzz-friendly module:

```rust
pub struct LineEditor {
    buf: Vec<char>, cursor: usize,
    history: VecDeque<String>,          // per-connection, cap 200
    mode: EchoMode,                     // Normal | Masked | NoEcho
    utf8: IncrementalUtf8,              // handles split multibyte sequences
    esc: EscParser,                     // CSI sequences for arrows/home/end/delete
}
pub enum EditorAction { Submit(String), None, Bell }

impl LineEditor {
    /// Feed raw channel bytes; returns completed lines and appends echo/redraw
    /// bytes for the client into `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> Vec<EditorAction>;
    /// Redraw prompt+buffer after async output interrupted the line (§5.5).
    pub fn redraw(&self, out: &mut Vec<u8>);
    pub fn set_mode(&mut self, m: EchoMode);   // Masked prints '*'; used for passwords
}
```

Supported editing: Backspace/DEL, ←/→, Home/End (`^A`/`^E`), `^U` kill-line, `^W` kill-word, ↑/↓ history, `^C` clears the line, `^D` on empty line = QUIT confirmation. CRLF and bare CR both terminate lines. Max line length 8 KiB (drop + bell beyond). Tab is reserved (future command completion). No multi-line editing — this is a MUSH, not a shell.

### 5.5 Async output vs. an in-progress line

The classic problem: someone `say`s while you're mid-typing. The SSH writer task serializes everything through one path:

1. If the editor buffer is non-empty: emit `\r\x1b[K` (CR + erase line), write the rendered event, then `LineEditor::redraw` re-prints the prompt and partial input.
2. Prompt is minimal by default (`> `), configurable per character; modules may set a rich prompt (e.g., WoD pools or BT heat later) via `Prompt(PromptSpec)`.

### 5.6 Terminal capabilities, resize, keepalive

- **Color detection**: `pty_request` gives `TERM`; `env_request` may give `COLORTERM`. Heuristic: `COLORTERM=truecolor|24bit` → TrueColor; `TERM` contains `256color` → Ansi256; `TERM=dumb` → None; else Ansi16. Player override: `@set me=COLOR:256` style preference wins over detection; `NOCOLOR` flag honored.
- **UTF-8**: assume yes (2026); `LANG`/`LC_ALL` sniffing only to *disable* (fall back to ASCII art borders).
- **Graphics** (doc 21 §4; as built by doc 21 cut G.1a, 2026-10-22): entering `@tui` sends a one-pixel kitty graphics query under image id 31 and then `CSI c`. A kitty `OK` arriving before the DA1 reply means the kitty graphics protocol; the DA1 reply first, an error reply, or 500 ms of silence means not. The DA1 reply's attributes are recorded whichever answer came first, and attribute `4` means sixel. The replies come back through the TUI's key parser as marker keys and never reach the input line, and every other APC is swallowed. The result (`mush_pixel::caps::Graphics`) chooses the encoder per connection (D-21-5): kitty, then sixel, then none. Since doc 21 cut G.2 the probe also sends `CSI 16 t` between the two, and a `CSI 6 ; h ; w t` reply is the cell size a sixel picture is drawn in (10 by 20 when a terminal does not answer, which is Windows Terminal's virtual cell); `@pixel/encoder [auto|kitty|sixel|off]`, read by the front end and never the world, forces or refuses an encoder for the connection. Not persisted; line mode never probes.
- **Resize**: `window_change_request` updates `ClientCaps` via `ArcSwap`; renderer uses width for wrapping/`WHO` columns.
- **Keepalive**: russh server config `keepalive_interval = 30s`, `keepalive_max = 3` (server-initiated `keepalive@openssh.com`), plus `inactivity_timeout` off (idlers welcome). Detect half-open links → detach → linkdead grace.
- **No PTY requested**: reject exec requests except the M6 authoring allow-list,
  exactly `mind push`, `mind pull`, or `mind check`. These require a successfully
  verified public-key authentication and an explicitly selected character owned
  by that account. One bounded typed JSON request arrives on stdin followed by
  EOF; structured results and an exit status use stdout. The matching authoring
  service enforces byte limits, operation allow-lists, current ownership, stale
  edit checks and cancellation. Password sessions remain interactive only. No
  shell commands, arbitrary paths, forwarding or general subsystem are enabled.
  See [the M6 plan](17d-unit-minds-through-m6-plan.md#13-command-and-transport-boundaries)
  and `help mindcraft`.

### 5.7 ANSI renderer (`ansi` crate)

```rust
pub struct AnsiTheme { /* SemanticTag -> Style, per-server config + per-player overrides */ }
pub fn render(ev: &OutputEvent, caps: &ClientCaps, theme: &AnsiTheme, w: &mut Vec<u8>);
```

- Color downgrade chain: `Rgb → nearest 256 → nearest 16 → plain`, computed once via lookup (OKLab nearest-neighbor table baked in).
- Word-wrap at `caps.width` (respecting ANSI zero-width), always reset (`\x1b[0m`) at line end; strip all ANSI when `ColorDepth::None`.
- Panel-ish events get text renderings: `Channel` → `<Public> Nick says, "hi"`, `MailNotice` → `MAIL: New message from Wizard (subject)`, `Who` → the traditional column table sized to terminal width, `Sheet`/`Map` → delegated to the owning module's registered text renderer.
- Write no raw `\n` — always `\r\n` on the wire (raw mode).

---

## 6. Web server (`net-web`)

### 6.1 Stack

**axum 0.8** (verified current), WS via the built-in `ws` feature. Routes:

```
GET  /                     -> SPA index.html (rust-embed)
GET  /assets/*             -> hashed static assets (rust-embed, immutable cache headers)
POST /api/auth/login       -> { account, password } -> { token, account, characters[] }
POST /api/auth/register    -> account creation (same AuthService as SSH)
POST /api/auth/logout
GET  /ws?token=...&char=ID -> WebSocket upgrade -> the session
GET  /healthz
```

Asset serving: `rust-embed` (verified maintained). The frontend build is an explicit
release prerequisite (`cd web && npm ci && npm run build`); `build.rs` rejects a
release build when `web/dist/index.html` is absent, but never invokes npm itself.
Debug builds read `web/dist` from disk (rust-embed's default), so a fresh Rust-only
checkout is valid and serves a diagnostic 404 until the frontend is built. During
frontend development, `vite dev` proxies `/api` and `/ws` to the Rust server —
standard, zero custom tooling.

### 6.2 Web auth: opaque tokens, not JWT

`POST /api/auth/login` verifies argon2id via the same `AuthService`, then mints a **256-bit random token** stored server-side (`DashMap<TokenHash, TokenEntry { account, expires, created_ip }>`; store the SHA-256 of the token, not the token). TTL 7 days sliding. Sent to the client, kept in `localStorage`, presented on WS connect.

Rejected JWT/Paseto: we are a single, stateful, long-lived server — stateless tokens buy nothing and cost revocation (bans must kick web sessions instantly). Rejected cookies as the primary mechanism: the WS handshake token-in-query (or `Sec-WebSocket-Protocol` subfield) is simpler cross-origin and avoids CSRF entirely; we still set `SameSite=Strict` cookie as a secondary convenience for the SPA reload path. Tokens in query strings are acceptable here because the token is single-purpose and short-listed for log scrubbing (never log query params on `/ws`).

### 6.3 WS wire protocol

Server → client envelope (JSON text frames):

```json
{ "seq": 4212, "ts": 1756300000123, "t": "narrative",
  "d": { "kind": "say", "actor": {"id":"#123","name":"Wizard"},
         "text": [ {"text":"Wizard says, \"", "tag":"say"},
                   {"text":"hello", "tag":"say"}, {"text":"\"", "tag":"say"} ] } }
```

`t`/`d` come straight from `serde_json::to_string(&OutputEvent)` — **the proto enum is the protocol**; the envelope adds only `seq`/`ts`. Client → server is `ClientCommand` serialized the same way. First frames after upgrade:

```
S->C  { "t":"hello", "d":{ "proto":1, "server":"rustmush 0.1", "features":["mail","channels","wod"] } }
C->S  { "t":"resume", "d":{ "last_seq": 4180 } }        // or omitted for fresh session
S->C  ...replayed events... then live stream
```

Versioning: `proto` integer bumped only on breaking change; additive fields/events are always allowed (clients ignore unknown `t`). `ts-rs` generates TypeScript types from the proto crate at build time so the frontend never hand-mirrors the enum (drop it if it fights us — the surface is small).

### 6.4 WS keepalive & reconnect

App-level ping every 25 s (`{"t":"ping","d":{nonce}}` / `Pong`) in addition to protocol pings (some proxies eat WS pings). Client reconnects with exponential backoff + jitter, then `Resume` (§4.3). Missed 2 pongs → server detaches the conn (session goes linkdead-or-other-conns like SSH).

### 6.5 TLS

- **Recommended prod**: reverse proxy (Caddy for automatic Let's Encrypt; nginx fine) → `127.0.0.1` plaintext; honor `X-Forwarded-For` **only** when `trusted_proxies` is configured (matters for rate limiting and bans).
- **Native**: `axum-server` with rustls, cert/key paths in config, for people who won't run a proxy.
- **Dev**: `rcgen` self-signed on first boot, or plain HTTP on localhost.
- SSH needs no TLS. Do not put SSH behind the proxy; expose port 2222 (or 22) directly.

---

## 7. Web frontend (`web/`)

### 7.1 Stack: Svelte 5 + Vite + TS (pure SPA)

Verified current: Svelte 5.55.x with runes is stable and mainstream. Rationale: signal-based fine-grained reactivity fits a "many small panels updated by an event firehose" app perfectly; tiny runtime; single-file components keep panel code approachable for future contributors. **No SvelteKit** — we have no SSR, no routing to speak of, and the server is Rust; plain `vite` + `svelte` plugin. Rejected React (heavier, noisier for this shape) and SolidJS (excellent reactivity but smaller ecosystem; Svelte 5 closed the reactivity gap).

**No xterm.js.** The scrollback is *not* a terminal — we receive semantic spans, so we render real DOM: `tag → CSS class`, `link → clickable span` that submits its command. Free wins: text selection, find-in-page, links, font scaling, accessibility, themes. Rejected terminal emulation as a category — it would throw away the structured protocol.

### 7.2 Architecture

- One `WsClient` module: connect/auth/resume/backoff, dispatches envelopes to a per-event-type registry.
- Rune stores per domain: `scrollback` (capped ring, 5k entries, virtualized list), `channels` (Map<channel, messages> + unread counts), `mail`, `who`, `sheet`, `map`, `session`.
- Command input: history (↑/↓), local echo of the typed command into scrollback (styled `tag:"input-echo"`), `/`-prefixed client-local commands (`/clear`, `/font`), everything else → `Line`.
- Module panels: `Sheet`/`Map` payloads route by `module` id to dynamically imported components (`wod/SheetPanel.svelte`; later a `tactics` window panel on canvas — the `atlas.map` panel of doc 14 §6).

### 7.3 Layout

```
┌────────┬──────────────────────────────┬────────────┐
│ WHO    │  [Public] [Guests] [+]  tabs │ Character  │
│ (left  │──────────────────────────────│ Sheet /    │
│ rail,  │        SCROLLBACK            │ Map /      │
│ collap-│   (virtualized, rich spans)  │ Mail       │
│ sible) │──────────────────────────────│ (right     │
│        │  > command input   [history] │ tabbed)    │
└────────┴──────────────────────────────┴────────────┘
```

Center column is sacred and always present (scrollback + input = the game). Channel tabs sit above scrollback; clicking a tab shows that channel's own backlog with an inline input that sends `Line("+pub hello")`-style. Rails collapse; mobile shows center only with a drawer. CSS grid, `localStorage` for layout prefs.

---

## 8. Security & abuse protection

### 8.1 Passwords & secrets
- Argon2id (`argon2` crate), params ~(m=64 MiB, t=3, p=1), per-hash salt; rehash-on-login when params change.
- Web tokens: 32 bytes from OS RNG, stored hashed. SSH public keys stored as `authorized_keys`-format lines per account.

### 8.2 SSH brute force / connection abuse
`IpGuard` (shared by both transports): per-IP sliding window. SSH: max 5 auth failures / 10 min → 15 min tempban (auth rejected pre-handler); max 8 concurrent connections and 20 handshakes/min per IP; pre-auth phase timeout 120 s. `guest`/`new`/`play` funnels count as auth attempts for in-band `connect` failures too. Persistent ban list managed by staff commands lives in the core; `IpGuard` consults it.

### 8.3 HTTP/WS
`tower_governor` 0.8 (GCRA): tight quota on `/api/auth/*` (e.g. burst 5, replenish 1/30 s per IP), loose global quota elsewhere; `SmartIpKeyExtractor` only when `trusted_proxies` set. Per-session **input** flood control in the session layer (applies to SSH too): token bucket ~10 commands/s burst 30; excess → "You're doing that too fast." WS message size cap 64 KiB.

### 8.4 Output backpressure
Bounded per-connection `mpsc` (256). On full: SSH → try 5 s then disconnect "too slow" (classic slow-modem behavior); Web → coalesce (drop oldest non-critical narrative, keep latest panel snapshots, mark gap with a "… output dropped …" event) since resume/replay can't help a live-but-slow reader.

### 8.5 Misc
- Strip/deny control characters in inbound lines (allow only printable + space; no ESC injection into other players' terminals — this is the classic MU* ANSI-injection attack; enforced in the session layer so both transports are covered).
- `LineEditor::feed` and the envelope decoder get fuzz targets (cargo-fuzz) — they parse hostile bytes by definition.

---

## 9. Implementation phases

1. **Proto + session skeleton** (`proto`, `session`, `ansi`): event enum, RichText, SessionManager with a stub echo core; unit tests for ANSI downgrade + wrapping.
2. **SSH MVP** (`net-ssh`): russh server, host key gen, `play` welcome screen, LineEditor (with masked mode), renderer wired, resize/keepalive. Test with OpenSSH on Windows Terminal + Linux. *Milestone: two SSH users see each other's `say` via stub core.*
3. **Accounts & auth**: `AuthService` against core DB; `new`/`guest`/`<account>` flows, publickey + `@sshkey`, char select, multi-connect + linkdead, `IpGuard`.
4. **Web MVP** (`net-web` + `web/`): axum, token auth, WS envelope, resume buffer; SPA with scrollback + input only. *Milestone: SSH and web user in the same room.*
5. **Panels**: channels tabs, WHO rail, mail panel (`Req` hydration), prompt spec, weather toasts; sheet panel once WoD module lands.
6. **Hardening**: tower_governor, flood control, backpressure policies, fuzzing, TLS paths, load test (500 sims via russh client).

## 10. Risks

- **russh API churn** (pre-1.0, fast release train). Mitigation: pin minor, all russh types quarantined in `net-ssh`, ~1-day upgrade budget per bump; the Aug 2026 security-fix cadence means we *must* track it.
- **Server-side line editing UX**: veterans coming from tf/TinTin++ lose client-side triggers over plain `ssh`. Mitigation: solid editor + history helps; the web client is the rich option; a future "api mode" (structured over SSH exec channel) is a clean extension.
- **Raw-mode edge cases across terminals** (Windows ConPTY, mosh, tmux, IME input of CJK): mitigated by the incremental UTF-8 decoder, ESC parser fuzzing, and an explicit test matrix (Windows Terminal, iTerm2, gnome-terminal, tmux).
- **Slow-consumer output storms** (big `WHO` + spammy room): backpressure policy in §8.4 must land before any public playtest, not after.

## 11. Considered and rejected (summary)

1. **Telnet-style dumb byte pipe for SSH** (send bytes, hope client line-buffers): SSH+PTY is raw mode; without a server-side editor there's no echo and no backspace. Rejected for the LineEditor design.
2. **JWT for web auth**: stateless tokens on a stateful singleton server = worse revocation for zero scaling benefit. Opaque server-side tokens.
3. **xterm.js scrollback**: terminal emulation would discard the structured protocol's selection/links/theming benefits. DOM-rendered rich spans.
4. **Dual output paths in the core** (core formats ANSI *and* JSON): one semantic `OutputEvent` + two renderers keeps the core presentation-free and gives modules one extension point.

Sources: [russh releases](https://github.com/Eugeny/russh/releases), [russh crates.io](https://crates.io/crates/russh), [axum releases](https://github.com/tokio-rs/axum/releases), [tower_governor](https://crates.io/crates/tower_governor/0.8.0), [rust-embed](https://crates.io/crates/rust-embed), [Solid vs Svelte 2026](https://www.pkgpulse.com/guides/solidjs-vs-svelte-2026), [Svelte 5 runes guide](https://www.pkgpulse.com/guides/svelte-5-runes-complete-guide-2026)
