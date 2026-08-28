//! Command specs, the registry, and the dispatch pipeline.
//!
//! Dispatch order is **exactly** treaty ruling 5, with the input-context
//! stack in front:
//!
//! 0. the actor's input-context stack (mail compose, chargen, consoles) —
//!    checked before everything else, with a hard `~q` escape
//! 1. single-char prefixes — `"` say, `:` pose, `;` semipose, `\` @emit
//! 2. the alias stage — registered [`AliasStage`] hooks (the comsys seam;
//!    empty in M0). **Prefix-expanded lines skip this stage**: the prefix
//!    character already settled the line's meaning, so a channel alias
//!    named `say`/`pose` can never capture `"hello`
//! 3. the `home` keyword
//! 4. exit match — room exits, then the room's zone's exits, then global
//!    exits on Limbo (the master room)
//! 5. the command table — longest-unambiguous-prefix for bare verbs,
//!    exact (case-insensitive) for `@`/`+` commands; `/switch` and arg
//!    shapes are table-driven from the [`CommandSpec`]
//! 6. `Huh?` — with a fuzzy (edit-distance) suggestion
//!
//! Handlers are `Arc<dyn CommandHandler>` trait objects (treaty ruling 4)
//! so modules can capture state; plain `fn` items work via the blanket impl.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use smol_str::SmolStr;

use mush_proto::OutputEvent;

use crate::commands::movement;
use crate::flags::{Powers, Role};
use crate::lock::{LockExpr, MAX_LOCK_DEPTH, eval_lock};
use crate::matchit::MatchErr;
use crate::module::{ContextOutcome, ModuleTables, restored_prompt};
use crate::obj::ObjId;
use crate::output::Outbox;
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
    /// Optional lock gate (treaty ruling 4), evaluated against the actor
    /// before the handler runs; failing it is "Permission denied.".
    pub lock: Option<LockExpr>,
    /// One-paragraph usage help, rendered by `help <cmd>`.
    pub help: String,
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
            lock: None,
            help: String::new(),
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
    /// Rewrite the line and continue dispatch with the result.
    Rewrite(String),
}

/// A pre-dispatch hook running after single-char prefix expansion and
/// before exit matching — the seam where comsys will hang per-player
/// channel aliases (`pub Hello`). Empty in M0.
///
/// **Multi-stage contract** (load-bearing once several modules register):
///
/// - Stages run in **registration order**; registration order is priority
///   order.
/// - The first stage returning [`AliasOutcome::Handled`] ends dispatch.
/// - The first stage returning [`AliasOutcome::Rewrite`] ends the *stage
///   loop*: later stages never see the line, the rewritten line is **not**
///   re-offered to any stage (no rewrite chains), and dispatch continues
///   with it at the `home`-keyword step. At most one rewrite per line.
pub trait AliasStage: Send + Sync {
    /// Inspect (and possibly consume or rewrite) an input line.
    fn try_alias(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> AliasOutcome;
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
    alias_stages: Vec<Arc<dyn AliasStage>>,
    /// Module registrations (hooks, ticks, observers, namespaces, …).
    pub(crate) mods: ModuleTables,
    /// Prose help topics by lowercased name (sorted for listings).
    help_topics: BTreeMap<SmolStr, HelpTopic>,
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
        self.register_tagged(spec, handler, None)
    }

    /// Register a command tagged with the registering module's identity
    /// (the [`Registrar`](crate::module::Registrar) path).
    pub(crate) fn register_tagged(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
        module: Option<SmolStr>,
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
        });
        for k in keys {
            self.index.insert(k, idx);
        }
        Ok(())
    }

    /// Append an alias-stage hook (runs in registration order).
    pub fn add_alias_stage(&mut self, stage: Arc<dyn AliasStage>) {
        self.alias_stages.push(stage);
    }

    /// Iterate all registered command specs (registration order).
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.iter().map(|e| &e.spec)
    }

    /// Find the spec registered under exactly this name or alias
    /// (case-insensitive) — used by `help`.
    #[must_use]
    pub fn spec_of(&self, name: &str) -> Option<&CommandSpec> {
        self.index
            .get(&name.to_lowercase())
            .map(|&i| &self.entries[i].spec)
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
            if !name.starts_with('@')
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
pub fn dispatch(reg: &Registry, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) {
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
                    dispatch(reg, w, actor, &cmd, out);
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

    // 2. Alias stage (comsys seam). Prefix-expanded lines skip it: the
    //    prefix character already settled what the line means (room
    //    speech), and letting an alias named "say"/"pose" capture the
    //    synthetic verb would reroute private speech onto a channel the
    //    player never named. A *typed* "say hello" still offers the alias
    //    stage first, per treaty ruling 5.
    if !prefix_expanded {
        for stage in &reg.alias_stages {
            match stage.try_alias(w, actor, &cur, out) {
                AliasOutcome::NotMine => {}
                AliasOutcome::Handled => return,
                AliasOutcome::Rewrite(s) => {
                    cur = s;
                    break;
                }
            }
        }
    }

    // 3. HOME keyword.
    if cur.trim().eq_ignore_ascii_case("home") {
        movement::go_home(reg, w, actor, out);
        return;
    }

    // 4. Exit match (room → zone → global) against the whole line.
    if let Some(exit) = movement::find_exit(w, actor, cur.trim()) {
        movement::traverse_exit(reg, w, actor, exit, out);
        return;
    }

    // 5. Command table.
    let (head, rest) = split_head(&cur);
    let mut parts = head.split('/');
    let verb = parts.next().unwrap_or("");
    let switch_names: Vec<&str> = parts.filter(|s| !s.is_empty()).collect();
    match reg.lookup_full(verb) {
        Err(LookupErr::NotFound) => huh(reg, actor, verb, out),
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
            for s in &switch_names {
                if !spec.switches.iter().any(|k| k.eq_ignore_ascii_case(s)) {
                    out.error(actor, format!("Unknown switch \"/{s}\" for {}.", spec.name));
                    return;
                }
            }
            if perm::role_of(w, actor) < spec.min_role {
                out.error(actor, "Permission denied.");
                return;
            }
            if let Some(p) = spec.power
                && !perm::has_power(w, actor, p)
            {
                out.error(actor, "Permission denied.");
                return;
            }
            if let Some(lock) = &spec.lock
                && !eval_lock(w, lock, actor, MAX_LOCK_DEPTH)
            {
                out.error(actor, "Permission denied.");
                return;
            }
            let args = ParsedArgs::parse(spec.shape, rest);
            let switches = SwitchSet::from_names(switch_names.iter().copied());
            // Snapshot the deferred queue so an Err can discard everything
            // this handler deferred: queued persistence ops must never
            // commit for a command that reported failure (world mutations
            // made before the error remain best-effort — handlers
            // validate-then-apply).
            let deferred_before = out.deferred().len();
            let result = {
                let mut ctx = Ctx {
                    w,
                    actor,
                    switches,
                    args,
                    out,
                    reg,
                    module,
                };
                handler.call(&mut ctx)
            };
            if let Err(e) = result {
                out.truncate_deferred(deferred_before);
                out.error(actor, e.to_string());
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
        dispatch(&reg, &mut w, ann, "sanctify", &mut out);
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
        dispatch(&reg, &mut w, GOD, "sanctify", &mut out);
        assert!(out.events().iter().any(
            |(_, e)| matches!(e, mush_proto::OutputEvent::Sys(t) if t.to_plain_text() == "Sanctified.")
        ));
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
