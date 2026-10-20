//! Explicit shipped hardware host composition. These are configuration values,
//! never certified stock, money, readiness, a title or a fitting capability.
use super::{HardwareProcess, PublishedHardware};
use crate::{TacsimRuleset, config::LoadedPack, definitions::core_replacement::GovernedLineCores};
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ModError, ObjId,
    OwnerId, Registrar,
};
use mush_economy::{
    AccountId, CatalogRef, EconomyModule, InstrumentId, VaultId,
    core_integration::{CoreDeliveryPromise, CorePaymentPolicy, ScopedCoreAccount},
    core_refit::RefitPaymentPolicy,
    registry_filing::InitialFilingPolicy,
};
use mush_industry::{
    IndustryModule,
    core_production::{
        CoreOriginConfig, CoreOriginMode, OriginAgent, OriginPolicy,
        integration::CoreFactoryConfig, origin_content_descriptor, refit::CoreRefitFactoryConfig,
    },
};
use mush_inventory::{Grade, InventoryHandleCell, InventoryModule};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use mush_tactics::{ComponentId, TacticsModule};
use std::{collections::BTreeSet, sync::Arc};

fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Existing launch-site identities. Prices, recipes, quota sizes, fitting roots
/// and delivery terms are deliberately derived from shipped content instead.
#[derive(Clone, Debug)]
pub struct HardwareDeployment {
    pub importer: OrgId,
    pub stock_scope: OrgScopeId,
    pub depot: VaultId,
    pub agents: [OriginAgent; 2],
    pub grades: [Grade; 3],
    pub supplier_account: AccountId,
    pub integrator: ScopedCoreAccount,
    pub issuer: SystemPrincipalId,
    pub factory: ObjId,
    pub output_container: ObjId,
}

#[derive(Clone)]
pub(crate) struct Registration {
    pub(crate) family: GovernedLineCores,
    content: DefinitionDescriptor,
}
impl Registration {
    pub(crate) fn register(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("content").map_err(bad)?,
            crate::definitions::exact_validator(vec![self.content.clone()]),
        )?;
        reg.definition(self.content.clone())?;
        Ok(())
    }
}

/// One exact policy shared by the five real owners. No ordinary server
/// constructor creates this object or supplies default deployment identities.
#[derive(Clone)]
pub struct GovernedHardware {
    state: Box<GovernedHardwareState>,
}

#[derive(Clone)]
struct GovernedHardwareState {
    publication: PublishedHardware,
    registration: Registration,
    origin: CoreOriginConfig,
    factory: CoreFactoryConfig,
    refit: CoreRefitFactoryConfig,
}

/// Configured modules returned by the explicit host entry. Add the host's real
/// Org, Sim and Roll modules and persist Initialize actions before Active boot.
/// Caller-specific sensor/IC options can be applied to `tactics` afterwards.
pub struct HardwareOwners {
    pub tacsim: TacsimRuleset,
    pub inventory: InventoryModule,
    pub industry: IndustryModule,
    pub economy: EconomyModule,
    pub tactics: TacticsModule,
}

/// Prevalidated configuration used by the shared server roster. Building it can
/// fail; applying it to the ordinary owner modules is infallible and makes no
/// state. This keeps the ordinary roster's API and owner order unchanged.
pub(crate) struct RosterPolicy {
    state: Box<RosterPolicyState>,
}

