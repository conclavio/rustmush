//! Fresh marked-draft access. Source/edit views stay same-dispatch; asynchronous
//! jobs retain only evidence and resolve a new operation-specific read at completion.
use super::commands::{publish, roll};
use super::{CardEdit, MindRef, TextEdit};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ClockStamp, Ctx, DefinitionRef, ModError, ObjId, Registrar};
use mush_roll::commerce::authored_access::{
    AuthoredDraftAccess, AuthoredDraftReader, AuthoredDraftRequest, AuthoredJobKind,
};
use mush_roll::commerce::authored_history::{
    AuthoredSavedAccess, AuthoredSavedReader, AuthoredSavedRequest,
};
use mush_roll::{EditBase, Mind, RevisionKey, RevisionRef, RollError, SourceAdapter, WorkingCopy};

#[derive(Clone)]
struct DraftAccessSeam {
    reader: OwnerReadHandle<AuthoredDraftReader>,
    history: OwnerReadHandle<AuthoredSavedReader>,
}
pub(crate) fn register(reg: &mut Registrar<'_>, manifest: &DefinitionRef) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("authored draft consumer belongs to Tacsim"));
    }
    let reader = AuthoredDraftReader::request(reg, manifest)?;
    let history = AuthoredSavedReader::request(reg, manifest)?;
    reg.state(DraftAccessSeam { reader, history });
    super::authored_jobs::register(reg)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Source,
    Diff,
    Edit,
}
impl Use {
    fn request(self, mind: u64) -> AuthoredDraftRequest {
        match self {
            Self::Source | Self::Diff => AuthoredDraftRequest::source(mind),
            Self::Edit => AuthoredDraftRequest::working_edit(mind),
        }
    }
}
struct Binding {
    actor: ObjId,
    conn: u64,
    lease: u64,
}
impl Binding {
    fn capture(ctx: &Ctx<'_>, conn: Option<u64>) -> Result<Self, RollError> {
        let conn = conn
            .or_else(|| ctx.w.dispatching_conn())
            .ok_or(RollError::Missing)?;
        let (actor, lease) = ctx
            .w
            .connection_binding(conn)
            .filter(|(actor, _)| *actor == ctx.actor)
            .ok_or(RollError::Missing)?;
        Ok(Self { actor, conn, lease })
    }
    fn check(&self, ctx: &Ctx<'_>) -> Result<(), RollError> {
        if ctx.actor != self.actor
            || ctx.w.connection_binding(self.conn) != Some((self.actor, self.lease))
        {
            return Err(RollError::Missing);
        }
        Ok(())
    }
}
fn current(ctx: &Ctx<'_>, id: u64, use_: Use) -> Result<AuthoredDraftAccess, RollError> {
    let seam = ctx
        .w
        .module_state::<DraftAccessSeam>()
        .ok_or(RollError::Missing)?;
    ctx.hook_read_context(crate::NS)
        .read_owner(&seam.reader, use_.request(id))
        .map_err(|_| RollError::Missing)
}

fn current_saved(
    ctx: &Ctx<'_>,
    revision: RevisionRef,
    use_: Use,
) -> Result<AuthoredSavedAccess, RollError> {
    let seam = ctx
        .w
        .module_state::<DraftAccessSeam>()
        .ok_or(RollError::Missing)?;
    let request = match use_ {
        Use::Source => AuthoredSavedRequest::source(revision),
        Use::Diff => AuthoredSavedRequest::diff(revision),
        Use::Edit => return Err(RollError::Missing),
    };
    ctx.hook_read_context(crate::NS)
        .read_owner(&seam.history, request)
        .map_err(|_| RollError::Missing)
}
fn saved_reference(
    ctx: &Ctx<'_>,
    mind: &Mind,
    reference: &MindRef,
) -> Result<RevisionRef, RollError> {
    let selector = reference.revision.as_ref().ok_or(RollError::Missing)?;
    if let Some(digest) = selector.strip_prefix("sha:") {
        return roll(ctx)?.resolve_digest(ctx.actor, &reference.name, digest);
    }
    let number = selector
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .or_else(|| mind.named().get(&selector.to_ascii_lowercase()).copied())
        .ok_or(RollError::Missing)?;
    Ok(roll(ctx)?
        .revision(
            ctx.actor,
            RevisionKey {
                character: ctx.actor,
                mind: mind.id(),
                number,
            },
        )?
        .reference())
}
enum MarkedAccess {
    Draft(AuthoredDraftAccess),
    Saved(AuthoredSavedAccess),
}

