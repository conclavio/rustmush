//! Private readback from actual committed owner history. A session notice is
//! not a business receipt and may repeat after restart, eviction or rollback.
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId,
    families::CommerceFamilies,
    rating::RatingFamilies,
    read::CommerceRead,
    records::{Record, RevisionPolicy},
    roots::{RequiredRoot, RootAnchor},
};
use super::*;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, Ctx, Flags, ModuleCtx, ObjKind,
    Registrar, StateFamilyHandle, TickCadence, World,
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

const PAGE: usize = 64;
const EMISSIONS: usize = 16;
// A full holding proof can reconstruct its immutable rating pages. Bound this
// work independently of emitted notices, including already-seen/live holdings.
const HOLDING_PROOFS: usize = 16;
const REMEMBERED: usize = 4096;
const COMMAND: &str = "@mindrecall";
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Notice {
    actor: ObjId,
    licence: LicenceId,
    filed: FiledRevisionId,
    operation: CommitToken,
}
impl Notice {
    fn text(self) -> String {
        format!(
            "Registry recall: your mind licence {} selects filed revision {}, recalled by operation {}. New use of that revision is unavailable; running sorties retain their captured mind.",
            self.licence.get(),
            self.filed.get(),
            self.operation
        )
    }
}
#[derive(Clone, Default)]
pub(crate) struct Readback(Arc<OnceLock<DefinitionRegistry>>);
impl Readback {
    pub(crate) fn enable(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| error("recall readback definitions"))?;
        check_definitions(defs).map_err(error)?;
        self.0
            .set(defs.clone())
            .map_err(|_| error("recall readback already enabled"))
    }
    fn check(&self, world: &World) -> Result<(), PublicationError> {
        check_definitions(self.0.get().ok_or(PublicationError::Unavailable)?)?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        if world
            .module_state::<mush_economy::EconomyStore>()
            .is_none_or(|s| s.is_frozen())
        {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}
fn check_definitions(defs: &DefinitionRegistry) -> Result<(), PublicationError> {
    for owner in [crate::NS, "economy"] {
        defs.require_owner_ready(&OwnerId::new(owner).map_err(bad)?)
            .map_err(bad)?;
    }
    Ok(())
}
#[derive(Default)]
struct Runtime {
    owner_after: Option<String>,
    paid_after: Option<String>,
    paid_first: bool,
    seen: BTreeSet<Notice>,
    fifo: VecDeque<Notice>,
}
impl Runtime {
    fn remember(&mut self, n: Notice) {
        if self.seen.insert(n) {
            self.fifo.push_back(n);
        }
        while self.fifo.len() > REMEMBERED {
            if let Some(old) = self.fifo.pop_front() {
                self.seen.remove(&old);
            }
        }
    }
}
#[derive(Clone)]
struct Service {
    f: CommerceFamilies,
    ratings: RatingFamilies,
    ready: Readback,
    runtime: Arc<Mutex<Runtime>>,
}
fn unavailable() -> CmdError {
    CmdError::msg("Mind recall status is unavailable or not held.")
}
fn live(world: &World, actor: ObjId) -> bool {
    world
        .get(actor)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}
fn read<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    h: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, PublicationError> {
    super::super::entitlement::history::read(ctx, h, key)
}
/// Resolve only historical ownership and the actual selected revision. This
/// deliberately does not call a current-use/source reader, which must refuse
/// a recalled holding. It never supplies source or execution authority.
fn holding(
    ctx: &impl CommerceRead,
    s: &Service,
    actor: ObjId,
    licence: LicenceId,
) -> Result<(FiledRevisionId, CharterId, LineId), PublicationError> {
    if !live(ctx.world(), actor) {
        return Err(PublicationError::Missing);
    }
    let owner = ctx.get(&s.f.licences, &licence.key()).map_err(bad)?;
    let paid = ctx.get(&s.f.paid_licences, &licence.key()).map_err(bad)?;
    let (charter, line, policy) = match (owner, paid) {
        (Some(owner), None) => {
            owner.validate()?;
            if owner.id != licence {
                return Err(PublicationError::Mismatch);
            }
            super::super::owner_qualification::current_title(ctx.world(), actor, owner.charter)?;
            let roll = ctx
                .world()
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?;
            let term = super::super::owner_attestation::history::current(ctx, roll, &s.f, owner)?;
            (owner.charter, owner.line, term.revision)
        }
        (None, Some(paid)) => {
            paid.validate()?;
            if paid.id() != licence || paid.holder() != actor {
                return Err(PublicationError::Missing);
            }
            let term = read(
                ctx,
                &s.f.paid_terms,
                format!("{}/{}", licence.key(), paid.term().key()),
            )?;
            super::super::entitlement::history::current(ctx, &s.f, &s.ratings, paid, term)?;
            (paid.charter(), paid.line(), term.policy())
        }
        _ => return Err(PublicationError::Missing),
    };
    let selected = match policy {
        RevisionPolicy::Pin { filed, revision } => {
            let row = read(ctx, &s.f.filed, filed.key())?;
            if row.content.revision() != revision {
                return Err(PublicationError::Mismatch);
            }
            filed
        }
        RevisionPolicy::Follow { line: follow } if follow == line => {
            let row = read(ctx, &s.f.lines, line.key())?;
            if row.charter != charter {
                return Err(PublicationError::Mismatch);
            }
            row.head
        }
        _ => return Err(PublicationError::Mismatch),
    };
    Ok((selected, charter, line))
}
fn own(
    ctx: &impl CommerceRead,
    s: &Service,
    actor: ObjId,
    licence: LicenceId,
) -> Result<Option<Notice>, PublicationError> {
    s.ready.check(ctx.world())?;
    let (filed, charter, line) = holding(ctx, s, actor, licence)?;
    let row = read(ctx, &s.f.filed, filed.key())?;
    if row.content.position().charter != charter || row.content.position().line != line {
        return Err(PublicationError::Mismatch);
    }
    let roll = ctx
        .world()
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(filed),
        identity: row.content.revision(),
    })
    .map_err(bad)?;
    let status = read(ctx, &s.f.status, filed.key())?;
    if status.filed != filed {
        return Err(PublicationError::Mismatch);
    }
    let Some(operation) = status.recalled_by else {
        return Ok(None);
    };
    let cause = super::audit::cause_root(ctx, &s.f, &s.ratings, operation)?;
    if cause.identity != row.content.revision() {
        return Err(PublicationError::Mismatch);
    }
    roll.check_required_root(&cause).map_err(bad)?;
    Ok(Some(Notice {
        actor,
        licence,
        filed,
        operation,
    }))
}
fn command(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), CmdError> {
    let raw = ctx.args.raw();
    let number = raw.parse::<u64>().map_err(|_| unavailable())?;
    if number.to_string() != raw {
        return Err(unavailable());
    }
    let licence = LicenceId::new(number).map_err(|_| unavailable())?;
    let result = own(ctx, s, ctx.actor, licence).map_err(|_| unavailable())?;
    let text = result.map(Notice::text).unwrap_or_else(|| {
        "No Registry recall is recorded for your selected filed revision.".to_owned()
    });
    ctx.out.sys_to_actor(ctx.actor, crate::NS, text);
    Ok(())
}
#[derive(Clone, Copy)]
enum Inventory {
    Owners,
    Paid,
}
struct Page {
    rows: Vec<(String, LicenceId, Option<ObjId>)>,
    more: bool,
}
fn page(
    ctx: &Ctx<'_>,
    s: &Service,
    which: Inventory,
    after: Option<&str>,
) -> Result<Page, PublicationError> {
    match which {
        Inventory::Owners => {
            let p = ctx
                .scan_action_state(&s.f.licences, after, PAGE)
                .map_err(bad)?;
            let snapshot = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .ok_or(PublicationError::Unavailable)?
                .snapshot();
            let mut rows = Vec::with_capacity(p.entries().len());
            for entry in p.entries() {
                let row = entry.value();
                row.validate()?;
                let actor =
                    snapshot
                        .asset(row.charter.object())
                        .and_then(|a| match &a.title_owner {
                            mush_org::PrincipalId::Object(actor) => Some(*actor),
                            _ => None,
                        });
                let actor = actor.filter(|actor| {
                    super::super::owner_qualification::current_title(ctx.w, *actor, row.charter)
                        .is_ok()
                });
                rows.push((entry.encoded_key().to_string(), row.id, actor));
            }
            Ok(Page {
                rows,
                more: p.next_after().is_some(),
            })
        }
        Inventory::Paid => {
            let p = ctx
                .scan_action_state(&s.f.paid_licences, after, PAGE)
                .map_err(bad)?;
            let mut rows = Vec::with_capacity(p.entries().len());
            for entry in p.entries() {
                let row = entry.value();
                row.validate()?;
                rows.push((
                    entry.encoded_key().to_string(),
                    row.id(),
                    Some(row.holder()),
                ));
            }
            Ok(Page {
                rows,
                more: p.next_after().is_some(),
            })
        }
    }
}
/// Plans both bounded inventories before emitting anything. Unavailable
/// inventory never counts as empty and never advances the retained cursors.
fn tick(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), PublicationError> {
    s.ready.check(ctx.w)?;
    let mut runtime = s.runtime.lock().map_err(bad)?;
    let owners = page(ctx, s, Inventory::Owners, runtime.owner_after.as_deref())?;
    let paid = page(ctx, s, Inventory::Paid, runtime.paid_after.as_deref())?;
    let order = if runtime.paid_first {
        [(Inventory::Paid, paid), (Inventory::Owners, owners)]
    } else {
        [(Inventory::Owners, owners), (Inventory::Paid, paid)]
    };
    let mut notices = Vec::with_capacity(EMISSIONS);
    let mut proofs = 0;
    let mut cursors = [runtime.owner_after.clone(), runtime.paid_after.clone()];
    for (which, p) in order {
        let at = match which {
            Inventory::Owners => 0,
            Inventory::Paid => 1,
        };
        let mut exhausted = true;
        for (key, licence, actor) in p.rows {
            if notices.len() == EMISSIONS || proofs == HOLDING_PROOFS {
                exhausted = false;
                break;
            }
            if let Some(actor) = actor.filter(|a| ctx.w.is_connected(*a) && live(ctx.w, *a)) {
                // A no-longer-held title is not a recipient. Corrupt actual
                // history fails this drain before any private output is emitted.
                proofs += 1;
                match own(ctx, s, actor, licence) {
                    Ok(Some(n)) if !runtime.seen.contains(&n) => notices.push(n),
                    Ok(_) => {}
                    Err(e) => return Err(e),
                }
            }
            cursors[at] = Some(key);
        }
        if exhausted && !p.more {
            cursors[at] = None;
        }
    }
    runtime.owner_after = cursors[0].take();
    runtime.paid_after = cursors[1].take();
    runtime.paid_first = !runtime.paid_first;
    for notice in notices {
        ctx.out.sys_to_actor(notice.actor, crate::NS, notice.text());
        runtime.remember(notice);
    }
    Ok(())
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    ratings: RatingFamilies,
) -> Result<Readback, ModError> {
    let ready = Readback::default();
    let service = Service {
        f,
        ratings,
        ready: ready.clone(),
        runtime: Arc::new(Mutex::new(Runtime::default())),
    };
    let command_service = service.clone();
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(CommandSpec{name:COMMAND.into(),switches:vec!["status".into()],shape:ArgShape::Raw,
        help:"@mindrecall/status <your licence> — inspect Registry recall of your selected filed revision.".into(),..Default::default()},
        Arc::new(move|ctx:&mut Ctx<'_>|command(ctx,&command_service)))?;
    reg.tick(
        "registry_recall_notices",
        TickCadence::Coarse(Duration::from_secs(5)),
        Arc::new(
            move |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx, &service);
            },
        ),
    )?;
    Ok(ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recall_session_notice_memory_is_bounded_and_duplicate_identity_is_exact() {
        let mut runtime = Runtime::default();
        for n in 1..=(REMEMBERED + 1) {
            let notice = Notice {
                actor: ObjId(1),
                licence: LicenceId::new(n as u64).unwrap(),
                filed: FiledRevisionId::new(1).unwrap(),
                operation: CommitToken::from_bytes([1; 16]),
            };
            runtime.remember(notice);
            runtime.remember(notice);
            assert!(runtime.seen.len() <= REMEMBERED && runtime.fifo.len() <= REMEMBERED);
        }
        assert_eq!(runtime.seen.len(), REMEMBERED);
        assert_eq!(runtime.fifo.len(), REMEMBERED);
        assert!(!runtime.seen.iter().any(|n| n.licence.get() == 1));
        assert!(
            runtime
                .seen
                .iter()
                .any(|n| n.licence.get() == (REMEMBERED + 1) as u64)
        );
    }
}
