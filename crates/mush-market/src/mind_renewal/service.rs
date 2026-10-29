//! Retained, bounded continuations. The durable owner records are inventory;
//! missing acknowledgement is never absence, and no path automatically funds.
use super::player::{
    FrontAnswer, FrontStep, RenewalOfferTerms, RenewalPlayerOutcome, RenewalPlayerStep,
    RenewalState, RenewalStatusView, Submission, Switches, front_bound, front_line,
    front_player_step, front_request, front_submitted, outcome_line, parse_front,
    personal_status_line, sent_outcome,
};
use super::{Families, MindRenewalOperationV1, RenewalConfig};
use mush_core::action::{AuthorityGuard, AuthorityGuardContext, IdempotencyKeyText, PerceivedRef};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::read_service::OwnerReadHandle;
use mush_core::submission_observation::SubmissionObservation;
use mush_core::{
    ArgShape, CmdError, CommandCompatibility, CommandSpec, CommitToken, ContinuationHandle, Ctx,
    DefinitionDigest, DefinitionRef, Flags, GOD, ModError, ModuleCtx, ObjId, ObjKind, Registrar,
    TickCadence, World,
};
use mush_economy::{EconomyStore, managed_renewal::RenewalServiceCloseReason as Reason};
use mush_roll::commerce::{
    LicenceId, TermGeneration,
    renewal::{
        RenewalStateV1,
        protocol::Transition,
        reader::{RenewalInventoryEntry, RenewalReader, RenewalRequest},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
const COMMAND: &str = "@mindrenewal";
const MAX_ENTRIES: usize = 63; // plus one independently retained service close
const PAGE: usize = 16;
#[derive(Clone, Default)]
pub(super) struct ServiceConfig {
    pub runtime: Arc<Mutex<Runtime>>,
    /// 17g C2h (the lead's ruling R3): the module the `front` continuation
    /// is delegated to, set by the composition that composes the player
    /// market (TacSim's `MindCommerce::configure_market`, with TacSim's own
    /// namespace; this crate never spells it). `None`: the `front` switch
    /// exists and no other module can schedule it. It enters no definition
    /// descriptor (`RenewalConfig::declarations` and the policy descriptor
    /// read the manifest, the participants and the policy only).
    pub player_front: Option<OwnerId>,
}
impl std::fmt::Debug for ServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RenewalServiceConfig")
    }
}
#[derive(Default)]
pub(super) struct Runtime {
    pub enabled: bool,
    pub close: Option<CloseIntent>,
    close_observation: Option<SubmissionObservation>,
    after: Option<String>,
    due_after: Option<String>,
    rotation: Option<String>,
    entries: BTreeMap<String, Entry>,
}
#[derive(Clone)]
pub(super) struct CloseIntent {
    pub before: MindRenewalOperationV1,
    pub action: DefinitionRef,
    pub reason: Reason,
    pub key: IdempotencyKeyText,
}
impl CloseIntent {
    pub fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"operation":self.before.operation(),"reason":self.reason})
    }
    fn identity(&self) -> Result<CanonicalId, String> {
        let d=DefinitionDigest::for_canonical_value("market.renewal_service_intent.v1",&serde_json::json!({"before":self.before,"action":self.action,"reason":self.reason,"key":self.key})).map_err(|_|super::records::invalid())?;
        // The digest is already domain-separated and uses the full 64-byte
        // canonical identifier budget; a prefix would make every guard fail.
        CanonicalId::new(d.to_hex()).map_err(|_| super::records::invalid())
    }
    pub fn guard(&self) -> Result<AuthorityGuard, String> {
        Ok(AuthorityGuard::Capability {
            provider: OwnerId::new(crate::NS).map_err(|_| super::records::invalid())?,
            actor: GOD,
            capability: self.identity()?,
            generation: 1,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Due(LicenceId, TermGeneration, u64),
    Retry(LicenceId, TermGeneration, CommitToken),
    Pending(CommitToken),
}
struct Entry {
    actor: ObjId,
    selection: Selection,
    after: i64,
    attempt: Option<(DefinitionRef, SubmissionObservation)>,
}
#[derive(Clone)]
struct Service {
    config: RenewalConfig,
    families: Families,
    reader: OwnerReadHandle<RenewalReader>,
    continuation: ContinuationHandle,
    scan: ContinuationHandle,
    /// 17g C2h: the `front` continuation (the player front `+minds/renew`
    /// hands its step to; `player_front`).
    front: ContinuationHandle,
}
fn unavailable() -> CmdError {
    CmdError::msg("Renewal is unavailable or has changed.")
}
fn live(world: &World, actor: ObjId) -> bool {
    world
        .get(actor)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}
fn service(ctx: &Ctx<'_>) -> Result<Service, CmdError> {
    ctx.state::<Service>().cloned().ok_or_else(unavailable)
}
fn runtime(service: &Service) -> Result<&ServiceConfig, CmdError> {
    service.config.service.as_ref().ok_or_else(unavailable)
}
pub(super) fn close_reason(world: &World, row: &MindRenewalOperationV1) -> Option<Reason> {
    if !matches!(row.state(), RenewalStateV1::Drafted {}) || world.now() < row.created_at_utc() {
        return None;
    }
    let now = world.world_ic_now();
    if now.clock != row.created_at_ic().clock {
        Some(Reason::ClockInvalidated)
    } else if now.at >= row.offer().terms.expires_at.at {
        Some(Reason::DeadlineReached)
    } else {
        None
    }
}
pub(crate) fn guard_current(ctx: &AuthorityGuardContext<'_>, guard: &AuthorityGuard) -> bool {
    let Some(service) = ctx.module_state::<Service>() else {
        return false;
    };
    let Ok(config) = runtime(service) else {
        return false;
    };
    let Ok(runtime) = config.runtime.lock() else {
        return false;
    };
    let Some(intent) = runtime.close.as_ref().filter(|_| runtime.enabled) else {
        return false;
    };
    if intent.guard().ok().as_ref() != Some(guard)
        || !live(ctx.world(), GOD)
        || close_reason(ctx.world(), &intent.before) != Some(intent.reason)
    {
        return false;
    }
    if ctx
        .action_state(&service.families.operations, &intent.before.operation())
        .ok()
        .flatten()
        != Some(&intent.before)
    {
        return false;
    }
    let Some(e) = ctx.world().module_state::<EconomyStore>() else {
        return false;
    };
    e.renewal_audit()
        .ok()
        .and_then(|a| a.get(intent.before.operation()).ok())
        .is_some_and(|a| intent.before.verify_history(&a).is_ok())
}
pub(super) fn register(
    reg: &mut Registrar<'_>,
    config: &RenewalConfig,
    families: Families,
    reader: OwnerReadHandle<RenewalReader>,
) -> Result<(), ModError> {
    if config.service.is_none() {
        return Ok(());
    }
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(CommandSpec { name:COMMAND.into(),switches:["status","request","accept","decline","retry","continue","scan","front"].into_iter().map(Into::into).collect(),shape:ArgShape::Raw,
        help:"@mindrenewal/status <licence> — inspect your retained offer. /request asks for an offer; /accept explicitly pays it; /decline closes it; /retry requests a fresh offer after closure. Processing is acknowledged asynchronously.".into(),..CommandSpec::default() },Arc::new(command))?;
    let continuation = reg.command_continuation(COMMAND, &["continue"])?;
    let scan = reg.command_continuation(COMMAND, &["scan"])?;
    // 17g C2h (the lead's ruling R3): the player front, delegated to the one
    // module the configuration names. Undocumented to players (R5).
    let front = reg.command_continuation(COMMAND, &["front"])?;
    if let Some(grantee) = config
        .service
        .as_ref()
        .and_then(|s| s.player_front.as_ref())
    {
        reg.delegate_continuation(
            &front,
            grantee.as_str(),
            "17g C2h: +minds/renew hands the actor's own renewal step to the service",
        )?;
    }
    reg.state(Service {
        config: config.clone(),
        families,
        reader,
        continuation,
        scan,
        front,
    });
    reg.tick(
        "personal_mind_renewal_service",
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                let _ = tick(ctx);
            },
        ),
    )
}
pub(crate) fn enable(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    let Some(s) = ctx.state::<Service>() else {
        return Ok(());
    };
    let mut r = runtime(s)
        .map_err(super::config::error)?
        .runtime
        .lock()
        .map_err(|_| super::config::error("renewal runtime unavailable"))?;
    if r.enabled || r.close.is_some() || !r.entries.is_empty() {
        return Err(super::config::error("renewal service already enabled"));
    }
    r.enabled = true;
    Ok(())
}
fn checked(
    ctx: &Ctx<'_>,
    s: &Service,
    operation: CommitToken,
) -> Result<MindRenewalOperationV1, CmdError> {
    let row = ctx
        .action_state(&s.families.operations, &operation)
        .map_err(|_| unavailable())?
        .cloned()
        .ok_or_else(unavailable)?;
    let roll = ctx
        .read_owner(&s.reader, RenewalRequest::operation(operation))
        .and_then(|v| v.into_operation())
        .map_err(|_| unavailable())?;
    row.matches_roll(&roll).map_err(|_| unavailable())?;
    let e = ctx
        .w
        .module_state::<EconomyStore>()
        .ok_or_else(unavailable)?;
    row.verify_history(
        &e.renewal_audit()
            .map_err(|_| unavailable())?
            .get(operation)
            .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    let receipt = ctx
        .action_state(&s.families.receipts, &operation)
        .map_err(|_| unavailable())?;
    if row.state().pending() {
        if receipt.is_some() {
            return Err(unavailable());
        }
    } else {
        receipt
            .ok_or_else(unavailable)?
            .matches_operation(&row)
            .map_err(|_| unavailable())?;
    }
    Ok(row)
}
fn key(selection: Selection) -> String {
    match selection {
        Selection::Due(l, g, 1) => format!("d/{}/{}", l.get(), g.get()),
        Selection::Due(l, g, holder) => format!("d/{}/{}/{}", l.get(), g.get(), holder),
        Selection::Pending(op) => format!("p/{}", op.to_hex()),
        Selection::Retry(l, g, previous) => {
            format!("r/{}/{}/{}", l.get(), g.get(), previous.to_hex())
        }
    }
}
fn admit(r: &mut Runtime, selection: Selection, actor: ObjId) -> Result<(), CmdError> {
    let key = key(selection);
    if let Some(old) = r.entries.get(&key) {
        if old.actor != actor || old.selection != selection {
            return Err(unavailable());
        }
        return Ok(());
    }
    if r.entries.len() >= MAX_ENTRIES {
        return Err(unavailable());
    }
    r.entries.insert(
        key,
        Entry {
            actor,
            selection,
            after: 0,
            attempt: None,
        },
    );
    Ok(())
}
fn rotation_order(r: &mut Runtime) -> Vec<String> {
    let keys: Vec<_> = r.entries.keys().cloned().collect();
    let start = r
        .rotation
        .as_ref()
        .and_then(|last| keys.iter().position(|k| k > last))
        .unwrap_or(0);
    let order: Vec<_> = keys
        .iter()
        .cycle()
        .skip(start)
        .take(keys.len())
        .cloned()
        .collect();
    r.rotation = order.first().cloned();
    order
}
fn tick(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = service(ctx)?;
    let mut r = runtime(&s)?.runtime.lock().map_err(|_| unavailable())?;
    if !r.enabled {
        return Ok(());
    }
    // Discovery runs under the actual registered service actor, not the tick's
    // incidental context. Its handler rechecks that actor and retained state.
    if live(ctx.w, GOD) && !ctx.authoritative_pending(GOD) {
        let _ = ctx.schedule_continuation(&s.scan, GOD, "", 0);
    }
    r.entries
        .retain(|_, e| e.attempt.is_some() || live(ctx.w, e.actor));
    let now = ctx.w.now();
    let order = rotation_order(&mut r);
    let mut actors = BTreeSet::new();
    for k in order {
        let Some(e) = r.entries.get_mut(&k) else {
            continue;
        };
        if e.after > now
            || !live(ctx.w, e.actor)
            || ctx.authoritative_pending(e.actor)
            || !actors.insert(e.actor)
        {
            continue;
        }
        e.after = now.saturating_add(1);
        let _ = ctx.schedule_continuation(&s.continuation, e.actor, &k, 0);
    }
    Ok(())
}
fn scan(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), CmdError> {
    if ctx.actor != GOD || !ctx.args.raw().is_empty() {
        return Err(unavailable());
    }
    let mut r = runtime(s)?.runtime.lock().map_err(|_| unavailable())?;
    if !r.enabled {
        return Err(unavailable());
    }
    // Exact completion proof wins even when the original observation is lost.
    if let Some(intent) = r.close.as_ref()
        && checked(ctx, s, intent.before.operation()).is_ok_and(|v| !v.state().pending())
    {
        r.close = None;
        r.close_observation = None;
    }
    if r.close_observation
        .as_mut()
        .is_some_and(|o| o.take_terminal().is_some())
    {
        r.close = None;
        r.close_observation = None;
    }
    let page = ctx
        .scan_action_state(&s.families.operations, r.after.as_deref(), PAGE)
        .map_err(|_| unavailable())?;
    let rows: Vec<_> = page.entries().iter().map(|v| v.value().clone()).collect();
    r.after = page.next_after().map(ToString::to_string);
    for row in rows {
        if matches!(row.state(), RenewalStateV1::Funded { .. })
            && live(ctx.w, row.binding().holder().acting_character())
        {
            // M8c B3: the service runs an organisation's steps as the acting
            // character its binding recorded.
            let _ = admit(
                &mut r,
                Selection::Pending(row.operation()),
                row.binding().holder().acting_character(),
            );
        }
        if r.close.is_none()
            && let Some(reason) = close_reason(ctx.w, &row)
        {
            let exact = checked(ctx, s, row.operation())?;
            if exact != row {
                return Err(unavailable());
            }
            let action = s
                .config
                .service_action()
                .map_err(|_| unavailable())?
                .ok_or_else(unavailable)?;
            let key = IdempotencyKeyText::parse_canonical(format!(
                "renewal.close.{}.{}.{}",
                if reason == Reason::DeadlineReached {
                    "d"
                } else {
                    "c"
                },
                row.operation().to_hex(),
                action.digest().to_hex()
            ))
            .map_err(|_| unavailable())?;
            r.close = Some(CloseIntent {
                before: row,
                action,
                reason,
                key,
            });
        }
    }
    if r.close_observation.is_none()
        && !ctx.authoritative_pending(GOD)
        && let Some(intent) = r.close.clone()
    {
        // Retain the immutable intent before entering the existing selector
        // queue. A dropped/refused timer or Unknown never authorizes a new one.
        r.close_observation = Some(ctx.submit_authoritative_observed(
            intent.action.clone(),
            PerceivedRef::SelfActor,
            vec![],
            // The private offer fixes the exact retained operation/reason.
            // Selectors supply only public parameters; this schema is empty.
            serde_json::json!({}),
            intent.key,
        )?);
    }
    let inventory = ctx
        .read_owner(&s.reader, RenewalRequest::inventory(r.due_after.clone()))
        .and_then(|v| v.into_inventory())
        .map_err(|_| unavailable())?;
    r.due_after = inventory.next_after().map(ToString::to_string);
    for item in inventory.entries() {
        if !item.attempted()
            && ctx.w.world_ic_now().at >= item.term_end().get()
            && live(ctx.w, item.holder().acting_character())
        {
            let _ = admit(
                &mut r,
                Selection::Due(item.licence(), item.generation(), item.holder_generation()),
                item.holder().acting_character(),
            );
        }
    }
    Ok(())
}
fn own(ctx: &Ctx<'_>, s: &Service, licence: LicenceId) -> Result<RenewalInventoryEntry, CmdError> {
    let list = ctx
        .read_owner(&s.reader, RenewalRequest::status(licence))
        .and_then(|v| v.into_inventory())
        .map_err(|_| unavailable())?;
    let item = list.into_single().map_err(|_| unavailable())?;
    // M8c B3: a character acting for the holding organisation owns its
    // renewal (`org_holder::acts_as`); a character holder must be the actor.
    if mush_roll::commerce::org_holder::acts_as(ctx.w, item.holder(), ctx.actor).is_err() {
        return Err(unavailable());
    }
    Ok(item)
}
fn action(s: &Service, t: Transition) -> Result<DefinitionRef, CmdError> {
    s.config
        .actions()
        .map_err(|_| unavailable())?
        .into_iter()
        .find(|a| a.key().id.as_str() == t.id())
        .ok_or_else(unavailable)
}
/// Submit `t` for the retained entry `e`. 17g C2h (the C2e plan §5.1,
/// review AD-F5): `Submission::Nothing` when nothing was sent, because the
/// entry still awaits an earlier attempt's terminal or the actor already has
/// an authoritative submission awaiting its receipt
/// (`Submission::may_send`); `Submission::Sent` after the one submission.
fn submit(
    ctx: &mut Ctx<'_>,
    s: &Service,
    e: &mut Entry,
    t: Transition,
    parameters: serde_json::Value,
) -> Result<Submission, CmdError> {
    if !Submission::may_send(e.attempt.is_some(), ctx.authoritative_pending(ctx.actor)) {
        return Ok(Submission::Nothing);
    }
    let action = action(s, t)?;
    let digest = DefinitionDigest::for_canonical_value(
        "market.renewal_submission.v1",
        &serde_json::json!({"actor":ctx.actor,"action":action,"parameters":parameters,"selection":key(e.selection)}),
    )
    .map_err(|_| unavailable())?;
    let key = IdempotencyKeyText::parse_canonical(format!("renewal.{}", digest.to_hex()))
        .map_err(|_| unavailable())?;
    let observation = ctx.submit_authoritative_observed(
        action.clone(),
        PerceivedRef::SelfActor,
        vec![],
        parameters,
        key,
    )?;
    e.attempt = Some((action, observation));
    e.after = ctx.w.now().saturating_add(1);
    Ok(Submission::Sent)
}
fn continuation(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), CmdError> {
    let id = ctx.args.raw().to_owned();
    let mut r = runtime(s)?.runtime.lock().map_err(|_| unavailable())?;
    if !r.enabled {
        return Err(unavailable());
    }
    let Some(e) = r.entries.get_mut(&id) else {
        return Err(unavailable());
    };
    if e.actor != ctx.actor || !live(ctx.w, ctx.actor) {
        return Err(unavailable());
    }
    let complete = match e.selection {
        Selection::Due(l, g, holder) => own(ctx, s, l)
            .is_ok_and(|v| v.generation() != g || v.holder_generation() != holder || v.attempted()),
        Selection::Pending(op) => checked(ctx, s, op).is_ok_and(|v| !v.state().pending()),
        Selection::Retry(l, g, previous) => own(ctx, s, l)
            .is_ok_and(|v| v.generation() != g || v.last_operation() != Some(previous)),
    };
    if complete {
        r.entries.remove(&id);
        return Ok(());
    }
    // An actual funded row proves this exact acceptance completed even when
    // its original observation remains Unknown; it can now advance to Grant.
    if let Selection::Pending(op) = e.selection
        && e.attempt
            .as_ref()
            .is_some_and(|(a, _)| a.key().id.as_str() == Transition::Fund.id())
        && checked(ctx, s, op).is_ok_and(|v| matches!(v.state(), RenewalStateV1::Funded { .. }))
    {
        e.attempt = None;
    }
    if let Some((_, observation)) = &mut e.attempt {
        if observation.take_terminal().is_none() {
            return Ok(());
        }
        e.attempt = None;
    }
    if ctx.authoritative_pending(ctx.actor) {
        return Ok(());
    }
    match e.selection {
        Selection::Due(l, g, holder) => {
            let v = own(ctx, s, l)?;
            if v.generation() != g
                || v.holder_generation() != holder
                || v.attempted()
                || ctx.w.world_ic_now().at < v.term_end().get()
            {
                return Err(unavailable());
            }
            // Nothing sent is retried on a later tick by this entry.
            let _sent = submit(
                ctx,
                s,
                e,
                Transition::Draft,
                serde_json::json!({"licence":l.get()}),
            )?;
        }
        Selection::Retry(l, g, previous) => {
            let v = own(ctx, s, l)?;
            if v.generation() != g || v.last_operation() != Some(previous) {
                return Err(unavailable());
            }
            let prior = checked(ctx, s, previous)?;
            if !prior.state().retryable() {
                return Err(unavailable());
            }
            let _sent = submit(
                ctx,
                s,
                e,
                Transition::Draft,
                serde_json::json!({"previous_operation":previous}),
            )?;
        }
        Selection::Pending(op) => {
            let row = checked(ctx, s, op)?;
            if !matches!(row.state(), RenewalStateV1::Funded { .. }) {
                r.entries.remove(&id);
                return Ok(());
            }
            if ctx.w.now() < row.created_at_utc() {
                return Ok(());
            }
            let now = ctx.w.world_ic_now();
            let cleanup = now.clock != row.created_at_ic().clock
                || now.at >= row.offer().terms.expires_at.at
                || row.manifest() != &s.config.manifest
                || row.participants() != &s.config.participants.ordered();
            if !cleanup {
                // Generic missing/failed owner reads retain funded custody.
                let Ok(facts) = ctx
                    .read_owner(&s.reader, RenewalRequest::own(row.binding().licence()))
                    .and_then(|v| v.into_facts())
                else {
                    return Ok(());
                };
                if facts.binding() != row.binding() {
                    return Ok(());
                }
            }
            let _sent = submit(
                ctx,
                s,
                e,
                if cleanup {
                    Transition::Refund
                } else {
                    Transition::Grant
                },
                serde_json::json!({"operation":op}),
            )?;
        }
    }
    Ok(())
}
fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let s = service(ctx)?;
    if ctx.switches.has("scan") {
        return scan(ctx, &s);
    }
    if ctx.switches.has("front") {
        return front(ctx, &s);
    }
    if ctx.switches.has("continue") {
        let result = continuation(ctx, &s);
        if result.is_err() {
            let mut r = runtime(&s)?.runtime.lock().map_err(|_| unavailable())?;
            let id = ctx.args.raw();
            if r.entries
                .get(id)
                .is_some_and(|e| e.actor == ctx.actor && e.attempt.is_none())
            {
                r.entries.remove(id);
            }
        }
        return result;
    }
    let text = ctx.args.raw();
    let n = text.parse::<u64>().map_err(|_| unavailable())?;
    if n.to_string() != text {
        return Err(unavailable());
    }
    let licence = LicenceId::new(n).map_err(|_| unavailable())?;
    // 17g C2d: the player branch is `step`, shared with `player_step`; this
    // command keeps its own switches and prints exactly its pre-C2d lines.
    let outcome = step(ctx, &s, licence, Switches::of_command(&ctx.switches))?;
    ctx.out
        .sys_to_actor(ctx.actor, "market", command_line(outcome));
    Ok(())
}

