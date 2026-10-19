//! One line-command adapter for the workshop. Remote transports call the same
//! services after authenticating their selected character; they do not dispatch
//! arbitrary MUSH text or provide revision validators.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_core::module::{
    ContextOutcome, EventHook, HookPoint, InputContext, ModError, Registrar, TickCadence,
};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, ParsedArgs, World};
use mush_proto::{InputMode, PromptSpec, RichText};
use mush_roll::{EditBase, PreparedChange, RollError, RollStore};

use super::executor::{JobKind, Principal, WorkshopExecutor};
use super::{CardEdit, MindRef, TextEdit, invalid};
use crate::config::ArenaConfig;
use crate::store::TacsimStore;

const SWITCHES: &[&str] = &[
    "doctrine",
    "live",
    "new",
    "rename",
    "remove",
    "version-name",
    "version-forget",
    "prune",
    "list",
    "info",
    "source",
    "paste",
    "line",
    "insert",
    "delete",
    "check",
    "save",
    "card",
    "set",
    "preset",
    "profile",
    "rule",
    "localcopy",
    "fork",
    "diff",
    "publish",
    "prove",
    "proves",
    "proving",
    "report",
    "cancel",
    "forget",
    "trace",
];
const HELP: &str = "@mind/doctrine [Line|Sentry|Sniper|Skirmisher|Hunter|Rearguard] (free teaching pages)\n@mind/new <name> [FROM Standard|<owned mind>]\n@mind/list; @mind/info <name>; @mind/source <name>[@rev] [from-to]\n@mind/rename <name>=<new name>; @mind/remove <name>\n@mind/version-name <name>@<old>=<new>; @mind/version-forget <name>@<version>\n@mind/prune [1..128] (unreferenced owned revisions; sources retained)\n@mind/paste <name>; @mind/line|insert <name> <line>=<text>; @mind/delete <name> <from-to>\n@mind/check <name>; @mind/save <name>=<note>\n@mind/card <name>[/<profile>]; @mind/set <name>/<profile> <key>=<stored value>\n@mind/profile <name> add <profile> [FROM <profile>] | del <profile> | rename <old>=<new>\n@mind/preset <name>/<profile> save|apply <preset>; @mind/preset <name> del <preset>\n@mind/rule <name>/<profile> <n>=<rule>|OFF (canonical weapon IDs; ALL binds at seal)\n@mind/localcopy <owned mind>[@rev]=<new name>; @mind/diff <owned mind>[@rev]=<owned mind>[@rev]\n@mind/prove <owned mind[@revision][/profile]|Standard[/preset]> VS <same|mirror> [10..100 even] [DUEL|MUSTER|HOLD] [1v1..6v6] [SEED <u64>] (JSON-quote references containing spaces)\n@mind/proves; @mind/proving <run>; @mind/report <run> [page]; @mind/cancel|forget <run>\n@mind/trace #<unit> [1..64] [FOLLOW|ONCE]\n@mind/live <unit>[=<preset> | =<slider> <value>] (staff) reads the card as checked out or stages a live edit.";

#[derive(Clone)]
pub(crate) struct WorkshopCommands {
    pub(super) executor: WorkshopExecutor,
    pub(super) remote: Arc<Mutex<super::structured::RemoteJobs>>,
    pub(super) authored: Arc<super::authored_jobs::Jobs>,
    cfg: ArenaConfig,
    pub(super) floor_permille: u16,
    /// Paste starts at depth zero and never pushes another context. This set
    /// identifies its bottom stack entry for disconnect cleanup.
    pasting: Arc<Mutex<BTreeSet<ObjId>>>,
}

