//! Command specs, the registry, and the dispatch pipeline.
//!
//! Dispatch order is **exactly** treaty ruling 5, with the input-context
//! stack in front:
//!
//! 0. the actor's input-context stack (mail compose, chargen, consoles) —
//!    checked before everything else, with a hard `~q` escape
//! 1. single-char prefixes — `"` say, `:` pose, `;` semipose, `\` @emit
//! 2. the command gates — registered [`CommandGate`]s (the onboarding gate).
//!    Consulted **before** the alias stage so a gated actor cannot broadcast
//!    on a channel through a per-player alias, and before exits/commands so
//!    they cannot move onto the live grid. Skipped entirely when no gate is
//!    registered (byte-identical default).
//! 3. the alias stage — registered [`AliasStage`] hooks: the kernel's
//!    macro stage first ([`MacroStage`](crate::commands::macros::MacroStage),
//!    16b cut AD.1), then the modules' stages (comsys's channel aliases ride
//!    it). A rewrite is gated at once and offered to the LATER stages only.
//!    **Prefix-expanded lines skip this stage**: the prefix
//!    character already settled the line's meaning, so a channel alias
//!    named `say`/`pose` can never capture `"hello`
//! 4. the `home` keyword
//! 5. exit match — room exits, then the room's zone's exits, then global
//!    exits on Limbo (the master room)
//! 6. the command table — longest-unambiguous-prefix for bare verbs,
//!    exact (case-insensitive) for `@`/`+` commands; `/switch` and arg
//!    shapes are table-driven from the [`CommandSpec`]
//! 7. `Huh?` — with a fuzzy (edit-distance) suggestion
//!
//! (Treaty ruling 5 fixes the relative order of aliases → exits → command
//! table → `Huh?`; the input-context stack and the command-gate seam are
//! additions in front of it.)
//!
//! Handlers are `Arc<dyn CommandHandler>` trait objects (treaty ruling 4)
//! so modules can capture state; plain `fn` items work via the blanket impl.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use smol_str::SmolStr;

use mush_proto::OutputEvent;

use crate::behavior::{
    BehaviorId, BehaviorRegError, BehaviorResolution, BehaviorSpec, DestroyGate, DestroyPolicy,
    DetachGate, resolve_behavior_command,
};
use crate::commands::movement;
use crate::flags::{Powers, Privileges, Role};
use crate::lock::{LockExpr, MAX_LOCK_DEPTH, eval_lock};
use crate::matchit::MatchErr;
use crate::module::{ContextOutcome, ModuleTables, restored_prompt};
use crate::obj::ObjId;
use crate::output::{Outbox, display_name};
use crate::perm;
use crate::world::{World, WorldError};

/// How a command's argument text is split, table-driven from the spec
/// (design doc §3.2) — handlers never do their own `=`/`,` surgery.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ArgShape {
    /// No arguments (anything given is ignored).
    None,
    /// The raw argument text, verbatim (trimmed).
    Raw,
    /// `<lhs> = <rhs>` (first `=`; both sides trimmed).
    EqPair,
    /// `<lhs> = <a>, <b>, ...` — rhs additionally split on `,`.
    EqPairList,
    /// `<lhs> = <a>; <b>; ...` — rhs additionally split on `;` (aliases).
    EqPairSemi,
}

/// Description of one command: name, aliases, switches, argument shape,
/// gating (role, power, lock), and real help text.
///
/// All string fields are **owned** so commands can be registered from data
/// as well as from source — Rhai `on_command` surfaces, help text loaded
/// from files, `@command/override` tooling — without `Box::leak` tricks.
/// Compiled-in call sites stay literal-friendly via `.into()`.
///
/// How a command relates to a turn-based scene (a timestop). Declared per
/// command so the scene system can enforce turn order **generally** — any
/// ruleset marks its mechanical commands [`Turn`](SceneClass::Turn) and the
/// scene gate does the rest, with no ruleset↔scene coupling.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SceneClass {
    /// Always allowed, turn or not (speech, poses, OOC, inspection, safety) —
    /// the default.
    #[default]
    Free,
    /// A mechanical action (`+roll`, an attack) that, inside a **turn-based**
    /// scene, only the participant whose turn it is may run. Outside a scene it
    /// behaves normally.
    Turn,
}

/// Maximum persistence authority a registered command is allowed to use.
///
/// This is compatibility metadata, not an authorization check.  It makes the
/// remaining pre-K8 mutation lanes visible in the frozen registry so modules,
/// tooling, and CI do not have to infer them from handler source.  A module
/// composed through [`Engine::compose_modules`](crate::Engine::compose_modules)
/// must choose a class before registering any commands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommandCompatibility {
    /// Inspection or presentation only; no durable mutation is part of the
    /// command's contract.
    ReadOnly,
    /// A bounded, validated whole-document post-image on the compatibility
    /// writer lane.  This is migration debt and cannot claim a K-owned typed
    /// family, lease, contact, window, reservation, or resolution.
    LegacyPostImage,
    /// An ingress adapter for the authoritative action coordinator.  The
    /// handler itself does not implement or persist the mechanic.
    Authoritative,
    /// The narrow economy exception: validated domain records may remain on
    /// the legacy post-image lane until their owner migration lands.
    EconomyCarveOut,
}

/// `CommandSpec` implements [`Default`]; registration sites are expected to
/// use struct-update syntax (`..CommandSpec::default()`) so that adding new
/// spec fields later is a non-breaking change (the module API freezes in
/// M3 — plan risk #5).
#[derive(Clone, Debug)]
pub struct CommandSpec {
    /// Primary verb: `"look"`, `"@dig"`, `"+help"`.
    pub name: SmolStr,
    /// Alternate verbs resolving to the same command (exact match only).
    pub aliases: Vec<SmolStr>,
    /// Valid `/switches`; anything else is rejected before the handler runs.
    pub switches: Vec<SmolStr>,
    /// Argument splitting rule.
    pub shape: ArgShape,
    /// Minimum staff role required.
    pub min_role: Role,
    /// Additional power requirement (Wizard+ implicitly passes).
    pub power: Option<Powers>,
    /// Explicit-privilege requirement: **no role passes it** — the actor's
    /// object must carry the bit (`perm::has_privilege`, D-D-14). Checked
    /// after `power`, refused with the same "Permission denied.".
    pub privilege: Option<Privileges>,
    /// Optional lock gate (treaty ruling 4), evaluated against the actor
    /// before the handler runs; failing it is "Permission denied.".
    pub lock: Option<LockExpr>,
    /// One-paragraph usage help, rendered by `help <cmd>`.
    pub help: String,
    /// Turn-based-scene relationship (default [`SceneClass::Free`]).
    pub scene_class: SceneClass,
}

impl Default for CommandSpec {
    /// An empty spec: no name, no aliases/switches, [`ArgShape::Raw`],
    /// usable by any [`Role::Player`], no power or lock gate, empty help.
    fn default() -> CommandSpec {
        CommandSpec {
            name: SmolStr::default(),
            aliases: Vec::new(),
            switches: Vec::new(),
            shape: ArgShape::Raw,
            min_role: Role::Player,
            power: None,
            privilege: None,
            lock: None,
            help: String::new(),
            scene_class: SceneClass::Free,
        }
    }
}

/// A command failed; the message is shown to the actor as an error line.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CmdError {
    /// A user-facing message, MUSH style ("Permission denied.").
    #[error("{0}")]
    Msg(String),
    /// A world mutation was rejected.
    #[error("{0}")]
    World(WorldError),
    /// Target matching failed.
    #[error("{0}")]
    Match(MatchErr),
}

impl CmdError {
    /// Shorthand for a user-facing message error.
    pub fn msg(m: impl Into<String>) -> CmdError {
        CmdError::Msg(m.into())
    }
}

impl From<WorldError> for CmdError {
    fn from(e: WorldError) -> Self {
        CmdError::World(e)
    }
}

impl From<MatchErr> for CmdError {
    fn from(e: MatchErr) -> Self {
        CmdError::Match(e)
    }
}

/// Parsed argument text, split according to the command's [`ArgShape`].
#[derive(Clone, Debug, Default)]
pub struct ParsedArgs {
    raw: String,
    lhs: Option<String>,
    rhs: Option<String>,
    rhs_list: Vec<String>,
}

impl ParsedArgs {
    /// Split `rest` according to `shape`.
    #[must_use]
    pub fn parse(shape: ArgShape, rest: &str) -> ParsedArgs {
        let raw = rest.trim().to_string();
        match shape {
            ArgShape::None | ArgShape::Raw => ParsedArgs {
                raw,
                ..ParsedArgs::default()
            },
            ArgShape::EqPair | ArgShape::EqPairList | ArgShape::EqPairSemi => {
                let (lhs, rhs) = match raw.split_once('=') {
                    Some((l, r)) => (l.trim().to_string(), Some(r.trim().to_string())),
                    None => (raw.clone(), None),
                };
                let sep = match shape {
                    ArgShape::EqPairList => Some(','),
                    ArgShape::EqPairSemi => Some(';'),
                    _ => None,
                };
                let rhs_list = match (&rhs, sep) {
                    (Some(r), Some(sep)) => r
                        .split(sep)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                    _ => Vec::new(),
                };
                ParsedArgs {
                    raw,
                    lhs: Some(lhs),
                    rhs,
                    rhs_list,
                }
            }
        }
    }