/// `@mindrenewal`'s one line for a step's outcome (the 17g C2d test 1 compares it
/// with the pre-C2d sentences).
pub(super) fn command_line(outcome: RenewalPlayerOutcome) -> String {
    outcome_line(outcome, personal_status_line)
}

/// 17g C2d (C2 plan §2.1's renewal exception): one player step on the
/// actor's own personal renewal (C2d's front for `+minds/renew`). It is the
/// `@mindrenewal` command's own player branch (`step`): the renewal reader's
/// holder check, the service's admission, retained entry, idempotency key and
/// submission, unchanged; a funded offer is still granted or refunded by the
/// service's continuation. Every refusal is the service's one uniform line,
/// `Renewal is unavailable or has changed.`, for a foreign licence and a
/// missing one alike.
///
/// 17g C2h: it reads through the market's own renewal read and families, so
/// it answers only in the market's own context; `+minds/renew` reaches the
/// same `step` through the service's delegated `front` continuation
/// (`player_front`), which runs under the market's identity.
pub fn player_step(
    ctx: &mut Ctx<'_>,
    licence: LicenceId,
    step_kind: RenewalPlayerStep,
) -> Result<RenewalPlayerOutcome, CmdError> {
    let s = service(ctx)?;
    step(ctx, &s, licence, Switches::of(step_kind))
}

