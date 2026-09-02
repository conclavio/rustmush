//! First-class object behaviors (design: `docs/design/06` §4) and the
//! pre-destroy veto seam (decision D8).
//!
//! An ordinary [`Obj`](crate::obj::Obj) can carry typed, durable **behavior
//! bindings** — `sim.domain`, `btech.mech`, `shadowrun.vehicle` — declaring that
//! a module's behavior applies to it. The binding itself is a lightweight marker
//! ([`BehaviorBinding`]); large or hot behavior state lives in the owning
//! module's persisted namespace (e.g. `mod:btech:unit:123`), never on the object.
//!
//! A module registers a [`BehaviorSpec`] through the
//! [`Registrar`](crate::module::Registrar) — one module owns each id — declaring
//! which [`ObjKind`]s it may attach to, which behaviors it conflicts with, the
//! role required to attach it, and its [`DestroyPolicy`]. Builders bind and
//! unbind through the lifecycle-aware `@behavior` command; a raw `@set` never
//! attaches behavior.
//!
//! **The destroy veto (D8).** A behavior may refuse destruction of its host, and
//! modules with dynamic structural dependencies register a [`DestroyGate`]. Both
//! are consulted at the **command layer** ([`Registry::check_destroy`], called by
//! `@destroy` before any mutation) — never inside `World::destroy`, which stays
//! observer-free. A domain host is therefore refused before its exit cascade can
//! begin, and the post-destroy `ObjDestroyed` observer remains only a backstop.
//! Programmatic `World::destroy` bypasses the veto by design (validate before you
//! mutate).

use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Role};
use crate::matchit::match_object;
use crate::module::can_perceive;
use crate::obj::{ObjId, ObjKind};
use crate::perm;
use crate::world::World;

// ----------------------------------------------------------------------
// Data types (persisted on the object)
// ----------------------------------------------------------------------

/// Maximum length of a [`BehaviorId`] string.
pub const BEHAVIOR_ID_MAX: usize = 64;

/// A validated, namespaced behavior identifier such as `sim.domain` or
/// `btech.mech`: two or more `.`-separated segments of `[a-z0-9_]`.
///
/// Serializes transparently as its string form, so it is a valid JSON map key
/// on [`Obj::behaviors`](crate::obj::Obj::behaviors). Construct through
/// [`BehaviorId::new`], which validates; deserialization is lenient (a
/// hand-corrupt document loads, then reads as an unregistered behavior).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BehaviorId(SmolStr);

/// Why a [`BehaviorId`] string was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BehaviorIdError {
    /// The string was empty.
    #[error("behavior id is empty")]
    Empty,
    /// The string exceeded [`BEHAVIOR_ID_MAX`].
    #[error("behavior id {0:?} is too long (max {BEHAVIOR_ID_MAX})")]
    TooLong(String),
    /// Not `ns.name` form: needs ≥2 dot-separated `[a-z0-9_]` segments.
    #[error("behavior id {0:?} must be dotted lowercase segments, e.g. `sim.domain`")]
    Malformed(String),
}

impl BehaviorId {
    /// Validate and construct. Requires `ns.name` form (≥2 segments), each a
    /// non-empty run of `[a-z0-9_]`, total length ≤ [`BEHAVIOR_ID_MAX`].
    pub fn new(s: impl Into<SmolStr>) -> Result<BehaviorId, BehaviorIdError> {
        let s = s.into();
        if s.is_empty() {
            return Err(BehaviorIdError::Empty);
        }
        if s.len() > BEHAVIOR_ID_MAX {
            return Err(BehaviorIdError::TooLong(s.to_string()));
        }
        let mut segments = 0;
        for seg in s.split('.') {
            if seg.is_empty()
                || !seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(BehaviorIdError::Malformed(s.to_string()));
            }
            segments += 1;
        }
        if segments < 2 {
            return Err(BehaviorIdError::Malformed(s.to_string()));
        }
        Ok(BehaviorId(s))
    }

    /// The namespace segment (before the first `.`).
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.0.split_once('.').map_or(self.0.as_str(), |(ns, _)| ns)
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BehaviorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for BehaviorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BehaviorId({:?})", self.0)
    }
}

impl FromStr for BehaviorId {
    type Err = BehaviorIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        BehaviorId::new(s)
    }
}

