# rustmush â€” Server Core Architecture & Design

**Area:** Server core (workspace layout, object model, command dispatch, async architecture, persistence, scripting, output pipeline)
**Status:** Design for implementation. Planning-phase document; no code written.

---

## 1. Cargo Workspace Layout (project-wide, owned by this doc)

Virtual workspace at repo root. All crates under `crates/`. Prefix `mush-` for libraries, binary is `mushd`.

```
rustmush/
â”œâ”€â”€ Cargo.toml                  # [workspace], shared [workspace.dependencies] + lints
â”œâ”€â”€ crates/
â”‚   â”œâ”€â”€ mush-core/              # THE hub: object model, World, command registry,
â”‚   â”‚                           #   module API, events, scheduler types, output model,
â”‚   â”‚                           #   ANSI renderer. No I/O, no tokio dependency in types.
â”‚   â”œâ”€â”€ mush-proto/             # Wire types: JSON event schema for the web client,
â”‚   â”‚                           #   serde only. Leaf crate (core depends on NOTHING else;
â”‚   â”‚                           #   proto depends only on serde/serde_json).
â”‚   â”œâ”€â”€ mush-db/                # Persistence: rusqlite, migrations, dirty-flush task,
â”‚   â”‚                           #   snapshot/backup. Depends: mush-core.
â”‚   â”œâ”€â”€ mush-script/            # mlua/Luau embedding, sandbox, hook plumbing.
â”‚   â”‚                           #   Depends: mush-core.
â”‚   â”œâ”€â”€ mush-llm/               # Provider trait + Anthropic + OpenAI-compatible clients.
â”‚   â”‚                           #   (LLM agent owns internals.) Depends: mush-core (types only).
â”‚   â”œâ”€â”€ mush-agents/            # NPC minds + worldbuilder orchestration.
â”‚   â”‚                           #   Depends: mush-core, mush-llm.
â”‚   â”œâ”€â”€ mush-net/               # SSH (russh) + WebSocket transports, session mgmt.
â”‚   â”‚                           #   (Connectivity agent owns internals.)
â”‚   â”‚                           #   Depends: mush-core, mush-proto.
â”‚   â”œâ”€â”€ mush-sys-comsys/        # Channel system module.   Depends: mush-core.
â”‚   â”œâ”€â”€ mush-sys-mail/          # @mail module.            Depends: mush-core.
â”‚   â”œâ”€â”€ mush-sys-weather/       # Weather module.          Depends: mush-core.
â”‚   â”œâ”€â”€ mush-mod-wod/           # World of Darkness ruleset module. Depends: mush-core.
â”‚   â”œâ”€â”€ mush-mod-btech/         # BattleTech ruleset (later; crate reserved).
â”‚   â”œâ”€â”€ mush-testkit/           # In-process world driver for integration tests of
â”‚   â”‚                           #   modules (fake connections, scripted input/output).
â”‚   â””â”€â”€ mushd/                  # Binary: config load, wiring, main(). Depends on all.
â””â”€â”€ web/                        # Web client (TypeScript, not in cargo workspace;
                                #   codegen of TS types from mush-proto via schemars/ts-rs)
```

**Dependency graph (arrows = depends-on):**

```
mushd â†’ {mush-net, mush-db, mush-script, mush-agents, all mush-sys-*, mush-mod-*}
mush-net    â†’ mush-core, mush-proto
mush-db     â†’ mush-core
mush-script â†’ mush-core
mush-agents â†’ mush-core, mush-llm
mush-llm    â†’ mush-core
mush-sys-*, mush-mod-* â†’ mush-core   (DB access via handle passed through module API,
                                      not a direct mush-db dependency)
mush-core   â†’ mush-proto
mush-proto  â†’ (serde only)
```

