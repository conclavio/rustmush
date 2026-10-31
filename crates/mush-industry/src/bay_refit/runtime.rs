//! **The registered bay refit**: its families, definitions, owner
//! participant, kernel protocol, providers and schedule source, and its
//! boot load and audit.
use super::records::*;
use super::state::BayRefitState;
use super::{BayFamilies, BayRefitConfig, BayRefitError as Error};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError, ModuleBootData,
    ModuleCtx, Registrar, World, canonical_json_bytes_with_limit,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn bad<T>(_: T) -> Error {
    Error::Integrity
}
fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
/// The registered runtime, published as module state for composition seams
/// and tests.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct BayHandle(pub(crate) Arc<BayRuntime>);
pub(crate) struct BayRuntime {
    pub(crate) config: BayRefitConfig,
    pub(crate) industry: crate::IndustryFamilies,
    pub(crate) families: BayFamilies,
    pub(crate) actions: [DefinitionRef; 6],
    /// Install's two native images, Economy's and Tactics', in the kernel's order.
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) outcome: DefinitionRef,
}
impl BayRuntime {
    pub(crate) fn parts(&self) -> Result<&mush_inventory::bay_part::BayPartHandle, Error> {
        self.industry
            .inventory
            .bay_parts()
            .ok_or(Error::Unavailable)
    }
}
/// The outcome every bay action reports.
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("outcome").map_err(mod_error)?,
        DefinitionId::new("bay_refit_changed").map_err(mod_error)?,
        1,
        json!({"format":1,"fields":["operation","offer","order","state"],"source":false}),
    )
    .map_err(mod_error)
}
/// The six action descriptors, for Industry's one exact action validator.
pub(crate) fn action_descriptors(
    c: &BayRefitConfig,
) -> Result<Vec<DefinitionDescriptor>, ModError> {
    super::ACTIONS
        .iter()
        .map(|a| c.policy.action_descriptor(a).map_err(mod_error))
        .collect()
}
/// Register the bay refit. The six actions, the outcome and the owner
/// participant must already be admitted by Industry's exact validators.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: BayRefitConfig,
    industry: crate::IndustryFamilies,
) -> Result<Arc<BayRuntime>, ModError> {
    if industry.inventory.bay_parts().is_none() {
        return Err(ModError::msg(
            "bay refit requires Inventory's bay part custody (`with_bay_parts`)",
        ));
    }
    let families = BayFamilies::register(reg)?;
    let actions: [DefinitionRef; 6] = action_descriptors(&config)?
        .into_iter()
        .map(|d| reg.definition(d))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| ModError::msg("bay refit action count"))?;
    let outcome = reg.definition(outcome_descriptor()?)?;
    let participant = reg.definition(config.participant_descriptor().map_err(mod_error)?)?;
    if participant != config.participant().map_err(mod_error)? {
        return Err(ModError::msg("bay refit owner identity"));
    }
    let tactics = config.tactics_participant().map_err(mod_error)?;
    reg.state(BayRefitState::default());
    reg.owner_participant(
        participant.clone(),
        actions
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 4)
            .map(|(_, a)| a.clone())
            .collect(),
        super::owner::BayOwner {
            config: config.clone(),
        },
    )?;
    reg.bay_refit_capacity(
        [participant, config.economy_owner.clone(), tactics.clone()],
        [
            actions[2].clone(),
            actions[3].clone(),
            actions[4].clone(),
            actions[5].clone(),
        ],
    )?;
    let runtime = Arc::new(BayRuntime {
        participants: [config.economy_owner.clone(), tactics],
        config,
        industry,
        families,
        actions,
        outcome,
    });
    reg.state(BayHandle(runtime.clone()));
    let custody = Arc::new(BayCustody {
        hulls: runtime.families.hulls().clone(),
    });
    reg.hook(mush_core::HookPoint::CarryAttempt, custody.clone());
    reg.hook(mush_core::HookPoint::MoveAttempt, custody);
    super::provider::register(reg, runtime.clone())?;
    super::schedule::register(reg, runtime.clone())?;
    Ok(runtime)
}
/// The census key, if retained, at most once.
pub(crate) fn load_index(data: &ModuleBootData) -> Result<Option<BayIndex>, Error> {
    let mut rows = data.kv.iter().filter(|(k, _)| k.as_str() == INDEX_KEY);
    let row = rows
        .next()
        .map(|(_, v)| decode(v, INDEX_BYTES))
        .transpose()?;
    if rows.next().is_some() {
        return Err(Error::Integrity);
    }
    Ok(row)
}
impl BayRuntime {
    pub(crate) fn load(
        &self,
        ctx: &mut ModuleCtx<'_>,
        data: &ModuleBootData,
    ) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| mod_error("bay refit definitions unavailable"))?
            .clone();
        self.config
            .definitions
            .set(defs)
            .map_err(|_| mod_error("bay refit definitions already captured"))?;
        let index = load_index(data).map_err(mod_error)?;
        let state = ctx
            .w
            .module_state_mut::<BayRefitState>()
            .ok_or_else(|| mod_error("bay refit state unregistered"))?;
        *state = BayRefitState {
            ready: false,
            generation: 1,
            index,
            ..BayRefitState::default()
        };
        Ok(())
    }
    pub(crate) fn finalize(&self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let rebuilt = super::audit::audit(ctx, self).map_err(mod_error)?;
        let state = ctx
            .w
            .module_state_mut::<BayRefitState>()
            .ok_or_else(|| mod_error("bay refit state unregistered"))?;
        state.offers = rebuilt.offers;
        state.orders = rebuilt.orders;
        state.ready = true;
        Ok(())
    }
}
/// Admission-only proof that the order's complete future image fits: the
/// longest progress and terminal receipt and the actual installation.
pub(crate) fn terminal_room(world: &World, r: &BayRuntime, row: &BayOrder) -> Result<(), Error> {
    row.validate()?;
    if row.terminal.is_some() || row.installation.is_some() {
        return Err(Error::Integrity);
    }
    let o = &row.intent.offer;
    let current =
        mush_tactics::component_replacement::current_refit_unit(world, o.requester, o.hull)
            .map_err(|_| Error::Unavailable)?;
    let (_, journal) =
        super::config::refit(r.config.registry()?, &r.config, o, current, world.now())?;
    let refit = RefitEvidence::capture(&journal);
    refit.digest()?;
    let installation = json!({"refit":refit,"participants":r.participants,"at_utc":i64::MIN});
    canonical_json_bytes_with_limit(&installation, INSTALLATION_BYTES).map_err(bad)?;
    let mut projected = serde_json::to_value(row).map_err(bad)?;
    let object = projected.as_object_mut().ok_or(Error::Integrity)?;
    object.insert("progress".into(), Value::String("x".repeat(32768 - 2)));
    object.insert("terminal".into(), Value::String("x".repeat(16_384 - 2)));
    object.insert("installation".into(), installation);
    canonical_json_bytes_with_limit(&projected, ORDER_BYTES).map_err(bad)?;
    Ok(())
}