impl WorkshopCommands {
    pub(crate) fn new(executor: WorkshopExecutor, cfg: ArenaConfig, floor_permille: u16) -> Self {
        Self {
            executor,
            remote: Arc::new(Mutex::new(super::structured::RemoteJobs::default())),
            authored: Arc::new(super::authored_jobs::Jobs::default()),
            cfg,
            floor_permille,
            pasting: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Replace the old @mind registration with this one; `/live` is the only
    /// route to the staff setter. Author `/set` never reaches arena mutation.
    pub(crate) fn register(self: &Arc<Self>, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(super::authored_jobs::WorkshopService(Arc::downgrade(self)));
        reg.invoke_handler("mind", self.clone())?;
        let command = self.clone();
        reg.command(
            CommandSpec {
                name: "@mind".into(),
                shape: ArgShape::Raw,
                switches: SWITCHES.iter().map(|word| (*word).into()).collect(),
                help: HELP.into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| command.dispatch(ctx)),
        )?;
        let poll = self.clone();
        reg.tick(
            "mind-author-results",
            TickCadence::Coarse(Duration::from_secs(1)),
            Arc::new(
                move |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| poll.poll(ctx),
            ),
        )?;
        reg.hook(HookPoint::Linkdead, self.clone());
        reg.hook(HookPoint::Disconnected, self.clone());
        Ok(())
    }

    pub(super) fn principal(&self, ctx: &Ctx<'_>) -> Result<Principal, RollError> {
        if !ctx.w.has_live_conns(ctx.actor) {
            return Err(RollError::Missing);
        }
        let account = ctx
            .w
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(ctx.actor))
            .ok_or(RollError::Missing)?;
        self.executor.principal(ctx.actor, account)
    }

    pub(crate) fn dispatch(&self, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
        let switches = ctx.switches.iter().map(str::to_owned).collect::<Vec<_>>();
        let [switch] = switches.as_slice() else {
            return Err(CmdError::msg(HELP));
        };
        if switch == "doctrine" {
            let page = crate::minds::doctrine_library::page(ctx.args.raw(), &self.cfg.reflex)?;
            ctx.out.info(ctx.actor, page);
            return Ok(());
        }
        if switch == "trace" {
            let text = ctx.args.raw().to_owned();
            return crate::minds::trace_feed::command(ctx, &text);
        }
        if switch == "live" {
            if !(mush_core::perm::has_power(ctx.w, ctx.actor, mush_core::Powers::SIM_ADMIN)
                || mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty))
            {
                return Err(CmdError::msg("Permission denied."));
            }
            let parsed = ParsedArgs::parse(ArgShape::EqPair, ctx.args.raw());
            let old = std::mem::replace(&mut ctx.args, parsed);
            let result = crate::match_::mind_set(ctx, &self.cfg);
            ctx.args = old;
            return result;
        }
        let switch = switch.clone();
        let text = ctx.args.raw().to_owned();
        let principal = self.principal(ctx).map_err(command_error)?;
        self.author(ctx, principal, &switch, &text)
            .map_err(command_error)
    }