/// The durable record placed on an object when a behavior is attached. Kept
/// lightweight on purpose (design §4.1): it marks that the behavior exists;
/// substantial state belongs in the owning module's namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorBinding {
    /// Binding schema version, for future migration of the marker itself.
    pub schema_version: u32,
    /// Who attached it.
    pub attached_by: ObjId,
    /// Unix seconds at attach time.
    pub attached_at: i64,
}

/// The current [`BehaviorBinding::schema_version`] written by `@behavior/add`.
pub const BINDING_SCHEMA_VERSION: u32 = 1;

// ----------------------------------------------------------------------
// Registration (owned by a module, held in the Registry)
// ----------------------------------------------------------------------

/// What happens when an object carrying a behavior is `@destroy`ed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DestroyPolicy {
    /// Destruction proceeds; any module cleanup runs via the post-destroy
    /// `ObjDestroyed` observer.
    #[default]
    Allow,
    /// Destruction is refused while the behavior is bound — staff must
    /// `@behavior/remove` it (or run recovery) first. For structural hosts such
    /// as a simulation domain or interior anchor.
    Refuse,
}

/// A behavior type registration (design §4.2). Exactly one module owns an id.
#[derive(Clone)]
pub struct BehaviorSpec {
    /// The behavior id.
    pub id: BehaviorId,
    /// Owning module namespace — filled in by the [`Registrar`](crate::module::Registrar).
    pub owner: SmolStr,
    /// Kinds this behavior may attach to; empty means any kind.
    pub allowed_kinds: Vec<ObjKind>,
    /// Behaviors that may not coexist with this one on the same object.
    pub conflicts: Vec<BehaviorId>,
    /// Destruction policy for a bound object.
    pub destroy: DestroyPolicy,
    /// Role required to attach/detach, in addition to controlling the object.
    /// Defaults to Builder; a structural behavior may require Wizard.
    pub min_role: Role,
    /// One-line help shown by `@behavior/list`.
    pub help: String,
    /// Contextual commands this behavior contributes (design §4.3): ordinary
    /// [`CommandSpec`]s that exist only for an actor who can reach an object
    /// carrying the behavior — never in the global table.
    pub commands: Vec<BehaviorCommand>,
}

impl BehaviorSpec {
    /// A permissive spec for `id`: any kind, no conflicts, [`DestroyPolicy::Allow`],
    /// Builder role. `owner` is filled in when registered through a `Registrar`.
    #[must_use]
    pub fn new(id: BehaviorId) -> BehaviorSpec {
        BehaviorSpec {
            id,
            owner: SmolStr::default(),
            allowed_kinds: Vec::new(),
            conflicts: Vec::new(),
            destroy: DestroyPolicy::Allow,
            min_role: Role::Builder,
            help: String::new(),
            commands: Vec::new(),
        }
    }

    /// Add a contextual command (builder style).
    #[must_use]
    pub fn with_command(mut self, cmd: BehaviorCommand) -> BehaviorSpec {
        self.commands.push(cmd);
        self
    }
}

// ----------------------------------------------------------------------
// Contextual commands (design §4.3) and their subjects (§4.4)
// ----------------------------------------------------------------------

/// How a command subject was reached from the actor — the resolution tier it
/// was found at. A [`BehaviorCommand`] declares which reaches it accepts, so a
/// room-wide `beacon` can opt into [`Ambient`](CommandReach::Ambient) while a
/// cockpit `startup` stays confined to what the actor operates, is, is inside
/// of, or carries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommandReach {
    /// An object the actor operates through a published
    /// [`CommandSubjectLink`](crate::world::CommandSubjectLink) — a pilot's
    /// mech, a rigger's drone. Highest priority.
    Operated,
    /// The actor object itself.
    Actor,
    /// The actor's immediate container when it is not a room (a cockpit).
    Container,
    /// An object the actor carries (one level; never nested).
    Carried,
    /// The containing room. Only for commands that opt in.
    Ambient,
}

impl CommandReach {
    /// The default reach set for a behavior command: everything except
    /// [`Ambient`](CommandReach::Ambient), which must be opted into.
    #[must_use]
    pub fn default_set() -> Vec<CommandReach> {
        vec![
            CommandReach::Operated,
            CommandReach::Actor,
            CommandReach::Container,
            CommandReach::Carried,
        ]
    }
}

