use super::{
    IntegrationError as Error, config::CoreFactoryConfig, families::Families, output_policy::Policy,
};
use crate::core_production::{authority::Definitions, provider::Families as Origins};
use mush_core::core_output::{CoreOutputBinding, CoreOutputReservations};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError, OwnerId, Registrar,
};
use mush_inventory::core_input::{CoreInputHandle, RegisteredCoreInputs};
use std::collections::BTreeSet;
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) config: CoreFactoryConfig,
    pub(crate) families: Families,
    pub(crate) industry: crate::IndustryFamilies,
    pub(crate) origins: Origins,
    pub(crate) definitions: Definitions,
    pub(crate) input_handle: CoreInputHandle,
    pub(crate) inputs: RegisteredCoreInputs,
    pub(crate) outputs: CoreOutputReservations<Policy>,
    pub(crate) actions: [DefinitionRef; 7],
    pub(crate) outcome: DefinitionRef,
}
fn err(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("outcome").map_err(err)?,DefinitionId::new("core_integration_changed").map_err(err)?,1,serde_json::json!({"format":1,"fields":["operation","offer","order","object","state"],"source":false})).map_err(err)
}
pub(crate) fn output_binding(config: &CoreFactoryConfig) -> Result<CoreOutputBinding, ModError> {
    CoreOutputBinding::new(
        [
            config.action(super::INITIALIZE).map_err(err)?,
            config.action(super::RESERVE).map_err(err)?,
            config.action(super::COMPLETE).map_err(err)?,
        ],
        config.payment.recipe.clone(),
    )
}
impl Runtime {
    pub(crate) fn register(
        reg: &mut Registrar<'_>,
        config: CoreFactoryConfig,
        industry: crate::IndustryFamilies,
        origins: Origins,
        definitions: Definitions,
    ) -> Result<Self, ModError> {
        let actions = super::ACTIONS
            .iter()
            .map(|name| reg.definition(config.payment.action_descriptor(name).map_err(err)?))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| err(Error::Integrity))?;
        let outcome = reg.definition(outcome_descriptor()?)?;
        let families = Families::register(reg)?;
        let input_handle = industry
            .inventory
            .core_inputs()
            .cloned()
            .ok_or_else(|| err("core integration requires exact input reservations"))?;
        let inputs = input_handle.request(reg)?;
        let deps = ["industry", "economy", "org", "inventory"]
            .into_iter()
            .map(|s| OwnerId::new(s).map_err(err))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let outputs = CoreOutputReservations::register(
            reg,
            output_binding(&config)?,
            Policy {
                config: config.clone(),
                definitions: definitions.clone(),
                families: families.clone(),
                industry: industry.clone(),
            },
            deps,
        )?;
        Ok(Self {
            config,
            families,
            industry,
            origins,
            definitions,
            input_handle,
            inputs,
            outputs,
            actions,
            outcome,
        })
    }
    pub(crate) fn action(&self, name: &str) -> Result<&DefinitionRef, Error> {
        self.actions
            .iter()
            .find(|a| a.key().id.as_str() == name)
            .ok_or(Error::Integrity)
    }
}
