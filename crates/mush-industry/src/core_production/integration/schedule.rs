//! The ordinary K driver is the only progress ingress. It re-plans the actual
//! stored cursor and never completes work by a caller-supplied elapsed duration.
use super::{authority, provider::AdvanceSelector, runtime::Runtime};
use mush_core::{
    ActionError, ActionErrorCode, CanonicalId, DefinitionRef, Engine, ModError, PerceivedRef,
    Registrar, RevisionSet, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance,
    ScheduleDriveCause, ScheduleEvaluation,
};
#[derive(Clone)]
struct Adapter(Runtime);
fn stale() -> ActionError {
    ActionError::new(
        ActionErrorCode::Stale,
        false,
        "Core production schedule is no longer current.",
    )
    .expect("fixed bounded schedule error")
}
impl ScheduleActionAdapter for Adapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let r = &self.0;
        if action != &r.actions[4]
            || r.config.mode != crate::core_production::CoreOriginMode::Active
            || advance.observed_at() != &engine.world.world_ic_now()
            || authority::cleanup(&engine.world, &r.definitions).is_err()
        {
            return Err(stale());
        }
        let store = engine.action_state();
        let cursor = store
            .get(&r.families.schedules, &advance.schedule())
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        if !matches!(cursor.plan_due(cursor.revision(),advance.observed_at(),advance.cause()), Ok(ScheduleEvaluation::Advance(actual)) if actual.as_ref()==advance)
        {
            return Err(stale());
        }
        let order_id = store
            .get(&r.families.jobs, &advance.schedule())
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        let order = store
            .get(&r.families.orders, order_id)
            .map_err(|_| stale())?
            .ok_or_else(stale)?;
        let projection = engine
            .world
            .module_state::<crate::core_production::participant::CoreProductionState>()
            .filter(|s| s.ready)
            .and_then(|s| s.integration.as_ref())
            .filter(|p| p.ready)
            .ok_or_else(stale)?;
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
                .is_none_or(|v| v.as_ref() != &super::state::OrderView::capture(order))
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
            actor: order.intent.requester,
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
pub(crate) fn register(reg: &mut Registrar<'_>, runtime: Runtime) -> Result<(), ModError> {
    reg.schedule_source(
        CanonicalId::new("core.production").map_err(|e| ModError::msg(e.to_string()))?,
        runtime.actions[4].clone(),
        runtime.families.schedules.clone(),
        Adapter(runtime),
    )
}