/// What a behavior command was invoked on: the resolved subject, the behavior
/// that supplied the verb, and how the subject was reached. Handed to
/// [`BehaviorCommandHandler::call`] alongside the ordinary [`Ctx`] (whose
/// `module` is the behavior's owner), so `Ctx` itself needs no new field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BehaviorTarget {
    /// The object whose behavior supplied the command.
    pub subject: ObjId,
    /// The behavior that supplied it.
    pub behavior: BehaviorId,
    /// The tier the subject was resolved at.
    pub reach: CommandReach,
}

/// A handler for a contextual behavior command: like
/// [`CommandHandler`](crate::cmd::CommandHandler) but also given the
/// [`BehaviorTarget`]. Plain `fn`s and closures implement it via the blanket
/// impl.
pub trait BehaviorCommandHandler: Send + Sync {
    /// Run the command on `target`.
    fn call(&self, ctx: &mut Ctx<'_>, target: &BehaviorTarget) -> Result<(), CmdError>;
}

impl<F> BehaviorCommandHandler for F
where
    F: Fn(&mut Ctx<'_>, &BehaviorTarget) -> Result<(), CmdError> + Send + Sync,
{
    fn call(&self, ctx: &mut Ctx<'_>, target: &BehaviorTarget) -> Result<(), CmdError> {
        self(ctx, target)
    }
}

/// A contextual command a behavior contributes (design §4.3). `spec` is an
/// ordinary [`CommandSpec`] — switches, argument shape, role/power/lock, help,
/// and [`SceneClass`](crate::cmd::SceneClass) all apply exactly as for a global
/// command — but the verb is **never** in the global table: it exists only for
/// an actor who can reach an object carrying the behavior, and it can never
/// shadow a core verb, a staff command, or an exit (dispatch consults it only
/// after those have all missed).
#[derive(Clone)]
pub struct BehaviorCommand {
    /// The spec. Name and aliases match exactly (case-insensitively); there is
    /// no prefix matching at this stage.
    pub spec: CommandSpec,
    /// The reaches at which a subject may supply this command.
    pub reach: Vec<CommandReach>,
    /// The handler.
    pub handler: Arc<dyn BehaviorCommandHandler>,
}

impl BehaviorCommand {
    /// A command with the default reach set (everything except ambient).
    pub fn new(spec: CommandSpec, handler: Arc<dyn BehaviorCommandHandler>) -> BehaviorCommand {
        BehaviorCommand {
            spec,
            reach: CommandReach::default_set(),
            handler,
        }
    }

    /// Replace the reach set.
    #[must_use]
    pub fn with_reach(mut self, reach: Vec<CommandReach>) -> BehaviorCommand {
        self.reach = reach;
        self
    }
}

/// Why a behavior registration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BehaviorRegError {
    /// Another module already owns this behavior id.
    #[error("behavior `{id}` is already owned by module {owner}")]
    Owned {
        /// The behavior id.
        id: String,
        /// The current owner.
        owner: SmolStr,
    },
    /// A contextual command has an empty name.
    #[error("behavior `{id}` declares a command with an empty name")]
    EmptyVerb {
        /// The behavior id.
        id: String,
    },
    /// A contextual command verb or alias appears twice within the spec.
    #[error("behavior `{id}` declares the command verb {verb:?} twice")]
    DuplicateVerb {
        /// The behavior id.
        id: String,
        /// The duplicated verb (lowercased).
        verb: String,
    },
    /// A contextual command verb collides with an existing global command. A
    /// global verb always wins at dispatch, so the behavior verb would be dead
    /// — refusing here makes that loud at boot.
    #[error("behavior `{id}` command {verb:?} collides with the global command owned by {owner}")]
    VerbCollision {
        /// The behavior id.
        id: String,
        /// The colliding verb (lowercased).
        verb: String,
        /// Who owns the global command (`core` for built-ins).
        owner: SmolStr,
    },
}

/// The outcome of resolving a verb against an actor's reachable subjects.
#[derive(Clone, Debug)]
pub enum BehaviorResolution {
    /// No reachable subject supplies the verb.
    None,
    /// Exactly one candidate at the winning tier.
    One {
        /// The subject, behavior, and reach.
        target: BehaviorTarget,
        /// Index into the behavior spec's `commands`.
        index: usize,
    },
    /// Two or more candidates at the winning tier — nothing runs.
    Ambiguous(Vec<BehaviorTarget>),
}

/// Maximum behavior-bearing carried objects scanned at the Carried tier
/// (bounded work). Carried objects with no behaviors do not count toward it,
/// so a bound console is never hidden behind a pile of ordinary props.
pub const MAX_CARRIED_SCAN: usize = 64;

/// One reachable subject: every tier it was found at (highest first) and, for
/// a subject first reached through a link, that link's priority.
struct Candidate {
    subject: ObjId,
    /// Reaches this subject was found at, highest tier first.
    reaches: Vec<CommandReach>,
    /// The link priority when first reached as Operated — a sub-tier.
    priority: Option<u16>,
}

/// The ordering key for "the first tier with a hit wins": the subject's highest
/// reach, then (within Operated) the link priority, so higher-priority links
/// form their own sub-tier and ambiguity arises only among equals.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TierKey {
    reach: CommandReach,
    priority: Option<u16>,
}