pub(super) fn current_job(
    ctx: &Ctx<'_>,
    id: u64,
    kind: AuthoredJobKind,
) -> Result<AuthoredDraftAccess, RollError> {
    let seam = ctx
        .w
        .module_state::<DraftAccessSeam>()
        .ok_or(RollError::Missing)?;
    ctx.read_owner(&seam.reader, AuthoredDraftRequest::job(id, kind))
        .map_err(|_| RollError::Missing)
}

/// Own data plus an optional current marked observation, owned only by this
/// synchronous command. No executor, paste context, World slot or response keeps it.
pub(super) struct View {
    mind: Mind,
    copy: WorkingCopy,
    marked: Option<(MarkedAccess, Binding)>,
    use_: Use,
    actor: ObjId,
}
impl View {
    fn capture(
        ctx: &Ctx<'_>,
        reference: &MindRef,
        conn: Option<u64>,
        use_: Use,
    ) -> Result<Box<Self>, RollError> {
        let mind = roll(ctx)?.mind(ctx.actor, &reference.name)?;
        if use_ == Use::Edit && reference.revision.is_some() {
            return Err(RollError::Missing);
        }
        if mind.require_original_access().is_ok() {
            let copy = if reference.revision.is_none() {
                mind.draft().clone()
            } else {
                super::owned_copy(
                    roll(ctx)?,
                    super::commands::sources(ctx)?,
                    ctx.actor,
                    reference,
                )?
            };
            return Ok(Box::new(Self {
                mind: mind.clone(),
                copy,
                marked: None,
                use_,
                actor: ctx.actor,
            }));
        }
        // A marked refusal never falls back to original-only source access.
        let binding = Binding::capture(ctx, conn)?;
        let (access, copy) = if reference.revision.is_some() {
            if ctx.w.dispatching_conn() != Some(binding.conn) {
                return Err(RollError::Missing);
            }
            let exact = saved_reference(ctx, mind, reference)?;
            let access = current_saved(ctx, exact, use_)?;
            if access.actor() != ctx.actor
                || access.revision().reference() != exact
                || access.revision() != roll(ctx)?.revision(ctx.actor, exact.key)?
            {
                return Err(RollError::Missing);
            }
            let sources = super::commands::sources(ctx)?;
            access.revision().verify_source(sources)?;
            let copy = WorkingCopy {
                source: sources.read_source(access.revision().source())?.to_owned(),
                envelope: access.revision().envelope().clone(),
            };
            (MarkedAccess::Saved(access), copy)
        } else {
            let access = current(ctx, mind.id(), use_)?;
            if access.actor() != ctx.actor || access.mind() != mind {
                return Err(RollError::Missing);
            }
            let copy = access.mind().draft().clone();
            (MarkedAccess::Draft(access), copy)
        };
        Ok(Box::new(Self {
            mind: mind.clone(),
            copy,
            marked: Some((access, binding)),
            use_,
            actor: ctx.actor,
        }))
    }
    pub(super) fn source(
        ctx: &Ctx<'_>,
        reference: &MindRef,
        conn: Option<u64>,
    ) -> Result<Box<Self>, RollError> {
        Self::capture(ctx, reference, conn, Use::Source)
    }
    pub(super) fn current_source(
        ctx: &Ctx<'_>,
        name: &str,
        conn: Option<u64>,
    ) -> Result<Box<Self>, RollError> {
        let reference = MindRef {
            name: mush_roll::canonical_name(name)?,
            revision: None,
        };
        Self::source(ctx, &reference, conn)
    }
    pub(super) fn mind(&self) -> &Mind {
        &self.mind
    }
    pub(super) fn copy(&self) -> &WorkingCopy {
        &self.copy
    }
    pub(super) fn check(&self, ctx: &Ctx<'_>) -> Result<(), RollError> {
        if ctx.actor != self.actor || roll(ctx)?.mind(self.actor, self.mind.name())? != &self.mind {
            return Err(RollError::StaleEdit);
        }
        if let Some((access, binding)) = &self.marked {
            binding.check(ctx)?;
            // Resolve the actual selector, status, current own grant and clock
            // again. Comparing two data objects alone would not be authority.
            match access {
                MarkedAccess::Draft(access) => {
                    let fresh = current(ctx, self.mind.id(), self.use_)?;
                    if !access.same_observation(&fresh) {
                        return Err(RollError::Missing);
                    }
                }
                MarkedAccess::Saved(access) => {
                    if ctx.w.dispatching_conn() != Some(binding.conn) {
                        return Err(RollError::Missing);
                    }
                    let fresh = current_saved(ctx, access.revision().reference(), self.use_)?;
                    if !access.same_observation(&fresh) {
                        return Err(RollError::Missing);
                    }
                    let sources = super::commands::sources(ctx)?;
                    fresh.revision().verify_source(sources)?;
                    if sources.read_source(fresh.revision().source())? != self.copy.source
                        || fresh.revision().envelope() != &self.copy.envelope
                    {
                        return Err(RollError::Missing);
                    }
                }
            }
            binding.check(ctx)?;
        } else {
            self.mind.require_original_access()?;
        }
        Ok(())
    }
    pub(super) fn emit(&self, ctx: &mut Ctx<'_>, lines: Vec<String>) -> Result<(), RollError> {
        self.check(ctx)?;
        self.emit_checked(ctx, lines);
        Ok(())
    }
    fn emit_checked(&self, ctx: &mut Ctx<'_>, lines: Vec<String>) {
        for line in lines {
            if let Some((_, binding)) = &self.marked {
                ctx.out.to_conn(
                    binding.conn,
                    mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(line)),
                );
            } else {
                ctx.out.info(self.actor, line);
            }
        }
    }
    fn publish(
        self: Box<Self>,
        ctx: &mut Ctx<'_>,
        base: EditBase,
        draft: WorkingCopy,
    ) -> Result<(), RollError> {
        if self.use_ != Use::Edit || self.mind.base() != base {
            return Err(RollError::StaleEdit);
        }
        let prepared = roll(ctx)?.prepare_edit(self.actor, self.mind.name(), base, draft)?;
        self.check(ctx)?;
        // No await, nested Engine dispatch or callback occurs from current read
        // through pure preparation and this final owner install. Ctx exclusively
        // borrows World; prepared Roll generation/CAS is checked by publication.
        publish(ctx, prepared)
    }
}
/// A paste retains only physical origin and CAS/clock metadata. It does not
/// retain source, a parent observation, or permission to finish later.
pub(super) struct PasteSession {
    binding: Binding,
    mind: u64,
    base: EditBase,
    observed: ClockStamp,
}
impl PasteSession {
    pub(super) fn capture(ctx: &Ctx<'_>, name: &str, base: EditBase) -> Result<Self, RollError> {
        let view = View::capture(ctx, &MindRef::parse(name)?, None, Use::Edit)?;
        if view.marked.is_none() || view.mind.base() != base {
            return Err(RollError::Missing);
        }
        view.check(ctx)?;
        Ok(Self {
            binding: Binding::capture(ctx, None)?,
            mind: view.mind.id(),
            base,
            observed: ctx.w.world_ic_now(),
        })
    }
    fn current(&self, ctx: &Ctx<'_>, name: &str) -> Result<Box<View>, RollError> {
        self.binding.check(ctx)?;
        if ctx.w.dispatching_conn() != Some(self.binding.conn) {
            return Err(RollError::Missing);
        }
        let now = ctx.w.world_ic_now();
        if now.clock != self.observed.clock || now.at < self.observed.at {
            return Err(RollError::Missing);
        }
        let view = View::capture(
            ctx,
            &MindRef::parse(name)?,
            Some(self.binding.conn),
            Use::Edit,
        )?;
        if view.marked.is_none() || view.mind.id() != self.mind || view.mind.base() != self.base {
            return Err(RollError::StaleEdit);
        }
        view.check(ctx)?;
        Ok(view)
    }
    pub(super) fn check(&mut self, ctx: &Ctx<'_>, name: &str) -> Result<(), RollError> {
        self.current(ctx, name)?;
        self.observed = ctx.w.world_ic_now();
        Ok(())
    }
    pub(super) fn replace(
        &self,
        ctx: &mut Ctx<'_>,
        name: &str,
        source: String,
    ) -> Result<(), RollError> {
        let view = self.current(ctx, name)?;
        let mut draft = view.copy.clone();
        draft.source = super::apply_text(&draft.source, TextEdit::ReplaceAll(source))?;
        view.publish(ctx, self.base, draft)
    }
    pub(super) fn emit(&self, ctx: &mut Ctx<'_>, text: &str) {
        // Even source-free completion/refusal stays on the original physical
        // connection; another connection cannot take over an actor's editor.
        if self.binding.check(ctx).is_ok() && ctx.w.dispatching_conn() == Some(self.binding.conn) {
            ctx.out.to_conn(
                self.binding.conn,
                mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(text)),
            );
        }
    }
}