    fn author(
        &self,
        ctx: &mut Ctx<'_>,
        principal: Principal,
        switch: &str,
        text: &str,
    ) -> Result<(), RollError> {
        let actor = principal.actor();
        match switch {
            "list" => {
                let names = roll(ctx)?
                    .names(actor)
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                ctx.out.info(
                    actor,
                    if names.is_empty() {
                        "No authored minds.".into()
                    } else {
                        names.join(", ")
                    },
                );
            }
            "new" => {
                let (name, from) = split_from(text);
                let prepared = super::create(roll(ctx)?, sources(ctx)?, actor, name, from)?;
                publish(ctx, prepared)?;
                ctx.out.info(
                    actor,
                    "Mind draft created. Use @mind/source, @mind/paste and @mind/check.",
                );
            }
            "rename" | "remove" | "version-name" | "version-forget" => {
                let (name, edit) = lifecycle_edit(switch, text)?;
                let base = roll(ctx)?.mind(actor, &name)?.base();
                if matches!(edit, mush_proto::MindLifecycle::Remove {})
                    && super::retirement::request(ctx, &name, Some(base), None)?.is_some()
                {
                    ctx.out.info(actor, "Draft removal requested. Keep the entry until the server confirms; refresh your library to check.");
                    return Ok(());
                }
                let prepared = super::lifecycle(roll(ctx)?, actor, &name, base, &edit)?;
                publish(ctx, prepared)?;
                ctx.out.info(actor, match edit {
                    mush_proto::MindLifecycle::Remove {} =>
                        "Working library entry removed. Named versions, recent history and external references remain retained.",
                    _ => "Library lifecycle change saved.",
                });
            }
            "prune" => {
                let limit = if text.trim().is_empty() {
                    128
                } else {
                    text.trim()
                        .parse::<u16>()
                        .map_err(|_| invalid("prune", "expected a batch size from 1 through 128"))?
                };
                let (revisions, retained) = prune(ctx, limit)?;
                ctx.out.info(actor, format!(
                    "Pruned {revisions} unreferenced owned revisions; retained {retained} candidate source blobs. Named versions, recent history and external references remain protected."
                ));
            }
            "localcopy" | "fork" => {
                let (from, name) = pair(text)?;
                let prepared = super::create(
                    roll(ctx)?,
                    sources(ctx)?,
                    actor,
                    name.trim(),
                    Some(from.trim()),
                )?;
                publish(ctx, prepared)?;
                ctx.out.info(actor, "Owned draft copied.");
            }
            "info" => {
                let view = super::authored_access::View::current_source(ctx, text, None)?;
                let mind = view.mind();
                let line = format!(
                    "{}: {} bytes; class {}; edit {}; revision {}; {} named versions.",
                    mind.name(),
                    mind.draft().source.len(),
                    mind.draft().envelope.declared_class,
                    mind.base().edit,
                    mind.base()
                        .revision
                        .map_or_else(|| "unsaved".into(), |number| number.to_string()),
                    mind.named().len()
                );
                view.emit(ctx, vec![line])?;
            }
            "source" => {
                let (reference, range) = text
                    .rsplit_once(' ')
                    .filter(|(_, range)| parse_range(range).is_ok())
                    .map_or((text, (1, 200)), |(name, range)| {
                        (name, parse_range(range).unwrap_or((1, 200)))
                    });
                if let Some(licence) = reference.strip_prefix("filed:") {
                    return super::filed_source::line(ctx, licence, range.0, range.1);
                }
                if reference.eq_ignore_ascii_case("Standard") {
                    for line in
                        super::source_lines(crate::minds::content::STANDARD_RHAI, range.0, range.1)?
                    {
                        ctx.out.info(actor, line);
                    }
                } else {
                    let view = super::authored_access::View::source(
                        ctx,
                        &MindRef::parse(reference)?,
                        None,
                    )?;
                    let lines = super::source_lines(&view.copy().source, range.0, range.1)?;
                    view.emit(ctx, lines)?;
                }
            }
            "diff" => {
                let (left, right) = pair(text)?;
                super::authored_access::diff(
                    ctx,
                    &MindRef::parse(left.trim())?,
                    &MindRef::parse(right.trim())?,
                )?;
            }
            "line" | "insert" | "delete" => {
                let (left, value) = if switch == "delete" {
                    (text, "")
                } else {
                    pair(text)?
                };
                let (name, number) = left
                    .trim_end()
                    .rsplit_once(' ')
                    .ok_or_else(|| invalid("edit", "expected name and line or range"))?;
                let edit = if switch == "delete" {
                    let (first, last) = parse_range(number)?;
                    TextEdit::Delete { first, last }
                } else if switch == "insert" {
                    TextEdit::Insert {
                        line: number
                            .parse()
                            .map_err(|_| invalid("line", "expected whole line number"))?,
                        text: value.to_owned(),
                    }
                } else {
                    TextEdit::Replace {
                        line: number
                            .parse()
                            .map_err(|_| invalid("line", "expected whole line number"))?,
                        text: value.to_owned(),
                    }
                };
                let base = roll(ctx)?.mind(actor, name)?.base();
                super::authored_access::edit_text(ctx, name, base, edit, None)?;
                ctx.out.info(actor, "Working source updated.");
            }
            "paste" => {
                if ctx.w.input_depth(actor) != 0 {
                    return Err(invalid("paste", "finish the current input first"));
                }
                let mind = roll(ctx)?.mind(actor, text)?;
                let name = mind.name().to_owned();
                let base = mind.base();
                let marked = if mind.require_original_access().is_ok() {
                    None
                } else {
                    Some(super::authored_access::PasteSession::capture(
                        ctx, &name, base,
                    )?)
                };
                let paste = Paste {
                    service: self.clone(),
                    principal,
                    name,
                    base,
                    marked,
                    buffer: String::new(),
                };
                self.pasting
                    .lock()
                    .map_err(|_| invalid("paste", "editor unavailable"))?
                    .insert(actor);
                ctx.push_input_context(Box::new(paste));
            }
            "check" | "save" => {
                let (name, notes) = if switch == "save" {
                    pair(text)?
                } else {
                    (text, "")
                };
                let (name, version) = if switch == "save" {
                    name.trim()
                        .split_once('@')
                        .map_or((name.trim(), None), |(name, version)| {
                            (name, Some(version.to_owned()))
                        })
                } else {
                    (name.trim(), None)
                };
                let base = roll(ctx)?.mind(actor, name.trim())?.base();
                let kind = if switch == "save" {
                    JobKind::Save {
                        notes: notes.to_owned(),
                        version,
                    }
                } else {
                    JobKind::Check
                };
                if roll(ctx)?
                    .mind(actor, name)?
                    .require_original_access()
                    .is_ok()
                {
                    let id = self
                        .executor
                        .submit(principal, roll(ctx)?, name, base, kind)?;
                    ctx.out
                        .info(actor, format!("Authoring job {} queued.", id.number()));
                } else {
                    let id = self.submit_authored(
                        ctx,
                        principal,
                        name,
                        base,
                        kind,
                        super::authored_jobs::Delivery::Line,
                    )?;
                    self.authored_queued(ctx, id);
                }
            }
            "card" => {
                let (name, profile) = text
                    .split_once('/')
                    .map_or((text, None), |(name, profile)| (name, Some(profile)));
                let view = super::authored_access::View::current_source(ctx, name, None)?;
                let card = super::decoded(&view.copy().envelope)?;
                view.emit(ctx, super::card_lines(&card, profile)?)?;
            }
            "set" | "preset" | "profile" | "rule" => {
                let (name, edit) = parse_card_edit(switch, text)?;
                let base = roll(ctx)?.mind(actor, &name)?.base();
                super::authored_access::edit_card(
                    ctx,
                    &name,
                    base,
                    edit,
                    self.floor_permille,
                    None,
                )?;
                ctx.out.info(actor, "Working card updated.");
            }
            "publish" => {
                return Err(invalid(
                    "publish",
                    "commercial mind publication is not available yet",
                ));
            }
            "prove" | "proves" | "proving" | "report" | "cancel" | "forget" => {
                crate::minds::bulk::world::command(ctx, switch, text)
                    .map_err(|e| invalid("proving", e.to_string()))?;
            }
            _ => return Err(invalid("@mind", HELP)),
        }
        Ok(())
    }