/// **A hull held in a bay stays in it** (doc 31 UD.4b): neither carried out
/// nor moved while an order holds it, so it can be neither launched nor taken
/// elsewhere until Install or Cancel releases it.
pub(crate) struct BayCustody {
    pub(crate) hulls: mush_core::StateFamilyHandle<mush_core::ObjId, super::BayHullHead>,
}
impl BayCustody {
    fn held(&self, ctx: &mush_core::HookReadContext<'_>, object: mush_core::ObjId) -> bool {
        // An unreadable head refuses: a hull's custody is never assumed free.
        ctx.action_state(&self.hulls, &object)
            .map_or(true, |h| h.is_some_and(|h| h.pending().is_some()))
    }
}
impl mush_core::EventHook for BayCustody {
    fn carry_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        _actor: mush_core::ObjId,
        object: mush_core::ObjId,
        _destination: mush_core::ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if self.held(ctx, object) {
            return Err(mush_proto::RichText::plain(
                "That hull is held in a bay for a refit.",
            ));
        }
        Ok(())
    }
    fn move_attempt(
        &self,
        ctx: &mush_core::HookReadContext<'_>,
        actor: mush_core::ObjId,
        _exit: Option<mush_core::ObjId>,
        _dest: mush_core::ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if self.held(ctx, actor) {
            return Err(mush_proto::RichText::plain(
                "That hull is held in a bay for a refit.",
            ));
        }
        Ok(())
    }
}