**Hard rules for other agents:**
- `mush-core` never depends on tokio *types* in its public API for world logic (it may use `tokio::sync::mpsc` sender handles behind the `Outbox`/`Effects` abstractions; keep `World` itself runtime-agnostic and fully unit-testable synchronously).
- Modules (`mush-sys-*`, `mush-mod-*`) talk to the world ONLY through the `mush-core` module API (Section 7). They never import `mush-db`, `mush-net`, or each other.
- `mush-proto` is the single source of truth for the web wire format. `mush-core::output` types convert into `mush-proto` types via `From` impls that live in `mush-core` (allowed because core â†’ proto).

**Key workspace dependencies** (versions verified current as of Aug 2026): `tokio 1.x` (full), `rusqlite 0.40` (`bundled` feature â€” compiles SQLite from source, no system dep on Windows or Linux), `rusqlite_migration 2.5`, `mlua 0.11/0.12` (`luau` + `vendored`), `russh 0.60` (in mush-net), `axum 0.8` (mush-net), `serde 1`, `serde_json 1`, `bitflags 2`, `smol_str`, `thiserror 2`, `tracing 0.1`, `reqwest 0.12` (mush-llm, `rustls-tls` to avoid OpenSSL pain on Windows).

**Plugin linkage decision: static.** Modules are compiled-in crates registered in `mushd::main` behind cargo features (`--no-default-features --features wod` style). *Rejected:* dynamic `libloading` plugins (no stable Rust ABI; a crash story and a versioning story we don't need for a project where the operator compiles the server) and WASM component plugins (real isolation, but massive interface tax against a rich in-memory world; revisit only if third-party untrusted plugins ever matter).

---

## 2. Object Model (`mush-core::obj`)

### 2.1 Ids

```rust
/// Classic dbref. Monotonic, never recycled. Displays/parses as "#123".
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjId(pub u32);

pub const GOD: ObjId = ObjId(1);      // #1
pub const ROOM_ZERO: ObjId = ObjId(0); // Limbo
```

**No dbref recycling** (departure from TinyMUX garbage/@purge semantics): recycled refs are the classic source of stale-pointer bugs in MU* servers, and at u32 scale we will never exhaust ids. `@destroy` marks `GOING`, a sweep removes the object and leaves a tombstone row in SQLite so old ids resolve to "garbage" forever.

### 2.2 Object

```rust
pub enum ObjKind { Room, Exit, Thing, Player, Npc }

pub struct Obj {
    pub id: ObjId,
    pub kind: ObjKind,
    pub name: String,
    pub aliases: Vec<SmolStr>,          // players & exits: "Nick;n;nick"
    pub owner: ObjId,                    // players own themselves
    pub location: Option<ObjId>,         // None for rooms; exit: source room
    pub destination: Option<ObjId>,      // exits only
    pub home: Option<ObjId>,
    pub zone: Option<ObjId>,             // â†’ Zone Master Object (Section 2.6)
    pub parent: Option<ObjId>,           // @parent chain, attribute inheritance
    pub contents: Vec<ObjId>,            // maintained inverse of location
    pub exits: Vec<ObjId>,               // rooms: maintained inverse
    pub flags: Flags,                    // core bitflags
    pub markers: BTreeSet<SmolStr>,      // module-defined flags, e.g. "wod:kindred"
    pub locks: BTreeMap<LockType, LockExpr>,
    pub attrs: AttrMap,
    pub created: i64, pub modified: i64, // unix seconds
}
```

`Npc` is a first-class kind, not a flagged Player: it shares an `Actor` capability check (`kind.is_actor()`) with Player for movement/speech/comsys, but has no login credentials, and carries agent config in structured attributes (`agent:*` namespace) consumed by `mush-agents`.

### 2.3 Attributes

Two value forms â€” classic text for the MUSH feel, structured JSON for sheets/agent config so rulesets don't serialize into strings:

```rust
pub struct Attr {
    pub value: AttrValue,
    pub owner: ObjId,               // who set it (staff auditing)
    pub flags: AttrFlags,           // WIZARD_ONLY | SELF_ONLY | HIDDEN | LOCKED | NO_INHERIT
}
pub enum AttrValue { Text(String), Json(serde_json::Value) }

/// Names are case-insensitive, namespaced by convention: "desc", "sex",
/// "wod:sheet", "agent:persona", "comsys:prefs". Module namespaces are
/// registered (Section 7) so `@set` can enforce ownership of a namespace.
pub struct AttrMap(BTreeMap<UncasedName, Attr>);

impl Obj { /* attr lookup walks self â†’ parent chain (bounded depth 10),
              honoring NO_INHERIT â€” this is how @parent-based prototype
              objects work without softcode */ }
```

### 2.4 Flags & staff hierarchy

```rust
bitflags! { pub struct Flags: u64 {
    const WIZARD; const ROYALTY; const BUILDER;      // hierarchy bits
    const DARK; const HAVEN; const QUIET; const STICKY; const ENTER_OK;
    const JUMP_OK; const OPAQUE; const GOING; const HALTED; const SAFE;
    const ZONE_MASTER; const NO_COMMAND; /* ... */ } }

pub enum Role { Player, Builder, Royalty, Wizard, God }  // derived: #1 > WIZARD > ROYALTY > BUILDER

bitflags! { pub struct Powers: u32 {   // Rhost-style granular powers, grantable by Wizard+
    const BOOT; const EXAMINE_ALL; const TEL_ANYWHERE; const TEL_ANYTHING;
    const CHOWN_ANY; const COMSYS_ADMIN; const BUILDER_LLM; const SEE_HIDDEN; } }
```

Permission checks are centralized in `mush_core::perm` (`can_examine(w, who, what)`, `can_modify`, `controls`) â€” command handlers never hand-roll flag math. `controls()` = God âˆ¨ Wizard âˆ¨ owner âˆ¨ zone-control (Section 2.6).

### 2.5 Locks

No softcode, but classic **lock expressions** survive as a small declarative AST with TinyMUX-compatible syntax (`me`, `#123`, `+#123` carry, `$#123` owner, `flag^WIZARD`, `attr:pattern`, `@#123/locktype` indirect, `& | ! ( )`):

```rust
pub enum LockType { Basic, Enter, Use, Page, Teleport, Speech, Zone, Drop, Give, Mail }
pub enum LockExpr {
    Const(bool), Is(ObjId), Carry(ObjId), OwnedBy(ObjId),
    HasFlag(SmolStr), AttrMatch { name: UncasedName, glob: String },
    Indirect(ObjId, LockType),
    Not(Box<LockExpr>), And(Box<LockExpr>, Box<LockExpr>), Or(Box<LockExpr>, Box<LockExpr>),
}
pub fn eval_lock(w: &World, expr: &LockExpr, who: ObjId, depth: u8) -> bool;
```

### 2.6 Zones (theming + delegated admin)

A zone is any Thing flagged `ZONE_MASTER`; objects point at it via `zone`. Three consumers:
1. **Theming (LLM):** ZMO attributes `theme:prose`, `theme:palette`, `theme:era` are read by the worldbuilder so generated @descs stay consistent per neighborhood. Zones nest via the ZMO's own `zone` field (district â†’ city), and the worldbuilder walks the chain to build its style context.
2. **Delegated control:** the ZMO's `Zone` lock grants `controls()` over member objects â€” zone-wizards without global WIZARD.
3. **Zone commands/hooks:** scripts and module behavior attached to a ZMO apply to all member rooms (e.g. weather region = zone).

### 2.7 World

```rust
pub struct World {
    objs: Vec<Option<Obj>>,                 // dense by ObjId (no recycling â†’ index = id)
    next_id: u32,
    player_index: HashMap<UncasedName, ObjId>,   // names + aliases
    zone_members: HashMap<ObjId, HashSet<ObjId>>, // maintained index
    dirty: HashSet<ObjId>,                   // for the persistence flusher
    pub sched: Scheduler,                    // Section 4.3
    pub registry: Registry,                  // commands, hooks, module state (Section 3, 7)
}
```

Entirely in RAM. All mutation goes through `World` methods (`move_to`, `set_attr`, `create`, â€¦) that maintain inverse indexes and mark dirty â€” handlers never poke fields directly.

---

## 3. Command Parsing & Dispatch (`mush-core::cmd`)

### 3.1 Input pipeline (per line, inside the world task)

```
raw line
  â†’ single-char alias expansion:  "  â†’ say, :/; â†’ pose (;=nospace), \\ â†’ @emit, ' â†’ whisper? (config)
  â†’ HOME keyword
  â†’ exact/prefix match against EXITS here (room exits, then zone exits, then master-room globals)
  â†’ command table lookup (longest-prefix on registered names, e.g. "l" â†’ look; @-cmds require exact or unambiguous)
  â†’ module fallback matchers (registered, e.g. comsys "chanalias <msg>" pattern)
  â†’ Huh?  (echoed only to actor; logged with fuzzy suggestion)
```

Exits before commands (classic order) so `north` always moves even if a module registers a `north` command.

### 3.2 Grammar & registry

MUSH surface: `@dig/teleport Grand Hall = north;n,south;s` â€” verb, `/switches`, arg text split by `=` and `,` per the command's declared shape. Parsing of the arg shape is table-driven, not per-handler string surgery:

```rust
pub enum ArgShape { None, Raw, EqPair, EqPairList /* rhs split on , */, EqPairSemi /* ; aliases */ }

pub struct CommandSpec {
    pub name: &'static str,             // "@dig", "look", "+sheet"
    pub aliases: &'static [&'static str],
    pub switches: &'static [&'static str],   // validated; unknown switch = error
    pub shape: ArgShape,
    pub min_role: Role,                  // plus optional Powers requirement
    pub handler: CmdHandler,
}
pub type CmdHandler = fn(&mut Ctx<'_>) -> Result<(), CmdError>;   // SYNC â€” see Section 4
```

```rust
pub struct Ctx<'w> {
    pub w: &'w mut World,
    pub actor: ObjId,
    pub conn: Option<ConnId>,           // None when script/NPC/queued origin
    pub switches: SwitchSet,
    pub args: ParsedArgs,               // lhs/rhs accessors + `%r`-free plain text
    pub out: &'w mut Outbox,            // queue OutEvents to actor/room/targets (Section 6)
    pub fx: &'w mut Effects,            // defer async work (Section 4.2)
}
```

**Registration & extensibility:** `Registry::register(spec)` errors on name collision unless `override` is explicit (staff can also `@command/disable`). Convention enforced at registration: `@` = build/admin, `+` = module/game commands (comsys, rulesets), bare verbs reserved to core (look, get, drop, give, say, pose, page, think, inventory, examine, go). Object-attached "$-command" feel comes from script hooks (Section 5.4), not the dispatcher.

**Name matching for targets** (`look here`, `examine *Nick`, `@set #123/desc=...`) lives in `mush_core::matchit`: one resolver implementing classic precedence (me/here/#dbref/*player/exact-in-location/prefix/english "2nd sword"), reused by every handler and exposed to scripts.

---

## 4. Async Architecture

### 4.1 Recommendation: single-writer world task (message-passing), not locks

One tokio task owns `World` outright â€” zero locks on game state. Everything else (SSH sessions, WebSocket sessions, LLM calls, DB flusher, script timers) is a separate task communicating over channels.

Rationale: at MU*-scale (hundreds of players, text-rate input) game logic is trivially single-core; every classic server (TinyMUX, Rhost, PennMUSH) is effectively single-threaded game logic, which is precisely why MUSH semantics (strict global event ordering: everyone sees the same things in the same order) are what veterans expect. A `RwLock<World>` invites held-across-await deadlocks and write starvation; sharded/per-room locking buys throughput nobody needs at the cost of cross-room invariants (movement touches two rooms atomically). *Rejected:* actor-per-object (ordering nightmares, and MUSH commands routinely touch 3+ objects transactionally); sharded `dashmap` world (same, plus iteration pain for `@find`/who-lists).

```rust
pub enum Inbound {
    Cmd      { conn: ConnId, actor: ObjId, line: String },
    ConnUp   { conn: ConnId, actor: ObjId, caps: ClientCaps },  // from mush-net after auth
    ConnDown { conn: ConnId },
    Fired    (ScheduledKind),                        // from the scheduler (Section 4.3)
    Apply    (Box<dyn FnOnce(&mut Ctx<'_>) + Send>), // completion of deferred async work
    Control  (ControlMsg),                           // @shutdown, @dump, SIGTERM
}

// world task main loop (in mushd, logic in mush-core):
loop {
    tokio::select! {
        Some(msg) = inbound.recv() => world.handle(msg, &mut outbox, &mut effects),
        _ = sched.sleep_until_next() => world.fire_due(&mut outbox, &mut effects),
    }
    outbox.drain_to_router();      // â†’ per-connection mpsc senders (mush-net owns delivery)
    effects.spawn_all(&handle);    // launch deferred futures
    persist.send_dirty(world.take_dirty());   // non-blocking; DB task owns writes
}
```

### 4.2 Slow work: the `Effects` two-phase pattern

Handlers are **sync** and must return in â‰ª1ms. Anything slow (LLM call, bcrypt at login, big DB query for @mail search) is deferred:

```rust
// inside a handler:
let prompt = build_prompt(ctx.w, target)?;             // phase 1: snapshot, sync
ctx.fx.defer(async move {
    let desc = llm.generate(prompt).await;             // off-loop
    move |ctx: &mut Ctx<'_>| {                         // phase 2: re-enter world task
        // world may have changed; re-validate target still exists, then apply
        apply_generated_desc(ctx, target, desc);
    }
});
```

`Effects::defer` boxes the continuation as `Inbound::Apply`. This is the **only** sanctioned way modules/agents touch the world from async code â€” it makes LLM latency invisible to the game loop and forces the re-validation step that prevents stale-world bugs.

### 4.3 Scheduler (`@wait`, ticks, NPC thinks)

Min-heap of deadlines inside the world task; `select!` sleeps until the earliest.

```rust
pub enum ScheduledKind {
    WaitCmd   { actor: ObjId, line: String },          // @wait 30=<command>
    ModuleTick{ module: SmolStr, tag: SmolStr },       // weather: every 300s per zone
    ScriptTimer{ obj: ObjId, hook: SmolStr },          // mush.schedule() from Lua
    NpcThink  { npc: ObjId },                          // agents crate re-arms with jitter
    SweepGarbage, Checkpoint,
}
pub struct Scheduler { heap: BinaryHeap<(Reverse<Instant>, u64 /*seq*/, ScheduledKind)>, /* + cancel set */ }
```

Long `@wait`s (> 5 min) and recurring module ticks are also journaled to SQLite so they survive restart; sub-minute ones are accepted as lost-on-crash. NPC think cadence is owned by `mush-agents` (it enqueues the next think when handling the current one â€” natural backpressure if the LLM is slow).

---

## 5. Persistence (`mush-db`)

### 5.1 Engine: SQLite via `rusqlite` (bundled) + `rusqlite_migration`

- **Why SQLite:** single file, zero ops, perfect at hundreds-of-players scale, real SQL for the things that want queries (mail folders, channel history, logs), WAL mode for concurrent read (backup/stats tools), and the `bundled` feature gives identical behavior on Windows and Linux with no system libraries.
- **Why `rusqlite` not `sqlx`:** the world task is a single synchronous writer with a dedicated DB thread â€” async DB access buys nothing and `sqlx`'s compile-time-checked queries fight the module-supplied-migrations design. `rusqlite` 0.40 is current and actively maintained; `rusqlite_migration` 2.5 handles versioned migrations.
- *Rejected:* **Postgres** (ops burden and no single-binary story for hobbyist operators; nothing here needs it), **sled** (still "beta champagne", storage rewrite perpetually in progress â€” not a base for a game DB), **redb** (genuinely solid and stable, but KV-only; we'd reimplement mail/channel/log querying that SQL gives for free).

### 5.2 RAM-authoritative + write-behind journal

The in-memory `World` is authoritative; SQLite is the durability layer. A dedicated persistence thread (std thread w/ `crossbeam-channel`, or `spawn_blocking` pool) receives:

```rust
pub enum Persist {
    Upsert(ObjSnapshot),          // serialized full object (core cols + attrs delta)
    Delete(ObjId),
    Module(Box<dyn FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<()> + Send>),
    Checkpoint { reason: CheckpointReason },   // SQLite backup API â†’ snapshots/mush-YYYYMMDD-HHMM.db
    Flush(oneshot::Sender<()>),   // barrier: used by @dump, shutdown, mail-send durability
}
```

Dirty objects flush every 2s (config) batched in one transaction. Ops that must not be lost (character creation, @mail send, purchases of any future kind) send `Flush` and the handler's confirmation output is deferred behind it via `Effects`. Crash window: â‰¤ flush interval, an accepted MU*-standard tradeoff (far better than classic hourly @dumps).

**Snapshots:** SQLite online backup API to timestamped files hourly + on `@dump` + on clean shutdown; retention config (e.g. 24 hourly / 14 daily). A snapshot is a complete, copyable single file â€” the whole backup story.

### 5.3 Schema (v1)

```sql
objects(id PK, kind, name, owner, location, destination, home, zone, parent,
        flags, created, modified, tombstone)
aliases(obj_id, alias)                          -- indexed, uncased
attrs  (obj_id, name, kind /*text|json*/, value, owner, flags, PK(obj_id,name))
locks  (obj_id, lock_type, expr_json, PK(obj_id,lock_type))
markers(obj_id, marker, PK(obj_id,marker))
sched  (id PK, due_at, kind_json)               -- persisted long waits/ticks
meta   (key PK, value)                          -- schema version, next_id, world config
-- module tables are namespaced: comsys_channels, comsys_members, mail_messages, ...
```

Modules contribute migrations through the registry (`Registry::migrations("comsys", M::up(...))`); `rusqlite_migration` runs core's set first, then each module's, tracked per-namespace in its user-version table. Attributes as rows (not one JSON blob per object) keeps `@find`-style queries and per-attr dirty writes cheap; `AttrValue::Json` covers structured ruleset data without schema churn.

---

## 6. Output Pipeline (coordinate-level; transports owned by connectivity agent)

Every player-visible emission is an `OutEvent` â€” semantic first, rendered per-connection last.

```rust
// mush-core::output
pub struct RichText(pub Vec<Span>);
pub struct Span { pub text: String, pub style: StyleClass }
pub enum StyleClass {           // SEMANTIC, not colors
    Plain, PlayerName, NpcName, ObjName, ExitName, ChannelName, RoomName,
    Emph, Quote, System, Success, Warning, Error, Dim, Custom(SmolStr),  // modules
}

pub enum OutEvent {
    Line(RichText),                                    // generic narrative line
    Say   { speaker: ObjId, name: String, text: RichText },
    Pose  { speaker: ObjId, name: String, text: RichText, nospace: bool },
    Room  (RoomView),           // structured look: name, desc, contents, exits, zone theme hints
    Channel{ chan: SmolStr, speaker: String, text: RichText },
    Page  { from: String, text: RichText },
    Typed { tag: SmolStr, payload: serde_json::Value },  // module â†’ web panel ("wod.sheet", "mail.unread")
    Prompt(PromptState),
}
```

- **ANSI path (SSH):** `mush_core::output::ansi::render(&OutEvent, &Theme, width) -> String`. Pure function in core (testable, shared). `Theme` maps `StyleClass â†’ ansi16/256/truecolor` per client caps; wraps to client width; `Typed` events degrade to a text fallback the emitting module supplies (or are dropped if marked panel-only).
- **JSON path (web):** `mush-proto` defines the wire enum (`{"t":"say","speaker":"#42","name":"Nick","text":[...spans]}`); `From<OutEvent>` in core. Spans keep their semantic classes so the web client themes them in CSS. `Typed` passes through verbatim â€” this is how the sheet/mail/map panels are fed without core knowing about them.
- **Routing:** `Outbox` collects `(Audience, OutEvent)` â€” `Audience::{Conn(ConnId), Actor(ObjId), Room{room, exclude}, Channel(SmolStr), All}`. The world task resolves audiences to connection ids and hands `(ConnId, OutEvent)` to the router channel; **mush-net owns everything after that** (rendering choice per connection, buffering, backpressure/slow-client policy).

---

## 7. Module API (`mush-core::module`) â€” the boundary other agents build against

```rust
pub trait GameModule: Send {
    fn name(&self) -> &'static str;                       // "comsys", "wod"
    fn register(&mut self, reg: &mut Registry) -> Result<(), ModError>;
    fn on_world_loaded(&mut self, ctx: &mut Ctx<'_>) -> Result<(), ModError>;
}

impl Registry {
    pub fn command(&mut self, spec: CommandSpec);
    pub fn marker_flag(&mut self, name: &str);                 // "wod:kindred"
    pub fn attr_namespace(&mut self, ns: &str);                // owns "wod:*" attrs
    pub fn migrations(&mut self, ns: &str, ms: Vec<Migration>);
    pub fn tick(&mut self, tag: &str, every: Duration);        // â†’ ScheduledKind::ModuleTick
    pub fn subscribe(&mut self, ev: WorldEventKind, f: WorldEventFn);
        // WorldEvent: Moved{who,from,to}, Said{..}, Connected, Disconnected,
        // ObjCreated/Destroyed â€” sync observers running in the world task
    pub fn state<T: Any + Send>(&mut self, init: T);           // typed module state slot in World
    pub fn db(&self) -> DbHandle;                              // Persist::Module sender, namespaced
}
```

Comsys, mail, weather, and the WoD ruleset are all *just modules* â€” if the module API can't express them, it gets fixed before BattleTech ever starts.

---

## 8. Scripting (`mush-script`)

### 8.1 Recommendation: **mlua with the Luau backend** (`mlua` 0.11/0.12, features `luau` + `vendored`)

Rationale:
- **Luau is sandboxed by design** (built for Roblox user scripts): no `io`/`os`/`require` by default, `Lua::sandbox(true)` mode, VM interrupt callback for CPU budgets, `set_memory_limit` for RAM caps. This is exactly the "builder-authored scripts are untrusted" story, off the shelf.
- Lua is the MUD/MU* modding lingua franca â€” builders are far more likely to know it; enormous documentation surface.
- Coroutines map naturally onto multi-step NPC/trigger behavior later.
- mlua is actively maintained (0.11.6 â†’ 0.12 line current, mlua-rs org) and vendors the Luau build â€” works on MSVC Windows and Linux (verify Luau C++ build in CI on day one; this is the top build risk).

*Rejected:* **Rhai** â€” genuinely attractive (pure Rust, no C++ build, built-in `max_operations`/depth limits), and it was a close call; rejected on interpreter speed (AST-walking, matters for per-line listen hooks), no coroutines, and an unfamiliar language for the builder audience. **Full trust Lua 5.4 via mlua** â€” instruction-count hooks can meter CPU, but the stdlib-stripping and sandbox burden lands on us instead of on Luau's design.

### 8.2 Sandbox contract

- One Luau VM per script **owner** (cheap; isolates memory accounting and blast radius), scripts stored in `script:*` attributes on objects, compiled on load, cached.
- Budgets per invocation: interrupt callback trips at N VM steps (~default 500k), wall-clock backstop 50ms, memory cap 16MB/VM (all config). Overrun â†’ script aborted, error to owner, offending hook auto-disabled after 3 strikes (`HALTED` flag on the object).
- API surface (`mush.*` table, userdata-backed, every call permission-checked **as the script's owner**): `get_attr/set_attr`, `name/loc/contents`, `emit(target|room, richtext)`, `move(obj, dest)`, `schedule(secs, fnname)`, `match(str)`, dice utils. No network, no filesystem, no LLM (LLM access is command/agent-level, not script-level, v1).
- Scripts run **synchronously in the world task** within budget â€” they see a consistent world and their effects are ordered like any command. The budget is what makes this safe.

### 8.3 Hook points

`on_command(pattern)` (the no-softcode replacement for `$`-commands on objects), `on_enter`/`on_leave` (room triggers), `on_use`, `on_hear(pattern)` (listen â€” powers scripted-NPC reflexes without LLM), `on_timer`, `on_connect`/`on_disconnect` (player objects, staff-gated). Hook registration is data (attribute presence), discovered by `mush-script` at load and on attribute change events.

---

## 9. Implementation Phases

1. **Skeleton (wk 1-2):** workspace, `mush-core` object model + World + matchit + perms, CI on Windows+Linux (including mlua/Luau build canary), stdin REPL driver in `mushd`, `mush-testkit`. Exit: `@dig/@desc/look/say/pose/movement` in-process tests pass.
2. **Loop + DB (wk 3-5):** world task, Inbound/Outbox/Effects, scheduler, `mush-db` (schema v1, dirty flush, checkpoint/backup, migrations). `mush-net` stubs handed to connectivity agent. Exit: kill -9 the server, restart, world intact.
3. **Module API + built-ins (wk 5-8):** Registry finalized; comsys and @mail built as modules (they are the API's proving ground); locks, zones, powers; output pipeline with ANSI renderer + proto JSON.
4. **Scripting (wk 8-10):** mush-script, sandbox budgets, hooks; weather module implemented *using* module ticks + zone theming attrs.
5. **LLM seams (wk 10+):** `Effects`-based integration points for worldbuilder (`@llm/desc here=<prose>` reading zone theme chain) and `NpcThink` scheduling for `mush-agents` (internals owned by LLM agent).
6. **WoD module**, then BattleTech later â€” btech is the module-boundary stress test (hex state = module `state<T>` + `Typed` panel events).

## 10. Risks

- **mlua/Luau C++ vendored build on Windows MSVC** â€” mitigate: CI matrix from day 1; fallback path is mlua/Lua 5.4-vendored with hook-based metering (API identical), Rhai as the nuclear option.
- **Single-writer loop stalls** from a pathological script or handler â€” mitigate: budgets (8.2), handlers-must-defer rule (4.2), loop-lag watchdog metric from Phase 2.
- **Module API ossifying wrong** â€” mitigate: comsys/mail/weather built as modules *before* declaring v1; no external module stability promise until WoD ships.
- **Output schema churn vs web client** â€” mitigate: `mush-proto` versioned envelope (`{"v":1,...}`) and TS type generation from day one.

## Key rejected alternatives (summary)

1. **ECS (bevy_ecs/hecs) object model** â€” MU* objects are heterogeneous attribute bags with almost no per-frame system iteration; ECS adds indirection and buys nothing here.
2. **`RwLock`/sharded world with async handlers** â€” ordering and cross-object atomicity pain for throughput no MU* needs; single-writer matches genre semantics exactly.
3. **sqlx/Postgres and sled** â€” see 5.1; SQLite+rusqlite wins on ops story and maintenance status.
4. **Dynamic (`libloading`) or WASM plugins** â€” no stable ABI / heavy interface tax; static crate modules behind cargo features instead.

Sources: [mlua (GitHub)](https://github.com/mlua-rs/mlua), [mlua docs.rs 0.11.6](https://docs.rs/crate/mlua/latest), [rhai (GitHub)](https://github.com/rhaiscript/rhai), [redb (GitHub)](https://github.com/cberner/redb), [sled (GitHub)](https://github.com/spacejam/sled), [rusqlite (crates.io)](https://crates.io/crates/rusqlite), [rusqlite_migration docs](https://docs.rs/crate/rusqlite_migration/latest), [russh (GitHub)](https://github.com/Eugeny/russh)