    fn poll(&self, ctx: &mut Ctx<'_>) {
        self.poll_authored(ctx);
        self.poll_remote(ctx);
        let caller = ctx.actor;
        for (actor, id) in self.executor.ready() {
            if self.is_remote(id) || self.is_authored(id) {
                continue;
            }
            ctx.actor = actor;
            let result = (|| {
                let principal = self.principal(ctx)?;
                let Some(completion) = self.executor.take(principal, id)? else {
                    return Ok(());
                };
                completion.check_current(principal, roll(ctx)?)?;
                if let Some(diagnostic) = completion.diagnostic {
                    ctx.out.error(actor, diagnostic_words(&diagnostic));
                    return Ok(());
                }
                match completion.kind {
                    JobKind::Check => {
                        for report in completion.reports {
                            ctx.out.info(
                                actor,
                                format!(
                                    "{:?} {}: ops {}; fees {}; {} intents; fault {}.",
                                    report.picture,
                                    if report.armed { "armed" } else { "unarmed" },
                                    report.operations,
                                    report.fees,
                                    report.intents.len(),
                                    report.fault.as_deref().unwrap_or("none")
                                ),
                            );
                            for line in report
                                .intents
                                .into_iter()
                                .chain(report.station_refusals)
                                .chain(report.class_violations)
                            {
                                ctx.out.info(actor, line);
                            }
                        }
                    }
                    JobKind::Save { notes, version } => {
                        let receipt = completion
                            .validated
                            .ok_or_else(|| invalid("save", "validation receipt missing"))?;
                        let prepared = receipt.prepare_save(
                            principal,
                            roll(ctx)?,
                            sources(ctx)?,
                            &notes,
                            version.as_deref(),
                        )?;
                        let reference = prepared.revision();
                        let line = format!(
                            "Saved revision {} ({})",
                            reference.key.number,
                            reference.digest.to_hex()
                        );
                        // FINAL fallible action: both owners publish before this
                        // returns. Nothing below can discard its deferred batch.
                        crate::store::publish_mind_revision(ctx, prepared)?;
                        ctx.out.info(actor, line);
                        crate::match_::tutorial::hint(
                            ctx,
                            actor,
                            crate::match_::tutorial::HintPoint::MindSaved,
                        );
                    }
                }
                Ok::<_, RollError>(())
            })();
            if let Err(error) = result {
                ctx.out.error(actor, error.to_string());
            }
        }
        ctx.actor = caller;
    }

    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        self.executor.cancel_character(actor);
        self.cancel_authored_character(actor);
        let had_paste = self
            .pasting
            .lock()
            .ok()
            .is_some_and(|mut pasting| pasting.remove(&actor));
        if had_paste {
            let mut stack = Vec::new();
            while let Some(context) = ctx.w.pop_input_context(actor) {
                stack.push(context);
            }
            // The paste was installed at depth zero. Preserve later contexts
            // from other modules; drop this editor before any reconnect input.
            if stack
                .last()
                .is_some_and(|context| context.module.as_deref() == Some(crate::store::NS))
            {
                stack.pop();
            }
            for context in stack.into_iter().rev() {
                ctx.w.push_input_context(actor, context.module, context.cx);
            }
        }
    }
}