fn push_candidate(cands: &mut Vec<Candidate>, id: ObjId, reach: CommandReach, priority: Option<u16>) {
    if let Some(c) = cands.iter_mut().find(|c| c.subject == id) {
        if !c.reaches.contains(&reach) {
            c.reaches.push(reach);
        }
    } else {
        cands.push(Candidate {
            subject: id,
            reaches: vec![reach],
            priority,
        });
    }
}

/// Resolve `verb` against the subjects `actor` can reach (design §4.3).
///
/// Tiers, highest first: Operated (published links — higher `priority` forms a
/// higher sub-tier), Actor, Container (the immediate non-room container),
/// Carried (one level; only behavior-bearing objects count toward
/// [`MAX_CARRIED_SCAN`]), then Ambient (the containing room) for commands that
/// opt in. A subject reachable at several tiers is ranked by its highest tier
/// but stays eligible for a command through *any* of its reaches — linking a
/// carried console does not silently remove its carried-only verbs — and the
/// highest reach the command accepts is what its handler sees. The first tier
/// with any match wins; two or more matches in that tier are ambiguous and
/// nothing runs — resolution never falls through to a lower tier. Matching is
/// exact on name or alias, case-insensitive with the same Unicode folding the
/// global table uses; there is no prefix matching (the global table owns prefix
/// semantics, and this stage only ever sees verbs it did not resolve).
///
/// Read-only over the world, so the scene turn-gate can classify a behavior
/// verb's [`SceneClass`](crate::cmd::SceneClass) with it before dispatch.
///
/// Leak guard: any subject other than the actor must be perceivable by the
/// actor (`can_perceive`), so a concealed carried, ambient, or remotely linked
/// object cannot answer. `GOING` husks and bindings whose behavior is
/// unregistered are inert.
#[must_use]
pub fn resolve_behavior_command(
    reg: &Registry,
    w: &World,
    actor: ObjId,
    verb: &str,
) -> BehaviorResolution {
    if verb.is_empty() {
        return BehaviorResolution::None;
    }
    let Some(actor_obj) = w.get(actor) else {
        return BehaviorResolution::None;
    };

    // Candidates in tier order; a repeated subject keeps its first (highest)
    // position and accumulates the lower reaches it was also found at.
    let mut candidates: Vec<Candidate> = Vec::new();
    for link in w.command_subjects_of(actor) {
        push_candidate(&mut candidates, link.subject, CommandReach::Operated, Some(link.priority));
    }
    push_candidate(&mut candidates, actor, CommandReach::Actor, None);
    if let Some(loc) = actor_obj.location
        && w.get(loc).is_some_and(|o| o.kind != ObjKind::Room)
    {
        push_candidate(&mut candidates, loc, CommandReach::Container, None);
    }
    let carried = w
        .contents_of(actor)
        .iter()
        .copied()
        .filter(|&c| w.get(c).is_some_and(|o| !o.behaviors.is_empty()))
        .take(MAX_CARRIED_SCAN);
    for c in carried {
        push_candidate(&mut candidates, c, CommandReach::Carried, None);
    }
    if let Some(room) = w.containing_room(actor) {
        push_candidate(&mut candidates, room, CommandReach::Ambient, None);
    }

    let verb_lc = verb.to_lowercase();
    let mut current: Option<TierKey> = None;
    let mut hits: Vec<(BehaviorTarget, usize)> = Vec::new();
    for cand in &candidates {
        let key = TierKey {
            reach: cand.reaches.first().copied().unwrap_or(CommandReach::Actor),
            priority: cand.priority,
        };
        if let Some(k) = current
            && k != key
            && !hits.is_empty()
        {
            break;
        }
        current = Some(key);
        let subject = cand.subject;
        let Some(obj) = w.get(subject) else {
            continue;
        };
        if obj.flags.contains(Flags::GOING) {
            continue;
        }
        if subject != actor && !can_perceive(reg, w, actor, subject) {
            continue;
        }
        for bid in obj.behaviors.keys() {
            let Some(spec) = reg.behavior_spec(bid) else {
                continue;
            };
            for (i, cmd) in spec.commands.iter().enumerate() {
                // Eligible through any reach this subject was found at; the
                // handler sees the highest one the command accepts.
                let Some(&reach) = cand.reaches.iter().find(|r| cmd.reach.contains(r)) else {
                    continue;
                };
                let matches = cmd.spec.name.to_lowercase() == verb_lc
                    || cmd.spec.aliases.iter().any(|a| a.to_lowercase() == verb_lc);
                if matches {
                    hits.push((
                        BehaviorTarget {
                            subject,
                            behavior: bid.clone(),
                            reach,
                        },
                        i,
                    ));
                }
            }
        }
    }

    if hits.is_empty() {
        return BehaviorResolution::None;
    }
    if hits.len() == 1 {
        let (target, index) = hits.remove(0);
        return BehaviorResolution::One { target, index };
    }
    BehaviorResolution::Ambiguous(hits.into_iter().map(|(t, _)| t).collect())
}

