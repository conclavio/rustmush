# rustmush — Game Systems & Built-ins Design

**Area:** Ruleset plugin API, dice engine, World of Darkness module, TacSim boundary, comsys, @mail, weather, staff tooling (jobs/notes).
**Status:** Design, planning phase. Assumes companion docs cover: core object DB/persistence, command parser/dispatcher, SSH + WebSocket transport and the JSON event protocol, Lua scripting sandbox, and the LLM provider layer. Where this doc needs those, it names the interface it expects and flags it as a cross-area contract.

---

## 1. Design principles

1. **Rulesets are compiled-in Rust crates behind a trait, not dynamically loaded.** One binary, one workspace, feature-flag optional rulesets. (Justification in §3.1.)
2. **The core knows nothing about d10s or tactical cells.** It knows: sheets are versioned, schema-described documents attached to objects; rulesets register commands, tick handlers, event hooks, and web panel feeds; rulesets may own opaque per-scene state. Everything WoD-shaped lives in `ruleset-wod`; everything tactical lives in `mush-tactics`, over the atlas's square windows (doc 14 D-14-4).
3. **Every random outcome flows through one dice engine** with an injectable RNG, so tests are deterministic, rolls are loggable/auditable, and LLM NPCs can only *request* rolls — never invent results.
4. **The web client is fed structured events, never scraped text.** Comsys, mail, weather, and rulesets each own a namespaced event vocabulary; the terminal rendering of the same event is produced server-side from the structured form.
5. **LLM output is flavor, never state.** Weather states, roll results, sheet values, and damage are deterministic; LLMs narrate around them (weather ambience, NPC dialogue) and act through validated tools.

---

## 2. Workspace layout (this area's crates)

```
rustmush/
  crates/
    mush-ruleset/      # Ruleset trait, sheet schema types, registrar, chargen framework
    mush-dice/         # expression parser + evaluator + pool-mechanic traits (no I/O)
    mush-jobs/         # job-lite ticket system + character notes (shared staff infra)
    mush-comsys/       # channels
    mush-mail/         # @mail
    mush-weather/      # weather cells, climate tables, ambience
    ruleset-wod/       # Chronicles of Darkness 2e (oWoD variant later)
    mush-tactics/     # reserved; created as an empty seam-test crate in phase 1 (see §6.5)
```

Dependency direction: `ruleset-*` → `mush-ruleset` + `mush-dice` + core crates. `mush-comsys` / `mush-mail` / `mush-weather` / `mush-jobs` depend only on core crates (they are built-ins, not rulesets — they exist on every game regardless of ruleset). `mush-dice` depends on nothing but `rand` + parser deps, so it's unit-testable in isolation and reusable from Lua bindings.