pub(super) fn edit_text(
    ctx: &mut Ctx<'_>,
    name: &str,
    base: EditBase,
    edit: TextEdit,
    conn: Option<u64>,
) -> Result<(), RollError> {
    let view = View::capture(ctx, &MindRef::parse(name)?, conn, Use::Edit)?;
    let mut draft = view.copy.clone();
    draft.source = super::apply_text(&draft.source, edit)?;
    view.publish(ctx, base, draft)
}
pub(super) fn edit_card(
    ctx: &mut Ctx<'_>,
    name: &str,
    base: EditBase,
    edit: CardEdit,
    floor: u16,
    conn: Option<u64>,
) -> Result<(), RollError> {
    // Keep the original editor's complete path intact; marked refusal never
    // falls back through it. Only the actual owned record chooses this branch.
    if roll(ctx)?
        .mind(ctx.actor, name)?
        .require_original_access()
        .is_ok()
    {
        let prepared = super::edit_card(roll(ctx)?, ctx.actor, name, base, edit, floor)?;
        return publish(ctx, prepared);
    }
    let view = View::capture(ctx, &MindRef::parse(name)?, conn, Use::Edit)?;
    let mut draft = view.copy.clone();
    super::apply_card_edit(&mut draft, edit, floor)?;
    view.publish(ctx, base, draft)
}
pub(super) fn replace(
    ctx: &mut Ctx<'_>,
    name: &str,
    base: EditBase,
    draft: WorkingCopy,
    conn: u64,
) -> Result<(), RollError> {
    View::capture(ctx, &MindRef::parse(name)?, Some(conn), Use::Edit)?.publish(ctx, base, draft)
}
pub(super) fn diff(ctx: &mut Ctx<'_>, left: &MindRef, right: &MindRef) -> Result<(), RollError> {
    let left = View::capture(ctx, left, None, Use::Diff)?;
    let right = View::capture(ctx, right, None, Use::Diff)?;
    let lines = super::diff(left.copy(), right.copy())?;
    left.check(ctx)?;
    right.check(ctx)?;
    // If either side is protected, every derived line uses that exact current
    // physical connection. Original-vs-original retains M6 actor output.
    if left.marked.is_some() {
        left.emit_checked(ctx, lines);
    } else {
        right.emit_checked(ctx, lines);
    }
    Ok(())
}

#[cfg(test)]
#[path = "authored_access_probe.rs"]
pub(in crate::minds) mod probe;