impl EventHook for WorkshopCommands {
    fn linkdead(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        self.disconnected(ctx, actor);
    }
    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        WorkshopCommands::disconnected(self, ctx, actor);
    }
}

struct Paste {
    service: WorkshopCommands,
    principal: Principal,
    name: String,
    base: EditBase,
    marked: Option<super::authored_access::PasteSession>,
    buffer: String,
}
impl Drop for Paste {
    fn drop(&mut self) {
        if let Ok(mut pasting) = self.service.pasting.lock() {
            pasting.remove(&self.principal.actor());
        }
    }
}
impl InputContext for Paste {
    fn prompt(&self, _world: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain(
                "mind paste (lone . saves working text; ~q cancels)> ",
            )),
        }
    }
    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        if self.service.principal(ctx).ok() != Some(self.principal) {
            if let Some(session) = &self.marked {
                session.emit(ctx, "Paste unavailable.");
            } else {
                ctx.out
                    .error(ctx.actor, "Paste cancelled; session changed.");
            }
            return ContextOutcome::Pop;
        }
        if let Some(session) = &mut self.marked {
            if line == "." {
                let result = session.replace(ctx, &self.name, std::mem::take(&mut self.buffer));
                session.emit(
                    ctx,
                    if result.is_ok() {
                        "Working source replaced; use @mind/check."
                    } else {
                        "Paste unavailable."
                    },
                );
                return ContextOutcome::Pop;
            }
            if session.check(ctx, &self.name).is_err() {
                session.emit(ctx, "Paste unavailable.");
                return ContextOutcome::Pop;
            }
        }
        if line == "." {
            let result = roll(ctx)
                .and_then(|roll| {
                    super::edit_text(
                        roll,
                        ctx.actor,
                        &self.name,
                        self.base,
                        TextEdit::ReplaceAll(std::mem::take(&mut self.buffer)),
                    )
                })
                .and_then(|prepared| publish(ctx, prepared));
            match result {
                Ok(()) => ctx
                    .out
                    .info(ctx.actor, "Working source replaced; use @mind/check."),
                Err(error) => ctx.out.error(ctx.actor, error.to_string()),
            }
            return ContextOutcome::Pop;
        }
        if line.contains('\0')
            || self
                .buffer
                .len()
                .saturating_add(line.len())
                .saturating_add(1)
                > mush_roll::MAX_SOURCE_BYTES
        {
            if let Some(session) = &self.marked {
                session.emit(ctx, "Paste cancelled: maximum 32 KiB; NUL is forbidden.");
            } else {
                ctx.out.error(
                    ctx.actor,
                    "Paste cancelled: maximum 32 KiB; NUL is forbidden.",
                );
            }
            return ContextOutcome::Pop;
        }
        self.buffer.push_str(line);
        self.buffer.push('\n');
        ContextOutcome::Consumed
    }
}