    /// The whole argument text, trimmed.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The left of `=` (or the whole text when there was no `=`); `""` for
    /// shapes without an lhs.
    #[must_use]
    pub fn lhs(&self) -> &str {
        self.lhs.as_deref().unwrap_or("")
    }

    /// The right of `=`, or `None` when no `=` was given. `Some("")` means
    /// an explicit empty rhs (`@desc thing=`).
    #[must_use]
    pub fn rhs(&self) -> Option<&str> {
        self.rhs.as_deref()
    }

    /// The rhs split on the shape's separator, trimmed, empties dropped.
    #[must_use]
    pub fn rhs_list(&self) -> &[String] {
        &self.rhs_list
    }
}

/// The set of `/switches` given with the verb (lowercased).
#[derive(Clone, Debug, Default)]
pub struct SwitchSet(Vec<SmolStr>);

impl SwitchSet {
    /// Build from raw switch names (stored lowercased).
    #[must_use]
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> SwitchSet {
        SwitchSet(
            names
                .into_iter()
                .map(|s| SmolStr::from(s.to_lowercase()))
                .collect(),
        )
    }

    /// Whether the switch was given (case-insensitive).
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|s| s.eq_ignore_ascii_case(name))
    }

    /// Iterate the given switches (lowercased) in order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(SmolStr::as_str)
    }

    /// Whether no switches were given.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Everything a command handler gets to work with. Hooks, ticks,
/// observers, and module boot receive the same type
/// ([`ModuleCtx`](crate::module::ModuleCtx) is an alias) — one context,
/// everywhere (treaty ruling 4).
pub struct Ctx<'a> {
    /// The world (single-writer: the handler owns it for its duration).
    pub w: &'a mut World,
    /// The acting object.
    pub actor: ObjId,
    /// The `/switches` given.
    pub switches: SwitchSet,
    /// The parsed arguments.
    pub args: ParsedArgs,
    /// Where output events are queued.
    pub out: &'a mut Outbox,
    /// The command registry (for `help` and introspection).
    pub reg: &'a Registry,
    /// The module this handler was registered by (`None` for core
    /// commands) — the identity attr-namespace enforcement checks.
    pub module: Option<SmolStr>,
    /// Owner-scoped, read-only access to engine-owned typed families. Engine
    /// dispatch installs a live reader; manually constructed compatibility
    /// contexts may use [`ModuleActionState::unavailable`](crate::ModuleActionState::unavailable).
    pub typed_state: crate::action_state::ModuleActionState<'a>,
}

impl Ctx<'_> {
    /// Whether `actor` already has an authoritative submission in flight —
    /// queued in the engine by an earlier drain, or deferred in this outbox
    /// (doc 18 cut IC8; [`Outbox::authoritative_pending`]). A tick acting as
    /// a principal asks before it submits (noted by `Engine::fire_due`), and
    /// since doc 18 cut IC10 so may a command that stages one (noted by
    /// `Engine::step` for every inbound message), so it refuses before it
    /// offers anything rather than after.
    #[must_use]
    pub fn authoritative_pending(&self, actor: ObjId) -> bool {
        self.out.authoritative_pending(actor)
    }
}