**Crates (versions verified Aug 2026):** `serde` 1, `serde_json` 1, `thiserror` 2, `rand` 0.9 + `rand_chacha` 0.9 (seedable RNG for tests/replays), `winnow` 0.7 for the dice grammar ([winnow](https://crates.io/crates/winnow)), `schemars` 1.2 for emitting JSON Schema of sheet documents to the web client ([schemars](https://crates.io/crates/schemars/versions)), `smol_str` for interned short strings. (Doc 14 D-14-4 struck the hexagonal grid: the tactical layer rides the atlas's square windows and no grid crate is needed.)

---

## 3. Ruleset plugin API (`mush-ruleset`)

> **Retired 2026-09-16 (owner, README superseded item 6, [16 §11](16-combat-arena.md)).** There is
> one game. `mush-ruleset` remains as the platform's sheet, chargen and vitals library with exactly one
> implementation (the TacSim character sheet); the plugin posture, `[rulesets].enabled`, the WoD and
> Shadowrun modules of §5 and their dice pools (§4.2) are deleted in [16b cut R](16b-combat-implementation-plan.md).
> The sections below are kept as history.

### 3.1 Compiled-in trait objects — decision and justification

**Decision: rulesets are statically compiled crates registering `Arc<dyn Ruleset>` at startup**, selectable per-game via config (`[rulesets] enabled = ["wod"]`) and per-crate Cargo features to shrink builds.

Rejected: **dynamic loading via `libloading`** — Rust has no stable ABI; every plugin would need to be built by the exact same compiler against the exact same crate versions, which on Windows+Linux cross-platform is all pain and no gain when we control the whole tree. Rejected: **WASM component plugins** — real isolation, but sheet access, DB access, and tick hooks would all become serialized boundary-crossings; the async story inside wasmtime hosts is still awkward for something as chatty as combat ticks; and our actual third-party-code story is Lua scripts (already sandboxed by the scripting area), not third-party rulesets. If a genuine external-ruleset ecosystem ever appears, WASM is the retrofit path — the trait below is deliberately `serde`-value-shaped at its boundaries so it could be projected across a WASM boundary later without redesign.

### 3.2 Core traits

```rust
pub type RulesetId = smol_str::SmolStr;       // "wod", "tacsim"

pub trait Ruleset: Send + Sync + 'static {
    fn id(&self) -> RulesetId;
    fn name(&self) -> &str;                    // "Chronicles of Darkness 2e"

    /// Declarative description of this ruleset's character sheet.
    fn sheet_schema(&self) -> &SheetSchema;

    /// Called once at boot. Registers commands, ticks, hooks, panels, Lua API.
    fn register(&self, reg: &mut RulesetRegistrar<'_>);

    /// Optional guided character generation (see §3.5).
    fn chargen(&self) -> Option<&dyn ChargenFlow> { None }

    /// Sheet document migration between schema versions.
    fn migrate_sheet(&self, from_version: u32, data: serde_json::Value)
        -> Result<serde_json::Value, SheetError>;

    /// Capability grounding for LLM NPC agents (see §5.8).
    fn agent_profile(&self, sheet: &SheetDoc) -> AgentCapabilityProfile;
}
```

```rust
pub struct RulesetRegistrar<'a> { /* borrows core registries */ }

impl RulesetRegistrar<'_> {
    /// Player/staff commands. Names are namespaced: the registrar rejects
    /// collisions with core verbs; rulesets conventionally use +prefixed verbs.
    pub fn command(&mut self, spec: CommandSpec, h: Arc<dyn CommandHandler>);

    /// Periodic work. Cadence is per-handler; TacSim needs sub-second later,
    /// so the enum carries Duration from day one even though WoD uses Coarse.
    pub fn tick(&mut self, cadence: TickCadence, h: Arc<dyn TickHandler>);

    /// Interception points in core flows (veto-capable where marked).
    pub fn hook(&mut self, point: HookPoint, h: Arc<dyn EventHook>);

    /// Reserve a namespaced web-panel event channel, e.g. "wod.sheet",
    /// "tactics.map". The web protocol routes `panel.<name>.*` frames.
    pub fn panel_feed(&mut self, name: &str) -> PanelFeedHandle;

    /// Expose read-only functions into the Lua sandbox under rs.<id>.*
    pub fn script_api(&mut self, table: ScriptTableBuilder<'_>);

    /// Register this ruleset's job queues (xp approval, chargen approval).
    pub fn job_queue(&mut self, queue: JobQueueSpec);
}

pub enum TickCadence {
    Coarse(std::time::Duration),   // >= 1s; weather, regen, upkeep
    Fast(std::time::Duration),     // sub-second; reserved for tactics (§6)
    Scene,                         // per-scene ticker, started on demand (§6)
}

pub enum HookPoint {
    CharacterCreated, CharacterConnected, CharacterDisconnected,
    MoveAttempt,        // veto-capable: tactics movement rules, wod barriers
    LookRendered,       // append ruleset lines to room desc (weather uses core version of this)
    SheetChanged,       // ruleset reacts to staff edits
    SceneStarted, SceneEnded,
}
```

Handlers receive a `WorldCtx<'_>` (cross-area contract with the core): object read/write access under the core's locking discipline, the output/event bus, `SheetStore`, `WeatherQuery`, `JobsApi`, an `RngHandle`, and the LLM gateway. Rulesets never touch the DB layer directly.

### 3.3 Sheet schema — declarative, typed

The schema is data, not code, so that: `+sheet` terminal rendering, the web character-sheet panel, staff `@sheet/set` validation, chargen UI, and LLM serialization are all generated from one source.

```rust
pub struct SheetSchema {
    pub ruleset: RulesetId,
    pub version: u32,
    pub sections: Vec<SheetSection>,
}

pub struct SheetSection {
    pub id: SmolStr,            // "attributes", "skills", "merits", "health"
    pub label: String,
    pub layout: SectionLayout,  // Grid { rows, cols }, List, Tracks — web hint
    pub fields: Vec<FieldDef>,
}

pub struct FieldDef {
    pub id: SmolStr,            // "strength", "brawl"
    pub label: String,
    pub ty: FieldType,
    pub category: Option<SmolStr>,  // "physical"/"mental"/"social"
    pub default: serde_json::Value,
    pub staff_only: bool,           // players can't self-edit (most fields)
}

pub enum FieldType {
    Dots { min: u8, max: u8 },                  // attributes, skills, merits
    Number { min: i32, max: i32 },              // size, speed overrides
    Text { max_len: u32 },                      // concept, virtue/vice
    Enum { options: Vec<SmolStr> },             // template: mortal/vampire/...
    Track { size: TrackSize, levels: Vec<DamageLevel> }, // health, willpower
    List { of: Box<FieldType>, max: u32 },      // merits, conditions, specialties
    Map { value: Box<FieldType> },              // named merit -> dots
}

pub enum TrackSize { Fixed(u8), Derived(DerivedExpr) } // health = stamina+size
```

`DerivedExpr` is a tiny arithmetic AST over field refs (`stamina + size`, `resolve + composure`) evaluated by `mush-ruleset` — enough for derived stats without inventing softcode.

**Storage:** the sheet is a single versioned JSON document per (object, ruleset):

```rust
pub struct SheetDoc {
    pub ruleset: RulesetId,
    pub schema_version: u32,
    pub data: serde_json::Value,   // validated against SheetSchema on write
    pub approved: ApprovalState,   // Unapproved | Pending(JobId) | Approved { by, at }
}

pub trait SheetStore {  // implemented by persistence area
    fn get(&self, obj: ObjId, ruleset: &RulesetId) -> Result<Option<SheetDoc>>;
    fn put(&self, obj: ObjId, doc: &SheetDoc) -> Result<()>;   // validates + audits
}
```

One JSON blob (not one attribute per stat) keeps reads/writes atomic, makes migration a pure `Value -> Value` function, and serializes trivially to the web panel. The `schemars`-emitted JSON Schema of each ruleset's sheet is served to the web client at handshake so the sheet panel renders generically.

### 3.4 Command surface contract

Rulesets get first-class commands (not softcode lookalikes). `CommandSpec` carries: names/aliases, switch grammar (`/rote`, `/priv`), argument shape, lock (who may run it), and help text — the help system and web command palette are generated from specs.

### 3.5 Chargen framework

`ChargenFlow` is a staged state machine the core drives; the web client renders it as a wizard, the terminal as a room-less menu context (the parser area's "input context stack" contract — also reused by mail compose, §8):

```rust
pub trait ChargenFlow: Send + Sync {
    fn stages(&self) -> Vec<ChargenStage>;         // ordered; each has prompts + validation
    fn validate_stage(&self, sheet: &SheetDoc, stage: &SmolStr) -> Vec<ChargenIssue>;
    fn finalize(&self, sheet: &mut SheetDoc) -> Result<(), ChargenIssue>; // derived stats, freebie audit
}
```

Finalize flips `ApprovalState::Pending` and opens a job in the ruleset's chargen queue (§9).

---

## 4. Dice engine (`mush-dice`)

Two layers: a **generic expression evaluator** and **pool mechanics** plugged per ruleset.

**Considered and rejected:** the `caith` crate — it is alive (4.2.4, Mar 2025; [caith](https://crates.io/crates/caith)) and fine for Discord-bot-style rolling, but it returns formatted strings first and structured data second, its grammar can't express CofD chance dice/rote rerolls/oWoD botch subtraction, and we need per-die provenance for the web client (render each die, highlight rerolls) and audit logs. The grammar below is ~300 lines of `winnow`; owning it is cheaper than wrapping.

### 4.1 Generic expressions

Grammar (winnow 0.7): `NdS[!][kh N|kl N][+|- mod]`, integers, `+`/`-` chains, parentheses. Examples: `3d6+2`, `2d10kh1`, `1d20+5`, `4d6kh3`. This serves TacSim (`2d6` cluster/to-hit), misc staff rolls, and Lua's `rs.dice.roll("3d6")`.

```rust
pub fn parse(expr: &str) -> Result<DiceExpr, DiceParseError>;

pub struct RollCtx<'r> {
    pub rng: &'r mut dyn rand::RngCore,   // ChaCha12 seeded per-server; fixed seed in tests
    pub limits: RollLimits,               // max dice 100, max depth — DoS guard
}

impl DiceExpr {
    pub fn eval(&self, ctx: &mut RollCtx) -> RollOutcome;  // total + per-die record
}

pub struct DieRecord { pub sides: u8, pub value: u8, pub kept: bool, pub reroll_of: Option<u16> }
```

### 4.2 Pool mechanics

```rust
pub trait PoolMechanic: Send + Sync {
    type Opts;
    type Outcome: serde::Serialize;    // goes straight onto the event bus
    fn roll(&self, pool: i32, opts: &Self::Opts, ctx: &mut RollCtx) -> Self::Outcome;
}
```

`ruleset-wod` implements `CofdPool` (and later `Owod20Pool`); `mush-tactics` implements `GunneryRoll` etc. Every outcome carries the full `Vec<DieRecord>` so terminal, web, and logs render from the same truth.

---

## 5. World of Darkness module (`ruleset-wod`)

### 5.1 Baseline: Chronicles of Darkness 2e. oWoD20 as a future variant.

**Recommendation: CofD 2e.** Rationale: one unified mechanic (d10 pool, success on 8+, 10-again) with modifiers as flags, versus oWoD's variable target numbers, 1s-subtract-successes, and botch table — CofD is both the cleaner first implementation and the stricter test of the schema system (conditions, tilts, beats). The differences from oWoD20 are confined to (a) roll resolution and (b) sheet layout (Talents/Skills/Knowledges split, virtues, backgrounds, per-splat pools like Rage/Gnosis) — so we structure for variants now:

```rust
pub enum RollVariant { Cofd2e, Owod20 }   // selects PoolMechanic + sheet schema pair
```

`ruleset-wod` ships `Cofd2e` complete; `Owod20` is a second `SheetSchema` + `PoolMechanic` inside the same crate later, sharing chargen framework, XP flow, and staff tools.

### 5.2 Sheet structure (CofD 2e)

- **Attributes** (3×3 grid, dots 1–5, default 1): Intelligence/Wits/Resolve, Strength/Dexterity/Stamina, Presence/Manipulation/Composure. Categories `mental|physical|social`, rows `power|finesse|resistance`.
- **Skills** (24, dots 0–5): 8 mental / 8 physical / 8 social; `specialties: List<Text>` keyed per skill.
- **Merits**: `Map { value: Dots{0..5} }` plus freeform note per entry.
- **Derived**: Health `Track{ size: Derived(stamina+size) }`, Willpower `Track{ Derived(resolve+composure) }`, Speed, Defense, Initiative as `Derived` numbers; Integrity `Dots{0..10}` default 7.
- **Supernatural template slot**: `template: Enum{mortal, vampire, werewolf, mage, ...}` plus a `template_data: Map` section whose fields the template registers (disciplines, gifts, arcana; power stat `Dots{1..10}`; resource pool `Track{Fixed}` for Vitae/Essence/Mana). Templates are sub-modules of `ruleset-wod` behind the same schema machinery — not separate rulesets.
- **Play state**: `conditions: List<Text>`, `tilts: List<Text>`, `aspirations: List<Text>` (3), `beats: Number`, `experiences: Number`.

### 5.3 Damage tracks

```rust
pub enum Damage { Bashing, Lethal, Aggravated }
pub struct HealthTrack { pub boxes: Vec<Option<Damage>> }  // index 0 = leftmost
```

`apply(damage)` implements CofD stacking: new damage fills leftmost empty box; if full, upgrades leftmost lesser damage (B→L, L→A); rightmost-box-filled states emit wound-penalty and dying events. Willpower is a simple spent/max track. All mutations go through `+heal/+harm` staff commands or ruleset internals — audited.

### 5.4 `+roll` syntax

```
+roll strength+brawl                 # pool from sheet fields (fuzzy prefix match)
+roll strength+brawl-2               # situational modifier
+roll dex+athletics+equipment:2      # named bonus tagged in output
+roll 7                              # raw pool (untied to sheet)
+roll/rote resolve+composure         # rote quality
+roll/9again firearms+dex            # 9-again (also /8again, /noagain)
+roll/wp presence+intimidation       # +3 dice, marks willpower spent
+roll/vs bob=strength+brawl          # contested: resolves both sides
+roll/ext 5 wits+investigation       # extended, target 5 successes, tracks progress
+roll/priv ...                       # visible to roller + staff only
+roll/job 142 ...                    # attaches outcome to job #142 (XP tests, etc.)
```

Resolution: chance die when pool ≤ 0 (success only on 10, dramatic failure on 1); exceptional success at 5+ successes (emits a beat-claim prompt). Every roll goes to the room (unless `/priv`), to the roll log (per-character ring buffer, staff-queryable via `+roll/last bob=10`), and to the web as `panel.wod.roll` with full `DieRecord`s so the client can render dice with reroll animations. `+sheet [target]` renders from schema; `@sheet/set target=field:value` is the staff editor (validated, audited, fires `SheetChanged`).

### 5.5 XP: beats → experiences, spend with approval

- Beat sources: staff award (`+beat bob=reason`), player claim on dramatic failure (`+beat/claim` — logged, revocable), condition resolution, scene end awards. 5 beats auto-convert to 1 experience.
- Costs live in ruleset data (attribute 4xp/dot, skill 2, merit 1, specialty 1, integrity 2, power stat 5 — table configurable per game).
- `+xp` shows balance/history; `+xp/spend merit=Fast Reflexes:2` validates cost and prerequisites, then opens a job in the `wod-xp` queue with the diff attached. Staff `+job/approve` applies the sheet mutation atomically; `+job/deny reason` refunds nothing (nothing was deducted — deduction happens on approval).

### 5.6 ST/staff tools

- `+note bob/backstory=text...` — categorized character notes; player-visible or staff-only; `+note/approve` stamps them (the classic MU* note-approval workflow).
- Job-lite: see §9 (`mush-jobs`, shared infra — WoD registers `wod-chargen` and `wod-xp` queues).
- `+roll/job`, `+sheet` on any target, roll-log queries, `+beat` awards — all lock-gated to a `Staff` power the core lock system provides.

### 5.7 NPC LLM agents and sheets

Two halves, both grounded in the same schema:

```rust
pub struct AgentCapabilityProfile {
    pub summary: String,            // compact prose: "Strong brawler; poor social..."
    pub stats: serde_json::Value,   // trimmed sheet: attributes, notable skills>=2, powers
    pub tags: Vec<SmolStr>,         // "combat:brawl", "social:weak", "vampire:daeva"
    pub tools: Vec<AgentToolSpec>,  // tool manifest for the LLM provider layer
}
```

`Ruleset::agent_profile` renders the NPC's `SheetDoc` into this. The LLM layer injects `summary`+`stats` into the agent's system prompt and exposes the tools; the critical one is `wod_roll(pool_expr, flags)` which the ruleset validates against *that NPC's own sheet* (an NPC can't roll `strength+brawl` at pool 9 when its sheet says 4) and resolves through `mush-dice` with the server RNG. The LLM narrates the returned outcome; it cannot fabricate one, and its narration frame is post-checked against the structured result (success/failure polarity) by the LLM area's validator. Damage the NPC deals/takes goes through the same `HealthTrack` code as players.

---

## 6. TacSim module boundary (design now, build later)

### 6.1 What btmux does (the requirements bar)

Square tactical windows over the atlas's tiles, with terrain classes and elevation per cell (doc 14 §5, D-14-4); unit templates (mech/vehicle/infantry variants with tonnage, movement curves, armor/internal structure by location, weapon inventories with range brackets and ammo); movement modes (walk/run/jump) resolved in real time; heat scale with shutdown/ammo-risk thresholds; weapon attacks with to-hit math (movement of both parties, range, terrain, heat) and hit-location tables; damage transfer (armor → internal → crit rolls → location destruction cascade); per-unit real-time state (facing, speed, heat, target lock) advanced continuously, not per-command.

### 6.2 Core facilities tactics needs that WoD does not

| Need | Reserved extension point |
|---|---|
| Sub-second combat tick | `TickCadence::Fast(Duration)` + `TickCadence::Scene` already in the enum (§3.2). **Cross-area contract:** the event loop must support dynamic timer registration/deregistration at runtime, per scene. |
| Per-battle instances | `SceneId` in core: a scene binds an object set + a ruleset + optional ticker. `HookPoint::SceneStarted/Ended` exists now; WoD uses scenes lightly (roll logs, beat awards), tactics uses them heavily. |
| Per-unit mutable state | `RulesetStateStore`: core-provided keyed blob store `(SceneId, ObjId, RulesetId) -> Value` with in-memory working set + periodic persistence. Units are ordinary objects; their combat state lives here, not in attributes. |
| Map storage | `MapBlob` per scene: ruleset-defined binary/CBOR blob, core provides CRUD + snapshot persistence. Core never interprets it. |
| Web map panel | `panel_feed("tactics.map")` reserved via the existing registrar call. **Cross-area contract with the web protocol:** panel frames support (a) namespaced event types `panel.<feed>.<evt>`, (b) client capability negotiation (unknown feeds ignored gracefully), (c) a delta+keyframe pattern (full map on subscribe, cell/unit deltas after) — the protocol must not assume every event is self-contained prose-adjacent JSON. |
| Command context capture | When piloting, bare input (`heading 120`, `fire 1`) routes to the unit, not the room. Same "input context stack" the parser area builds for mail compose and chargen — tactics pushes a `UnitConsole` context. Reserve it now. |
| Movement veto/override | `HookPoint::MoveAttempt` (veto-capable) exists now — tactics replaces room-exit movement with metric movement over the window inside scenes (doc 14 §3.4). |
| Environment queries | `WeatherQuery` (§10.5) — tactics reads visibility/precipitation for to-hit mods. |

### 6.3 No-breaking-changes checklist

The following must be true in phase 1 even though only WoD ships, and is verified by a **stub `mush-tactics` crate** containing a toy ruleset (one fast ticker, one scene type, one panel feed emitting deltas, one `MoveAttempt` veto) kept compiling and integration-tested from the start:

1. `TickCadence` has `Fast`/`Scene` variants and the scheduler honors dynamic add/remove.
2. `SceneId` exists in core object model; hooks fire.
3. `RulesetStateStore` and `MapBlob` APIs exist (backed naively at first).
4. Web protocol has panel namespacing + capability negotiation + delta frames.
5. Parser has an input context stack.
6. `Ruleset::register` is the *only* wiring point — adding tactics touches config and `Cargo.toml`, zero core code.

Heat curves and hit tables: entirely internal to `mush-tactics`, no reservation needed; movement costs, sight and fire traces are the engine's (doc 14 §5).

---

## 7. Comsys (`mush-comsys`)

### 7.1 Command surface — recommendation

**Recommend the TinyMUX comsys surface**: `addcom pub=Public`, `delcom pub`, `comlist`, `comtitle pub=title`, `clist`, and speaking via the personal alias as a verb (`pub Hello`, `pub :waves`, `pub ;grins`), with `@channel` for administration (`@channel/create`, `/destroy`, `/perm`, `/color`, `/who`, `/history`, `/mute`). Rationale: it's the muscle memory of the stated audience (TinyMUX/Rhost veterans), and since aliases are per-player dynamic verbs we need a reserved early dispatch stage anyway — the parser checks the speaker's alias table before general command matching (cross-area contract). Rejected: a unified `+com <chan>=<msg>` surface — cleaner to implement, alien to the audience; and `@chan`-only speaking — too verbose for the highest-frequency command on any MUSH.

### 7.2 Data model

```rust
pub struct Channel {
    pub id: ChannelId,
    pub name: SmolStr,                 // "Public"
    pub description: String,
    pub perms: ChannelPerms { pub join: Lock, pub speak: Lock, pub see: Lock, pub admin: Lock },
    pub color: ChannelColor { pub ansi: AnsiSpec, pub web_hex: SmolStr },
    pub flags: ChannelFlags,           // LOUD (login announce), NO_TITLES, LOGGED
    pub history: HistoryPolicy { pub persist: bool, pub cap: u32 /* msgs */, pub retention_days: u16 },
}

pub struct ChannelMembership {
    pub player: ObjId, pub channel: ChannelId,
    pub alias: SmolStr,                // per-player: "pub"
    pub title: Option<String>,         // comtitle
    pub gagged: bool,                  // stay joined, suppress delivery
    pub last_read: MsgSeq,             // unread cursor, driven by web client acks
}

pub struct ChanMsg { pub seq: MsgSeq, pub channel: ChannelId, pub speaker: ObjId,
    pub speaker_name: SmolStr, pub title: Option<String>,
    pub kind: SpeechKind /* Say|Pose|SemiPose|Emit */, pub text: String, pub ts: Timestamp }
```

History: per-channel in-memory ring (last `cap` messages) + append-through to the persistence layer when `persist`; `@channel/recall pub=30` in terminal.

### 7.3 Web feed

On connect: `chan.state` (joined channels, colors, unread counts computed from `last_read`). Live: `chan.msg` (full `ChanMsg`), `chan.member` (join/leave for who-lists per tab). Client requests: `chan.history {channel, before_seq, limit}` (backlog paging for tabs), `chan.read {channel, seq}` (advances cursor — also what makes unread badges work), `chan.send {channel, kind, text}` (web speaking path, same handler as the verb). Terminal rendering of `ChanMsg` is one formatter: `[Public] Bob the Builder: hello` with the channel's ANSI color.

---

## 8. @mail (`mush-mail`)

### 8.1 Shape — recommendation

**Flat messages with a `reply_to` chain, threads derived client-side.** Rationale: the terminal experience of MUSH mail is flat and should stay that way (`@mail` lists messages, `@mail 5` reads one); the web panel groups by walking `reply_to` into conversations — no server-side thread entity to keep consistent. Rejected: first-class thread objects — complexity (split/merge/orphan handling) for a feature only the web view wants and can compute.

Named folders (default `inbox`, `sent`, `archive`; user-creatable, cap 16) rather than TinyMUX's numbered 0–15 — same power, self-documenting, maps directly to a web sidebar.

```rust
pub struct MailMsg {
    pub id: MailId, pub from: ObjId, pub to: Vec<ObjId>,   // multi-recipient fan-out
    pub subject: String, pub body: String,
    pub reply_to: Option<MailId>, pub sent_at: Timestamp,
}
pub struct MailboxEntry {   // per-recipient state; body stored once, entries reference it
    pub owner: ObjId, pub msg: MailId, pub folder: SmolStr,
    pub status: MailStatus /* Unread|Read|Urgent */, pub read_at: Option<Timestamp>,
}
```

### 8.2 Commands, quotas, notification

- `@mail` (list current folder), `@mail 5` (read), `@mail/send bob,alice=Subject` → pushes a **compose input context** (multi-line body, `--` to send, `~q` to abort — the shared context stack again); `@mail/quick bob=Subject/one-liner` for one-shots; `@mail/reply 5`, `@mail/fwd 5=carol`, `@mail/file 5=archive`, `@mail/delete 5` (soft-delete to a purge queue; hard purge is a staff/cron action, per the no-permanent-delete posture), `@mail/folder archive`.
- Quotas: default 300 entries/player (config; staff exempt). At quota, sends to that player bounce with a notice. Optional auto-expiry of read mail after N days, off by default.
- Notification: connect-time summary line ("You have 4 unread messages (1 urgent)."); live terminal notice + `mail.new` web event on delivery.

### 8.3 Web feed

`mail.state` (folders + unread counts) on connect; `mail.list {folder, page}`, `mail.read {id}` (returns body, flips status, emits updated counts), `mail.send {to, subject, body, reply_to}` as request/response frames; `mail.new` push. The panel builds conversation view from `reply_to` chains client-side.

---

## 9. Jobs & notes (`mush-jobs`) — shared staff infrastructure

Job-lite: sequential-numbered tickets in named queues with owner, opener, tags, status (`Open|InProgress|Resolved|Denied`), comment trail, and an optional **structured payload** (`serde_json::Value` + a handler id) — this is how `wod-xp` approval works: the payload is the sheet diff, and `+job/approve 142` invokes the registered `JobPayloadHandler` (the WoD crate's applier) atomically. Queues are registered by rulesets (`reg.job_queue(...)`) or core (`REQ` for player requests). Commands: `+job/list [queue]`, `+job 142`, `+job/comment`, `+job/assign`, `+job/approve`, `+job/deny`, `+request title=text` for players. Notes (`+note`, §5.6) live here too since approval/visibility semantics are shared. Web: `jobs.*` panel feed for the staff view. This deliberately stops far short of Anomaly Jobs — no cloning, no mail-in gateways; it's a ticket list with an approval hook.

---

## 10. Weather (`mush-weather`)

### 10.1 Model: zone-scoped Markov state machine, deterministic

```rust
pub struct WxStateDef { pub id: SmolStr /* "clear","overcast","rain","storm","snow","fog" */,
    pub visibility: u8 /* 0-100 */, pub precip: u8, pub wind: u8, pub temp_band: (i16, i16) }

pub struct ClimateDef {   // "temperate-coastal", "desert", "arctic" — data files (RON/TOML)
    pub states: Vec<WxStateDef>,
    pub seasonal: [TransitionMatrix; 4],       // per season: P(state -> state)
    pub tod_modifiers: TodTable,               // fog at dawn more likely, etc.
}

pub struct WeatherCell {   // one per zone that opts in
    pub zone: ZoneId, pub climate: SmolStr,
    pub state: SmolStr, pub since: Timestamp, pub temp: i16,
}
```

A `TickCadence::Coarse(15 min)` handler (config) rolls each cell's transition against the current season/time-of-day matrix using the server RNG. Season and time-of-day come from the core game clock. Staff override: `@weather zone=storm` pins a state (`/release` to resume the machine) — plot control matters more than realism.

### 10.2 Room opt-in

Rooms carry an environment flag: `INDOORS` (default — silence), `OUTDOORS` (full weather), `SHELTERED` (windows/awnings — sees state changes, muffled text, no ambient drip). Set via `@set here=outdoors` style flags; zones without a `WeatherCell` are inert regardless.

### 10.3 Messaging

- **Transition messages**: on state change, one broadcast to all `OUTDOORS`/`SHELTERED` rooms in the zone ("The drizzle thickens into steady rain.").
- **Ambience**: low-probability per-room periodic line (default ~1 per room per 30–60 min, jittered, suppressed in rooms with no players) to avoid the classic weather-spam failure.
- **Look integration**: `HookPoint::LookRendered` appends a current-conditions line to outdoor room descs; web gets a persistent `weather.state {zone, state, temp, icon}` event for a status-bar widget.

### 10.4 LLM flavor within the deterministic machine

States/transitions are never LLM-driven. Flavor text is: for each `(climate, state, season, time_of_day)` bucket the system keeps a pool of lines — seeded with hand-written defaults shipped in the climate files, and **asynchronously topped up** by an LLM batch task (via the provider layer) to ~10 variants per bucket, cached persistently, staff-reviewable (`@weather/lines`). Message selection at tick time is a pure cache read; if generation is unavailable the hand-written lines carry it. Optionally a zone theme prompt (from the worldbuilding area's zone theme record) conditions generation so a gothic city's rain reads differently from a tropical port's.

### 10.5 Query API for rulesets

```rust
pub trait WeatherQuery: Send + Sync {
    fn current(&self, zone: ZoneId) -> Option<WxSnapshot>;  // state, visibility, precip, wind, temp
}
```

Exposed on `WorldCtx` and to Lua as `rs.weather.current(zone)`. WoD uses visibility for perception-roll modifiers via an optional `+roll` auto-modifier; tactics will map `visibility/precip` onto to-hit modifiers. Rulesets read snapshots; they never mutate weather.

---

## 11. Phased implementation

**Phase 1 — skeleton + seams:** `mush-ruleset` traits, `SheetSchema`/`SheetDoc`, registrar, `mush-dice` (grammar + generic eval, seeded-RNG golden tests), stub `mush-tactics` seam test (§6.3), `mush-jobs` core. *Exit: toy ruleset registers a command and a fast ticker; sheets round-trip with validation and migration.*

**Phase 2 — comsys + mail:** full §7/§8 including web feeds and unread cursors; input context stack lands (parser area) for mail compose. *Exit: two clients chat on a colored channel with web tabs + backlog; mail with folders/quotas/notifications.*

**Phase 3 — WoD playable core:** CofD schema, `+sheet`, `@sheet/set`, `CofdPool` with all flags, `+roll` full syntax incl. contested/extended/priv/job, damage tracks, roll log, `panel.wod.roll` + sheet panel. *Exit: a scene can be played and ST'd end to end.*

**Phase 4 — WoD progression + staff:** chargen flow + approval queue, beats/XP, `+xp/spend` → job approval applying diffs, `+note`, supernatural template slot with one template (vampire) proving the template sub-module machinery.

**Phase 5 — weather:** cells, climate data files, tick transitions, room flags, look/status integration, hand-written line pools; LLM top-up last.

**Phase 6 — NPC grounding:** `agent_profile`, `wod_roll` agent tool, narration/outcome consistency check with the LLM area.

**Phase 7+ — tactics:** grow the stub crate: square windows over the atlas, unit templates, scene tickers, map panel deltas — touching only reserved seams (that's the acceptance test of this whole design).

## 12. Risks

- **Schema expressiveness creep:** CofD conditions/tilts and per-template powers will pressure `FieldType` toward a type system. Mitigation: `List<Text>`/`Map` escape hatches plus ruleset-side interpretation; resist adding computed logic beyond `DerivedExpr`.
- **The tactics seams are speculative:** reserved APIs nobody exercises rot. Mitigation is structural — the stub seam-test crate runs in CI from phase 1.
- **Cross-area contract density:** this area leans on the parser's input context stack, the protocol's panel namespacing/deltas, and the event loop's dynamic timers. Each is called out above; they need to be in those areas' phase-1 scope or phases 2/7 here slip.
- **Weather/comsys spam and LLM flavor quality:** both are tuning problems; defaults above (jittered ambience, empty-room suppression, staff-reviewable line cache) are the guardrails.

## 13. Decisions considered and rejected (summary)

1. **Dynamic ruleset loading (`libloading`/WASM)** — rejected for ABI instability and serialization-boundary cost; compiled-in trait objects with a WASM-projectable boundary reserved (§3.1).
2. **`caith` for dice** — maintained but string-first and can't express CofD/oWoD mechanics; own the ~300-line winnow grammar for structured, auditable per-die results (§4).
3. **Server-side mail threads** — rejected for flat + `reply_to`, client-derived conversations (§8.1). Likewise **`+com`-style unified channel speaking** rejected in favor of TinyMUX alias-verbs (§7.1), and **per-room weather** rejected for zone cells (per-room is state explosion with no gameplay payoff; rooms differentiate via flags).

Sources: [caith on crates.io](https://crates.io/crates/caith), [schemars versions](https://crates.io/crates/schemars/versions), [winnow on crates.io](https://crates.io/crates/winnow)