/// The status branch's view of the actor's own licence (`own`'s term end and
/// the checked last operation), read exactly as `@mindrenewal/status` reads
/// it.
pub(super) fn status_view(
    term_end_ic: i64,
    row: Option<&MindRenewalOperationV1>,
) -> RenewalStatusView {
    let state = match row.map(|r| r.state()) {
        None => RenewalState::None,
        Some(RenewalStateV1::Drafted {}) => RenewalState::Drafted,
        Some(RenewalStateV1::Funded { .. }) => RenewalState::Funded,
        Some(RenewalStateV1::Granted { .. }) => RenewalState::Granted,
        Some(_) => RenewalState::Closed,
    };
    let offer = row.map(|row| RenewalOfferTerms {
        price_minor: row.offer().terms.price.minor(),
        instrument: row.offer().terms.instrument.to_string(),
        expires_ic: row.offer().terms.expires_at.at,
        term_seconds: row.binding().term_seconds(),
        grace_seconds: row.binding().grace_seconds(),
        registry_permille: row.offer().terms.registry.permille,
        venue_permille: row.offer().terms.venue.as_ref().map_or(0, |c| c.permille),
        parent_permille: row
            .offer()
            .terms
            .parents
            .iter()
            .map(|p| u32::from(p.permille))
            .sum::<u32>(),
    });
    RenewalStatusView::new(state, term_end_ic, offer)
}