/// A command implementation. Registered as `Arc<dyn CommandHandler>`
/// (treaty ruling 4) so modules can capture state; the blanket impl lets
/// plain functions and closures register directly.
pub trait CommandHandler: Send + Sync {
    /// Run the command. `Err` is rendered to the actor as an error line.
    fn call(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError>;
}

impl<F> CommandHandler for F
where
    F: Fn(&mut Ctx<'_>) -> Result<(), CmdError> + Send + Sync,
{
    fn call(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        self(ctx)
    }
}

/// What an [`AliasStage`] did with a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasOutcome {
    /// Not this stage's business; dispatch continues unchanged.
    NotMine,
    /// Fully handled (output already queued); dispatch stops.
    Handled,
    /// Rewrite the line and continue dispatch with the result: the command
    /// gates see the substitute at once, and it is offered to the LATER
    /// stages only (never back to the stage that produced it, nor to any
    /// earlier one) before dispatch goes on at the `home`-keyword step.
    Rewrite(String),
}

/// A pre-dispatch hook running after single-char prefix expansion and
/// before exit matching. The kernel registers the first one, the macro
/// stage ([`MacroStage`](crate::commands::macros::MacroStage), 16b cut
/// AD.1); the modules' stages follow in composition order, comsys's
/// per-player channel aliases (`pub Hello`) among them.
///
/// **Multi-stage contract** (load-bearing once several stages register):
///
/// - Stages run in **registration order**; registration order is priority
///   order.
/// - The first stage returning [`AliasOutcome::Handled`] ends dispatch.
/// - A stage returning [`AliasOutcome::Rewrite`] replaces the line; the
///   command gates see the substitute AT ONCE (a refusal ends dispatch),
///   and the loop goes on with the NEXT stage on the new line. A stage
///   never sees a line twice and never sees its own output, so a chain of
///   rewrites is bounded by the stage count and no stage can expand what it
///   produced (the macro stage, being first, expands exactly once). With
///   the loop done, dispatch continues at the `home`-keyword step.
pub trait AliasStage: Send + Sync {
    /// Inspect (and possibly consume or rewrite) an input line.
    fn try_alias(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> AliasOutcome;

    /// Whether this stage would claim a line whose first word is `word`, typed by
    /// `actor` — the question `@macro` asks before it takes a name. Default: no.
    ///
    /// **Observer contract** (doc 16 §5): a `true` is shown to `actor` as a
    /// refusal, so the answer may depend only on the actor's own settings (their
    /// channel aliases, their own macros) or on static vocabulary — never on
    /// world state the actor has not perceived (a unit's callsign, an object in
    /// another room), or the refusal becomes an oracle.
    fn claims_word(&self, _w: &World, _actor: ObjId, _word: &str) -> bool {
        false
    }
}

/// A **pre-dispatch command gate**, consulted after single-char prefix
/// expansion and **before** the alias stage, the `home` keyword, exit
/// matching, and the command table. Every registered gate sees the
/// (expanded) line's leading verb **and** the whole expanded line; the first
/// to return `Err(reason)` refuses the line — dispatch shows the reason to
/// the actor and stops.
///
/// This is the seam the **onboarding gate** hangs on: while
/// `chargen_required` is set, a freshly-created, not-yet-approved character
/// is held to a restricted command set (chargen, look, speech, help) and
/// every other verb is refused. Running **before the alias stage** is
/// load-bearing: a per-player comsys alias (`pub Hello`) would otherwise be
/// fully handled by the alias stage and never reach the gate, letting a
/// gated character broadcast game-wide. Running before exit matching lets
/// the gate refuse movement even when an exit's name/alias collides with an
/// allowlisted verb — which is why the gate is handed the whole `line` (to
/// resolve it against [`find_exit`](crate::commands::movement::find_exit)),
/// not just the leading verb. Prefix-expanded speech (`"`/`:`/`;`/`\`)
/// reaches the gate as its expanded verb (`say`/`pose`/…), so a gate can
/// wave room roleplay through while blocking movement, building, and channel
/// broadcast. An actor inside an input context never reaches the gate (the
/// stack is checked first), so a pushed chargen wizard is unaffected.
///
/// Gates are consulted in registration order and are **read-only over the
/// world** — a gate decides, it never mutates. With no gate registered
/// (the default), dispatch is byte-identical to before this seam existed.
pub trait CommandGate: Send + Sync {
    /// Decide whether `actor` may run the command whose leading verb is
    /// `verb` (already prefix-expanded; switches stripped; original case).
    /// `line` is the whole expanded, trimmed command line — the same query
    /// dispatch will hand to exit matching — so a gate can refuse a line that
    /// resolves to an exit. `reg` is the live registry, so a gate can resolve
    /// the verb to its [`CommandSpec`] (e.g. to read [`SceneClass`]) — read-only.
    /// Return `Err(reason)` to refuse the whole line with `reason` shown to the
    /// actor.
    fn check(
        &self,
        w: &World,
        reg: &Registry,
        actor: ObjId,
        verb: &str,
        line: &str,
    ) -> Result<(), String>;
}

/// A **post-command-table** fallback matcher (design doc §3.1 "module
/// fallback matchers"), consulted only when the command table produced
/// [`LookupErr::NotFound`], **after** exits and built-in commands and
/// **before** `Huh?`.
///
/// This is the seam mush-script hangs object `$`-commands
/// (`script:command:*`) on: a fallback that never shadows a built-in verb
/// or an exit (classic dollar-command semantics — built-ins win). Fallbacks
/// run in registration order; the first to return `true` handled the line
/// and dispatch stops, otherwise the next fallback (then `Huh?`) runs. The
/// signature mirrors [`AliasStage`] deliberately — `&mut World` + `&mut
/// Outbox`, no `Ctx`.
pub trait CommandFallback: Send + Sync {
    /// Try to handle `line` (the whole post-alias command line) as `actor`.
    /// Return `true` if handled (dispatch stops before `Huh?`).
    fn try_command(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> bool;
}

/// Registration failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The name or one of its aliases is already registered.
    #[error("command name {0:?} is already registered")]
    Collision(String),
}

/// Command lookup failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupErr {
    /// No registered command matches the verb.
    NotFound,
    /// The verb is a prefix of several commands (their names listed).
    Ambiguous(Vec<SmolStr>),
}

struct Entry {
    spec: CommandSpec,
    handler: Arc<dyn CommandHandler>,
    /// The registering module (`None` for core commands).
    module: Option<SmolStr>,
    /// Declared maximum persistence authority for this command.
    compatibility: CommandCompatibility,
    /// Resolved only typed in full (case-folded): the prefix scan in
    /// `lookup_idx` skips it. Set by [`Registry::register_exact`] alone
    /// (16b cut AP.4).
    exact: bool,
}

struct AliasStageEntry {
    stage: Arc<dyn AliasStage>,
    module: Option<SmolStr>,
    compatibility: CommandCompatibility,
}

struct CommandFallbackEntry {
    fallback: Arc<dyn CommandFallback>,
    module: Option<SmolStr>,
    compatibility: CommandCompatibility,
}

/// One prose help topic (loaded at boot from `docs/help/*.md` by the
/// driver's I/O loader; this registry itself stays sync and file-free).
/// Served by `help <topic>` — an exact command-name match always wins over
/// a topic of the same name.
#[derive(Clone, Debug)]
pub struct HelpTopic {
    /// Canonical topic key (lowercased file stem, e.g. `"getting-started"`).
    pub name: SmolStr,
    /// Human-readable title (the file's leading `# heading`, or the name).
    pub title: String,
    /// Full topic text, served verbatim.
    pub body: String,
}

/// The command table, alias-stage hooks, and (treaty ruling 4 — one
/// registry, ever) the module registration tables
/// ([`ModuleTables`]; accessors live in [`crate::module`]).
#[derive(Default)]
pub struct Registry {
    entries: Vec<Entry>,
    /// Lowercased name and alias → entry index.
    index: HashMap<String, usize>,
    alias_stages: Vec<AliasStageEntry>,
    /// Pre-dispatch command gates (the onboarding gate); consulted before the
    /// alias stage and `home`/exits/commands, in registration order.
    command_gates: Vec<Arc<dyn CommandGate>>,
    /// Post-command-table fallback matchers (object `$`-commands); consulted
    /// on [`LookupErr::NotFound`] before `Huh?`, in registration order.
    command_fallbacks: Vec<CommandFallbackEntry>,
    /// Module registrations (hooks, ticks, observers, namespaces, …).
    pub(crate) mods: ModuleTables,
    /// Prose help topics by lowercased name (sorted for listings).
    help_topics: BTreeMap<SmolStr, HelpTopic>,
    /// Registered behavior types by id (design §4.2); exactly one module owns
    /// each id.
    behaviors: HashMap<BehaviorId, BehaviorSpec>,
    /// Pre-destroy veto gates (design D8), consulted by [`Registry::check_destroy`]
    /// at the command layer.
    destroy_gates: Vec<Arc<dyn DestroyGate>>,
    /// Pre-detach veto gates, consulted by `@behavior/remove` via
    /// [`Registry::check_detach`].
    detach_gates: Vec<Arc<dyn DetachGate>>,
    /// K1 definition composition is mutable only while modules register.
    pub(crate) definition_builder: crate::mechanics::DefinitionRegistryBuilder,
    /// K1 cross-owner bridge declarations, frozen after definitions.
    pub(crate) bridge_builder: crate::mechanics::BridgeRegistryBuilder,
    /// K1 pure, exact-version migration declarations.
    pub(crate) definition_migration_builder: crate::mechanics::DefinitionMigrationRegistryBuilder,
    /// K2 action providers, mutable only while modules register.
    pub(crate) action_provider_builder: crate::action::ActionProviderRegistryBuilder,
    /// K2 revision validators, mutable only while modules register.
    pub(crate) revision_validator_builder: crate::action::RevisionValidatorRegistryBuilder,
    /// K2 authority validators, mutable only while modules register.
    pub(crate) authority_guard_validator_builder:
        crate::action::AuthorityGuardValidatorRegistryBuilder,
    /// K2 engine-owned typed state families, mutable only while modules register.
    pub(crate) state_family_builder: crate::action_state::StateFamilyRegistryBuilder,
    /// K4 module-owned typed clock providers, mutable only while modules register.
    pub(crate) clock_provider_builder: crate::clock_provider::ClockProviderRegistryBuilder,
    /// K4 read adapters over ruleset-owned resource/effect truth.
    pub(crate) resource_effect_provider_builder:
        crate::resource_provider::ResourceEffectProviderRegistryBuilder,
    /// K4 declarative durable schedule sources, mutable only while modules register.
    pub(crate) schedule_source_builder: crate::schedule_source::ScheduleSourceRegistryBuilder,
    /// K7 declarative delayed-decision sources, mutable only while modules register.
    pub(crate) decision_source_builder: crate::decision_source::DecisionSourceRegistryBuilder,
    /// Process-lifetime, one-way readiness for the bounded composed owner roster.
    pub(crate) mechanics_owner_readiness: Option<crate::mechanics::MechanicalOwnerReadiness>,
    /// Frozen before any persisted rules-bearing record is loaded.
    pub(crate) definitions: Option<crate::mechanics::DefinitionRegistry>,
    /// Frozen directional bridge lookup.
    pub(crate) bridges: Option<crate::mechanics::BridgeRegistry>,
    /// Frozen dry-run-only definition migrations.
    pub(crate) definition_migrations: Option<crate::mechanics::DefinitionMigrationRegistry>,
    /// Frozen exact-action provider lookup.
    pub(crate) action_providers: Option<crate::action::ActionProviderRegistry>,
    /// Frozen exact revision-validator lookup.
    pub(crate) revision_validators: Option<crate::action::RevisionValidatorRegistry>,
    /// Frozen exact authority-validator lookup.
    pub(crate) authority_guard_validators: Option<crate::action::AuthorityGuardValidatorRegistry>,
    /// Frozen owner-stamped state-family registry.
    pub(crate) state_families: Option<crate::action_state::StateFamilyRegistry>,
    /// Frozen owner-stamped typed clock-provider registry.
    pub(crate) clock_providers: Option<crate::clock_provider::ClockProviderRegistry>,
    /// Frozen owner-stamped resource/effect read adapters.
    pub(crate) resource_effect_providers:
        Option<crate::resource_provider::ResourceEffectProviderRegistry>,
    /// Frozen owner-stamped durable schedule-source registry.
    pub(crate) schedule_sources: Option<crate::schedule_source::ScheduleSourceRegistry>,
    /// Frozen owner-stamped delayed-decision source registry.
    pub(crate) decision_sources: Option<crate::decision_source::DecisionSourceRegistry>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Register a command. Name and alias collisions are errors — there is
    /// no silent override (staff-level `@command/override` comes later).
    pub fn register(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), RegistryError> {
        // Bare registry construction is host/core compatibility code.  It is
        // deliberately classified conservatively; composed modules must use
        // `Registrar` and make an explicit declaration.
        self.register_tagged(spec, handler, None, CommandCompatibility::LegacyPostImage)
    }

    /// Register a core command that resolves only when its name is typed in
    /// full (case-folded), never as a prefix — otherwise as
    /// [`register`](Self::register) does (core, no module), with one
    /// difference: it publishes [`CommandCompatibility::ReadOnly`].
    ///
    /// PennMUSH's reason (16b cut AP.4, review `AP4-5`): a socket-level
    /// setting such as `OUTPUTPREFIX` is never abbreviated. Prefix-matched,
    /// it would take `o` from `ORDER` and `out` from an exit; exact-only, it
    /// claims nothing but its own name. The exactness lives here on the
    /// registry's entry, not on [`CommandSpec`], so `help`, the census and
    /// every gate still read it as an ordinary command.
    ///
    /// The class is `ReadOnly` because a setting of a connection makes no
    /// durable mutation (its row is transient World state), and the legacy
    /// handler allowlist cross-checks a stanza's `read-only` category against
    /// the class the composition publishes; `LegacyPostImage` would claim a
    /// post-image lane these verbs never ride.
    pub fn register_exact(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), RegistryError> {
        self.insert_entry(spec, handler, None, CommandCompatibility::ReadOnly, true)
    }

    /// Register a command tagged with the registering module's identity
    /// (the [`Registrar`](crate::module::Registrar) path).
    pub(crate) fn register_tagged(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
        module: Option<SmolStr>,
        compatibility: CommandCompatibility,
    ) -> Result<(), RegistryError> {
        self.insert_entry(spec, handler, module, compatibility, false)
    }

    /// The one insertion path behind [`register_tagged`](Self::register_tagged)
    /// and [`register_exact`](Self::register_exact): collision-checked over
    /// the name and every alias.
    fn insert_entry(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
        module: Option<SmolStr>,
        compatibility: CommandCompatibility,
        exact: bool,
    ) -> Result<(), RegistryError> {
        let mut keys = vec![spec.name.to_lowercase()];
        keys.extend(spec.aliases.iter().map(|a| a.to_lowercase()));
        for k in &keys {
            if self.index.contains_key(k) {
                return Err(RegistryError::Collision(k.clone()));
            }
        }
        let idx = self.entries.len();
        self.entries.push(Entry {
            spec,
            handler,
            module,
            compatibility,
            exact,
        });
        for k in keys {
            self.index.insert(k, idx);
        }
        Ok(())
    }