// ----------------------------------------------------------------------
// The pre-destroy veto seam (D8)
// ----------------------------------------------------------------------

/// A pre-destroy veto (design D8) for structural dependencies a static
/// [`DestroyPolicy`] cannot express — a domain host that still has units on it,
/// an occupied control station. Registered via
/// [`Registrar::destroy_gate`](crate::module::Registrar::destroy_gate) and
/// consulted at the command layer by [`Registry::check_destroy`], read-only over
/// the world (like a [`CommandGate`](crate::cmd::CommandGate)).
pub trait DestroyGate: Send + Sync {
    /// Return `Err(reason)` to refuse destroying `target`; `reason` is shown to
    /// `actor`.
    fn check(&self, w: &World, reg: &Registry, actor: ObjId, target: ObjId)
    -> Result<(), String>;
}

/// A pre-detach veto: consulted by `@behavior/remove` before a binding is
/// detached, so a module can refuse stripping a behavior its own records still
/// depend on (a live domain's `sim.domain`, a placed unit's `sim.participant`).
/// Without it the documented `@behavior/remove` escape hatch could orphan live
/// structural state. Read-only over the world; `World::detach_behavior` itself
/// never consults it. Registered via
/// [`Registrar::detach_gate`](crate::module::Registrar::detach_gate).
pub trait DetachGate: Send + Sync {
    /// Return `Err(reason)` to refuse removing `behavior` from `obj`.
    fn check(
        &self,
        w: &World,
        reg: &Registry,
        actor: ObjId,
        obj: ObjId,
        behavior: &BehaviorId,
    ) -> Result<(), String>;
}

// ----------------------------------------------------------------------
// Commands: @behavior/add|remove|list
// ----------------------------------------------------------------------

