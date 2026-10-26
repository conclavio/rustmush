use super::{CoreRefitFactoryConfig, Error, families::Families, policies::ExchangePolicy};
use mush_core::core_exchange::CoreComponentExchanges;
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError, OwnerId, Registrar,
};
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) config: CoreRefitFactoryConfig,
    pub(crate) industry: crate::IndustryFamilies,
    pub(crate) families: Families,
    pub(crate) manufacture: crate::core_production::integration::families::Families,
    pub(crate) actions: [DefinitionRef; 6],
    pub(crate) exchanges: CoreComponentExchanges<ExchangePolicy>,
    pub(crate) participants: [DefinitionRef; 2],
    pub(crate) outcome: DefinitionRef,
}
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(|e|ModError::msg(e.to_string()))?,DefinitionId::new("core_refit_changed").map_err(|e|ModError::msg(e.to_string()))?,1,serde_json::json!({"format":1,"fields":["operation","offer","order","state"],"source":false})).map_err(|e|ModError::msg(e.to_string()))
}
impl Runtime {
    pub(crate) fn register(
        reg: &mut Registrar<'_>,
        config: CoreRefitFactoryConfig,
        industry: crate::IndustryFamilies,
        economy_owner: DefinitionRef,
        manufacture: crate::core_production::integration::families::Families,
    ) -> Result<Self, ModError> {
        let err = |e: Error| ModError::msg(e.to_string());
        let actions = super::ACTIONS
            .iter()
            .map(|a| reg.definition(config.action_descriptor(a).map_err(err)?))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| err(Error::Integrity))?;
        let families = Families::register(reg)?;
        let outcome = reg.definition(outcome_descriptor()?)?;
        let participants = [economy_owner, config.tactics_owner.clone()];
        let dependencies = ["industry", "economy", "org", "tactics", "sim"]
            .into_iter()
            .map(|s| OwnerId::new(s).map_err(|e| ModError::msg(e.to_string())))
            .collect::<Result<_, _>>()?;
        let exchanges = CoreComponentExchanges::register(
            reg,
            config.action(super::INSTALL).map_err(err)?,
            participants.clone(),
            ExchangePolicy {
                config: config.clone(),
                families: families.clone(),
            },
            dependencies,
        )?;
        Ok(Self {
            config,
            industry,
            families,
            manufacture,
            actions,
            exchanges,
            participants,
            outcome,
        })
    }
}