pub(super) fn roll<'a>(ctx: &'a Ctx<'_>) -> Result<&'a RollStore, RollError> {
    ctx.w.module_state::<RollStore>().ok_or(RollError::Missing)
}
pub(super) fn sources<'a>(ctx: &'a Ctx<'_>) -> Result<&'a TacsimStore, RollError> {
    ctx.w
        .module_state::<TacsimStore>()
        .ok_or(RollError::Missing)
}
fn command_error(error: RollError) -> CmdError {
    CmdError::msg(error.to_string())
}

fn diagnostic_words(diagnostic: &super::executor::Diagnostic) -> String {
    let location = match (diagnostic.line, diagnostic.column) {
        (Some(line), Some(column)) => format!(" at line {line}, column {column}"),
        (Some(line), None) => format!(" at line {line}"),
        (None, Some(column)) => format!(" at column {column}"),
        (None, None) => String::new(),
    };
    format!("{}{location}: {}", diagnostic.field, diagnostic.message)
}

/// Same final-action rule as combined publication: preflight namespace and all
/// serialized writes before applying the owner; output afterward is infallible.
pub(super) fn publish(ctx: &mut Ctx<'_>, prepared: PreparedChange) -> Result<(), RollError> {
    if ctx.module.as_deref() != Some(crate::store::NS)
        || ctx
            .reg
            .persist_delegation(crate::store::NS, mush_roll::NS)
            .is_none()
    {
        return Err(invalid("workshop", "persistence delegation unavailable"));
    }
    let mut operations = Vec::new();
    ctx.w
        .module_state_mut::<RollStore>()
        .ok_or(RollError::Missing)?
        .publish_with(prepared, |writes| operations.extend_from_slice(writes))?;
    for operation in operations {
        ctx.persist(operation);
    }
    Ok(())
}

/// This is revision collection, not source garbage collection. The source
/// owner has no complete index of historical seals/content, so candidate blobs
/// are deliberately retained. All Roll roots, including other modules' exact
/// references, are checked before one serialized bounded publication.
pub(super) fn prune(ctx: &mut Ctx<'_>, limit: u16) -> Result<(u16, u16), RollError> {
    let prepared = roll(ctx)?
        .prepare_prune_owned(ctx.actor, usize::from(limit))
        .map_err(prune_refusal)?;
    let revisions = u16::try_from(prepared.removed.len())
        .map_err(|_| invalid("prune", "revision count exceeded the batch bound"))?;
    // Never expose the collector's global-dedup candidate set: it would reveal
    // whether another character retains identical source bytes. Report only
    // unique source identities already owned by these removed revisions.
    let owned_sources = prepared
        .removed
        .iter()
        .map(|revision| {
            roll(ctx)?
                .revision(ctx.actor, revision.key)
                .map(|revision| revision.source())
        })
        .collect::<Result<BTreeSet<_>, RollError>>()?;
    let retained = u16::try_from(owned_sources.len())
        .map_err(|_| invalid("prune", "source count exceeded the batch bound"))?;
    publish(ctx, prepared.change)?;
    Ok((revisions, retained))
}