fn behavior_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("list") {
        return behavior_list(ctx);
    }
    let add = ctx.switches.has("add");
    let remove = ctx.switches.has("remove");
    if add == remove {
        return Err(CmdError::msg(
            "Usage: @behavior/add <object>=<id>, @behavior/remove <object>=<id>, \
             @behavior/list <object>",
        ));
    }

    let target = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let id = BehaviorId::new(ctx.args.rhs().unwrap_or("").trim())
        .map_err(|e| CmdError::msg(e.to_string()))?;

    // Mutating a binding needs control and a live object (refuses a GOING husk,
    // like @set/@desc/@name).
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("Permission denied."));
    }
    // The behavior's own role gate applies only when it is registered. An
    // unregistered binding (its owning module gone or renamed) can still be
    // *removed* by a controller to clean up orphaned cruft, so the spec is
    // optional here and required only on the add path.
    let spec = ctx.reg.behavior_spec(&id).cloned();
    if let Some(spec) = &spec
        && !perm::has_role(ctx.w, ctx.actor, spec.min_role)
    {
        return Err(CmdError::msg("Permission denied."));
    }

    if add {
        let spec = spec.ok_or_else(|| CmdError::msg(format!("No such behavior: `{id}`.")))?;
        // Read everything needed from the object before the mutable borrow.
        let (kind, already, conflict) = {
            let obj = ctx.w.get(target).ok_or_else(|| CmdError::msg("No such object."))?;
            let conflict = spec
                .conflicts
                .iter()
                .find(|&c| obj.behaviors.contains_key(c))
                .cloned()
                .or_else(|| {
                    // Symmetric: an existing behavior may forbid this one.
                    obj.behaviors
                        .keys()
                        .find(|&held| {
                            ctx.reg
                                .behavior_spec(held)
                                .is_some_and(|s| s.conflicts.contains(&id))
                        })
                        .cloned()
                });
            (obj.kind, obj.behaviors.contains_key(&id), conflict)
        };
        if already {
            return Err(CmdError::msg(format!("That object already has `{id}`.")));
        }
        if !spec.allowed_kinds.is_empty() && !spec.allowed_kinds.contains(&kind) {
            return Err(CmdError::msg(format!(
                "`{id}` cannot attach to that kind of object."
            )));
        }
        if let Some(c) = conflict {
            return Err(CmdError::msg(format!(
                "`{id}` conflicts with `{c}` already on that object."
            )));
        }
        let binding = BehaviorBinding {
            schema_version: BINDING_SCHEMA_VERSION,
            attached_by: ctx.actor,
            attached_at: ctx.w.now(),
        };
        ctx.w
            .attach_behavior(target, id.clone(), binding)
            .map_err(|e| CmdError::msg(e.to_string()))?;
        ctx.out.info(ctx.actor, format!("Attached `{id}`."));
    } else {
        // A module may refuse stripping a binding its records still depend on
        // (a live domain's `sim.domain`), so the escape hatch stays honest.
        if let Err(reason) = ctx.reg.check_detach(ctx.w, ctx.actor, target, &id) {
            return Err(CmdError::msg(reason));
        }
        let removed = ctx
            .w
            .detach_behavior(target, &id)
            .map_err(|e| CmdError::msg(e.to_string()))?;
        if !removed {
            return Err(CmdError::msg(format!("That object does not carry `{id}`.")));
        }
        ctx.out.info(ctx.actor, format!("Removed `{id}`."));
    }
    Ok(())
}

fn behavior_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    if !perm::controls(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg("Permission denied."));
    }
    let lines: Vec<String> = {
        let obj = ctx.w.get(target).ok_or_else(|| CmdError::msg("No such object."))?;
        obj.behaviors
            .iter()
            .map(|(id, b)| {
                let tag = if ctx.reg.behavior_spec(id).is_some() {
                    ""
                } else {
                    " [unregistered]"
                };
                format!("  {id}{tag}  (by {}, v{})", b.attached_by, b.schema_version)
            })
            .collect()
    };
    if lines.is_empty() {
        ctx.out.info(ctx.actor, "That object carries no behaviors.");
        return Ok(());
    }
    ctx.out.info(ctx.actor, "Behaviors:");
    for line in lines {
        ctx.out.info(ctx.actor, line);
    }
    Ok(())
}

/// Register the `@behavior` command. Called from
/// [`register_core_commands`](crate::commands::register_core_commands).
pub(crate) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@behavior".into(),
            switches: ["add", "remove", "list"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            shape: ArgShape::EqPair,
            min_role: Role::Builder,
            help: "@behavior/add <object>=<id>  --  Attach a typed behavior \
                   (e.g. `btech.mech`) to an object you control. \
                   @behavior/remove <object>=<id> detaches it. \
                   @behavior/list <object> shows what an object carries. \
                   Behaviors are registered by modules; some require higher rank \
                   or forbid certain object kinds, and some refuse @destroy while \
                   attached (remove them first)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(behavior_cmd),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_id_validation() {
        for ok in ["sim.domain", "btech.mech", "a.b.c", "x9.y_z"] {
            assert!(BehaviorId::new(ok).is_ok(), "{ok} should be valid");
        }
        for bad in ["", "nodot", "sim.", ".domain", "Sim.Domain", "sim domain", "sim..d", "a-b.c"] {
            assert!(BehaviorId::new(bad).is_err(), "{bad:?} should be invalid");
        }
        assert_eq!(BehaviorId::new("sim.domain").unwrap().namespace(), "sim");
    }

    #[test]
    fn behavior_id_length_bound() {
        let long = format!("a.{}", "b".repeat(BEHAVIOR_ID_MAX));
        assert!(matches!(
            BehaviorId::new(long),
            Err(BehaviorIdError::TooLong(_))
        ));
    }
}
