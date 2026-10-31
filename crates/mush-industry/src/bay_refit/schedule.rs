//! The ordinary K driver is the only progress ingress. It re-plans the actual
//! stored cursor and never completes work by a caller-supplied elapsed time.
use super::provider::AdvanceSelector;
use super::runtime::BayRuntime;
use super::{CLOCK_POLICY, authority};
use mush_core::{
    ActionError, ActionErrorCode, CanonicalId, DefinitionRef, Engine, ModError, PerceivedRef,
    Registrar, RevisionSet, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance,
    ScheduleDriveCause, ScheduleEvaluation,
};
use std::sync::Arc;
#[derive(Clone)]
struct Adapter {
    runtime: Arc<BayRuntime>,
    refusal: ActionError,
}
impl ScheduleActionAdapter for Adapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let r = &self.runtime;
        let stale = || self.refusal.clone();
        if action != &r.actions[3]
            || advance.observed_at() != &engine.world.world_ic_now()
            || authority::cleanup(&engine.world, &r.config).is_err()
        {
            return Err(stale());
        }
        let store = engine.action_state();
        let cursor = store
            .get(r.families.schedules(), &advance.schedule())
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        if !matches!(cursor.plan_due(cursor.revision(), advance.observed_at(), advance.cause()), Ok(ScheduleEvaluation::Advance(actual)) if actual.as_ref() == advance)
        {
            return Err(stale());
        }
        let order_id = store
            .get(r.families.jobs(), &advance.schedule())
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        let order = store
            .get(&r.families.orders, order_id)
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        let projection = super::config::state(&engine.world).map_err(|_| stale())?;
        if order.terminal.is_some()
            || order.progress.production().is_complete()
            || order.progress.cursor() != cursor
            || advance.skipped().is_some()
            || advance.occurrences().is_empty()
            || advance.observed_at().clock != order.intent.accepted_at.clock
            || advance.observed_at().at >= order.intent.expires_at.at
            || projection
                .orders
                .get(order_id)
                .is_none_or(|v| v.as_ref() != order)
        {
            return Err(stale());
        }
        let defs = engine.registry.definitions().ok_or_else(stale)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| stale())?;
        guards
            .add_definition(action.clone(), defs)
            .map_err(|_| stale())?;
        Ok(ScheduleActionDraft {
            actor: order.intent.offer.requester,
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: serde_json::to_value(AdvanceSelector {
                order: *order_id,
                observed: advance.observed_at().clone(),
                restart: matches!(advance.cause(), ScheduleDriveCause::Restart),
            })
            .map_err(|_| stale())?,
            guards: guards.finish().map_err(|_| stale())?,
        })
    }
}
pub(crate) fn register(reg: &mut Registrar<'_>, runtime: Arc<BayRuntime>) -> Result<(), ModError> {
    reg.schedule_source(
        CanonicalId::new(CLOCK_POLICY).map_err(|e| ModError::msg(e.to_string()))?,
        runtime.actions[3].clone(),
        runtime.families.schedules().clone(),
        Adapter {
            runtime,
            refusal: ActionError::new(
                ActionErrorCode::Stale,
                false,
                "The bay refit schedule is no longer current.",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        },
    )
}