/// The player branch, by the switches `sw` selects (the command's own, or
/// `player_step`'s): status first, then request, accept, decline and retry
/// in the command's original order.
fn step(
    ctx: &mut Ctx<'_>,
    s: &Service,
    licence: LicenceId,
    sw: Switches,
) -> Result<RenewalPlayerOutcome, CmdError> {
    let item = own(ctx, s, licence)?;
    let row = item
        .last_operation()
        .map(|op| checked(ctx, s, op))
        .transpose()?;
    if sw.status {
        return Ok(RenewalPlayerOutcome::Status(status_view(
            item.term_end().get(),
            row.as_ref(),
        )));
    }
    let (selection, t, parameters) = if sw.request && !item.attempted() {
        (
            Selection::Due(licence, item.generation(), item.holder_generation()),
            Transition::Draft,
            serde_json::json!({"licence":licence.get()}),
        )
    } else {
        let row = row.ok_or_else(unavailable)?;
        if sw.accept && matches!(row.state(), RenewalStateV1::Drafted {}) {
            (
                Selection::Pending(row.operation()),
                Transition::Fund,
                serde_json::json!({"operation":row.operation()}),
            )
        } else if sw.decline && matches!(row.state(), RenewalStateV1::Drafted {}) {
            (
                Selection::Pending(row.operation()),
                Transition::Close,
                serde_json::json!({"operation":row.operation(),"reason":"declined"}),
            )
        } else if sw.retry && row.state().retryable() {
            (
                Selection::Retry(licence, item.generation(), row.operation()),
                Transition::Draft,
                serde_json::json!({"previous_operation":row.operation()}),
            )
        } else {
            return Err(unavailable());
        }
    };
    let mut r = runtime(s)?.runtime.lock().map_err(|_| unavailable())?;
    if !r.enabled {
        return Err(unavailable());
    }
    admit(&mut r, selection, ctx.actor)?;
    let e = r.entries.get_mut(&key(selection)).ok_or_else(unavailable)?;
    if e.attempt.is_some() {
        return Ok(RenewalPlayerOutcome::Unresolved);
    }
    // 17g C2h: a step that sent nothing is `Unresolved`, never `Queued`.
    Ok(sent_outcome(submit(ctx, s, e, t, parameters)?))
}

