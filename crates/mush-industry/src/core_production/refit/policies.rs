//! Concrete trusted Tactics and Core policies over acknowledged work. Neither
//! client request parameters nor decoded records supply a writable unit image.
use super::{CoreRefitFactoryConfig, Error, authority, bad, families::Families, records::*};
use crate::core_production::{
    CoreOriginMode, authority::Definitions, participant::CoreProductionState,
};
use crate::workorder::WorkOrderId;
use mush_core::{
    DefinitionRegistry, ObjId, World,
    core_exchange::{CoreExchangePolicy, ExchangeFacts, ExchangeRequest},
    core_output::OutputRecheckContext,
    participant::{OwnerPrepareContext, ParticipantError},
    read_service::{OwnerReadContext, OwnerReadError},
};
use mush_tactics::{
    TacticalUnit, UnitMutationOp, UnitMutationPlan,
    component_replacement::{ReplacementChoice, ReplacementPolicy},
    unit::replacement::{ComponentState, ReplacementJournal},
};

pub(crate) fn replacement(
    world: &World,
    registry: &DefinitionRegistry,
    c: &CoreRefitFactoryConfig,
    intent: &AcceptedIntent,
) -> Result<(ReplacementChoice, ReplacementJournal), Error> {
    authority::install(world, c, intent)?;
    let o = &intent.offer;
    let current =
        mush_tactics::component_replacement::current_refit_unit(world, o.requester, o.hull)
            .map_err(bad)?;
    replacement_from(current, registry, c, intent, world.now())
}
fn replacement_from(
    current: &TacticalUnit,
    registry: &DefinitionRegistry,
    c: &CoreRefitFactoryConfig,
    intent: &AcceptedIntent,
    now: i64,
) -> Result<(ReplacementChoice, ReplacementJournal), Error> {
    let o = &intent.offer;
    if current.object() != o.hull
        || current.revision() != o.unit_revision
        || current.schema() != &o.before
        || SavedComponent::of_unit(current, &c.component)? != o.displaced
    {
        return Err(Error::Unavailable);
    }
    let target = c
        .payment
        .target_for(o.kind, &o.before, o.class())
        .map_err(bad)?;
    registry.get_exact(target).map_err(bad)?;
    let spec = mush_tactics::unit::replacement::registered_spec(registry, target).map_err(bad)?;
    let baseline = TacticalUnit::from_spec(&spec, o.hull).map_err(bad)?;
    let asset = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentAsset {
            component: c.component.clone(),
            asset: Some(o.incoming),
        }],
        Some(baseline.revision()),
        None,
    )
    .map_err(bad)?;
    let (with_asset, _) = mush_tactics::apply(&baseline, &asset, now).map_err(bad)?;
    let incoming = ComponentState::capture(&with_asset, &c.component).map_err(bad)?;
    let (_, journal) = mush_tactics::unit::replacement::replace_component(
        registry,
        current,
        target,
        &c.component,
        &incoming,
        o.unit_revision,
        now,
    )
    .map_err(bad)?;
    Ok((ReplacementChoice::new(target.clone(), incoming), journal))
}
fn state(world: &World) -> Result<&CoreProductionState, Error> {
    if world
        .module_state_owner(std::any::TypeId::of::<CoreProductionState>())
        .is_none_or(|o| o.as_str() != crate::NS)
    {
        return Err(Error::Unavailable);
    }
    world
        .module_state::<CoreProductionState>()
        .filter(|s| s.ready)
        .ok_or(Error::Unavailable)
}
fn ready<'a>(
    world: &World,
    state: &'a CoreProductionState,
    c: &CoreRefitFactoryConfig,
    id: WorkOrderId,
    actor: ObjId,
) -> Result<&'a RefitOrder, Error> {
    if c.mode != CoreOriginMode::Active {
        return Err(Error::Unavailable);
    }
    authority::cleanup(world, c)?;
    let p = state
        .refit
        .as_ref()
        .filter(|p| p.ready && p.index.is_some())
        .ok_or(Error::Unavailable)?;
    let row = p.orders.get(&id).ok_or(Error::Unavailable)?;
    row.validate()?;
    let native = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(Error::Unavailable)?
        .core_refit_history(row.accepted)
        .map_err(bad)?;
    // Install updates only typed history and E4+Tactics. The independent owner
    // work projection deliberately remains an immutable acknowledgement image.
    if row.intent.offer.requester != actor
        || native.subject() != &row.intent.subject()?
        || native.funding() != &row.funding
        || native.terminal().is_some()
        || !row.progress.production().is_complete()
    {
        return Err(Error::Unavailable);
    }
    authority::install(world, c, &row.intent)?;
    Ok(row)
}
/// Only trusted module composition can construct this concrete policy. Its
/// request is a local order identity, never incoming condition or unit bytes.
#[derive(Clone)]
pub struct RefitReplacementPolicy {
    pub(crate) config: CoreRefitFactoryConfig,
    pub(crate) definitions: Definitions,
}
impl ReplacementPolicy for RefitReplacementPolicy {
    type Request = WorkOrderId;
    fn authorize(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        id: &WorkOrderId,
        current: &TacticalUnit,
    ) -> Result<ReplacementChoice, ParticipantError> {
        let refused = || ParticipantError::Authority;
        if ctx.action() != &self.config.action(super::INSTALL).map_err(|_| refused())?
            || ctx.invocation().principal()
                != &(mush_core::PrincipalId::Object {
                    id: ctx.invocation().actor(),
                })
            || ctx.invocation().subject() != &mush_core::PerceivedRef::SelfActor
            || !ctx.invocation().targets().is_empty()
        {
            return Err(refused());
        }
        let s = state(ctx.world()).map_err(|_| refused())?;
        let row = ready(ctx.world(), s, &self.config, *id, ctx.invocation().actor())
            .map_err(|_| refused())?;
        let definitions = self.definitions.0.get().ok_or_else(refused)?;
        replacement_from(
            current,
            definitions,
            &self.config,
            &row.intent,
            ctx.world().now(),
        )
        .map(|v| v.0)
        .map_err(|_| refused())
    }
}
#[derive(Clone)]
pub(crate) struct ExchangePolicy {
    pub(crate) config: CoreRefitFactoryConfig,
    pub(crate) families: Families,
}
impl ExchangePolicy {
    fn facts(
        &self,
        state: &CoreProductionState,
        world: &World,
        actor: ObjId,
        id: WorkOrderId,
    ) -> Result<ExchangeFacts, OwnerReadError> {
        let row = ready(world, state, &self.config, id, actor)
            .map_err(|_| OwnerReadError::Unavailable)?;
        let o = &row.intent.offer;
        ExchangeFacts::new(
            state.generation,
            id.0,
            actor,
            o.hull,
            o.unit_revision,
            o.incoming,
            actor,
            o.displaced_asset().map(|a| (a, actor)),
        )
        .map_err(|_| OwnerReadError::Integrity)
    }
}
impl CoreExchangePolicy for ExchangePolicy {
    type State = CoreProductionState;
    fn generation(&self, s: &Self::State) -> u64 {
        s.generation
    }
    fn read(
        &self,
        s: &Self::State,
        ctx: &OwnerReadContext<'_>,
        r: &ExchangeRequest,
    ) -> Result<ExchangeFacts, OwnerReadError> {
        let id = WorkOrderId(r.order());
        let facts = self.facts(s, ctx.world(), ctx.actor(), id)?;
        let row = ctx
            .action_state(&self.families.orders, &id)?
            .ok_or(OwnerReadError::Integrity)?;
        if row.terminal.is_some()
            || s.refit
                .as_ref()
                .and_then(|p| p.orders.get(&id))
                .is_none_or(|v| v.as_ref() != row)
        {
            return Err(OwnerReadError::Integrity);
        }
        let a = ctx
            .action_state(&self.families.assets, &facts.incoming())?
            .ok_or(OwnerReadError::Integrity)?;
        let h = ctx
            .action_state(&self.families.hulls, &facts.hull())?
            .ok_or(OwnerReadError::Integrity)?;
        if a.position != (AssetPosition::Pending { order: id })
            || h.pending != Some(id)
            || h.mounted != facts.displaced().map(|x| x.0)
        {
            return Err(OwnerReadError::Integrity);
        }
        if let Some((old, _)) = facts.displaced() {
            let a = ctx
                .action_state(&self.families.assets, &old)?
                .ok_or(OwnerReadError::Integrity)?;
            if !matches!(a.position,AssetPosition::Mounted{hull,..}if hull==facts.hull()) {
                return Err(OwnerReadError::Integrity);
            }
        }
        Ok(facts)
    }
    fn recheck(
        &self,
        s: &Self::State,
        ctx: &OutputRecheckContext<'_>,
        r: &ExchangeRequest,
        f: &ExchangeFacts,
    ) -> Result<(), OwnerReadError> {
        if ctx.action()
            != &self
                .config
                .action(super::INSTALL)
                .map_err(|_| OwnerReadError::Integrity)?
            || &self.facts(s, ctx.world(), ctx.actor(), WorkOrderId(r.order()))? != f
        {
            return Err(OwnerReadError::Unavailable);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct CustodyHook {
    pub(crate) config: CoreRefitFactoryConfig,
    pub(crate) families: Families,
    pub(crate) participants: [mush_core::DefinitionRef; 2],
}
impl CustodyHook {
    fn check(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), Error> {
        let a = ctx
            .action_state(&self.families.assets, &object)
            .map_err(bad)?;
        let h = ctx
            .action_state(&self.families.hulls, &object)
            .map_err(bad)?;
        if h.is_some_and(|v| v.pending.is_some()) {
            return Err(Error::Unavailable);
        }
        if a.is_none_or(|v| {
            matches!(
                v.position,
                AssetPosition::Released { .. } | AssetPosition::Detached { .. }
            )
        }) {
            return Ok(());
        }
        let exact = ctx.core_exchange_attempt().ok_or(Error::Unavailable)?;
        let id = WorkOrderId(exact.order());
        let row = ctx
            .action_state(&self.families.orders, &id)
            .map_err(bad)?
            .ok_or(Error::Integrity)?;
        let o = &row.intent.offer;
        let s = state(ctx.world())?;
        ready(ctx.world(), s, &self.config, id, actor)?;
        if row.terminal.is_some()
            || exact.requester() != actor
            || actor != o.requester
            || exact.action() != &self.config.action(super::INSTALL)?
            || exact.participants() != &self.participants
            || exact.commit() == row.accepted
            || exact.hull() != o.hull
            || exact.unit_revision() != o.unit_revision
            || exact.incoming() != o.incoming
            || exact.source() != actor
            || exact.displaced() != o.displaced_asset().map(|v| (v, actor))
            || !((object == o.incoming && destination == o.hull)
                || (Some(object) == o.displaced_asset() && destination == actor))
        {
            return Err(Error::Unavailable);
        }
        let pending = ctx
            .action_state(&self.families.hulls, &o.hull)
            .map_err(bad)?
            .ok_or(Error::Integrity)?;
        if pending.pending != Some(id) || pending.mounted != o.displaced_asset() {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
impl mush_core::EventHook for CustodyHook {
    fn carry_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        self.check(ctx, actor, object, destination).map_err(|_| {
            mush_proto::RichText::plain(
                "That core is retained by a fitting or installed in a hull.",
            )
        })
    }
    fn move_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        self.check(ctx, actor, actor, dest).map_err(|_| {
            mush_proto::RichText::plain(
                "That core is retained by a fitting or installed in a hull.",
            )
        })
    }
}