fn prune_refusal(error: RollError) -> RollError {
    match error {
        // Global quarantine must block collection, but its record key is not
        // this character's information to read through a maintenance command.
        RollError::Quarantined(_) => invalid(
            "prune",
            "Collection unavailable while retained records need repair.",
        ),
        other => other,
    }
}

fn lifecycle_edit(
    switch: &str,
    text: &str,
) -> Result<(String, mush_proto::MindLifecycle), RollError> {
    use mush_proto::MindLifecycle;
    match switch {
        "rename" => {
            let (name, new_name) = pair(text)?;
            Ok((
                mush_roll::canonical_name(name.trim())?,
                MindLifecycle::Rename {
                    new_name: new_name.trim().into(),
                },
            ))
        }
        "remove" => Ok((
            mush_roll::canonical_name(text.trim())?,
            MindLifecycle::Remove {},
        )),
        "version-name" | "version-forget" => {
            let (reference, new) = if switch == "version-name" {
                let (reference, new) = pair(text)?;
                (reference, Some(new.trim()))
            } else {
                (text, None)
            };
            let reference = MindRef::parse(reference.trim())?;
            let old = reference
                .revision
                .ok_or_else(|| invalid("version", "expected <name>@<version>"))?;
            let edit = match new {
                Some(new) => MindLifecycle::VersionRename {
                    old,
                    new: new.into(),
                },
                None => MindLifecycle::VersionForget { version: old },
            };
            Ok((reference.name, edit))
        }
        _ => Err(invalid("lifecycle", "unknown operation")),
    }
}

fn pair(text: &str) -> Result<(&str, &str), RollError> {
    text.split_once('=')
        .ok_or_else(|| invalid("command", "expected ="))
}
fn split_from(text: &str) -> (&str, Option<&str>) {
    let upper = text.to_ascii_uppercase();
    upper.find(" FROM ").map_or((text, None), |index| {
        (text.get(..index).unwrap_or(text), text.get(index + 6..))
    })
}
fn parse_range(text: &str) -> Result<(usize, usize), RollError> {
    let (first, last) = text.split_once('-').unwrap_or((text, text));
    let first = first
        .parse::<usize>()
        .map_err(|_| invalid("range", "expected whole line numbers"))?;
    let last = last
        .parse::<usize>()
        .map_err(|_| invalid("range", "expected whole line numbers"))?;
    if first == 0 || last < first {
        return Err(invalid("range", "invalid source range"));
    }
    Ok((first, last))
}