/// 17g C2h (the lead's ruling R3, the C2e plan §5.1): the player front's
/// handle, when the service is composed and enabled. TacSim's `+minds/renew`
/// schedules it for the actor whose typed line it is dispatching (the
/// delegation this service declared at composition admits nothing else).
#[must_use]
pub fn player_front(ctx: &Ctx<'_>) -> Option<ContinuationHandle> {
    player_front_in(ctx.w)
}

/// `player_front`, read from the world itself (the service's state slot and
/// its runtime), so a composition test can prove the handle TacSim would be
/// handed after activation without a dispatched context.
#[must_use]
pub fn player_front_in(world: &World) -> Option<ContinuationHandle> {
    let s = world.module_state::<Service>()?;
    let enabled = runtime(s).ok()?.runtime.lock().ok()?.enabled;
    enabled.then(|| s.front.clone())
}

/// 17g C2h: the `front` arm, `<licence> request | accept <operation> |
/// decline <operation>` (`player::parse_front`; anything else is the uniform
/// refusal). It runs under this service's own identity, as a continuation
/// TacSim's `+minds/renew` scheduled or as a typed `COMMAND/front` line,
/// the same step for the same actor either way:
/// - `request`: the actor's own licence (`own`) and its checked last
///   operation, as `step(Status)` reads them; a drafted offer prints
///   `RENEW_OFFER` and `RENEW_HOW`, a funded one `RENEW_FUNDED`, and neither
///   submits; with none, or after a closure or grant, `RENEW_NOT_DUE` while
///   the term runs, else the request-or-retry step and `RENEWING`.
/// - `accept` and `decline`: the actor's own licence, whose last operation
///   must be exactly the named one and drafted (the payment is bound to the
///   offer the holder saw), then the step and `RENEW_PAYING` with the row's
///   frozen price, or `RENEW_DECLINING`.
///
/// Every line prints AFTER the step, and only when it submitted; any other
/// outcome is the uniform refusal.
fn front(ctx: &mut Ctx<'_>, s: &Service) -> Result<(), CmdError> {
    let (licence, kind) = parse_front(ctx.args.raw()).ok_or_else(unavailable)?;
    let shown = licence.get().to_string();
    let item = own(ctx, s, licence)?;
    let price_minor = match kind {
        FrontStep::Request => {
            let row = item
                .last_operation()
                .map(|op| checked(ctx, s, op))
                .transpose()?;
            let view = status_view(item.term_end().get(), row.as_ref());
            match front_request(&shown, &view, ctx.w.world_ic_now().at) {
                FrontAnswer::Say(lines) => {
                    let actor = ctx.actor;
                    for line in lines {
                        ctx.out.info(actor, line);
                    }
                    return Ok(());
                }
                FrontAnswer::Refuse(line) => return Err(CmdError::msg(line)),
                FrontAnswer::Ask => 0,
            }
        }
        FrontStep::Accept(operation) | FrontStep::Decline(operation) => {
            if !front_bound(kind, item.last_operation()) {
                return Err(unavailable());
            }
            let row = checked(ctx, s, operation)?;
            if !matches!(row.state(), RenewalStateV1::Drafted {}) {
                return Err(unavailable());
            }
            row.offer().terms.price.minor()
        }
    };
    let outcome = step(ctx, s, licence, Switches::of(front_player_step(kind)))?;
    let line =
        front_submitted(&outcome, front_line(&shown, kind, price_minor)).ok_or_else(unavailable)?;
    let actor = ctx.actor;
    ctx.out.info(actor, line);
    Ok(())
}

#[cfg(test)]
mod tests;