    /// Append an alias-stage hook (runs in registration order).
    pub fn add_alias_stage(&mut self, stage: Arc<dyn AliasStage>) {
        self.add_tagged_alias_stage(stage, None, CommandCompatibility::LegacyPostImage);
    }

    pub(crate) fn add_tagged_alias_stage(
        &mut self,
        stage: Arc<dyn AliasStage>,
        module: Option<SmolStr>,
        compatibility: CommandCompatibility,
    ) {
        self.alias_stages.push(AliasStageEntry {
            stage,
            module,
            compatibility,
        });
    }

    /// Append a pre-dispatch [`CommandGate`] (the onboarding gate). Runs in
    /// registration order, before the alias stage and
    /// `home`/exits/commands.
    pub fn add_command_gate(&mut self, gate: Arc<dyn CommandGate>) {
        self.command_gates.push(gate);
    }

    /// Number of registered command gates (tests/inspection).
    #[must_use]
    pub fn command_gate_count(&self) -> usize {
        self.command_gates.len()
    }

    /// Register a behavior type (design §4.2) with its contextual commands
    /// (§4.3). Exactly one module owns an id. Each command's name and aliases
    /// must be unique within the spec and must not resolve against the
    /// existing global table **the way dispatch resolves a verb** — an exact
    /// name/alias hit, or, for a bare verb, a prefix of one or more bare global
    /// names. Any of those means the global stage would claim the verb first
    /// (or refuse it as ambiguous) and the behavior verb would be dead; failing
    /// here makes that loud at boot. (The reverse order — a global verb
    /// registered *after* a behavior verb — shadows it silently: global wins by
    /// design.)
    pub fn register_behavior(&mut self, spec: BehaviorSpec) -> Result<(), BehaviorRegError> {
        if let Some(existing) = self.behaviors.get(&spec.id) {
            return Err(BehaviorRegError::Owned {
                id: spec.id.to_string(),
                owner: existing.owner.clone(),
            });
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cmd in &spec.commands {
            // A channelled command MUST be Operated-exclusive, or a non-Operated
            // reach (Container/Carried/Ambient) would resolve it with the channel
            // gate never firing — an authority bypass.
            if cmd.required_channel.is_some()
                && cmd.reach.as_slice() != [crate::behavior::CommandReach::Operated]
            {
                return Err(BehaviorRegError::ChannelledNotOperated {
                    id: spec.id.to_string(),
                    verb: cmd.spec.name.to_lowercase(),
                });
            }
            let mut verbs: Vec<String> = vec![cmd.spec.name.to_lowercase()];
            verbs.extend(cmd.spec.aliases.iter().map(|a| a.to_lowercase()));
            for v in verbs {
                if v.is_empty() {
                    return Err(BehaviorRegError::EmptyVerb {
                        id: spec.id.to_string(),
                    });
                }
                // Resolve exactly as dispatch will: exact hit, or (bare verbs)
                // a unique or ambiguous prefix of bare global names.
                match self.lookup_idx(&v) {
                    Err(LookupErr::NotFound) => {}
                    Ok(idx) => {
                        let owner = self.entries[idx]
                            .module
                            .clone()
                            .unwrap_or_else(|| SmolStr::new_static("core"));
                        return Err(BehaviorRegError::VerbCollision {
                            id: spec.id.to_string(),
                            verb: v,
                            owner,
                        });
                    }
                    Err(LookupErr::Ambiguous(names)) => {
                        let names: Vec<&str> = names.iter().map(SmolStr::as_str).collect();
                        return Err(BehaviorRegError::VerbCollision {
                            id: spec.id.to_string(),
                            verb: v,
                            owner: SmolStr::new(format!(
                                "(an ambiguous prefix of {})",
                                names.join(", ")
                            )),
                        });
                    }
                }
                if !seen.insert(v.clone()) {
                    return Err(BehaviorRegError::DuplicateVerb {
                        id: spec.id.to_string(),
                        verb: v,
                    });
                }
            }
        }
        self.behaviors.insert(spec.id.clone(), spec);
        Ok(())
    }

    /// The registration for a behavior id, if one exists.
    #[must_use]
    pub fn behavior_spec(&self, id: &BehaviorId) -> Option<&BehaviorSpec> {
        self.behaviors.get(id)
    }

    /// Every control channel any registered behavior command requires — the set
    /// an authoring lint checks a station's granted channels against.
    #[must_use]
    pub fn required_channels(&self) -> std::collections::BTreeSet<crate::behavior::ChannelId> {
        self.behaviors
            .values()
            .flat_map(|spec| {
                spec.commands
                    .iter()
                    .filter_map(|c| c.required_channel.clone())
            })
            .collect()
    }

    /// Refuse a global command that would capture a verb some behavior
    /// declares (fu3-4). Run **once, after every module has registered** and
    /// before the composition freezes; the diagnostic names the verb, the
    /// behavior that declared it and the global that eats it.
    ///
    /// [`register_behavior`](Registry::register_behavior) already refuses the
    /// forward order — a behavior verb declared against a global that exists
    /// *already* — and stays as the better-localised diagnostic. It cannot see
    /// the reverse: a global registered by a LATER-composed module against a
    /// behavior declared by an earlier one registers cleanly, and the behavior
    /// verb is then permanently dead at dispatch with nothing logged, because
    /// the command table is consulted first and the behavior stage is reached
    /// only on [`LookupErr::NotFound`]. This pass is the complete one.
    ///
    /// It deliberately does **not** live in
    /// [`register_tagged`](Registry::register_tagged): checking there would
    /// make intra-module registration ORDER load-bearing — one module calling
    /// `reg.behavior(spec)` before its own `reg.command(..)` would be refused
    /// while the same module with the two calls swapped composed and the verb
    /// still died. Here the answer is the same whichever order the roster
    /// composes in.
    ///
    /// Resolution is [`lookup_idx`](Registry::lookup_idx) — the same resolver
    /// dispatch uses — so the exact-hit, unique-prefix and
    /// [`Ambiguous`](LookupErr::Ambiguous) cases all fall out of it rather than
    /// being re-derived (and no `behavior_verbs` side map would answer a prefix
    /// query anyway). A refusal is a hard boot error, not a warning: a verb a
    /// player can never reach is worse than a boot that says why, and no
    /// shipped `BehaviorSpec` declares commands today, so the blast radius is
    /// empty until the control-channel commands land.
    pub(crate) fn check_behavior_verb_shadowing(&self) -> Result<(), String> {
        // `behaviors` is a HashMap; sort so a roster with two shadowed verbs
        // reports the same one every boot.
        let mut specs: Vec<&BehaviorSpec> = self.behaviors.values().collect();
        specs.sort_by_key(|spec| spec.id.to_string());
        for spec in specs {
            for cmd in &spec.commands {
                let mut verbs: Vec<String> = vec![cmd.spec.name.to_lowercase()];
                verbs.extend(cmd.spec.aliases.iter().map(|a| a.to_lowercase()));
                for verb in verbs {
                    match self.lookup_idx(&verb) {
                        Err(LookupErr::NotFound) => {}
                        Ok(idx) => {
                            let global = &self.entries[idx];
                            let owner = global
                                .module
                                .clone()
                                .unwrap_or_else(|| SmolStr::new_static("core"));
                            return Err(format!(
                                "behavior command {verb:?} (behavior `{}`, module {}) is shadowed by the global command {:?} owned by {owner}: the global table resolves the verb first, so the behavior command would be dead at dispatch",
                                spec.id,
                                spec.owner,
                                global.spec.name.as_str(),
                            ));
                        }
                        Err(LookupErr::Ambiguous(names)) => {
                            let names: Vec<&str> = names.iter().map(SmolStr::as_str).collect();
                            return Err(format!(
                                "behavior command {verb:?} (behavior `{}`, module {}) is shadowed by an ambiguous global prefix ({}): the global stage refuses the verb before the behavior stage, so the behavior command would be dead at dispatch",
                                spec.id,
                                spec.owner,
                                names.join(", "),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Append a pre-destroy [`DestroyGate`] (design D8), consulted by
    /// [`check_destroy`](Registry::check_destroy) in registration order.
    pub fn add_destroy_gate(&mut self, gate: Arc<dyn DestroyGate>) {
        self.destroy_gates.push(gate);
    }

    /// Append a pre-detach [`DetachGate`], consulted by
    /// [`check_detach`](Registry::check_detach) in registration order.
    pub fn add_detach_gate(&mut self, gate: Arc<dyn DetachGate>) {
        self.detach_gates.push(gate);
    }

    /// The pre-detach veto: refuse removing `behavior` from `obj` if any
    /// registered [`DetachGate`] objects. Called by `@behavior/remove` before
    /// the binding is detached; `World::detach_behavior` itself never consults
    /// it (programmatic callers validate first).
    pub fn check_detach(
        &self,
        w: &World,
        actor: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String> {
        for gate in &self.detach_gates {
            gate.check(w, self, actor, obj, behavior)?;
        }
        Ok(())
    }

    /// The pre-destroy veto (design D8): refuse destroying `target` if it — or
    /// any exit that `World::destroy` would take with it — carries a
    /// [`DestroyPolicy::Refuse`] behavior, or if any registered [`DestroyGate`]
    /// refuses. Returns `Err(reason)` on the first refusal.
    ///
    /// Called by `@destroy` **before** any mutation; `World::destroy` itself
    /// never consults this (it stays observer-free), so programmatic callers
    /// must validate first. The veto covers the whole cascade set — the target
    /// plus a room's exits (contents are sent home, not destroyed; exits have no
    /// exits, so one level suffices) — so an exit's veto is not bypassed when its
    /// room is destroyed.
    ///
    /// A binding whose behavior is **unregistered** (its module absent) is inert
    /// here: a `Refuse` host loses protection only when the module that cared is
    /// not loaded to enforce it. This is deliberate — a removed module must not
    /// leave objects permanently undestroyable.
    pub fn check_destroy(&self, w: &World, actor: ObjId, target: ObjId) -> Result<(), String> {
        self.check_one_destroy(w, actor, target, true)?;
        let exits: Vec<ObjId> = w.get(target).map(|o| o.exits.clone()).unwrap_or_default();
        for e in exits {
            self.check_one_destroy(w, actor, e, false)?;
        }
        Ok(())
    }

    /// One object's contribution to the destroy veto (behavior policies then
    /// gates). `is_target` only shapes the refusal message.
    fn check_one_destroy(
        &self,
        w: &World,
        actor: ObjId,
        id: ObjId,
        is_target: bool,
    ) -> Result<(), String> {
        if let Some(obj) = w.get(id) {
            for bid in obj.behaviors.keys() {
                if self.behaviors.get(bid).map(|s| s.destroy) == Some(DestroyPolicy::Refuse) {
                    return Err(if is_target {
                        format!(
                            "That object carries the `{bid}` behavior, which refuses \
                             destruction. Remove it first (@behavior/remove) or use recovery."
                        )
                    } else {
                        format!(
                            "Destroying that would also destroy {id}, which carries the \
                             `{bid}` behavior that refuses destruction. Remove it first."
                        )
                    });
                }
            }
        }
        for gate in &self.destroy_gates {
            gate.check(w, self, actor, id)?;
        }
        Ok(())
    }

    /// Append a post-command-table [`CommandFallback`] (object
    /// `$`-commands). Runs in registration order, after built-ins/exits and
    /// before `Huh?`.
    pub fn add_command_fallback(&mut self, fallback: Arc<dyn CommandFallback>) {
        self.add_tagged_command_fallback(fallback, None, CommandCompatibility::LegacyPostImage);
    }

    pub(crate) fn add_tagged_command_fallback(
        &mut self,
        fallback: Arc<dyn CommandFallback>,
        module: Option<SmolStr>,
        compatibility: CommandCompatibility,
    ) {
        self.command_fallbacks.push(CommandFallbackEntry {
            fallback,
            module,
            compatibility,
        });
    }

    /// Number of registered command fallbacks (tests/inspection).
    #[must_use]
    pub fn command_fallback_count(&self) -> usize {
        self.command_fallbacks.len()
    }

    /// Declared compatibility classes for alias-stage ingress, in
    /// registration order. `None` is reserved for host/core registration.
    pub fn alias_stage_compatibilities(
        &self,
    ) -> impl Iterator<Item = (Option<&str>, CommandCompatibility)> {
        self.alias_stages
            .iter()
            .map(|entry| (entry.module.as_deref(), entry.compatibility))
    }

    /// Whether any registered alias stage would claim a line whose first
    /// word is `word`, typed by `actor` ([`AliasStage::claims_word`]; its
    /// observer contract binds every stage). `@macro` refuses such a name
    /// (16b cut AD.1): a macro named after the actor's channel alias would
    /// shadow the alias for them.
    #[must_use]
    pub fn alias_stage_claims_word(&self, w: &World, actor: ObjId, word: &str) -> bool {
        self.alias_stages
            .iter()
            .any(|entry| entry.stage.claims_word(w, actor, word))
    }

    /// Declared compatibility classes for fallback-command ingress, in
    /// registration order. `None` is reserved for host/core registration.
    pub fn command_fallback_compatibilities(
        &self,
    ) -> impl Iterator<Item = (Option<&str>, CommandCompatibility)> {
        self.command_fallbacks
            .iter()
            .map(|entry| (entry.module.as_deref(), entry.compatibility))
    }

    /// Iterate all registered command specs (registration order).
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.iter().map(|e| &e.spec)
    }

    /// Iterate command specifications with their declared compatibility
    /// class, in registration order.
    pub fn specs_with_compatibility(
        &self,
    ) -> impl Iterator<Item = (&CommandSpec, CommandCompatibility)> {
        self.entries.iter().map(|e| (&e.spec, e.compatibility))
    }

    /// Iterate command specifications with their registering module and
    /// declared compatibility class. `None` identifies host/core entries.
    pub fn command_registrations(
        &self,
    ) -> impl Iterator<Item = (&CommandSpec, Option<&str>, CommandCompatibility)> {
        self.entries
            .iter()
            .map(|entry| (&entry.spec, entry.module.as_deref(), entry.compatibility))
    }

    /// Return the compatibility class registered for an exact name or alias.
    #[must_use]
    pub fn compatibility_of(&self, name: &str) -> Option<CommandCompatibility> {
        self.index
            .get(&name.to_lowercase())
            .map(|&i| self.entries[i].compatibility)
    }

    /// Find the spec registered under exactly this name or alias
    /// (case-insensitive) — used by `help`.
    #[must_use]
    pub fn spec_of(&self, name: &str) -> Option<&CommandSpec> {
        self.index
            .get(&name.to_lowercase())
            .map(|&i| &self.entries[i].spec)
    }

    /// The primary name of the **kernel- or host-registered** command indexed
    /// under exactly this word (name or alias, case-folded), or `None`.
    ///
    /// "Kernel or host" is `module: None` — the core command set and the
    /// server's own staff verbs: the words that work in every room on every
    /// composition. A module's verb is contextual (its owner gates where it
    /// means anything) and is deliberately **not** reported here; 16b cut D.5
    /// uses this to let content take a module's word and never the kernel's.
    ///
    /// Exact, never the command table's prefix matching — `n` is a prefix of
    /// every bare name beginning with `n`, and the question this answers is
    /// "would this exact word be swallowed", not "what could it grow into".
    #[must_use]
    pub fn kernel_command_named(&self, word: &str) -> Option<&str> {
        self.index
            .get(&word.to_lowercase())
            .map(|&i| &self.entries[i])
            .filter(|e| e.module.is_none())
            .map(|e| e.spec.name.as_str())
    }

    /// Resolve a typed verb: exact name/alias first; then, for bare verbs
    /// only, an unambiguous prefix of a primary name. `@`/`+` commands
    /// never prefix-match.
    pub fn lookup(&self, verb: &str) -> Result<(&CommandSpec, Arc<dyn CommandHandler>), LookupErr> {
        let i = self.lookup_idx(verb)?;
        let e = &self.entries[i];
        Ok((&e.spec, Arc::clone(&e.handler)))
    }

    /// [`Registry::lookup`] returning the registering module too (dispatch
    /// internals: the handler's `Ctx` carries the module identity).
    #[allow(clippy::type_complexity)] // private, one call site
    fn lookup_full(
        &self,
        verb: &str,
    ) -> Result<(&CommandSpec, Arc<dyn CommandHandler>, Option<SmolStr>), LookupErr> {
        let i = self.lookup_idx(verb)?;
        let e = &self.entries[i];
        Ok((&e.spec, Arc::clone(&e.handler), e.module.clone()))
    }

    fn lookup_idx(&self, verb: &str) -> Result<usize, LookupErr> {
        let v = verb.to_lowercase();
        if v.is_empty() {
            return Err(LookupErr::NotFound);
        }
        if let Some(&i) = self.index.get(&v) {
            return Ok(i);
        }
        if v.starts_with('@') || v.starts_with('+') {
            return Err(LookupErr::NotFound);
        }
        let mut hits: Vec<usize> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            let name = e.spec.name.as_str();
            // An exact-only name (16b cut AP.4) answers the index hit above
            // alone; it is never a prefix match.
            if !e.exact
                && !name.starts_with('@')
                && !name.starts_with('+')
                && name.to_lowercase().starts_with(&v)
                && !hits.contains(&i)
            {
                hits.push(i);
            }
        }
        match hits.len() {
            0 => Err(LookupErr::NotFound),
            1 => Ok(hits[0]),
            _ => Err(LookupErr::Ambiguous(
                hits.iter()
                    .map(|&i| self.entries[i].spec.name.clone())
                    .collect(),
            )),
        }
    }

    /// Add (or replace) a prose help topic. Names are case-insensitive.
    /// Called by the driver's boot-time loader; also usable by modules.
    pub fn add_help_topic(
        &mut self,
        name: &str,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        let key = SmolStr::from(name.to_lowercase());
        self.help_topics.insert(
            key.clone(),
            HelpTopic {
                name: key,
                title: title.into(),
                body: body.into(),
            },
        );
    }

    /// Look up a help topic by name (case-insensitive).
    #[must_use]
    pub fn help_topic(&self, name: &str) -> Option<&HelpTopic> {
        self.help_topics.get(name.to_lowercase().as_str())
    }

    /// All help-topic names, sorted.
    pub fn help_topic_names(&self) -> impl Iterator<Item = &str> {
        self.help_topics.keys().map(SmolStr::as_str)
    }

    /// The closest registered name/alias to `verb` by edit distance
    /// (≤ 2 edits and closer than the word's own length) — the `Huh?`
    /// suggestion.
    #[must_use]
    pub fn suggest(&self, verb: &str) -> Option<SmolStr> {
        let v = verb.to_lowercase();
        if v.is_empty() {
            return None;
        }
        let mut best: Option<(usize, &SmolStr)> = None;
        for e in &self.entries {
            for name in std::iter::once(&e.spec.name).chain(e.spec.aliases.iter()) {
                let d = edit_distance(&v, &name.to_lowercase());
                if d <= 2
                    && d < v.len().max(1)
                    && best
                        .is_none_or(|(bd, bn)| d < bd || (d == bd && name.as_str() < bn.as_str()))
                {
                    best = Some((d, name));
                }
            }
        }
        best.map(|(_, n)| n.clone())
    }
}

/// Plain Levenshtein distance (small strings; O(m·n)).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Run one input line through the full dispatch pipeline (module docs for
/// the exact order). All output — including errors and `Huh?` — is queued
/// on `out`.
pub fn dispatch(
    reg: &Registry,
    w: &mut World,
    actor: ObjId,
    line: &str,
    out: &mut Outbox,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    if w.get(actor).is_none() {
        return;
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    // 0. Input-context stack — BEFORE everything else (even prefix
    //    chars), with one hard escape: a lone "~q" always pops the top
    //    context with an abort message, so no context can trap a player.
    if w.input_depth(actor) > 0 {
        if trimmed.eq_ignore_ascii_case("~q") {
            w.pop_input_context(actor);
            out.info(actor, "Aborted.");
            let spec = restored_prompt(w, actor);
            out.to_actor(actor, OutputEvent::Prompt(spec));
            return;
        }
        if let Some(mut stacked) = w.pop_input_context(actor) {
            // Remember where it sat so it can go back beneath anything the
            // feed itself pushes.
            let index = w.input_depth(actor);
            let outcome = {
                let mut ctx = Ctx {
                    w,
                    actor,
                    switches: SwitchSet::default(),
                    args: ParsedArgs::default(),
                    out,
                    reg,
                    // The context runs with the identity of the module that
                    // pushed it (mail compose persists as "mail"; a chargen
                    // context may write its ruleset's attr namespace).
                    module: stacked.module.clone(),
                    typed_state: crate::action_state::ModuleActionState::for_module(
                        lens,
                        stacked.module.as_deref(),
                    ),
                };
                stacked.cx.feed(&mut ctx, trimmed)
            };
            match outcome {
                ContextOutcome::Consumed => {
                    w.restore_input_context_at(actor, index, stacked);
                }
                ContextOutcome::Pop => {
                    let spec = restored_prompt(w, actor);
                    out.to_actor(actor, OutputEvent::Prompt(spec));
                }
                ContextOutcome::PopAndRun(cmd) => {
                    let spec = restored_prompt(w, actor);
                    out.to_actor(actor, OutputEvent::Prompt(spec));
                    dispatch(reg, w, actor, &cmd, out, lens);
                }
            }
            return;
        }
    }

    // 1. Single-char prefixes.
    let first = trimmed.chars().next().unwrap_or(' ');
    let rest = &trimmed[first.len_utf8()..];
    let prefix_expanded = matches!(first, '"' | ':' | ';' | '\\');
    let mut cur = match first {
        '"' => format!("say {rest}"),
        ':' => format!("pose {rest}"),
        ';' => format!("semipose {rest}"),
        '\\' => format!("@emit {rest}"),
        _ => trimmed.to_string(),
    };

    // 2. Command gates (the onboarding gate). Consulted on the expanded
    //    line's leading verb and whole line, BEFORE the alias stage and
    //    before home/exits/commands — so a gate that holds an unapproved
    //    character to a chargen command set blocks channel broadcast (a
    //    per-player comsys alias would otherwise be fully handled by the
    //    alias stage below and never reach the gate), movement (exit names,
    //    `home`), and building. Speech has already been rewritten to
    //    say/pose/… by the prefix expansion, so a gate sees it as a normal
    //    verb it can wave through. Skipped entirely when no gate is
    //    registered (the default) — dispatch is then byte-identical.
    if command_gates_refuse(reg, w, actor, &cur, out) {
        return;
    }

    // 3. Alias stages: the kernel's macro stage first, then the modules'
    //    (comsys's channel aliases). Prefix-expanded lines skip them: the
    //    prefix character already settled what the line means (room
    //    speech), and letting an alias named "say"/"pose" capture the
    //    synthetic verb would reroute private speech onto a channel the
    //    player never named. A *typed* "say hello" still offers the alias
    //    stages first, per treaty ruling 5.
    //
    //    A rewrite (16b cut AD.1, D9) substitutes a DIFFERENT verb for the
    //    one step 2 gated, and everything downstream (the later stages,
    //    home, exits, the command table) acts on the substitute — so the
    //    gates see it AT ONCE, or the whole gate is one alias stage away
    //    from being defeated. Then the loop goes on with the NEXT stage on
    //    the new line: a macro whose line is a channel alias (`@macro
    //    r=rad $1`) reaches comsys instead of falling past it to `Huh?`. A
    //    stage never sees a line twice and never its own output, so a chain
    //    is bounded by the stage count and the macro stage (first) expands
    //    exactly once. With no rewrite (and always with no gate registered)
    //    dispatch does exactly the work it did before.
    if !prefix_expanded {
        for entry in &reg.alias_stages {
            match entry.stage.try_alias(w, actor, &cur, out) {
                AliasOutcome::NotMine => {}
                AliasOutcome::Handled => return,
                AliasOutcome::Rewrite(s) => {
                    cur = s;
                    if command_gates_refuse(reg, w, actor, &cur, out) {
                        return;
                    }
                }
            }
        }
    }

    // 4. HOME keyword.
    if cur.trim().eq_ignore_ascii_case("home") {
        movement::go_home(reg, w, actor, out, lens);
        return;
    }

    // 5. Exit match (room → zone → global) against the whole line.
    if let Some(exit) = movement::find_exit(w, actor, cur.trim()) {
        movement::traverse_exit(reg, w, actor, exit, out, lens);
        return;
    }

    // 6. Command table.
    let (head, rest) = split_head(&cur);
    let mut parts = head.split('/');
    let verb = parts.next().unwrap_or("");
    let switch_names: Vec<&str> = parts.filter(|s| !s.is_empty()).collect();
    match reg.lookup_full(verb) {
        Err(LookupErr::NotFound) => {
            // 5. Behavior commands on reachable subjects (design 06 §4.3).
            // Consulted only after `home`, exits, and the global table have all
            // missed, so a behavior verb can never shadow a core verb, a staff
            // command, or an exit. The first tier with a match wins; an
            // ambiguous tier refuses and never falls through.
            match resolve_behavior_command(reg, w, actor, verb, lens) {
                BehaviorResolution::One { target, index } => {
                    let Some(bspec) = reg.behavior_spec(&target.behavior) else {
                        huh(reg, actor, verb, out);
                        return;
                    };
                    let Some(cmd) = bspec.commands.get(index) else {
                        huh(reg, actor, verb, out);
                        return;
                    };
                    if let Some((args, switches)) =
                        gate_spec(w, actor, &cmd.spec, &switch_names, rest, out)
                    {
                        let handler = Arc::clone(&cmd.handler);
                        let module = Some(bspec.owner.clone());
                        run_handler(reg, w, actor, switches, args, module, out, lens, |ctx| {
                            handler.call(ctx, &target)
                        });
                    }
                    return;
                }
                BehaviorResolution::Ambiguous(targets) => {
                    let names: Vec<String> = targets
                        .iter()
                        .map(|t| {
                            format!(
                                "{} ({}, via {})",
                                display_name(w, t.subject),
                                t.subject,
                                t.behavior
                            )
                        })
                        .collect();
                    out.error(
                        actor,
                        format!("Ambiguous \"{verb}\": could mean {}.", names.join(" or ")),
                    );
                    return;
                }
                BehaviorResolution::None => {}
            }
            // 6. Post-command-table fallbacks (object `$`-commands): a
            // fallback that claims the whole line ends dispatch before
            // `Huh?`. They run against the full post-alias line.
            let line = cur.trim().to_string();
            for entry in &reg.command_fallbacks {
                if entry.fallback.try_command(w, actor, &line, out) {
                    return;
                }
            }
            huh(reg, actor, verb, out);
        }
        Err(LookupErr::Ambiguous(names)) => {
            let names: Vec<&str> = names.iter().map(SmolStr::as_str).collect();
            out.error(
                actor,
                format!(
                    "Ambiguous command \"{verb}\" (could be: {}).",
                    names.join(", ")
                ),
            );
        }
        Ok((spec, handler, module)) => {
            if let Some((args, switches)) = gate_spec(w, actor, spec, &switch_names, rest, out) {
                run_handler(reg, w, actor, switches, args, module, out, lens, |ctx| {
                    handler.call(ctx)
                });
            }
        }
    }
}

/// Split off the first whitespace-delimited word.
fn split_head(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// Validate `spec`'s switches, role, power, and lock for `actor`. On refusal
/// the error is queued to `out` and `None` is returned; on success, the parsed
/// arguments and switch set. Shared by the global command table and the
/// behavior-command stage so both gate identically.
fn gate_spec(
    w: &World,
    actor: ObjId,
    spec: &CommandSpec,
    switch_names: &[&str],
    rest: &str,
    out: &mut Outbox,
) -> Option<(ParsedArgs, SwitchSet)> {
    for s in switch_names {
        if !spec.switches.iter().any(|k| k.eq_ignore_ascii_case(s)) {
            out.error(actor, format!("Unknown switch \"/{s}\" for {}.", spec.name));
            return None;
        }
    }
    if perm::role_of(w, actor) < spec.min_role {
        out.error(actor, "Permission denied.");
        return None;
    }
    if let Some(p) = spec.power
        && !perm::has_power(w, actor, p)
    {
        out.error(actor, "Permission denied.");
        return None;
    }
    if let Some(p) = spec.privilege
        && !perm::has_privilege(w, actor, p)
    {
        out.error(actor, "Permission denied.");
        return None;
    }
    if let Some(lock) = &spec.lock
        && !eval_lock(w, lock, actor, MAX_LOCK_DEPTH)
    {
        out.error(actor, "Permission denied.");
        return None;
    }
    Some((
        ParsedArgs::parse(spec.shape, rest),
        SwitchSet::from_names(switch_names.iter().copied()),
    ))
}

/// Run a handler body under a fresh [`Ctx`] with the deferred-queue
/// discipline: everything the handler deferred is discarded if it returns
/// `Err` (queued persistence must never commit for a command that reported
/// failure; world mutations made before the error remain best-effort —
/// handlers validate-then-apply).
#[allow(clippy::too_many_arguments)]
fn run_handler(
    reg: &Registry,
    w: &mut World,
    actor: ObjId,
    switches: SwitchSet,
    args: ParsedArgs,
    module: Option<SmolStr>,
    out: &mut Outbox,
    lens: crate::action_state::TypedStateLens<'_>,
    body: impl FnOnce(&mut Ctx<'_>) -> Result<(), CmdError>,
) {
    let events_before = out.len();
    let deferred_before = out.deferred().len();
    let result = {
        let typed_state =
            crate::action_state::ModuleActionState::for_module(lens, module.as_deref());
        let mut ctx = Ctx {
            w,
            actor,
            switches,
            args,
            out,
            reg,
            module,
            typed_state,
        };
        body(&mut ctx)
    };
    if let Err(e) = result {
        out.retain_abort_since(events_before);
        // Only past the pin: a terminal commit (`mush_canon::commit`) marks the
        // prefix it and its synchronous observers queued as non-truncatable,
        // because dispatch does not roll RAM back and dropping those writes would
        // leave the store behind a world that has already moved.
        out.truncate_deferred(deferred_before);
        out.error(actor, e.to_string());
    }
}

/// Consult every registered [`CommandGate`] on `line`'s leading verb and the
/// whole line. `true` means refused — the gate's reason is already queued on
/// `out` and the caller must stop dispatching.
///
/// Skipped entirely when no gate is registered (the default), so dispatch is
/// byte-identical to the un-gated pipeline. Called once on the
/// prefix-expanded line and once more on each [`AliasOutcome::Rewrite`], at
/// the rewrite, before any later stage sees the substitute.
fn command_gates_refuse(
    reg: &Registry,
    w: &World,
    actor: ObjId,
    line: &str,
    out: &mut Outbox,
) -> bool {
    if reg.command_gates.is_empty() {
        return false;
    }
    let line = line.trim();
    let (head, _) = split_head(line);
    let verb = head.split('/').next().unwrap_or("");
    for gate in &reg.command_gates {
        if let Err(reason) = gate.check(w, reg, actor, verb, line) {
            out.error(actor, reason);
            return true;
        }
    }
    false
}

/// Queue the classic `Huh?`, with a fuzzy suggestion when one is close.
fn huh(reg: &Registry, actor: ObjId, verb: &str, out: &mut Outbox) {
    let mut msg = String::from("Huh?  (Type \"help\" for help.)");
    if let Some(s) = reg.suggest(verb) {
        msg.push_str(&format!("  Did you mean \"{s}\"?"));
    }
    out.error(actor, msg);
}

#[cfg(test)]
mod tests {
    /// A store-less lens for the dispatch calls below: these tests pin the
    /// answer a caller gets with no typed state at all, which is exactly what
    /// the old store-less twin handed out by default.
    const DETACHED: crate::action_state::TypedStateLens<'static> =
        crate::action_state::TypedStateLens::Detached(
            crate::action_state::DetachedTypedState::Test,
        );

    use super::*;

    fn spec(name: &str, shape: ArgShape) -> CommandSpec {
        CommandSpec {
            name: name.into(),
            shape,
            help: "test".into(),
            ..CommandSpec::default()
        }
    }

    fn noop() -> Arc<dyn CommandHandler> {
        Arc::new(|_ctx: &mut Ctx<'_>| Ok(()))
    }

    #[test]
    fn arg_shapes() {
        let a = ParsedArgs::parse(ArgShape::Raw, "  hello  world  ");
        assert_eq!(a.raw(), "hello  world");
        assert_eq!(a.lhs(), "");
        assert_eq!(a.rhs(), None);

        let a = ParsedArgs::parse(ArgShape::EqPair, "target = some text");
        assert_eq!(a.lhs(), "target");
        assert_eq!(a.rhs(), Some("some text"));
        assert!(a.rhs_list().is_empty());

        // No '=': everything is lhs, rhs None.
        let a = ParsedArgs::parse(ArgShape::EqPair, "just a target");
        assert_eq!(a.lhs(), "just a target");
        assert_eq!(a.rhs(), None);

        // Explicit empty rhs is Some("").
        let a = ParsedArgs::parse(ArgShape::EqPair, "thing=");
        assert_eq!(a.rhs(), Some(""));

        // Only the FIRST '=' splits.
        let a = ParsedArgs::parse(ArgShape::EqPair, "a=b=c");
        assert_eq!(a.lhs(), "a");
        assert_eq!(a.rhs(), Some("b=c"));

        let a = ParsedArgs::parse(ArgShape::EqPairList, "Hall = north;n , south;s ,, ");
        assert_eq!(a.lhs(), "Hall");
        assert_eq!(a.rhs_list(), ["north;n".to_string(), "south;s".to_string()]);

        let a = ParsedArgs::parse(ArgShape::EqPairSemi, "sword = blade; sharp ;");
        assert_eq!(a.rhs_list(), ["blade".to_string(), "sharp".to_string()]);

        let a = ParsedArgs::parse(ArgShape::None, "ignored");
        assert_eq!(a.lhs(), "");
        assert_eq!(a.rhs(), None);
    }

    #[test]
    fn registry_collision_and_lookup() {
        let mut reg = Registry::new();
        reg.register(spec("look", ArgShape::Raw), noop()).unwrap();
        reg.register(
            CommandSpec {
                aliases: vec!["l".into()],
                ..spec("lie", ArgShape::Raw)
            },
            noop(),
        )
        .unwrap();
        // Name collision.
        assert_eq!(
            reg.register(spec("look", ArgShape::Raw), noop()),
            Err(RegistryError::Collision("look".into()))
        );
        // Alias collision.
        assert_eq!(
            reg.register(
                CommandSpec {
                    aliases: vec!["LOOK".into()],
                    ..spec("peer", ArgShape::Raw)
                },
                noop()
            ),
            Err(RegistryError::Collision("look".into()))
        );

        // Exact (case-insensitive) beats prefix.
        assert!(reg.lookup("LOOK").is_ok());
        assert!(reg.lookup("l").is_ok()); // exact alias
        // Prefix: "lo" only matches look.
        assert_eq!(reg.lookup("lo").unwrap().0.name, "look");
        // "li" only matches lie.
        assert_eq!(reg.lookup("li").unwrap().0.name, "lie");
        assert!(matches!(reg.lookup("zz"), Err(LookupErr::NotFound)));
    }

    #[test]
    fn at_commands_require_exact() {
        let mut reg = Registry::new();
        reg.register(spec("@dig", ArgShape::EqPairList), noop())
            .unwrap();
        reg.register(spec("@digest", ArgShape::Raw), noop())
            .unwrap();
        assert!(reg.lookup("@dig").is_ok());
        assert!(reg.lookup("@DIG").is_ok());
        assert!(matches!(reg.lookup("@di"), Err(LookupErr::NotFound)));
        assert!(matches!(reg.lookup("@d"), Err(LookupErr::NotFound)));
    }

    #[test]
    fn ambiguous_prefix() {
        let mut reg = Registry::new();
        reg.register(spec("get", ArgShape::Raw), noop()).unwrap();
        reg.register(spec("give", ArgShape::EqPair), noop())
            .unwrap();
        match reg.lookup("g") {
            Err(LookupErr::Ambiguous(names)) => {
                assert!(names.iter().any(|n| n == "get") && names.iter().any(|n| n == "give"));
            }
            other => panic!(
                "expected ambiguous, got {:?}",
                other.map(|(s, _)| s.name.clone())
            ),
        }
    }

    #[test]
    fn suggest_finds_close_names() {
        let mut reg = Registry::new();
        reg.register(spec("look", ArgShape::Raw), noop()).unwrap();
        reg.register(spec("pose", ArgShape::Raw), noop()).unwrap();
        assert_eq!(reg.suggest("lok").as_deref(), Some("look"));
        assert_eq!(reg.suggest("poze").as_deref(), Some("pose"));
        assert_eq!(reg.suggest("xyzzy"), None);
    }

    #[test]
    fn command_lock_gate_is_enforced() {
        use crate::obj::{GOD, LIMBO, ObjKind};
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let mut reg = Registry::new();
        reg.register(
            CommandSpec {
                lock: Some(LockExpr::Is(GOD)),
                ..spec("sanctify", ArgShape::Raw)
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "Sanctified.");
                Ok(())
            }),
        )
        .unwrap();
        // Ann fails the lock gate before the handler runs.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "sanctify", &mut out, DETACHED);
        let texts: Vec<_> = out
            .events()
            .iter()
            .filter_map(|(_, e)| match e {
                mush_proto::OutputEvent::Sys(t) => Some(t.to_plain_text()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["Permission denied.".to_string()]);
        // God passes.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, GOD, "sanctify", &mut out, DETACHED);
        assert!(out.events().iter().any(
            |(_, e)| matches!(e, mush_proto::OutputEvent::Sys(t) if t.to_plain_text() == "Sanctified.")
        ));
    }

    #[test]
    fn command_gate_blocks_and_allows() {
        use crate::obj::{GOD, LIMBO, ObjKind};
        // A gate that allows only "look" and refuses everything else.
        struct OnlyLook;
        impl CommandGate for OnlyLook {
            fn check(
                &self,
                _w: &World,
                _reg: &Registry,
                _actor: ObjId,
                verb: &str,
                _line: &str,
            ) -> Result<(), String> {
                if verb.eq_ignore_ascii_case("look") {
                    Ok(())
                } else {
                    Err("blocked".to_string())
                }
            }
        }
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let mut reg = Registry::new();
        reg.register(
            spec("look", ArgShape::Raw),
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "ok-look");
                Ok(())
            }),
        )
        .unwrap();
        reg.register(
            spec("pose", ArgShape::Raw),
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "ok-pose");
                Ok(())
            }),
        )
        .unwrap();
        reg.add_command_gate(Arc::new(OnlyLook));

        let plain = |out: &Outbox| -> Vec<String> {
            out.events()
                .iter()
                .filter_map(|(_, e)| match e {
                    mush_proto::OutputEvent::Sys(t) => Some(t.to_plain_text()),
                    _ => None,
                })
                .collect()
        };

        // Allowed verb runs.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "look", &mut out, DETACHED);
        assert_eq!(plain(&out), vec!["ok-look".to_string()]);

        // A non-allowlisted command is refused with the gate's reason; its
        // handler never runs.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "pose", &mut out, DETACHED);
        assert_eq!(plain(&out), vec!["blocked".to_string()]);
    }

    #[test]
    fn command_gate_runs_before_the_alias_stage() {
        use crate::obj::{GOD, LIMBO, ObjKind};
        // A gate refusing "pub" and an alias stage that would fully handle any
        // line whose first word is "pub" (the comsys broadcast seam). The gate
        // must win: an unapproved character cannot broadcast on a channel.
        struct BlockPub;
        impl CommandGate for BlockPub {
            fn check(
                &self,
                _w: &World,
                _reg: &Registry,
                _actor: ObjId,
                verb: &str,
                _line: &str,
            ) -> Result<(), String> {
                if verb.eq_ignore_ascii_case("pub") {
                    Err("gated".to_string())
                } else {
                    Ok(())
                }
            }
        }
        struct PubChannel;
        impl AliasStage for PubChannel {
            fn try_alias(
                &self,
                _w: &mut World,
                actor: ObjId,
                line: &str,
                out: &mut Outbox,
            ) -> AliasOutcome {
                if line.split_whitespace().next() == Some("pub") {
                    out.info(actor, "broadcast");
                    AliasOutcome::Handled
                } else {
                    AliasOutcome::NotMine
                }
            }
        }
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let mut reg = Registry::new();
        reg.add_alias_stage(Arc::new(PubChannel));
        reg.add_command_gate(Arc::new(BlockPub));

        let plain = |out: &Outbox| -> Vec<String> {
            out.events()
                .iter()
                .filter_map(|(_, e)| match e {
                    mush_proto::OutputEvent::Sys(t) => Some(t.to_plain_text()),
                    _ => None,
                })
                .collect()
        };

        // The gate refuses "pub spam" before the alias stage ever sees it: the
        // channel broadcast never happens.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "pub spam", &mut out, DETACHED);
        assert_eq!(plain(&out), vec!["gated".to_string()]);
        assert!(
            !plain(&out).iter().any(|l| l == "broadcast"),
            "the alias stage must not have run"
        );
    }

    #[test]
    fn command_gate_re_runs_after_an_alias_rewrite() {
        use crate::obj::{GOD, LIMBO, ObjKind};
        // A gate holding an unapproved character to "+stat", and an alias
        // stage that rewrites anything else into "@dig x" — the bypass: the
        // gate saw the ORIGINAL verb, and without a second pass the rewritten
        // one dispatches ungated.
        struct OnlyStat;
        impl CommandGate for OnlyStat {
            fn check(
                &self,
                _w: &World,
                _reg: &Registry,
                _actor: ObjId,
                verb: &str,
                _line: &str,
            ) -> Result<(), String> {
                if verb.eq_ignore_ascii_case("+stat") {
                    Ok(())
                } else {
                    Err("gated".to_string())
                }
            }
        }
        struct RewriteToDig;
        impl AliasStage for RewriteToDig {
            fn try_alias(
                &self,
                _w: &mut World,
                _actor: ObjId,
                line: &str,
                _out: &mut Outbox,
            ) -> AliasOutcome {
                if line.split_whitespace().next() == Some("+stat") {
                    AliasOutcome::Rewrite("@dig x".to_string())
                } else {
                    AliasOutcome::NotMine
                }
            }
        }
        let mut w = World::new_minimal();
        let ann = w.create(ObjKind::Player, "Ann", GOD, Some(LIMBO)).unwrap();
        let mut reg = Registry::new();
        reg.register(
            spec("@dig", ArgShape::Raw),
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "dug");
                Ok(())
            }),
        )
        .unwrap();
        reg.register(
            spec("+stat", ArgShape::Raw),
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.info(ctx.actor, "stat");
                Ok(())
            }),
        )
        .unwrap();
        reg.add_alias_stage(Arc::new(RewriteToDig));
        reg.add_command_gate(Arc::new(OnlyStat));

        let plain = |out: &Outbox| -> Vec<String> {
            out.events()
                .iter()
                .filter_map(|(_, e)| match e {
                    mush_proto::OutputEvent::Sys(t) => Some(t.to_plain_text()),
                    _ => None,
                })
                .collect()
        };

        // "+stat" passes the first gate pass, the alias stage turns it into
        // "@dig x", and the second pass refuses THAT verb: the handler never
        // runs.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "+stat", &mut out, DETACHED);
        assert_eq!(plain(&out), vec!["gated".to_string()]);
        assert!(
            !plain(&out).iter().any(|l| l == "dug"),
            "the rewritten verb must not have dispatched"
        );

        // A line no stage rewrites is gated exactly once, on its own verb.
        let mut out = Outbox::new();
        dispatch(&reg, &mut w, ann, "@dig y", &mut out, DETACHED);
        assert_eq!(plain(&out), vec!["gated".to_string()]);
    }

    /// 16b cut AP.4 (review `AP4-5`): an exact-only name resolves typed in
    /// full, case-folded, and never as a prefix — so `o` stays `ORDER`'s
    /// with `OUTPUTPREFIX` registered beside it.
    #[test]
    fn ap4_an_exact_only_name_resolves_typed_in_full_and_never_as_a_prefix() {
        let mut reg = Registry::new();
        reg.register(spec("ORDER", ArgShape::Raw), noop()).unwrap();
        reg.register_exact(spec("OUTPUTPREFIX", ArgShape::Raw), noop())
            .unwrap();
        let name = |verb: &str| reg.lookup(verb).map(|(s, _)| s.name.to_string());
        assert_eq!(name("o").as_deref(), Ok("ORDER"));
        assert_eq!(name("or").as_deref(), Ok("ORDER"));
        assert_eq!(name("outputprefix").as_deref(), Ok("OUTPUTPREFIX"));
        assert_eq!(name("OutputPrefix").as_deref(), Ok("OUTPUTPREFIX"));
        assert_eq!(name("output"), Err(LookupErr::NotFound));
        assert_eq!(name("outputprefi"), Err(LookupErr::NotFound));
        // Exact-only does not exempt it from collision.
        assert_eq!(
            reg.register_exact(spec("outputprefix", ArgShape::Raw), noop()),
            Err(RegistryError::Collision("outputprefix".into()))
        );
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("look", "look"), 0);
        assert_eq!(edit_distance("lok", "look"), 1);
        assert_eq!(edit_distance("luk", "look"), 2);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn switch_set() {
        let s = SwitchSet::from_names(["Teleport", "QUIET"]);
        assert!(s.has("teleport"));
        assert!(s.has("TELEPORT"));
        assert!(s.has("quiet"));
        assert!(!s.has("override"));
        assert!(!s.is_empty());
        assert_eq!(s.iter().collect::<Vec<_>>(), vec!["teleport", "quiet"]);
    }
}