fn parse_card_edit(switch: &str, text: &str) -> Result<(String, CardEdit), RollError> {
    match switch {
        "set" | "rule" => {
            let (left, value) = pair(text)?;
            let (target, key) = left
                .trim_end()
                .rsplit_once(' ')
                .ok_or_else(|| invalid("card", "expected name/profile and field or rule number"))?;
            let (name, profile) = target
                .split_once('/')
                .ok_or_else(|| invalid("card", "expected name/profile"))?;
            let edit = if switch == "set" {
                CardEdit::Set {
                    profile: profile.into(),
                    field: key.into(),
                    value: value.trim().into(),
                }
            } else {
                CardEdit::Rule {
                    profile: profile.into(),
                    slot: key
                        .parse()
                        .map_err(|_| invalid("rule", "expected slot number"))?,
                    text: value.trim().into(),
                }
            };
            Ok((name.into(), edit))
        }
        "preset" => {
            let (left, preset) = text
                .rsplit_once(' ')
                .ok_or_else(|| invalid("preset", "expected preset name"))?;
            let (target, action) = left
                .rsplit_once(' ')
                .ok_or_else(|| invalid("preset", "expected save, apply or del"))?;
            if action == "del" {
                return Ok((
                    target.into(),
                    CardEdit::PresetDelete {
                        preset: preset.into(),
                    },
                ));
            }
            let (name, profile) = target
                .split_once('/')
                .ok_or_else(|| invalid("preset", "expected name/profile"))?;
            let edit = match action {
                "save" => CardEdit::PresetSave {
                    profile: profile.into(),
                    preset: preset.into(),
                },
                "apply" => CardEdit::PresetApply {
                    profile: profile.into(),
                    preset: preset.into(),
                },
                _ => return Err(invalid("preset", "expected save or apply")),
            };
            Ok((name.into(), edit))
        }
        "profile" => {
            for (marker, action) in [(" add ", "add"), (" del ", "del"), (" rename ", "rename")] {
                if let Some((name, tail)) = text.rsplit_once(marker) {
                    let edit = match action {
                        "add" => {
                            let (name, from) = split_from(tail);
                            CardEdit::ProfileAdd {
                                name: name.into(),
                                from: from.map(str::to_owned),
                            }
                        }
                        "del" => CardEdit::ProfileDelete { name: tail.into() },
                        _ => {
                            let (old, new) = pair(tail)?;
                            CardEdit::ProfileRename {
                                old: old.trim().into(),
                                new: new.trim().into(),
                            }
                        }
                    };
                    return Ok((name.into(), edit));
                }
            }
            Err(invalid("profile", "expected name add|del|rename profile"))
        }
        _ => Err(invalid("card", "unknown edit")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_diagnostics_preserve_only_actual_parser_coordinates() {
        let mut diagnostic = super::super::executor::Diagnostic {
            field: "compile".into(),
            message: "Unexpected token".into(),
            line: Some(12),
            column: Some(7),
        };
        assert_eq!(
            diagnostic_words(&diagnostic),
            "compile at line 12, column 7: Unexpected token"
        );
        diagnostic.column = None;
        assert_eq!(
            diagnostic_words(&diagnostic),
            "compile at line 12: Unexpected token"
        );
        diagnostic.line = None;
        assert_eq!(diagnostic_words(&diagnostic), "compile: Unexpected token");
    }
    #[test]
    fn global_quarantine_refuses_prune_without_disclosing_foreign_record_keys() {
        let first = prune_refusal(RollError::Quarantined("library:987".into()));
        let second = prune_refusal(RollError::Quarantined("ref:private-owner:sealed-42".into()));
        assert_eq!(first, second);
        assert!(!first.to_string().contains("987"));
        assert!(first.to_string().contains("unavailable"));
    }
    #[test]
    fn lifecycle_grammar_is_distinct_from_source_and_proving_deletion() {
        use mush_proto::MindLifecycle;
        assert_eq!(
            lifecycle_edit("remove", " long mind ").unwrap(),
            ("long mind".into(), MindLifecycle::Remove {})
        );
        assert_eq!(
            lifecycle_edit("rename", "long mind=next mind").unwrap(),
            (
                "long mind".into(),
                MindLifecycle::Rename {
                    new_name: "next mind".into()
                }
            )
        );
        assert_eq!(
            lifecycle_edit("version-name", "long mind@release one=release two").unwrap(),
            (
                "long mind".into(),
                MindLifecycle::VersionRename {
                    old: "release one".into(),
                    new: "release two".into()
                }
            )
        );
        assert_eq!(
            lifecycle_edit("version-forget", "long mind@release two").unwrap(),
            (
                "long mind".into(),
                MindLifecycle::VersionForget {
                    version: "release two".into()
                }
            )
        );
        for (operation, text) in [
            ("delete", "mind 1-2"),
            ("forget", "1"),
            ("version-forget", "mind"),
            ("version-name", "mind@one@two=three"),
            ("rename", "mind"),
        ] {
            assert!(lifecycle_edit(operation, text).is_err());
        }
    }
    #[test]
    fn authored_set_and_live_have_distinct_syntax() {
        assert!(parse_card_edit("set", "#77=aggressive").is_err());
        assert_eq!(
            parse_card_edit("set", "defender/Guard hold_m=450").unwrap(),
            (
                "defender".into(),
                CardEdit::Set {
                    profile: "Guard".into(),
                    field: "hold_m".into(),
                    value: "450".into()
                }
            )
        );
        assert!(parse_card_edit("rule", "defender/Guard 1=FIRE cannon WHEN hit within 2s").is_ok());
    }
}