struct RosterPolicyState {
    hardware: GovernedHardware,
    input: mush_inventory::core_input::CoreInputBinding,
    economy: mush_economy::core_integration::CoreIntegrationConfig,
    fitting: mush_economy::core_refit::CoreRefitConfig,
    install: mush_core::DefinitionRef,
}
impl RosterPolicy {
    pub(crate) fn inventory(&self, module: InventoryModule) -> InventoryModule {
        module.with_core_inputs(self.state.input.clone())
    }
    pub(crate) fn industry(&self, module: IndustryModule) -> IndustryModule {
        self.state.hardware.configure_industry(module)
    }
    pub(crate) fn economy(&self, module: EconomyModule) -> EconomyModule {
        module
            .with_core_integration(self.state.economy.clone())
            .with_core_refit(self.state.fitting.clone())
    }
    pub(crate) fn tactics(&self) -> TacticsModule {
        TacticsModule::with_catalog(
            self.state
                .hardware
                .state
                .registration
                .family
                .catalog()
                .units
                .clone(),
        )
        .with_component_replacement(
            self.state.hardware.state.refit.replacement_policy(),
            self.state.install.clone(),
            self.state.hardware.state.refit.component().clone(),
            self.state
                .hardware
                .state
                .refit
                .payment()
                .fitted_units
                .iter()
                .cloned()
                .collect(),
        )
    }
}
impl GovernedHardware {
    pub(crate) fn roster_policy(&self) -> Result<RosterPolicy, ModError> {
        Ok(RosterPolicy {
            state: Box::new(RosterPolicyState {
                hardware: self.clone(),
                input: mush_inventory::core_input::CoreInputBinding::new(
                    self.state
                        .factory
                        .action("accept_core_integration")
                        .map_err(bad)?,
                    self.state
                        .factory
                        .action("complete_core_integration")
                        .map_err(bad)?,
                    self.state
                        .factory
                        .action("cancel_core_integration")
                        .map_err(bad)?,
                )
                .map_err(bad)?,
                economy: self
                    .state
                    .factory
                    .economy_config(self.state.origin.policy())
                    .map_err(bad)?,
                fitting: self
                    .state
                    .factory
                    .economy_refit_config(self.state.origin.policy())
                    .map_err(bad)?,
                install: self
                    .state
                    .refit
                    .action(mush_economy::core_refit::INSTALL_ACTION)
                    .map_err(bad)?,
            }),
        })
    }
    pub fn prepare(
        layer: &TacsimRuleset,
        mode: CoreOriginMode,
        deployment: HardwareDeployment,
        filing: &InitialFilingPolicy,
    ) -> Result<Self, ModError> {
        if !layer.config().enabled || layer.governed_hardware.is_some() {
            return Err(bad(
                "governed hardware requires one explicitly enabled composition",
            ));
        }
        if deployment.grades[0] == deployment.grades[1]
            || deployment.grades[0] == deployment.grades[2]
            || deployment.grades[1] == deployment.grades[2]
            || deployment.issuer != *filing.issuer()
        {
            return Err(bad(
                "hardware grade identities or actual native issuer disagree",
            ));
        }
        let publication = PublishedHardware::shipped().map_err(bad)?;
        let catalog = crate::definitions::Catalog::build_fielded(&layer.loaded_packs()?, crate::NS)
            .map_err(bad)?;
        let family = GovernedLineCores::build(catalog).map_err(bad)?;
        let content = origin_content_descriptor(deployment.grades)?;
        let mut defs = DefinitionRegistryBuilder::new();
        let content_ref = defs
            .register(OwnerId::new(crate::NS).map_err(bad)?, content.clone())
            .map_err(bad)?;
        let origin = CoreOriginConfig::new(
            OriginPolicy {
                importer: deployment.importer,
                scope: deployment.stock_scope,
                depot: deployment.depot,
                agents: deployment.agents,
                content: content_ref,
                grades: deployment.grades,
                monthly_quotas: publication.catalogue().monthly_dies(),
            },
            mode,
        )?;
        let economy = defs
            .register(
                OwnerId::new("economy").map_err(bad)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(bad)?,
                    DefinitionId::new("initial_filing").map_err(bad)?,
                    1,
                    filing.definition_payload(),
                )
                .map_err(bad)?,
            )
            .map_err(bad)?;
        let payment = CorePaymentPolicy {
            issuer: deployment.issuer.clone(),
            instrument: InstrumentId::new(crate::economy::CREDIT)
                .ok_or_else(|| bad("credit identity"))?,
            supplier: ScopedCoreAccount {
                account: deployment.supplier_account,
                organization: deployment.importer,
                scope: deployment.stock_scope,
            },
            integrator: deployment.integrator.clone(),
            supplier_stock: origin.policy().agents.each_ref().map(|a| a.stock),
            factory: deployment.factory,
            output_container: deployment.output_container,
            recipe: publication.definition(HardwareProcess::Integrate).clone(),
            catalogs: [
                "tacsim.core.doctrine",
                "tacsim.core.line",
                "tacsim.core.command",
            ]
            .map(|code| CatalogRef {
                ruleset: crate::NS.into(),
                code: code.into(),
            }),
            delivery: Some(CoreDeliveryPromise::PersonalCustodyV1),
        };
        let core_classes = [
            crate::arena::core::CoreClass::I,
            crate::arena::core::CoreClass::II,
            crate::arena::core::CoreClass::III,
        ];
        let fitted_units = core_classes
            .map(|class| family.definition(class).cloned())
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad("finite governed family"))?
            .try_into()
            .map_err(|_| bad("three fitted roots"))?;
        let fitting = RefitPaymentPolicy {
            issuer: deployment.issuer,
            instrument: payment.instrument.clone(),
            provider: deployment.integrator,
            factory: deployment.factory,
            recipe: publication.definition(HardwareProcess::Refit).clone(),
            fitted_units,
        };
        let component = ComponentId::new("core").map_err(bad)?;
        let roots = fitting
            .fitted_units
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let action = fitting
            .action_ref(mush_economy::core_refit::INSTALL_ACTION)
            .map_err(bad)?;
        let tactics = defs
            .register(
                OwnerId::new("tactics").map_err(bad)?,
                mush_tactics::component_replacement::participant_descriptor(
                    &action, &component, &roots,
                )?,
            )
            .map_err(bad)?;
        let refit = CoreRefitFactoryConfig::new(
            mode,
            fitting,
            component,
            Arc::new(publication.clone()),
            tactics,
        )
        .map_err(bad)?;
        let factory = CoreFactoryConfig::new(mode, payment, Arc::new(publication.clone()), economy)
            .map_err(bad)?
            .with_refits(refit.clone())
            .map_err(bad)?;
        Ok(Self {
            state: Box::new(GovernedHardwareState {
                publication,
                registration: Registration { family, content },
                origin,
                factory,
                refit,
            }),
        })
    }

    pub fn origin_policy(&self) -> &OriginPolicy {
        self.state.origin.policy()
    }
    pub fn payment_policy(&self) -> &CorePaymentPolicy {
        self.state.factory.payment()
    }
    pub fn fitting_policy(&self) -> &RefitPaymentPolicy {
        self.state.refit.payment()
    }
    pub fn family(&self) -> &GovernedLineCores {
        &self.state.registration.family
    }

    /// Uses the normal acknowledged pack-load route. Preparing composition alone
    /// does not create a publication receipt or authorize either recipe callback.
    pub fn configure_tacsim(&self, mut layer: TacsimRuleset) -> Result<TacsimRuleset, ModError> {
        if !layer.config().enabled || layer.governed_hardware.is_some() {
            return Err(bad(
                "governed hardware requires one explicitly enabled composition",
            ));
        }
        let family = GovernedLineCores::build(
            crate::definitions::Catalog::build_fielded(&layer.loaded_packs()?, crate::NS)
                .map_err(bad)?,
        )
        .map_err(bad)?;
        if family.catalog().descriptors != self.state.registration.family.catalog().descriptors {
            return Err(bad("hardware composition catalogue changed"));
        }
        let packs = layer.loaded.as_mut().map_err(|e| bad(e.as_str()))?;
        let manifest = self.state.publication.pack().manifest();
        match packs.iter().find(|p| {
            p.pack.manifest().id() == manifest.id()
                && p.pack.manifest().version() == manifest.version()
        }) {
            Some(p) if &p.pack != self.state.publication.pack() => {
                return Err(bad("shipped hardware publication changed"));
            }
            Some(_) => {}
            None => packs.push(LoadedPack {
                source: "shipped governed hardware".into(),
                pack: self.state.publication.pack().clone(),
            }),
        }
        layer.governed_hardware = Some(Box::new(self.state.registration.clone()));
        Ok(layer)
    }
    pub fn configure_inventory(
        &self,
        module: InventoryModule,
    ) -> Result<InventoryModule, ModError> {
        Ok(module.with_core_inputs(
            mush_inventory::core_input::CoreInputBinding::new(
                self.state
                    .factory
                    .action("accept_core_integration")
                    .map_err(bad)?,
                self.state
                    .factory
                    .action("complete_core_integration")
                    .map_err(bad)?,
                self.state
                    .factory
                    .action("cancel_core_integration")
                    .map_err(bad)?,
            )
            .map_err(bad)?,
        ))
    }
    pub fn configure_industry(&self, module: IndustryModule) -> IndustryModule {
        module
            .with_core_origins(self.state.origin.clone())
            .with_core_factory(self.state.factory.clone())
    }
    /// Preserves the caller's already configured initial filing owner and all
    /// other native profiles. Registration requires the exact captured owner.
    pub fn configure_economy(&self, module: EconomyModule) -> Result<EconomyModule, ModError> {
        Ok(module
            .with_core_integration(
                self.state
                    .factory
                    .economy_config(self.state.origin.policy())
                    .map_err(bad)?,
            )
            .with_core_refit(
                self.state
                    .factory
                    .economy_refit_config(self.state.origin.policy())
                    .map_err(bad)?,
            ))
    }
    pub fn tactics(&self) -> Result<TacticsModule, ModError> {
        Ok(
            TacticsModule::with_catalog(self.state.registration.family.catalog().units.clone())
                .with_component_replacement(
                    self.state.refit.replacement_policy(),
                    self.state
                        .refit
                        .action(mush_economy::core_refit::INSTALL_ACTION)
                        .map_err(bad)?,
                    self.state.refit.component().clone(),
                    self.state
                        .refit
                        .payment()
                        .fitted_units
                        .iter()
                        .cloned()
                        .collect(),
                ),
        )
    }
    /// Executable opt-in host entry. Keep the returned owners together with the
    /// host's real Org/Sim/Roll modules; no ordinary server roster calls this.
    pub fn compose(
        &self,
        layer: TacsimRuleset,
        economy: EconomyModule,
        inventory: InventoryModule,
        industry: IndustryModule,
        inventory_handles: InventoryHandleCell,
    ) -> Result<HardwareOwners, ModError> {
        Ok(HardwareOwners {
            tacsim: self.configure_tacsim(layer.with_inventory(inventory_handles.clone()))?,
            economy: self.configure_economy(economy)?,
            inventory: self
                .configure_inventory(inventory.with_handle_cell(inventory_handles.clone()))?,
            industry: self.configure_industry(industry.with_inventory(inventory_handles)),
            tactics: self.tactics()?,
        })
    }
}
